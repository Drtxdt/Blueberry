//! Persistent system-module metadata, never session functions or aliases.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Cache {
    pub path: PathBuf,
    pub identity: String,
    _lock: fs::File,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn executable_version(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    };
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut()) };
    if size == 0 || size > 4 * 1024 * 1024 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
        return None;
    }
    let mut value = std::ptr::null_mut();
    let mut length = 0;
    if unsafe {
        VerQueryValueW(
            data.as_ptr().cast(),
            [92u16, 0].as_ptr(),
            &mut value,
            &mut length,
        )
    } == 0
        || value.is_null()
        || (length as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
    {
        return None;
    }
    let offset = (value as usize).checked_sub(data.as_ptr() as usize)?;
    if offset.checked_add(std::mem::size_of::<VS_FIXEDFILEINFO>())? > data.len() {
        return None;
    }
    let version = unsafe { std::ptr::read_unaligned(value.cast::<VS_FIXEDFILEINFO>()) };
    if version.dwSignature != 0xfeef04bd {
        return None;
    }
    Some(format!(
        "{}.{}.{}.{}",
        version.dwProductVersionMS >> 16,
        version.dwProductVersionMS & 65535,
        version.dwProductVersionLS >> 16,
        version.dwProductVersionLS & 65535
    ))
}
fn stamp(path: &Path, entries: &mut Vec<String>, depth: usize) -> Result<()> {
    ensure!(
        depth < 32 && entries.len() < 100_000,
        "module inventory limit"
    );
    let metadata = fs::symlink_metadata(path)?;
    // Reparse targets can change independently of the apparent inventory.
    use std::os::windows::fs::MetadataExt;
    ensure!(
        metadata.file_attributes() & 0x400 == 0,
        "module inventory contains reparse point"
    );
    entries.push(format!(
        "{}:{}:{}:{}",
        path.display(),
        metadata.file_size(),
        metadata.creation_time(),
        metadata.last_write_time()
    ));
    if metadata.is_dir() {
        let mut children = fs::read_dir(path)?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        children.sort();
        for child in children {
            stamp(&child, entries, depth + 1)?;
        }
    }
    Ok(())
}
pub fn identity(shell: &Path, script: &str) -> Result<String> {
    let shell = if shell.is_file() {
        shell.to_path_buf()
    } else {
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .map(|p| p.join(shell))
            .find(|p| p.is_file())
            .context("resolve metadata shell")?
    };
    let shell = fs::canonicalize(shell)?;
    let home = shell.parent().context("shell parent")?;
    let mut entries = vec!["system-metadata-v1".into(), hash(script.as_bytes())];
    stamp(&shell, &mut entries, 0)?;
    entries.push(format!("shell-version:{:?}", executable_version(&shell)));
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let file = fs::File::open(&shell)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    ensure!(
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0,
        "shell file identity unavailable"
    );
    entries.push(format!(
        "file-id:{}:{}:{}",
        info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
    ));
    // Bind the versioned engine as well as its launcher (PS7 launcher can be
    // unchanged across servicing updates).
    for name in [
        "System.Management.Automation.dll",
        "powershell.dll",
        "pwsh.dll",
        "powershell.runtimeconfig.json",
        "pwsh.runtimeconfig.json",
    ] {
        let path = home.join(name);
        if path.is_file() {
            stamp(&path, &mut entries, 0)?;
        }
    }
    let mut roots = vec![home.join("Modules")];
    if let Some(windows) = std::env::var_os("windir") {
        let root = PathBuf::from(windows).join("System32/WindowsPowerShell/v1.0/Modules");
        if crate::pty::module_search_paths(&shell)
            .iter()
            .any(|p| fs::canonicalize(p).ok() == fs::canonicalize(&root).ok())
            && !roots.contains(&root)
        {
            roots.push(root);
        }
    }
    for root in roots {
        if root.is_dir() {
            stamp(&root, &mut entries, 0)?;
        } else {
            entries.push(format!("missing:{}", root.display()));
        }
    }
    Ok(hash(serde_json::to_string(&entries)?.as_bytes()))
}
impl Cache {
    pub fn open(identity: String, cancelled: impl Fn() -> bool) -> Result<Self> {
        let root =
            PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA unavailable")?)
                .join("Blueberry/cache/system-commands");
        Self::open_at(&root, identity, cancelled)
    }
    fn open_at(root: &Path, identity: String, cancelled: impl Fn() -> bool) -> Result<Self> {
        fs::create_dir_all(root)?;
        let path = root.join(format!("{identity}.json"));
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        let start = std::time::Instant::now();
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => {
                    ensure!(!cancelled(), "metadata cache cancelled");
                    ensure!(
                        start.elapsed().as_secs() < 35,
                        "metadata cache lock timed out"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Self {
            path,
            identity,
            _lock: lock,
        })
    }
    pub fn read(&self, newer_than: Option<SystemTime>) -> Option<String> {
        let file = fs::File::open(&self.path).ok()?;
        let mut bytes = Vec::new();
        file.take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > 8 * 1024 * 1024 {
            return None;
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let output = value["output"].as_str()?;
        if value["identity"] != self.identity || value["sha256"] != hash(output.as_bytes()) {
            return None;
        }
        if let Some(start) = newer_than {
            let threshold = start.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
            if value["created_ms"].as_u64()? < threshold {
                return None;
            }
        }
        Some(output.into())
    }
    pub fn publish(&self, output: &str) -> Result<()> {
        let value = serde_json::json!({"identity":self.identity,"created_ms":SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64,"sha256":hash(output.as_bytes()),"output":output});
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec(&value)?)?;
            file.sync_all()?;
            drop(file);
            crate::engine::replace_file(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_publishes_and_replaces_beyond_win32_max_path() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut root = dir.path().to_owned();
        while root.as_os_str().len() < 280 {
            root.push("metadata-cache-long-directory");
        }
        let cache = Cache::open_at(&root, "a".repeat(64), || false)
            .context("open long-path cache and lock")?;
        cache
            .publish("COMPLETE\n")
            .context("publish first long-path cache")?;
        assert_eq!(cache.read(None).as_deref(), Some("COMPLETE\n"));
        cache.publish("C\t676974\tFunction\nCOMPLETE\n")?;
        assert_eq!(
            cache.read(None).as_deref(),
            Some("C\t676974\tFunction\nCOMPLETE\n")
        );
        Ok(())
    }

    #[test]
    fn atomic_cache_rejects_corruption_and_stale_refresh() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cache = Cache::open_at(dir.path(), "test-key".into(), || false)?;
        assert!(cache.read(None).is_none());
        cache.publish("C\t676974\tFunction\nCOMPLETE\n")?;
        assert!(cache.read(None).is_some());
        assert!(
            cache
                .read(Some(SystemTime::now() + std::time::Duration::from_secs(1)))
                .is_none()
        );
        fs::write(&cache.path, b"{truncated")?;
        assert!(cache.read(None).is_none());
        cache.publish("COMPLETE\n")?;
        let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&cache.path)?)?;
        value["output"] = serde_json::json!("changed");
        fs::write(&cache.path, serde_json::to_vec(&value)?)?;
        assert!(cache.read(None).is_none());
        Ok(())
    }
    #[test]
    fn concurrent_refresh_waits_for_atomic_snapshot() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let first = Cache::open_at(dir.path(), "shared".into(), || false)?;
        let root = dir.path().to_owned();
        let reader = std::thread::spawn(move || {
            Cache::open_at(&root, "shared".into(), || false)
                .unwrap()
                .read(None)
        });
        first.publish("COMPLETE\n")?;
        drop(first);
        assert_eq!(reader.join().unwrap().as_deref(), Some("COMPLETE\n"));
        Ok(())
    }
    #[test]
    fn inventory_changes_when_module_members_change() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let shell = dir.path().join("pwsh.exe");
        fs::write(&shell, b"shell")?;
        let modules = dir.path().join("Modules");
        fs::create_dir(&modules)?;
        let a = identity(&shell, "script")?;
        fs::write(modules.join("Example.psd1"), b"module")?;
        let b = identity(&shell, "script")?;
        assert_ne!(a, b);
        assert_ne!(b, identity(&shell, "new-parser")?);
        Ok(())
    }

    #[test]
    fn inventory_tracks_runtime_and_module_removal() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let shell = dir.path().join("pwsh.exe");
        fs::write(&shell, b"shell")?;
        let modules = dir.path().join("Modules");
        fs::create_dir(&modules)?;
        let base = identity(&shell, "script")?;
        assert_eq!(base, identity(&shell, "script")?);

        // PS7 names this file after pwsh, not powershell. Runtime servicing
        // can change the selected runtime without replacing the launcher.
        let runtime = dir.path().join("pwsh.runtimeconfig.json");
        fs::write(
            &runtime,
            br#"{"runtimeOptions":{"rollForward":"LatestPatch"}}"#,
        )?;
        let configured = identity(&shell, "script")?;
        assert_ne!(base, configured);
        fs::write(&runtime, br#"{"runtimeOptions":{"rollForward":"Disable"}}"#)?;
        assert_ne!(configured, identity(&shell, "script")?);

        let module = modules.join("Removed.psd1");
        fs::write(&module, b"module")?;
        let installed = identity(&shell, "script")?;
        fs::remove_file(module)?;
        assert_ne!(installed, identity(&shell, "script")?);
        Ok(())
    }
}

//! Pinned Microsoft ConPTY, loaded only when Blueberry creates a pseudoterminal.
//!
//! The inbox host can leave ReadConsoleInput blocked with keys already queued
//! (microsoft/terminal#18816). Do not work around that by injecting extra keys.
//! portable-pty reuses the already loaded conpty.dll by basename. We first load
//! our verified absolute path and retain its module and read-only file handles.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
    sync::OnceLock,
};
use windows_sys::Win32::{
    Foundation::HMODULE,
    Storage::FileSystem::FILE_SHARE_READ,
    System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleW, GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
    },
};

include!(concat!(env!("OUT_DIR"), "/conpty.rs"));

struct Runtime {
    identity: Value,
    // Do not close these or unload the DLL while portable-pty retains pointers.
    _files: Vec<File>,
    _module: usize,
}
static RUNTIME: OnceLock<std::result::Result<Runtime, String>> = OnceLock::new();

pub fn build_identity() -> Value {
    serde_json::from_str(include_str!("../vendor/conpty/upstream.json")).expect("ConPTY metadata")
}

pub fn loaded_identity() -> Value {
    match RUNTIME.get() {
        Some(Ok(runtime)) => runtime.identity.clone(),
        Some(Err(error)) => json!({"mode":"failed", "error":error}),
        None => json!({"mode":"not_loaded"}),
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn module_path(module: HMODULE) -> Result<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let count =
        unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    ensure!(
        count > 0 && count < buffer.len(),
        "Cannot identify loaded ConPTY DLL"
    );
    Ok(std::fs::canonicalize(PathBuf::from(String::from_utf16(
        &buffer[..count],
    )?))?)
}

fn lock_asset(path: &Path, bytes: &[u8], expected: &str) -> Result<File> {
    crate::editor::install_asset(path, bytes)?;
    // Deny writes and replacement from verification through the last session.
    let mut file = File::options()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .with_context(|| format!("Cannot lock ConPTY asset {}", path.display()))?;
    let mut actual = Vec::new();
    file.read_to_end(&mut actual)?;
    ensure!(
        format!("{:x}", Sha256::digest(&actual)) == expected,
        "ConPTY asset digest mismatch: {}",
        path.display()
    );
    Ok(file)
}

fn load() -> Result<Runtime> {
    let mut identity = build_identity();
    let root = std::env::temp_dir()
        .join("blueberry-runtime")
        .join("conpty")
        .join(
            identity["sha256"]
                .as_str()
                .context("ConPTY digest")?
                .to_ascii_lowercase(),
        );
    let mut files = Vec::new();
    for (name, bytes, hash) in CONPTY_FILES {
        files.push(lock_asset(&root.join(name), bytes, hash)?);
    }
    let path = std::fs::canonicalize(root.join("conpty.dll"))?;
    let basename = wide(Path::new("conpty.dll"));
    let existing = unsafe { GetModuleHandleW(basename.as_ptr()) };
    if !existing.is_null() {
        ensure!(
            module_path(existing)? == path,
            "A different ConPTY DLL is already loaded; refusing an unverified runtime"
        );
    }
    let filename = wide(&path);
    let module = unsafe {
        LoadLibraryExW(
            filename.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        bail!(
            "Cannot load pinned ConPTY runtime: {}",
            std::io::Error::last_os_error()
        );
    }
    ensure!(
        module_path(module)? == path,
        "Loaded ConPTY path does not match its verified identity"
    );
    for export in [
        c"CreatePseudoConsole",
        c"ResizePseudoConsole",
        c"ClosePseudoConsole",
    ] {
        ensure!(
            unsafe { GetProcAddress(module, export.as_ptr().cast()) }.is_some(),
            "Pinned ConPTY is missing {:?}",
            export
        );
    }
    identity["mode"] = json!("pinned");
    identity["dll_path"] = json!(path);
    identity["host_path"] = json!(std::fs::canonicalize(root.join("OpenConsole.exe"))?);
    Ok(Runtime {
        identity,
        _files: files,
        _module: module as usize,
    })
}

pub fn ensure_loaded() -> Result<()> {
    RUNTIME
        .get_or_init(|| load().map_err(|error| format!("{error:#}")))
        .as_ref()
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!(error.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loads_verified_runtime_by_absolute_path() {
        ensure_loaded().unwrap();
        ensure_loaded().unwrap();
        let identity = loaded_identity();
        assert_eq!(identity["mode"], "pinned");
        assert!(Path::new(identity["dll_path"].as_str().unwrap()).is_absolute());
        for entry in identity["files"].as_array().unwrap() {
            let root = Path::new(identity["dll_path"].as_str().unwrap())
                .parent()
                .unwrap();
            assert_eq!(
                format!(
                    "{:x}",
                    Sha256::digest(
                        std::fs::read(root.join(entry["path"].as_str().unwrap())).unwrap()
                    )
                ),
                entry["sha256"].as_str().unwrap()
            );
        }
    }
    #[test]
    fn verified_assets_cannot_be_replaced_while_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asset.dll");
        let data = b"verified payload";
        let hash = format!("{:x}", Sha256::digest(data));
        std::fs::write(&path, b"stale payload").unwrap();
        let lock = lock_asset(&path, data, &hash).unwrap();
        assert!(std::fs::write(&path, b"changed").is_err());
        assert!(std::fs::remove_file(&path).is_err());
        let other = lock_asset(&path, data, &hash).unwrap();
        drop((lock, other));
        std::fs::write(&path, b"changed").unwrap();
        assert!(lock_asset(&path, data, &"0".repeat(64)).is_err());
    }
}

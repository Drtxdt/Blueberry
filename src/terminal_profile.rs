//! Official Windows Terminal JSON fragment registration, independent of settings.json.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path};

pub const GUID: &str = "{cda29eb9-0b16-4c08-bc90-75d9275a8ae9}";

fn quote(value: &Path) -> Result<String> {
    let value = value.to_str().context("启动路径需要 Unicode")?;
    ensure!(!value.contains(['"', '\r', '\n']), "启动路径含非法字符");
    // Windows command-line quoting: trailing slashes must precede an escaped closing quote.
    let trailing = value.chars().rev().take_while(|c| *c == '\\').count();
    Ok(format!("\"{value}{}\"", "\\".repeat(trailing)))
}

pub fn profile(executable: &Path, shell: Option<&Path>) -> Result<Value> {
    let mut commandline = format!("{} run", quote(executable)?);
    if let Some(shell) = shell {
        commandline.push_str(&format!(" --shell {}", quote(shell)?));
    }
    Ok(json!({
        "name": "Blueberry PowerShell", "commandline": commandline,
        "startingDirectory": "%USERPROFILE%", "guid": GUID, "hidden": false
    }))
}

pub fn manage(root: &Path, profile: Value, install: bool, remove: bool) -> Result<Value> {
    let path = root.join("Microsoft/Windows Terminal/Fragments/Blueberry/blueberry.json");
    let desired = json!({"profiles": [profile], "blueberry_fragment_schema": 1});
    let read = || -> Result<Option<Vec<u8>>> {
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    };
    // Mutations share a lock; status is read-only and never creates directories.
    let _lock = if install || remove {
        fs::create_dir_all(path.parent().unwrap())?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("lock"))?;
        lock.try_lock().context("启动入口正被另一个操作修改")?;
        Some(lock)
    } else {
        None
    };
    let original = read()?;
    let existing = original
        .as_deref()
        .map(serde_json::from_slice::<Value>)
        .transpose()?;
    let owned = existing.as_ref().is_some_and(|value| {
        value["blueberry_fragment_schema"] == 1
            && value["profiles"]
                .as_array()
                .is_some_and(|profiles| profiles.len() == 1 && profiles[0]["guid"] == GUID)
    });
    if install || remove {
        ensure!(
            existing.is_none() || owned,
            "拒绝修改不属于 Blueberry 的启动入口：{}",
            path.display()
        );
    }
    if remove && original.is_some() {
        ensure!(read()? == original, "启动入口在移除前已变化");
        fs::remove_file(&path)?;
    }
    if install && existing.as_ref() != Some(&desired) {
        let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(&serde_json::to_vec_pretty(&desired)?)?;
            file.sync_all()?;
            drop(file);
            ensure!(read()? == original, "启动入口在保存前已变化");
            crate::engine::replace_file(&temp, &path)?;
            Ok(())
        })();
        let _ = fs::remove_file(&temp);
        result?;
    }
    let actual = read()?
        .as_deref()
        .map(serde_json::from_slice::<Value>)
        .transpose()?;
    Ok(
        json!({"path": path, "installed": actual.is_some(), "matches_requested": actual.as_ref() == Some(&desired), "fragment": actual}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_preserves_other_terminal_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let settings = dir.path().join("settings.json");
        fs::write(&settings, "user settings")?;
        let first = profile(
            Path::new("C:/Program Files/Blueberry/blueberry.exe"),
            Some(Path::new("pwsh.exe")),
        )?;
        assert_eq!(
            manage(dir.path(), first.clone(), false, false)?["installed"],
            false
        );
        assert_eq!(
            manage(dir.path(), first.clone(), true, false)?["matches_requested"],
            true
        );
        let second = profile(
            Path::new("blueberry.exe"),
            Some(Path::new("powershell.exe")),
        )?;
        assert_eq!(
            manage(dir.path(), second.clone(), true, false)?["matches_requested"],
            true
        );
        assert_eq!(manage(dir.path(), second, false, true)?["installed"], false);
        assert_eq!(fs::read_to_string(settings)?, "user settings");
        let state = manage(dir.path(), first.clone(), true, false)?;
        let path = Path::new(state["path"].as_str().unwrap());
        fs::write(path, "{\"profiles\":[]}")?;
        assert!(manage(dir.path(), first, false, true).is_err());
        assert_eq!(fs::read_to_string(path)?, "{\"profiles\":[]}");
        Ok(())
    }
}

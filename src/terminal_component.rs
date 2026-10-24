//! Optional Terminal lifecycle, separate from the core installation manifest.
use anyhow::Result;
use clap::ValueEnum;
use std::path::PathBuf;

/// Read installation evidence without launching the component or a shell.
pub fn status(root: &std::path::Path) -> serde_json::Value {
    use anyhow::{Context, ensure};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    fn canonical(value: &Value) -> String {
        match value {
            Value::String(value) => {
                let mut output = String::from("\"");
                for unit in value.encode_utf16() {
                    if !(32..127).contains(&unit) || unit == 34 || unit == 92 {
                        output.push_str(&format!("\\u{unit:04x}"));
                    } else {
                        output.push(char::from_u32(u32::from(unit)).unwrap());
                    }
                }
                output.push('"');
                output
            }
            Value::Array(values) => format!(
                "[{}]",
                values.iter().map(canonical).collect::<Vec<_>>().join(",")
            ),
            Value::Object(values) => {
                let mut pairs = values.iter().collect::<Vec<_>>();
                pairs.sort_by_cached_key(|(key, _)| key.encode_utf16().collect::<Vec<_>>());
                format!(
                    "{{{}}}",
                    pairs
                        .into_iter()
                        .map(|(key, value)| format!(
                            "{}:{}",
                            canonical(&Value::String(key.clone())),
                            canonical(value)
                        ))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            _ => value.to_string(),
        }
    }
    let path = root.join("components/terminal/component-state.json");
    let read = || -> Result<Value> {
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(json!({"installed":false,"state":"absent"}));
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(
            metadata.len() <= 8 * 1024 * 1024,
            "Component state exceeds size limit"
        );
        let mut state: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let hash = state["metadata_sha256"]
            .as_str()
            .context("Missing component checksum")?
            .to_owned();
        state["metadata_sha256"] = json!("");
        ensure!(
            hash.eq_ignore_ascii_case(&format!(
                "{:x}",
                Sha256::digest(format!("{}\n", canonical(&state)).as_bytes())
            )),
            "Component checksum mismatch"
        );
        ensure!(
            state["schema_version"] == 1 && state["product"] == "Blueberry.Terminal",
            "Unsupported component state"
        );
        ensure!(
            state["installed"].is_boolean(),
            "Missing component installed state"
        );
        if state["installed"] == false {
            return Ok(json!({"installed":false,"state":"inactive","history":state["history"]}));
        }
        let id = state["current"]
            .as_str()
            .context("Missing active component")?;
        ensure!(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid component identity"
        );
        let version = &state["versions"][id];
        let executable = root
            .join("components/terminal/versions")
            .join(id)
            .join("WindowsTerminal.exe");
        let record = version["files"]
            .as_array()
            .context("Missing component files")?
            .iter()
            .find(|file| file["path"] == "WindowsTerminal.exe")
            .context("Missing component executable record")?;
        ensure!(
            std::fs::metadata(&executable)?.len() <= 512 * 1024 * 1024,
            "Component executable exceeds size limit"
        );
        let bytes = std::fs::read(executable)?;
        ensure!(
            record["bytes"].as_u64() == Some(bytes.len() as u64)
                && record["sha256"].as_str().is_some_and(
                    |hash| hash.eq_ignore_ascii_case(&format!("{:x}", Sha256::digest(&bytes)))
                ),
            "Component executable changed"
        );
        Ok(
            json!({"installed":true,"state":"active","version":version["version"],"identity":id,"dependency":version["dependency"],"history":state["history"]}),
        )
    };
    match read() {
        Ok(value) => value,
        Err(error) => json!({"installed":null,"state":"invalid","error":format!("{error:#}")}),
    }
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Action {
    Install,
    Upgrade,
    Rollback,
    Uninstall,
    Status,
    #[value(hide = true)]
    Launch,
}
#[cfg(windows)]
pub fn run(action: Action, package: Option<PathBuf>, install_root: Option<PathBuf>) -> Result<u32> {
    use anyhow::{Context, ensure};
    use sha2::{Digest, Sha256};
    let executable = std::env::current_exe()?;
    let root = install_root.unwrap_or(
        executable
            .parent()
            .context("executable parent")?
            .to_path_buf(),
    );
    ensure!(
        matches!(action, Action::Install | Action::Upgrade) == package.is_some(),
        "install/upgrade require --package; other actions do not accept a package"
    );
    let files = [
        (
            "manage-terminal.ps1",
            include_bytes!("../scripts/manage-terminal.ps1").as_slice(),
        ),
        (
            "manage-install.ps1",
            include_bytes!("../scripts/manage-install.ps1").as_slice(),
        ),
        (
            "install-common.ps1",
            include_bytes!("../scripts/install-common.ps1").as_slice(),
        ),
    ];
    let mut hash = Sha256::new();
    for (name, bytes) in &files {
        hash.update(name);
        hash.update(bytes);
    }
    let runtime = std::env::temp_dir()
        .join("blueberry-runtime")
        .join(format!("terminal-{:x}", hash.finalize()));
    fn reject_reparse(path: &std::path::Path) -> Result<()> {
        use std::os::windows::fs::MetadataExt;
        for part in path.ancestors() {
            match std::fs::symlink_metadata(part) {
                Ok(metadata) => ensure!(
                    metadata.file_attributes() & 0x400 == 0,
                    "Terminal maintenance path contains a reparse point: {}",
                    part.display()
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
    reject_reparse(&runtime)?;
    std::fs::create_dir_all(&runtime)?;
    for (name, bytes) in files {
        let path = runtime.join(name);
        reject_reparse(&path)?;
        if std::fs::read(&path).ok().as_deref() != Some(bytes) {
            use std::io::Write;
            let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            if let Err(error) = crate::engine::replace_file(&temp, &path) {
                let _ = std::fs::remove_file(&temp);
                if std::fs::read(&path).ok().as_deref() != Some(bytes) {
                    return Err(error.into());
                }
            }
        }
    }
    let mut command = std::process::Command::new(crate::pty::default_shell());
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(runtime.join("manage-terminal.ps1"))
        .arg("-Operation")
        .arg(format!("{action:?}"))
        .arg("-InstallRoot")
        .arg(root);
    if let Some(package) = package {
        command
            .arg("-PackagePath")
            .arg(std::fs::canonicalize(package)?);
    }
    Ok(command.status()?.code().unwrap_or(1) as u32)
}
#[cfg(not(windows))]
pub fn run(_: Action, _: Option<PathBuf>, _: Option<PathBuf>) -> Result<u32> {
    anyhow::bail!("Terminal component requires Windows")
}

use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::path::PathBuf;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
};

pub struct Session {
    pub master: Box<dyn MasterPty + Send>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
}

/// Keep startup key configuration identical for the host and paired probe.
/// JSON remains available for older adapters; the five checked strings avoid
/// cold JSON parsing solely for initial key arbitration in current adapters.
pub fn key_environment(keys: &crate::config::KeyBindings) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::from([
        ("BLUEBERRY_PUBLIC_KEYS_VERSION".into(), "1".into()),
        (
            "BLUEBERRY_PUBLIC_KEYS".into(),
            serde_json::to_string(keys).expect("key strings serialize"),
        ),
    ]);
    for (name, value) in [
        ("TRIGGER", &keys.trigger),
        ("NATIVE", &keys.native),
        ("DETAILS", &keys.details),
        ("REFRESH", &keys.refresh),
        ("RELOAD", &keys.reload),
    ] {
        environment.insert(format!("BLUEBERRY_PUBLIC_KEY_{name}"), value.clone());
    }
    environment
}

pub fn spawn(
    program: &Path,
    args: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    rows: u16,
    cols: u16,
) -> Result<Session> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("Cannot create a pseudoterminal")?;
    let mut command = CommandBuilder::new(program);
    // portable-pty refreshes registry environment values on Windows. Preserve
    // the actual launching session instead (virtualenvs, PATH edits, etc.).
    command.env_clear();
    for (key, value) in std::env::vars_os() {
        command.env(key, value);
    }
    command.args(args);
    command.cwd(cwd);
    for (key, value) in env {
        command.env(key, value);
    }
    let child = pair
        .slave
        .spawn_command(command)
        .with_context(|| format!("Cannot start {}", program.display()))?;
    drop(pair.slave);
    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    Ok(Session {
        master: pair.master,
        reader,
        writer,
        child,
    })
}

pub fn ensure_integration(directory: &Path) -> Result<std::path::PathBuf> {
    std::fs::create_dir_all(directory)?;
    let content = include_str!("../shell/integration.ps1").replace(
        "([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'legacy-json.cs')))",
        &format!("@'\n{}\n'@", include_str!("../shell/legacy-json.cs")),
    );
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hash);
    let path = directory.join(format!("integration-{:016x}.ps1", hash.finish()));
    if std::fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
        let temporary = directory.join(format!("integration.{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temporary, &content)?;
        if let Err(error) = std::fs::rename(&temporary, &path) {
            let _ = std::fs::remove_file(&temporary);
            if std::fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
                return Err(error.into());
            }
        }
    }
    Ok(path)
}

pub fn shell_args(integration: &Path, no_profile: bool) -> Vec<String> {
    let mut args = vec!["-NoLogo".into(), "-NoExit".into()];
    if no_profile {
        args.push("-NoProfile".into());
    }
    let isolated = std::env::var("BLUEBERRY_NO_HISTORY").as_deref() == Ok("1");
    let module = if isolated {
        std::env::var("BLUEBERRY_TEST_PSREADLINE_MODULE")
            .ok()
            .filter(|s| !s.is_empty())
    } else {
        None
    };
    let mut module_import = module
        .map(|path| format!("Import-Module '{}' -Force; ", path.replace('\'', "''")))
        .unwrap_or_default();
    if isolated {
        // Keep a malformed development adapter from falling through to a
        // shell that writes a real user's history during isolated tests.
        module_import.push_str(
            "Import-Module PSReadLine; Set-PSReadLineOption -HistorySaveStyle SaveNothing; ",
        );
    }
    args.extend([
        "-Command".into(),
        format!(
            "{module_import}. '{}'",
            integration.to_string_lossy().replace('\'', "''")
        ),
    ]);
    args
}

/// Prefer PowerShell 7 and fall back to the Windows inbox shell.
pub fn default_shell() -> PathBuf {
    if let Some(path) = std::env::var_os("BLUEBERRY_TEST_SHELL").filter(|path| !path.is_empty()) {
        return path.into();
    }
    let name = if cfg!(windows) { "pwsh.exe" } else { "pwsh" };
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            let path = directory.join(name);
            if path.is_file() {
                return path;
            }
        }
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        return PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    }
    name.into()
}

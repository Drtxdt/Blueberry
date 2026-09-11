use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
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
    let content = include_str!("../shell/integration.ps1");
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hash);
    let path = directory.join(format!("integration-{:016x}.ps1", hash.finish()));
    if std::fs::read_to_string(&path).ok().as_deref() != Some(content) {
        let temporary = directory.join(format!("integration.{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temporary, content)?;
        if let Err(error) = std::fs::rename(&temporary, &path) {
            let _ = std::fs::remove_file(&temporary);
            if std::fs::read_to_string(&path).ok().as_deref() != Some(content) {
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
    args.extend([
        "-Command".into(),
        format!(". '{}'", integration.to_string_lossy().replace('\'', "''")),
    ]);
    args
}

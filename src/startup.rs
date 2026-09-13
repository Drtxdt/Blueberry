use anyhow::{Context, Result, bail};
use std::path::Path;

pub fn run(action: &str, profile: Option<&Path>, owned_only: bool) -> Result<u32> {
    if !cfg!(windows) {
        bail!("PowerShell startup integration is supported on Windows");
    }
    let directory = crate::config::cache_dir();
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(format!("startup-{}.ps1", uuid::Uuid::new_v4()));
    std::fs::write(&path, include_bytes!("../scripts/startup.ps1"))?;
    let result = (|| {
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
            .arg(&path)
            .args(["-Action", action, "-Executable"])
            .arg(std::env::current_exe()?);
        if let Some(profile) = profile {
            command.arg("-ProfilePath").arg(profile);
        }
        if owned_only {
            command.arg("-OwnedOnly");
        }
        let status = command
            .status()
            .context("Unable to manage PowerShell startup")?;
        Ok(status.code().unwrap_or(1) as u32)
    })();
    let _ = std::fs::remove_file(path);
    result
}

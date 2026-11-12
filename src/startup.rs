use anyhow::{Context, Result, bail};
use std::path::Path;

pub fn run(action: &str, profile: Option<&Path>, owned_only: bool) -> Result<u32> {
    if !cfg!(windows) {
        bail!("PowerShell startup integration is supported on Windows");
    }
    let temporary = tempfile::tempdir()?;
    let directory = temporary.path();
    let path = directory.join("startup.ps1");
    std::fs::write(
        directory.join("profile-module.ps1"),
        include_bytes!("../scripts/profile-module.ps1"),
    )?;
    if action == "enable" {
        crate::editor::export_startup_editors(&directory.join("editors"))?;
    }
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
        command.arg("-EditorPayload").arg(directory.join("editors"));
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

//! Session-scoped metadata helper. No profile, import, terminal I/O or polling.
use crate::model::ShellCommand;
use anyhow::{Context, Result, ensure};
use std::{
    io::Read,
    os::windows::{
        io::{AsHandle, AsRawHandle},
        process::CommandExt,
    },
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
};
use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, WaitForSingleObject};

pub struct Discovery(Arc<Mutex<Child>>);
impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.0.lock().unwrap().kill();
    }
}
pub fn start(
    shell: &Path,
    done: impl FnOnce(Result<Vec<ShellCommand>>) + Send + 'static,
) -> Result<Discovery> {
    start_script(shell, include_str!("../shell/system-commands.ps1"), done)
}
fn start_script(
    shell: &Path,
    script: &str,
    done: impl FnOnce(Result<Vec<ShellCommand>>) + Send + 'static,
) -> Result<Discovery> {
    let mut child = Command::new(shell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env(
            "PSModulePath",
            std::env::join_paths(crate::pty::module_search_paths(shell))?,
        )
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let handle = child.as_handle().try_clone_to_owned()?;
    let stdout = child.stdout.take().context("metadata stdout")?;
    let stderr = child.stderr.take().context("metadata stderr")?;
    let shared = Arc::new(Mutex::new(child));
    let process = shared.clone();
    thread::spawn(move || {
        let out = thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let err = thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr
                .take(64 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let result = (|| -> Result<Vec<ShellCommand>> {
            let waited = unsafe { WaitForSingleObject(handle.as_raw_handle(), 30_000) };
            if waited != 0 {
                let _ = process.lock().unwrap().kill();
            }
            let status = process.lock().unwrap().wait()?;
            let bytes = out
                .join()
                .map_err(|_| anyhow::anyhow!("metadata reader failed"))??;
            let errors = err
                .join()
                .map_err(|_| anyhow::anyhow!("metadata error reader failed"))??;
            ensure!(waited == 0, "system metadata helper timed out");
            ensure!(
                status.success(),
                "system metadata helper: {}",
                String::from_utf8_lossy(&errors)
            );
            ensure!(
                bytes.len() <= 4 * 1024 * 1024,
                "system metadata output limit"
            );
            parse(&String::from_utf8(bytes)?)
        })();
        done(result);
    });
    Ok(Discovery(shared))
}
fn parse(output: &str) -> Result<Vec<ShellCommand>> {
    let mut commands = Vec::new();
    let mut lines = output.lines();
    while let Some(line) = lines.next() {
        if line == "COMPLETE" {
            ensure!(lines.next().is_none(), "trailing metadata");
            return Ok(commands);
        }
        let fields: Vec<_> = line.split('\t').collect();
        ensure!(
            fields.len() == 3 && fields[0] == "C",
            "invalid metadata record"
        );
        ensure!(
            matches!(fields[2], "Alias" | "Function" | "Cmdlet"),
            "invalid command kind"
        );
        ensure!(
            fields[1].len() % 2 == 0 && fields[1].is_ascii(),
            "invalid command encoding"
        );
        let name = (0..fields[1].len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&fields[1][i..i + 2], 16))
            .collect::<Result<Vec<_>, _>>()?;
        commands.push(ShellCommand {
            name: String::from_utf8(name)?,
            kind: fields[2].into(),
            definition: String::new(),
        });
    }
    anyhow::bail!("incomplete system metadata snapshot")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_analysis_does_not_execute_or_import_script_module() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let module = dir.path().join("SideEffect");
        std::fs::create_dir(&module)?;
        let marker = dir.path().join("EXECUTED");
        std::fs::write(
            module.join("SideEffect.psm1"),
            format!(
                "[IO.File]::WriteAllText('{}','bad'); function Get-BlueberryMetadataOnly {{ throw 'must not execute' }}",
                marker.display().to_string().replace('\'', "''")
            ),
        )?;
        std::fs::write(
            module.join("SideEffect.psd1"),
            "@{ ModuleVersion='1.0'; RootModule='SideEffect.psm1'; FunctionsToExport=@('Get-BlueberryMetadataOnly'); CmdletsToExport=@(); AliasesToExport=@() }",
        )?;
        let script = include_str!("../shell/system-commands.ps1").replace(
            "$roots = @([IO.Path]::Combine($PSHOME, 'Modules'))",
            &format!(
                "$roots = @('{}')",
                dir.path().display().to_string().replace('\'', "''")
            ),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let _discovery = start_script(&crate::pty::default_shell(), &script, move |result| {
            let _ = tx.send(result);
        })?;
        let commands = rx.recv_timeout(std::time::Duration::from_secs(35))??;
        assert!(
            commands
                .iter()
                .any(|c| c.name == "Get-BlueberryMetadataOnly")
        );
        assert!(!marker.exists(), "module body was executed");
        Ok(())
    }
    #[test]
    fn incomplete_or_corrupt_output_is_not_a_snapshot() {
        assert!(parse("C\t476574\tCmdlet\n").is_err());
        assert!(parse("C\tXX\tCmdlet\nCOMPLETE\n").is_err());
        assert_eq!(
            parse("C\t476574\tCmdlet\nCOMPLETE\n").unwrap()[0].name,
            "Get"
        );
    }
}

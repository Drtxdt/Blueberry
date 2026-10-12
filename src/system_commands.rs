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

struct DiscoveryState {
    cancelled: bool,
    child: Option<Child>,
    job: Option<crate::latency_layers::DirectChildJob>,
}
pub struct Discovery(Arc<Mutex<DiscoveryState>>);
impl Drop for Discovery {
    fn drop(&mut self) {
        let mut state = self.0.lock().unwrap();
        state.cancelled = true;
        state.job.take();
        if let Some(child) = &mut state.child {
            let _ = child.kill();
        }
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
    let shared = Arc::new(Mutex::new(DiscoveryState {
        cancelled: false,
        child: None,
        job: None,
    }));
    let process = shared.clone();
    let shell = shell.to_owned();
    let script = script.to_owned();
    thread::Builder::new()
        .name("blueberry module metadata".into())
        .spawn(move || {
            let result = (|| -> Result<Vec<ShellCommand>> {
                ensure!(
                    !process.lock().unwrap().cancelled,
                    "system metadata cancelled"
                );
                // CreateProcess itself can stall the caller. Keep process creation,
                // module-path inspection, and collection off the host dispatcher.
                let mut child = Command::new(&shell)
                    .args([
                        "-NoLogo",
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        &script,
                    ])
                    .env(
                        "PSModulePath",
                        std::env::join_paths(crate::pty::module_search_paths(&shell))?,
                    )
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()?;
                let job = match crate::latency_layers::DirectChildJob::attach(&child) {
                    Ok(job) => job,
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(error);
                    }
                };
                let handle = child.as_handle().try_clone_to_owned()?;
                let stdout = child.stdout.take().context("metadata stdout")?;
                let stderr = child.stderr.take().context("metadata stderr")?;
                {
                    let mut state = process.lock().unwrap();
                    if state.cancelled {
                        drop(job);
                        let _ = child.kill();
                        let _ = child.wait();
                        anyhow::bail!("system metadata cancelled");
                    }
                    state.child = Some(child);
                    state.job = Some(job);
                }
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
                let waited = unsafe { WaitForSingleObject(handle.as_raw_handle(), 30_000) };
                let mut child = {
                    let mut state = process.lock().unwrap();
                    // Also close any descendants' inherited output handles before
                    // joining readers, including on cancellation or timeout.
                    state.job.take();
                    state.child.take().context("metadata process missing")?
                };
                if waited != 0 {
                    let _ = child.kill();
                }
                let status = child.wait()?;
                let bytes = out
                    .join()
                    .map_err(|_| anyhow::anyhow!("metadata reader failed"))??;
                let errors = err
                    .join()
                    .map_err(|_| anyhow::anyhow!("metadata error reader failed"))??;
                ensure!(
                    !process.lock().unwrap().cancelled,
                    "system metadata cancelled"
                );
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
        })
        .context("start system metadata worker")?;
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

    #[test]
    fn cancelled_metadata_reaps_hanging_helper_and_descendant() -> Result<()> {
        use std::os::windows::io::{FromRawHandle, OwnedHandle};
        use std::time::{Duration, Instant};
        use windows_sys::Win32::System::Threading::OpenProcess;
        let dir = tempfile::tempdir()?;
        let marker = dir.path().join("metadata-processes.json");
        let shell = crate::pty::default_shell();
        let script = format!(
            "$info=New-Object Diagnostics.ProcessStartInfo; $info.FileName='{}'; $info.Arguments='-NoLogo -NoProfile -NonInteractive -Command Start-Sleep -Seconds 60'; $info.UseShellExecute=$false; $info.CreateNoWindow=$true; $owned=New-Object Diagnostics.Process; $owned.StartInfo=$info; $null=$owned.Start(); [IO.File]::WriteAllText('{}', ('['+$PID+','+$owned.Id+']')); Start-Sleep -Seconds 60",
            shell.display().to_string().replace('\'', "''"),
            marker.display().to_string().replace('\'', "''")
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let discovery = start_script(&shell, &script, move |result| {
            let _ = tx.send(result);
        })?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let ids: Vec<u32> = loop {
            if let Ok(bytes) = std::fs::read(&marker)
                && let Ok(ids) = serde_json::from_slice(&bytes)
            {
                break ids;
            }
            ensure!(Instant::now() < deadline, "metadata helper did not start");
            thread::sleep(Duration::from_millis(10));
        };
        ensure!(ids.len() == 2, "missing owned descendant");
        let handles: Vec<_> = ids
            .into_iter()
            .map(|pid| {
                let raw = unsafe { OpenProcess(0x0010_0000, 0, pid) };
                ensure!(!raw.is_null(), "cannot observe metadata child");
                Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
            })
            .collect::<Result<_>>()?;
        drop(discovery);
        let result = rx.recv_timeout(Duration::from_secs(5))?;
        ensure!(
            result.is_err_and(|error| error.to_string().contains("cancelled")),
            "cancelled metadata reported success"
        );
        for handle in handles {
            ensure!(
                unsafe { WaitForSingleObject(handle.as_raw_handle(), 5_000) } == 0,
                "owned metadata process survived cancellation"
            );
        }
        Ok(())
    }

    #[test]
    fn helper_start_failure_is_reported_by_background_worker() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (tx, rx) = std::sync::mpsc::channel();
        let _discovery = start_script(&dir.path().join("missing.exe"), "", move |result| {
            let _ = tx.send(result);
        })?;
        ensure!(
            rx.recv_timeout(std::time::Duration::from_secs(5))?.is_err(),
            "helper launch failure was not reported"
        );
        Ok(())
    }
}

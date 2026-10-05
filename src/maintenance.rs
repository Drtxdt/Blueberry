//! Managed installation commands. The worker runs from outside the installation
//! and uses the same transactional PowerShell engine as the standalone installer.
use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct Upgrade {
    #[arg(long, conflicts_with = "package")]
    pub check: bool,
    #[arg(long, conflicts_with = "package")]
    pub version: Option<String>,
    #[arg(long)]
    pub package: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Maintenance {
    Status,
    Cancel,
}

#[cfg(not(windows))]
pub fn run(_: &str, _: Option<Upgrade>) -> Result<u32> {
    bail!("Managed installation commands are available on Windows x64")
}

#[cfg(not(windows))]
pub fn resume_before_run() -> Result<()> {
    Ok(())
}

#[cfg(windows)]
mod windows {
    use super::*;
    use anyhow::{Context, ensure};
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
        path::Path,
        process::Command,
    };

    struct Installation {
        root: PathBuf,
        state: PathBuf,
    }
    impl Installation {
        fn reject_reparse(path: &Path) -> Result<()> {
            use std::os::windows::fs::MetadataExt;
            for part in path.ancestors() {
                match fs::symlink_metadata(part) {
                    Ok(metadata) => ensure!(
                        metadata.file_attributes() & 0x400 == 0,
                        "Maintenance path contains a reparse point: {}",
                        part.display()
                    ),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(())
        }
        fn discover() -> Result<Self> {
            let exe = std::env::current_exe()?;
            let root = exe.parent().context("executable directory")?.to_path_buf();
            Self::reject_reparse(&root)?;
            ensure!(
                root.join("install.json").is_file(),
                "此副本不是受管理安装；请使用官方 install.ps1 安装后再运行维护命令"
            );
            let manifest: Value = serde_json::from_slice(&fs::read(root.join("install.json"))?)?;
            ensure!(
                manifest["product"] == "Blueberry"
                    && manifest["current"]["path"] == "blueberry.exe",
                "Invalid managed installation manifest"
            );
            ensure!(
                Path::new(manifest["install_root"].as_str().unwrap_or(""))
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&root.to_string_lossy()),
                "Installation root does not match this executable"
            );
            let digest = format!(
                "{:x}",
                Sha256::digest(root.to_string_lossy().to_lowercase().as_bytes())
            );
            let state = root
                .parent()
                .context("installation parent")?
                .join(format!(".blueberry-maintenance-{}", &digest[..16]));
            Self::reject_reparse(&state)?;
            Ok(Self { root, state })
        }
        fn scripts(&self) -> Result<PathBuf> {
            let assets = [
                (
                    "maintenance.ps1",
                    include_bytes!("../scripts/maintenance.ps1").as_slice(),
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
            let mut digest = Sha256::new();
            for (name, bytes) in assets {
                digest.update(name);
                digest.update(bytes);
            }
            let directory = self.state.join(format!("tools-{:x}", digest.finalize()));
            for (name, bytes) in assets {
                // UTF-8 BOM is necessary when Windows PowerShell 5.1 loads
                // the embedded scripts containing Chinese diagnostics.
                let mut encoded = vec![0xef, 0xbb, 0xbf];
                encoded.extend_from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes));
                crate::editor::install_asset(&directory.join(name), &encoded)?;
            }
            Ok(directory.join("maintenance.ps1"))
        }
        fn command(&self, script: &Path, mode: &str) -> Result<Command> {
            let shell = PathBuf::from(std::env::var_os("SystemRoot").context("SystemRoot")?)
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
            let modules = std::env::join_paths(crate::pty::module_search_paths(&shell))?;
            let mut command = Command::new(shell);
            command.env("PSModulePath", modules);
            command
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(script)
                .args(["-Mode", mode, "-InstallRoot"])
                .arg(&self.root)
                .arg("-StateRoot")
                .arg(&self.state);
            Ok(command)
        }
        fn launch_worker(&self, script: &Path) -> Result<()> {
            use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::*};
            let command = self.command(script, "Worker")?;
            let application: Vec<u16> =
                command.get_program().encode_wide().chain(Some(0)).collect();
            let mut line = Vec::new();
            for argument in std::iter::once(command.get_program()).chain(command.get_args()) {
                if !line.is_empty() {
                    line.push(32);
                }
                line.extend(quote_windows(argument));
            }
            line.push(0);
            let mut variables: Vec<_> = std::env::vars_os().collect();
            for (key, value) in command.get_envs() {
                variables.retain(|(existing, _)| {
                    !existing
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&key.to_string_lossy())
                });
                if let Some(value) = value {
                    variables.push((key.to_os_string(), value.to_os_string()));
                }
            }
            variables.sort_by_key(|(key, _)| key.to_string_lossy().to_uppercase());
            let mut environment = Vec::<u16>::new();
            for (key, value) in variables {
                environment.extend(key.encode_wide());
                environment.push(61);
                environment.extend(value.encode_wide());
                environment.push(0);
            }
            environment.push(0);
            let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
            startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
            let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
            // No handle inheritance, including unrelated inheritable stdout
            // pipes supplied by a calling application. Leaking one would keep
            // that caller waiting for EOF while this worker waits for its exit.
            let created = unsafe {
                CreateProcessW(
                    application.as_ptr(),
                    line.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB | CREATE_UNICODE_ENVIRONMENT,
                    environment.as_ptr().cast(),
                    std::ptr::null(),
                    &startup,
                    &mut process,
                )
            };
            if created == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("无法启动独立维护助手；操作仍已排队，将在下次允许启动时重试");
            }
            unsafe {
                CloseHandle(process.hThread);
                CloseHandle(process.hProcess);
            }
            Ok(())
        }
    }

    fn quote_windows(argument: &std::ffi::OsStr) -> Vec<u16> {
        let mut quoted = vec![34];
        let mut slashes = 0;
        for unit in argument.encode_wide() {
            if unit == 92 {
                slashes += 1;
                continue;
            }
            quoted.extend(std::iter::repeat_n(
                92,
                if unit == 34 { slashes * 2 + 1 } else { slashes },
            ));
            slashes = 0;
            quoted.push(unit);
        }
        quoted.extend(std::iter::repeat_n(92, slashes * 2));
        quoted.push(34);
        quoted
    }

    pub fn run(operation: &str, upgrade: Option<Upgrade>) -> Result<u32> {
        let installation = Installation::discover()?;
        let script = installation.scripts()?;
        let mode = match operation {
            "Status" => "Status",
            "Cancel" => "Cancel",
            "Upgrade" if upgrade.as_ref().is_some_and(|u| u.check) => "Check",
            _ => "Prepare",
        };
        let mut command = installation.command(&script, mode)?;
        if mode == "Prepare" {
            command.args(["-Operation", operation]);
        }
        if let Some(upgrade) = upgrade {
            if let Some(version) = upgrade.version {
                command.arg("-Version").arg(version);
            }
            if let Some(package) = upgrade.package {
                command
                    .arg("-PackagePath")
                    .arg(std::path::absolute(package)?);
            }
        }
        let output = command
            .output()
            .context("run managed installation command")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        ensure!(
            output.status.success(),
            "{}{}",
            stdout,
            String::from_utf8_lossy(&output.stderr)
        );
        for line in stdout.lines() {
            if let Some(json) = line.strip_prefix("BLUEBERRY_MAINTENANCE:") {
                let value: Value = serde_json::from_str(json)?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else if !line.trim().is_empty() {
                println!("{line}");
            }
        }
        if mode == "Prepare" {
            installation.launch_worker(&script)?;
            println!(
                "已排队：相关 Blueberry 会话自然退出后自动执行。使用 blueberry maintenance status 查看结果。\n维护日志：{}",
                installation.state.join("worker.log").display()
            );
        }
        Ok(0)
    }

    pub fn resume_before_run() -> Result<()> {
        // Portable/development builds have no maintenance state and pay no
        // PowerShell startup cost. The normal no-pending path is file reads only.
        if !std::env::current_exe()?
            .parent()
            .context("executable parent")?
            .join("install.json")
            .is_file()
        {
            return Ok(());
        }
        let installation = Installation::discover()?;
        let path = installation.state.join("operation.json");
        if !path.is_file() {
            return Ok(());
        }
        let operation: Value = serde_json::from_slice(&fs::read(path)?)?;
        let committing = matches!(
            operation["status"].as_str(),
            Some("running" | "recovering" | "recovery_required")
        );
        if matches!(
            operation["status"].as_str(),
            Some(
                "preparing" | "queued" | "waiting" | "running" | "recovering" | "recovery_required"
            )
        ) {
            match fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .share_mode(0)
                .open(installation.state.join("operation.lock"))
            {
                Ok(lock) => {
                    drop(lock);
                    if let Err(error) = installation.launch_worker(&installation.scripts()?) {
                        eprintln!("blueberry: {error:#}");
                    }
                }
                Err(error) if error.raw_os_error() == Some(32) => {} // active worker
                Err(error) => return Err(error.into()),
            }
        }
        if committing {
            bail!("安装维护事务正在提交或恢复；请运行 blueberry maintenance status 检查结果");
        }
        Ok(())
    }
}

#[cfg(windows)]
pub use windows::{resume_before_run, run};

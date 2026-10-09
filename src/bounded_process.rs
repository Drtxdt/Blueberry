//! Bounded, cancellable process capture shared by help learning and tools UI.
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(crate) const OUTPUT_LIMIT: usize = 256 * 1024;
#[derive(Debug)]
pub(crate) struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub(crate) fn capture(
    target: &Path,
    args: &[String],
    cwd: &Path,
    cancelled: &AtomicBool,
) -> Result<Output, String> {
    if cancelled.load(Ordering::Relaxed) {
        return Err("查询已取消".into());
    }
    let mut command = Command::new(target);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .env("AWS_PAGER", "")
        .env("AZURE_CORE_ONLY_SHOW_ERRORS", "true")
        .env("CLOUDSDK_CORE_DISABLE_PROMPTS", "1")
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000004);
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let job = match Job::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    #[cfg(windows)]
    if let Err(error) = resume_child(child.id()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(4);
    let streams = [
        Box::new(child.stdout.take().expect("piped stdout")) as Box<dyn Read + Send>,
        Box::new(child.stderr.take().expect("piped stderr")),
    ];
    let mut readers = Vec::new();
    for (index, mut stream) in streams.into_iter().enumerate() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => {
                        let _ = tx.send((index, Ok(Vec::new())));
                        break;
                    }
                    Ok(n) => {
                        if tx.send((index, Ok(buffer[..n].to_vec()))).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = tx.send((index, Err(error.to_string())));
                        break;
                    }
                }
            }
        }));
    }
    drop(tx);
    let deadline = Instant::now() + Duration::from_secs(2);
    let result = (|| {
        let mut output = Output {
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        let mut closed = 0;
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err("查询已取消".into());
            }
            if Instant::now() >= deadline {
                return Err("查询超过 2 秒".into());
            }
            match rx.recv_timeout(Duration::from_millis(5)) {
                Ok((index, chunk)) => {
                    let chunk = chunk?;
                    if chunk.is_empty() {
                        closed += 1;
                    }
                    if output.stdout.len() + output.stderr.len() + chunk.len() > OUTPUT_LIMIT {
                        return Err("查询输出超过 256 KiB".into());
                    }
                    if index == 0 {
                        output.stdout.extend(chunk);
                    } else {
                        output.stderr.extend(chunk);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) if closed != 2 => {
                    return Err("查询输出读取中断".into());
                }
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
            if child.try_wait().map_err(|e| e.to_string())?.is_some() && closed == 2 {
                return Ok(output);
            }
        }
    })();
    let _ = child.kill();
    let _ = child.wait();
    #[cfg(windows)]
    drop(job);
    drop(rx);
    // Windows Job termination closes inherited pipe handles as well.
    #[cfg(windows)]
    for reader in readers {
        let _ = reader.join();
    }
    result
}

#[cfg(windows)]
fn resume_child(process_id: u32) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::*,
        System::{Diagnostics::ToolHelp::*, Threading::*},
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        let mut found = Thread32First(snapshot, &mut entry);
        let mut resumed = false;
        while found != 0 {
            if entry.th32OwnerProcessID == process_id {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if !thread.is_null() {
                    resumed = ResumeThread(thread) != u32::MAX;
                    CloseHandle(thread);
                }
                break;
            }
            found = Thread32Next(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        if resumed {
            Ok(())
        } else {
            Err("无法恢复帮助进程".into())
        }
    }
}
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Job {
    fn attach(child: &std::process::Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let job = Self(handle);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as _,
                std::mem::size_of_val(&info) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle() as _) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn run(script: &str) -> Result<Output, String> {
        let directory = tempfile::tempdir().unwrap();
        capture(
            &crate::pty::default_shell(),
            &[
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                script.into(),
            ],
            directory.path(),
            &AtomicBool::new(false),
        )
    }
    #[test]
    fn streams_remain_separate_and_share_one_output_budget() {
        let result =
            run("[Console]::Error.WriteLine('stderr'); [Console]::WriteLine('stdout'); exit 9")
                .unwrap();
        assert!(String::from_utf8_lossy(&result.stdout).contains("stdout"));
        assert!(String::from_utf8_lossy(&result.stderr).contains("stderr"));
        assert!(
            run("[Console]::Write('x'*150000); [Console]::Error.Write('y'*150000)")
                .unwrap_err()
                .contains("256")
        );
        assert!(
            run("while ($true) { [Console]::Write('x'*4096) }")
                .unwrap_err()
                .contains("256")
        );
    }
    #[test]
    fn cancellation_prevents_spawn_and_interrupts_running_capture() {
        let directory = tempfile::tempdir().unwrap();
        let cancelled = AtomicBool::new(true);
        assert!(
            capture(
                Path::new("nonexistent-test-program"),
                &[],
                directory.path(),
                &cancelled
            )
            .unwrap_err()
            .contains("取消")
        );
        cancelled.store(false, Ordering::Relaxed);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(150));
                cancelled.store(true, Ordering::Relaxed);
            });
            let start = Instant::now();
            let result = capture(
                &crate::pty::default_shell(),
                &[
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    "Start-Sleep -Seconds 15".into(),
                ],
                directory.path(),
                &cancelled,
            );
            assert!(result.unwrap_err().contains("取消"));
            assert!(start.elapsed() < Duration::from_secs(2));
        });
    }
    #[test]
    fn timeout_terminates_descendants() {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("child.pid");
        let script = format!(
            "$p = Start-Process -FilePath '{}' -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 30' -WindowStyle Hidden -PassThru; [IO.File]::WriteAllText('{}', [string]$p.Id); Start-Sleep -Seconds 15",
            crate::pty::default_shell().display(),
            pid_file.display()
        );
        assert!(run(&script).unwrap_err().contains("2 秒"));
        let pid: u32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let stopped = unsafe {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if handle.is_null() {
                    true
                } else {
                    let mut code = 259;
                    let success = GetExitCodeProcess(handle, &mut code) != 0;
                    CloseHandle(handle);
                    success && code != 259
                }
            };
            if stopped {
                break;
            }
            assert!(Instant::now() < deadline, "descendant survived the Job");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

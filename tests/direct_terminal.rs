#![cfg(all(windows, debug_assertions))]

//! Inherited-console tests run in an outer observing ConPTY, never a second
//! product ConPTY. Every session uses an isolated directory and no history.
#[path = "support/incremental_trace.rs"]
mod incremental_trace;
#[path = "support/workflow_cases.rs"]
mod workflow_cases;
use anyhow::{Context, Result, ensure};
use blueberry::probe::Harness;
use std::{collections::BTreeMap, time::Duration};
const TIMEOUT: Duration = Duration::from_secs(20);

fn workflow_trace(
    h: &mut Harness,
    trace: &mut incremental_trace::Trace,
) -> Result<incremental_trace::State> {
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        let _ = h.pump(Duration::from_millis(1));
        if trace.poll()? {
            return Ok(trace.state);
        }
        ensure!(
            std::time::Instant::now() < deadline,
            "trace reader fell behind"
        );
    }
}

fn workflow_text(h: &mut Harness, text: &str) -> Result<()> {
    // Exercise form input without blocking the terminal observer behind a
    // whole native-key batch. Burst/paste semantics have separate regressions.
    for character in text.chars() {
        h.send_text(&character.to_string())?;
        let _ = h.pump(Duration::from_millis(1));
    }
    Ok(())
}

fn direct_fixture() -> Result<tempfile::TempDir> {
    // Keep the live fixture outside repository ancestry, and retain both
    // passing and failing traces at a path uploaded by Windows CI.
    let evidence = std::env::var_os("RUNNER_TEMP")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("blueberry-direct-evidence");
    std::fs::create_dir_all(&evidence)?;
    let mut directory = tempfile::Builder::new()
        .prefix("direct-")
        .tempdir_in(evidence)?;
    directory.disable_cleanup(true);
    Ok(directory)
}

#[path = "support/console_snapshot.rs"]
mod console_snapshot;

#[test]
#[ignore = "isolated console snapshot helper"]
fn console_snapshot_helper() -> Result<()> {
    console_snapshot::write_attached()
}

fn capture_live_console(h: &mut Harness, path: &std::path::Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .args(["console_snapshot_helper", "--exact", "--ignored"])
        .env(
            "BLUEBERRY_SNAPSHOT_PID",
            h.process_id().context("missing console PID")?.to_string(),
        )
        .env("BLUEBERRY_SNAPSHOT_PATH", path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .spawn()?;
    // PSReadLine may request cursor position during resize. Keep answering
    // while the snapshot helper starts; a late CPR can become editor input.
    let deadline = std::time::Instant::now() + TIMEOUT;
    while child.try_wait()?.is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("console snapshot timed out");
        }
        let _ = h.pump(Duration::from_millis(5));
    }
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "console snapshot failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn wait_native_selected_path(
    h: &mut Harness,
    path: &std::path::Path,
    name: &str,
) -> Result<String> {
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        capture_live_console(h, path)?;
        let native: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let screen = native["screen"]
            .as_str()
            .context("native viewport missing")?;
        if screen
            .lines()
            .any(|row| row.contains('›') && row.contains(name))
        {
            return Ok(screen.to_owned());
        }
        // The VT observer can already contain a frame while a later async
        // completion is clearing/repainting the native buffer. Observe the
        // selected state in this buffer too, rather than sampling it once.
        ensure!(
            std::time::Instant::now() < deadline,
            "native selection never reached {name}: {screen}"
        );
        let _ = h.pump(Duration::from_millis(10));
    }
}

fn wait_output_line(h: &mut Harness, expected: &str) -> Result<()> {
    let deadline = std::time::Instant::now() + TIMEOUT;
    while !h
        .viewport_contents()
        .lines()
        .any(|line| line.trim() == expected)
    {
        ensure!(
            std::time::Instant::now() < deadline,
            "missing output line {expected}: {}",
            h.viewport_contents()
        );
        let _ = h.pump(Duration::from_millis(10));
    }
    Ok(())
}

fn send_unicode_input(h: &mut Harness, input: &str) -> Result<()> {
    if input.is_ascii() {
        return h.send(input.as_bytes());
    }
    // Compare exact UTF-16 native editor input. Old ConPTY clamps CSI values
    // to 32767 (terminal#12977); raw UTF-8 synthesis instead depends on the
    // code page temporarily restored by PSReadLine. Wire decoding retains its
    // separate input_protocol and parser coverage; this tests editing semantics.
    use std::os::windows::process::CommandExt;
    let result = std::process::Command::new(std::env::current_exe()?)
        .args(["native_unicode_input_helper", "--exact", "--ignored"])
        .env(
            "BLUEBERRY_UNICODE_CONSOLE_PID",
            h.process_id().context("missing console owner")?.to_string(),
        )
        .env("BLUEBERRY_UNICODE_INPUT", input)
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .output()?;
    ensure!(
        result.status.success(),
        "native Unicode fixture failed: {} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

#[test]
#[ignore = "child helper invoked by native Unicode editing regressions"]
fn native_unicode_input_helper() -> Result<()> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{CreateFileW, OPEN_EXISTING},
        System::Console::{
            AttachConsole, FreeConsole, INPUT_RECORD, KEY_EVENT, WriteConsoleInputW,
        },
    };
    let pid: u32 = std::env::var("BLUEBERRY_UNICODE_CONSOLE_PID")?.parse()?;
    let text = std::env::var("BLUEBERRY_UNICODE_INPUT")?;
    unsafe {
        FreeConsole();
    }
    ensure!(
        unsafe { AttachConsole(pid) } != 0,
        "AttachConsole: {}",
        std::io::Error::last_os_error()
    );
    let name: Vec<u16> = "CONIN$".encode_utf16().chain(Some(0)).collect();
    let input = unsafe {
        CreateFileW(
            name.as_ptr(),
            0xc0000000,
            3,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    ensure!(input != INVALID_HANDLE_VALUE, "open native input failed");
    let mut records = Vec::new();
    for unit in text.encode_utf16() {
        for down in [1, 0] {
            let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
            record.EventType = KEY_EVENT as u16;
            record.Event.KeyEvent.bKeyDown = down;
            record.Event.KeyEvent.wRepeatCount = 1;
            record.Event.KeyEvent.wVirtualKeyCode = if unit == 13 { 13 } else { 0 };
            record.Event.KeyEvent.uChar.UnicodeChar = unit;
            records.push(record);
        }
    }
    let mut written = 0;
    let success =
        unsafe { WriteConsoleInputW(input, records.as_ptr(), records.len() as u32, &mut written) };
    unsafe {
        CloseHandle(input);
        FreeConsole();
    }
    ensure!(
        success != 0 && written as usize == records.len(),
        "native input write incomplete"
    );
    Ok(())
}

#[test]
#[ignore = "isolated output timing experiment; never part of acceptance"]
fn console_output_transport_experiment() -> Result<()> {
    let mut samples = Vec::new();
    for mode in 0..3 {
        let dir = direct_fixture()?;
        let script = dir.path().join("experiment.ps1");
        let log = dir.path().join("qpc.csv");
        std::fs::write(&script, include_str!("output-fence.ps1"))?;
        let mut h = Harness::start(
            &blueberry::pty::default_shell(),
            &[
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NoExit".into(),
                "-File".into(),
                script.to_string_lossy().into_owned(),
                "-Log".into(),
                log.to_string_lossy().into_owned(),
                "-Mode".into(),
                mode.to_string(),
            ],
            dir.path(),
            &BTreeMap::from([("BLUEBERRY_PROBE_QPC".into(), "1".into())]),
            String::new(),
        )?;
        h.wait_line("OUTPUT-EXPERIMENT-READY", TIMEOUT)?;
        h.wait_text("PS ", TIMEOUT)?;
        for sequence in 1..=31 {
            h.send(b"\x1b[24~")?;
            h.wait_text(&format!("BB-FRAME-{sequence:04}"), TIMEOUT)?;
            let arrival = h.last_output_qpc().context("missing observer QPC")?;
            // Log writes follow console writes and may finish after observation.
            let deadline = std::time::Instant::now() + TIMEOUT;
            let record = loop {
                let contents = std::fs::read_to_string(&log).unwrap_or_default();
                if let Some(line) = contents
                    .lines()
                    .find(|line| line.starts_with(&format!("{sequence},")))
                {
                    let values = line
                        .split(',')
                        .map(str::parse::<i64>)
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    if values.len() == 4 {
                        break values;
                    }
                }
                ensure!(
                    std::time::Instant::now() < deadline,
                    "output timestamp missing"
                );
                std::thread::sleep(Duration::from_millis(1));
            };
            if sequence > 1 {
                samples.push(serde_json::json!({"mode":mode,"sample":sequence-2,"start_qpc":record[1],"written_qpc":record[2],"arrival_qpc":arrival,"frequency":record[3],"write_to_arrival_ms":(arrival-record[2]) as f64*1000.0/record[3] as f64}));
            }
        }
        h.finish(TIMEOUT)?;
    }
    std::fs::create_dir_all("target/editor-explore")?;
    std::fs::write(
        "target/editor-explore/output-fence.json",
        serde_json::to_vec_pretty(&samples)?,
    )?;
    Ok(())
}

#[test]
fn compiled_bridge_codec_runs_in_selected_shell() -> Result<()> {
    let dir = direct_fixture()?;
    let bootstrap = blueberry::host::direct::ensure_bootstrap(dir.path())?;
    let script = dir.path().join("codec.ps1");
    std::fs::write(
        &script,
        format!("\u{feff}{}", include_str!("direct-bridge.tests.ps1")),
    )?;
    std::fs::write(
        dir.path().join("direct-diagnostics.json"),
        include_str!("../shell/direct-diagnostics.json"),
    )?;
    let result = std::process::Command::new(blueberry::pty::default_shell())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-AssemblyPath")
        .arg(bootstrap.with_file_name("direct-bridge.dll"))
        .output()?;
    ensure!(
        result.status.success(),
        "compiled codec failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

#[test]
fn plain_psreadline_preserves_the_same_unicode_input() -> Result<()> {
    let dir = direct_fixture()?;
    let module = std::env::var("BLUEBERRY_TEST_PSREADLINE_MODULE")
        .unwrap_or("PSReadLine".into())
        .replace('\'', "''");
    let mut harness = Harness::start(
        &blueberry::pty::default_shell(),
        &[
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NoExit".into(),
            "-Command".into(),
            format!("Import-Module '{module}'; Set-PSReadLineOption -HistorySaveStyle SaveNothing"),
        ],
        dir.path(),
        &BTreeMap::from([("BLUEBERRY_NO_HISTORY".into(), "1".into())]),
        String::new(),
    )?;
    harness.wait_text("PS ", TIMEOUT)?;
    harness.send("Write-Output '中文😀👩‍💻终端'\r".as_bytes())?;
    harness.wait_line("中文😀👩‍💻终端", TIMEOUT)?;
    harness.finish(TIMEOUT)
}

fn start(directory: &std::path::Path) -> Result<Harness> {
    start_with_delay(directory, 0)
}

fn start_with_delay(directory: &std::path::Path, delay: u32) -> Result<Harness> {
    let executable = std::env::var_os("BLUEBERRY_TEST_PRODUCT_EXE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_blueberry").into());
    start_delayed_executable(directory, delay, &executable)
}

fn start_delayed_executable(
    directory: &std::path::Path,
    delay: u32,
    executable: &std::path::Path,
) -> Result<Harness> {
    start_delayed_executable_with_pages(directory, delay, executable, false)
}

fn start_delayed_executable_with_pages(
    directory: &std::path::Path,
    delay: u32,
    executable: &std::path::Path,
    terminal_pages: bool,
) -> Result<Harness> {
    let mut args = vec![
        "run".into(),
        "--host-mode".into(),
        "direct".into(),
        "--shell".into(),
        blueberry::pty::default_shell()
            .to_string_lossy()
            .into_owned(),
        "--no-profile".into(),
        "--data-dir".into(),
        directory.to_string_lossy().into_owned(),
        "--trace".into(),
        directory.join("trace.jsonl").to_string_lossy().into_owned(),
    ];
    let trace_disabled = directory.join("trace-disabled").is_file();
    if trace_disabled {
        args.truncate(args.len() - 2);
    }
    if directory.join("config.toml").is_file() {
        args.extend([
            "--config".into(),
            directory.join("config.toml").to_string_lossy().into_owned(),
        ]);
    }
    let mut harness = Harness::start(
        executable,
        &args,
        directory,
        &BTreeMap::from([
            ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
            ("TERM".into(), "xterm-256color".into()),
            ("BLUEBERRY_TEST_FRAME_DELAY_MS".into(), delay.to_string()),
            (
                "BLUEBERRY_TEST_UI_WAKE_DELAY_MS".into(),
                if directory.join("delay-ui-wake").exists() {
                    "600"
                } else {
                    "0"
                }
                .into(),
            ),
            (
                "BLUEBERRY_TEST_OMIT_MENU_OWNERSHIP".into(),
                if directory.join("omit-menu-ownership").exists() {
                    "1"
                } else {
                    "0"
                }
                .into(),
            ),
            (
                "BLUEBERRY_TEST_TERMINAL_PAGES".into(),
                if terminal_pages { "1" } else { "0" }.into(),
            ),
        ]),
        String::new(),
    )?;
    if trace_disabled {
        harness.wait_text("PS ", TIMEOUT)?;
    } else {
        wait_editor_begin(&mut harness, directory, 1)?;
    }
    Ok(harness)
}

#[test]
fn unknown_diagnostic_events_do_not_disable_completion() -> Result<()> {
    for tracing in [true, false] {
        let dir = direct_fixture()?;
        if !tracing {
            std::fs::write(dir.path().join("trace-disabled"), "1")?;
        }
        // Deliberately bypass the sender validator to exercise receiver policy.
        std::fs::write(
            dir.path().join("diagnostics.ps1"),
            r#"
$flags=[Reflection.BindingFlags]'NonPublic,Static'
$bridge=[Blueberry.Direct.Bridge]
$pipe=$bridge.GetField('pipe',$flags).GetValue($null)
$token=$bridge.GetField('token',$flags).GetValue($null)
$gate=$bridge.GetField('writeLock',$flags).GetValue($null)
foreach ($kind in @('trace','trace_point')) {
    foreach ($stage in @('future_diagnostic','editor_resize_reflow')) {
        $message=[Collections.Generic.Dictionary[string,object]]::new()
        $message['event']=$kind; $message['stage']=$stage; $message['token']=$token
        $message['qpc']='invalid'
        $bytes=[Text.Encoding]::UTF8.GetBytes([Blueberry.Direct.Json]::Encode($message)+"`n")
        [Threading.Monitor]::Enter($gate)
        try { $pipe.Write($bytes,0,$bytes.Length); $pipe.Flush() } finally { [Threading.Monitor]::Exit($gate) }
    }
}
Write-Output 'DIAGNOSTICS-INJECTED'
"#,
        )?;
        let mut h = start(dir.path())?;
        let result = (|| -> Result<()> {
            h.send_text(". .\\diagnostics.ps1\r")?;
            h.wait_line("DIAGNOSTICS-INJECTED", TIMEOUT)?;
            h.send_text("git sw")?;
            h.wait_text("switch", TIMEOUT)?;
            h.send(b"\t")?;
            h.wait_text("git switch", TIMEOUT)?;
            h.finish(TIMEOUT)
        })();
        if let Err(error) = result {
            let _ = h.save_evidence(dir.path());
            let _ = h.stop();
            return Err(error).with_context(|| {
                format!("diagnostics tracing={tracing}: {}", dir.keep().display())
            });
        }
    }
    Ok(())
}

#[test]
#[ignore = "isolated fixed-machine 5 ms wake qualification; requires explicit artifact and output directory"]
fn direct_wake_latency_qualification() -> Result<()> {
    use sha2::{Digest, Sha256};
    let executable = std::path::PathBuf::from(std::env::var("BLUEBERRY_QUALIFICATION_EXE")?);
    let output = std::path::PathBuf::from(std::env::var("BLUEBERRY_QUALIFICATION_OUTPUT")?);
    ensure!(!output.exists(), "qualification output must be new");
    std::fs::create_dir_all(&output)?;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryPerformanceFrequency(value: *mut i64) -> i32;
    }
    let mut frequency = 0i64;
    ensure!(
        unsafe { QueryPerformanceFrequency(&mut frequency) } != 0,
        "QPC frequency unavailable"
    );
    let identity = std::process::Command::new(&executable)
        .args(["doctor", "--json"])
        .output()?;
    ensure!(identity.status.success(), "candidate doctor failed");
    std::fs::write(output.join("identity.json"), identity.stdout)?;
    std::fs::write(
        output.join("artifacts.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "executable":executable,"executable_sha256":format!("{:x}",Sha256::digest(std::fs::read(&executable)?)),
            "probe_sha256":format!("{:x}",Sha256::digest(std::fs::read(std::env::current_exe()?)?)),
            "shell":blueberry::pty::default_shell(),"trace_enabled":true
        }))?,
    )?;
    let mut results = Vec::new();
    let mut passed = true;
    for delay in [0, 3, 5, 10, 40, 100] {
        let directory = output.join(format!("delay-{delay}"));
        std::fs::create_dir(&directory)?;
        let mut h = start_delayed_executable(&directory, delay, &executable)?;
        let mut last_revision = -1;
        for _ in 0..30 {
            h.send(b"\x01\x7fgit sw")?;
            let deadline = std::time::Instant::now() + TIMEOUT;
            loop {
                let trace =
                    std::fs::read_to_string(directory.join("trace.jsonl")).unwrap_or_default();
                let revision = trace
                    .lines()
                    .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                    .filter(|event| event["event"] == "editor_menu_written")
                    .filter_map(|event| event["revision"].as_i64())
                    .max()
                    .unwrap_or(-1);
                if revision > last_revision {
                    last_revision = revision;
                    break;
                }
                ensure!(
                    std::time::Instant::now() < deadline,
                    "idle editor failed to refresh; evidence at {}",
                    directory.display()
                );
                let _ = h.pump(Duration::from_millis(5));
            }
        }
        h.send(b"\x01\x7f")?;
        h.finish(TIMEOUT)?;
        let status = std::fs::read_dir(&directory)?
            .filter_map(Result::ok)
            .find_map(|entry| std::fs::read(entry.path().join("adapter.json")).ok())
            .context("missing actual editor identity")?;
        let status: serde_json::Value = serde_json::from_slice(&status)?;
        ensure!(
            status["editor_mode"] == "editor_hooks_v1",
            "fallback is not qualified"
        );
        let trace = std::fs::read_to_string(directory.join("trace.jsonl"))?;
        let events: Vec<serde_json::Value> = trace
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .collect();
        let receipts: BTreeMap<_, _> = events
            .iter()
            .filter(|event| event["event"] == "editor_frame_received")
            .map(|event| {
                (
                    (event["revision"].as_u64(), event["frame_id"].as_u64()),
                    event["qpc"].as_i64().unwrap(),
                )
            })
            .collect();
        let mut waits = Vec::new();
        for event in events
            .iter()
            .filter(|event| event["event"] == "editor_menu_written")
        {
            let key = (event["revision"].as_u64(), event["frame_id"].as_u64());
            let received = receipts
                .get(&key)
                .context("paint has no matching receive timestamp")?;
            waits.push(
                (event["qpc"].as_i64().context("missing paint timestamp")? - received) as f64
                    * 1000.0
                    / frequency as f64,
            );
        }
        ensure!(waits.len() >= 30, "missing qualified samples");
        waits.sort_by(f64::total_cmp);
        let p95 = waits[((waits.len() as f64 * 0.95).ceil() as usize).saturating_sub(1)];
        passed &= p95 <= 5.0;
        results.push(
            serde_json::json!({"delay_ms":delay,"samples":waits,"qpc_frequency":frequency,"endpoint":"editor_menu_written","p95_ms":p95,"passed":p95<=5.0}),
        );
        std::fs::write(
            output.join("results.json"),
            serde_json::to_vec_pretty(&results)?,
        )?;
    }
    ensure!(
        passed,
        "receive-to-paint P95 exceeded 5 ms; all samples retained"
    );
    Ok(())
}

fn wait_editor_begin(h: &mut Harness, directory: &std::path::Path, count: usize) -> Result<()> {
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        let trace = std::fs::read_to_string(directory.join("trace.jsonl")).unwrap_or_default();
        if trace
            .lines()
            .filter(|line| line.contains("\"direct_readline_begin\""))
            .count()
            >= count
        {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            let events: Vec<serde_json::Value> = trace
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            let adapter = std::fs::read_dir(directory)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .find_map(|entry| std::fs::read(entry.path().join("adapter.json")).ok())
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            let disabled = adapter.as_ref().and_then(|a| a["disabled_reason"].as_str());
            let hello = events.iter().rev().find(|e| e["event"] == "direct_hello");
            let last_frame = events
                .iter()
                .rev()
                .find(|e| e["event"] == "editor_menu_written");
            let classification = if disabled.is_some() {
                "service_disabled"
            } else if hello.is_none() {
                "handshake_not_observed"
            } else {
                "readline_begin_not_observed"
            };
            let _ = h.save_evidence(directory);
            let native_capture =
                console_snapshot::capture(h, &directory.join("readline-timeout-native.json"))
                    .err()
                    .map(|error| format!("{error:#}"));
            std::fs::write(
                directory.join("readline-timeout.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "classification": classification,
                    "expected_begin_count": count,
                    "observed_begin_count": events.iter().filter(|e| e["event"] == "direct_readline_begin").count(),
                    "timeout_seconds": TIMEOUT.as_secs(),
                    "process": h.process_status(),
                    "adapter": adapter,
                    "handshake": hello,
                    "last_painted_frame": last_frame,
                    "trace_tail": &events[events.len().saturating_sub(80)..],
                    "native_capture_error": native_capture,
                }))?,
            )?;
            anyhow::bail!(
                "expected ReadLine trace not observed within 20 seconds ({classification}); evidence: {}",
                directory.display()
            );
        }
        let _ = h.pump(Duration::from_millis(5));
    }
}

#[test]
fn direct_empty_up_keeps_native_history_until_a_real_edit() -> Result<()> {
    for delay in [0, 40] {
        let dir = direct_fixture()?;
        let mut h = start_with_delay(dir.path(), delay)?;
        let result = (|| -> Result<()> {
            h.send(b"function global:TabExpansion2 { param($inputScript,$cursorColumn,$options) $m=[Collections.ObjectModel.Collection[Management.Automation.CompletionResult]]::new(); $m.Add([Management.Automation.CompletionResult]::new('git status','git status','Command','fixture')); [Management.Automation.CommandCompletion]::new($m,0,0,$inputScript.Length) }; [Microsoft.PowerShell.PSConsoleReadLine]::AddToHistory('git sw'); [Microsoft.PowerShell.PSConsoleReadLine]::AddToHistory('git st')\r")?;
            wait_editor_begin(&mut h, dir.path(), 2)?;
            for (key, expected) in [
                (b"\x1b[A".as_slice(), "git st"),
                (b"\x1b[A".as_slice(), "git sw"),
                (b"\x1b[B".as_slice(), "git st"),
            ] {
                h.send(key)?;
                let deadline = std::time::Instant::now() + TIMEOUT;
                loop {
                    let _ = h.pump(Duration::from_millis(10));
                    let screen = h.screen_snapshot();
                    let row = screen
                        .rows(0, screen.size().1)
                        .nth(screen.cursor_position().0 as usize)
                        .unwrap_or_default();
                    if row.contains(expected) {
                        break;
                    }
                    ensure!(
                        std::time::Instant::now() < deadline,
                        "native history did not reach {expected}"
                    );
                }
                let _ = h.pump(Duration::from_millis(350));
                ensure!(
                    !h.screen_snapshot().contents().contains("› "),
                    "history recall reopened Blueberry completion"
                );
            }
            for key in [
                b"\x1b[D".as_slice(),
                b"\x1b[H",
                b"\x7f",
                b"\t",
                b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_",
            ] {
                h.send(key)?;
                let until = std::time::Instant::now() + Duration::from_millis(350);
                while std::time::Instant::now() < until {
                    let _ = h.pump(Duration::from_millis(10));
                }
                if key == b"\t" {
                    let until = std::time::Instant::now() + TIMEOUT;
                    loop {
                        let screen = h.screen_snapshot();
                        let row = screen
                            .rows(0, screen.size().1)
                            .nth(screen.cursor_position().0 as usize)
                            .unwrap_or_default();
                        if row.contains("git status") {
                            break;
                        }
                        ensure!(
                            std::time::Instant::now() < until,
                            "native Tab did not complete the fixture"
                        );
                        let _ = h.pump(Duration::from_millis(10));
                    }
                }
                ensure!(
                    !h.screen_snapshot().contents().contains("› "),
                    "native history key reopened completion: {key:?}"
                );
            }
            h.send(b"\x03")?;
            h.wait_text("^C", TIMEOUT)?;
            wait_editor_begin(&mut h, dir.path(), 3)?;
            h.send(b"\x1b[A")?;
            let _ = h.pump(Duration::from_millis(350));
            h.send(b"\x1b[F\x7f")?;
            h.wait_text("› ", TIMEOUT)?;
            h.send(b"\x03")?;
            h.finish(TIMEOUT)
        })();
        if let Err(error) = result {
            let _ = h.save_evidence(dir.path());
            let _ = h.stop();
            return Err(error)
                .with_context(|| format!("native history regression: {}", dir.keep().display()));
        }
    }
    Ok(())
}

#[test]
fn direct_history_respects_disabled_policy_and_custom_up_binding() -> Result<()> {
    for custom in [false, true] {
        let dir = direct_fixture()?;
        if !custom {
            std::fs::write(
                dir.path().join("config.toml"),
                "[completion]\nup_arrow_history = false\n",
            )?;
        }
        let mut h = start(dir.path())?;
        let result = (|| -> Result<()> {
            if custom {
                h.send(b"Set-PSReadLineKeyHandler -Chord UpArrow -ScriptBlock { [Microsoft.PowerShell.PSConsoleReadLine]::Insert('git st') }\r")?;
            } else {
                h.send(b"[Microsoft.PowerShell.PSConsoleReadLine]::AddToHistory('git st')\r")?;
            }
            wait_editor_begin(&mut h, dir.path(), 2)?;
            h.send(b"\x1b[A")?;
            h.wait_text("› ", TIMEOUT)?;
            ensure!(
                h.screen_snapshot().contents().contains("git st"),
                "Up binding text changed"
            );
            h.send(b"\x03")?;
            h.finish(TIMEOUT)
        })();
        if let Err(error) = result {
            let _ = h.save_evidence(dir.path());
            let _ = h.stop();
            return Err(error).with_context(|| {
                format!("history policy/custom binding: {}", dir.keep().display())
            });
        }
    }
    Ok(())
}

#[test]
fn direct_late_frames_wake_editor_without_another_key_or_character_bindings() -> Result<()> {
    let evidence =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/input-protocol-evidence");
    std::fs::create_dir_all(&evidence)?;
    for delay in [0, 3, 5, 10, 40, 100] {
        let dir = tempfile::Builder::new()
            .prefix("direct-wake-")
            .tempdir_in(&evidence)?;
        let mut h = start_with_delay(dir.path(), delay)?;
        h.wait_text("PS ", TIMEOUT)?;
        h.send(b"if ([Blueberry.Direct.Bridge]::EditorHooks -and @(Get-PSReadLineKeyHandler).Count -lt 1000) { Write-Output 'HOOKS-VERIFIED' }\r")?;
        wait_output_line(&mut h, "HOOKS-VERIFIED")?;
        wait_editor_begin(&mut h, dir.path(), 2)?;
        h.send(b"git sw")?;
        if let Err(error) = h.wait_text("switch", TIMEOUT) {
            let retained = dir.keep();
            return Err(error).with_context(|| {
                format!(
                    "delay {delay} ms evidence retained at {}",
                    retained.display()
                )
            });
        }
        let status = std::fs::read_dir(dir.path())?
            .filter_map(Result::ok)
            .find_map(|entry| std::fs::read(entry.path().join("adapter.json")).ok())
            .context("missing actual editor status")?;
        let status: serde_json::Value = serde_json::from_slice(&status)?;
        ensure!(
            status["editor_mode"] == "editor_hooks_v1",
            "test silently used legacy editor"
        );
        h.send(b"\x01\x7f")?;
        h.finish(TIMEOUT)?;
        let trace = std::fs::read_to_string(dir.path().join("trace.jsonl"))?;
        let waits: Vec<_> = trace
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|event| event["event"] == "direct_frame_to_paint")
            .filter_map(|event| event["duration_ms"].as_f64())
            .collect();
        // This is a liveness regression with generous CI scheduling tolerance.
        // The stricter 5 ms P95 belongs to the isolated performance measurement.
        if waits.is_empty() || waits.iter().any(|wait| *wait >= 200.0) {
            let retained = dir.keep();
            anyhow::bail!(
                "receive-to-paint regression at delay {delay} ms: {waits:?}; all trace events retained at {}",
                retained.display()
            );
        }
        eprintln!("frame delay {delay} ms: receive-to-paint {waits:?}");
    }
    Ok(())
}

#[test]
fn direct_empty_line_cancels_menu_and_disabled_autotrigger_preserves_typing() -> Result<()> {
    for automatic in [true, false] {
        let dir = direct_fixture()?;
        if !automatic {
            std::fs::write(
                dir.path().join("config.toml"),
                "[completion]\nauto_trigger = false\n",
            )?;
        }
        let mut h = start(dir.path())?;
        h.wait_text("PS ", TIMEOUT)?;
        h.send(b"git sw")?;
        if automatic {
            h.wait_text("switch", TIMEOUT)?;
        } else {
            h.wait_text("git sw", TIMEOUT)?;
        }
        h.send(b"\x01\x7f")?;
        let deadline = std::time::Instant::now() + Duration::from_millis(400);
        while std::time::Instant::now() < deadline {
            let _ = h.pump(Duration::from_millis(25));
        }
        ensure!(
            !h.viewport_contents().contains("› "),
            "empty/disabled auto query left command menu visible"
        );
        if !automatic {
            let status = std::fs::read_dir(dir.path())?
                .filter_map(Result::ok)
                .find_map(|entry| std::fs::read(entry.path().join("adapter.json")).ok())
                .context("missing disabled-menu session status")?;
            let status: serde_json::Value = serde_json::from_slice(&status)?;
            ensure!(
                status["automatic_menu"] == false
                    && status["disabled_reason"] == "completion.auto_trigger is disabled",
                "doctor status ignored automatic-menu configuration"
            );
        }
        h.send(b"Write-Output 'empty-edit-restored'\r")?;
        h.wait_line("empty-edit-restored", TIMEOUT)?;
        h.finish(TIMEOUT)?;
    }
    Ok(())
}

#[test]
fn direct_menu_accept_is_fill_only_and_shell_still_edits_unicode() -> Result<()> {
    let dir = direct_fixture()?;
    let mut harness = start(dir.path())?;
    harness.wait_text("PS ", TIMEOUT)?;
    send_unicode_input(&mut harness, "Write-Output '中文😀👩‍💻终端'\r")?;
    wait_output_line(&mut harness, "中文😀👩‍💻终端")?;
    wait_editor_begin(&mut harness, dir.path(), 2)?;
    harness.send(b"git sw")?;
    if let Err(error) = harness.wait_text("switch", TIMEOUT) {
        let statuses = std::fs::read_dir(dir.path())?
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::read_to_string(entry.path().join("adapter.json")).ok())
            .collect::<Vec<_>>();
        anyhow::bail!(
            "{error:#}; adapter status: {statuses:?}; numeric trace: {}",
            std::fs::read_to_string(dir.path().join("trace.jsonl")).unwrap_or_default()
        );
    }
    ensure!(
        !harness.contents().contains("已停用"),
        "bridge disabled: {}",
        harness.contents()
    );
    harness.send(b"\t")?;
    harness.wait_text("git switch", TIMEOUT).map_err(|error| {
        anyhow::anyhow!(
            "{error:#}; trace: {}",
            std::fs::read_to_string(dir.path().join("trace.jsonl")).unwrap_or_default()
        )
    })?;
    harness.send(b"\x01\x7f")?;
    send_unicode_input(&mut harness, "Write-Output '中文😀👩‍💻终端'")?;
    harness.send(b"\r")?;
    wait_output_line(&mut harness, "中文😀👩‍💻终端")?;
    harness.finish(TIMEOUT)
}

#[test]
fn direct_runtime_custom_binding_is_preserved() -> Result<()> {
    let dir = direct_fixture()?;
    let mut harness = start(dir.path())?;
    harness.wait_text("PS ", TIMEOUT)?;
    harness.send(b"Set-PSReadLineKeyHandler -Chord g -ScriptBlock { [Microsoft.PowerShell.PSConsoleReadLine]::Insert('CUSTOM') }; Write-Output 'binding-installed'\r")?;
    harness.wait_line("binding-installed", TIMEOUT)?;
    harness.send(b"g")?;
    harness.wait_text("CUSTOM", TIMEOUT)?;
    harness.send(b"\x01\x7f")?;
    harness.finish(TIMEOUT)
}

#[test]
fn direct_runtime_builtin_rebinding_takes_priority_over_menu_accept() -> Result<()> {
    let dir = direct_fixture()?;
    let mut h = start(dir.path())?;
    h.wait_text("PS ", TIMEOUT)?;
    h.send(
        b"Set-PSReadLineKeyHandler -Chord Tab -Function BackwardChar; Write-Output 'TAB-REBOUND'\r",
    )?;
    h.wait_line("TAB-REBOUND", TIMEOUT)?;
    h.send(b"git sw")?;
    h.wait_text("switch", TIMEOUT)?;
    h.send(b"\tX")?;
    h.wait_text("git sXw", TIMEOUT)?;
    h.send(b"\x01\x7f")?;
    h.finish(TIMEOUT)
}

#[test]
fn direct_file_entry_runs_profile_once_and_preserves_profile_state() -> Result<()> {
    let dir = direct_fixture()?;
    let user = dir.path().join("user profile");
    for edition in ["PowerShell", "WindowsPowerShell"] {
        let documents = user.join("Documents").join(edition);
        std::fs::create_dir_all(&documents)?;
        std::fs::write(
            documents.join("Microsoft.PowerShell_profile.ps1"),
            r#"
$global:BlueberryProfileCount++
function global:bb-profile-function { 'PROFILE-FUNCTION' }
Set-Alias -Name bb-profile-alias -Value bb-profile-function -Scope Global
Set-PSReadLineKeyHandler -Chord Ctrl+g -ScriptBlock { [Microsoft.PowerShell.PSConsoleReadLine]::Insert('PROFILE_BINDING') }
# A second manifest record must not replace the actual cmdlet/assembly identity.
New-Module -Name PSReadLine -ScriptBlock {} | Import-Module -Global
$global:BlueberryProfileModuleCount = @(Get-Module PSReadLine).Count
$global:LASTEXITCODE = 37
"#,
        )?;
    }
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data)?;
    let mut h = Harness::start(
        std::path::Path::new(env!("CARGO_BIN_EXE_blueberry")),
        &[
            "run".into(),
            "--host-mode".into(),
            "direct".into(),
            "--shell".into(),
            blueberry::pty::default_shell()
                .to_string_lossy()
                .into_owned(),
            "--data-dir".into(),
            data.to_string_lossy().into_owned(),
            "--trace".into(),
            data.join("trace.jsonl").to_string_lossy().into_owned(),
        ],
        dir.path(),
        &BTreeMap::from([
            ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
            ("USERPROFILE".into(), user.to_string_lossy().into_owned()),
            (
                "APPDATA".into(),
                dir.path().join("roaming").to_string_lossy().into_owned(),
            ),
            ("TERM".into(), "xterm-256color".into()),
        ]),
        String::new(),
    )?;
    wait_editor_begin(&mut h, &data, 1)?;
    h.send_text("bb-profile-functio")?;
    h.wait_text("bb-profile-function", TIMEOUT)?;
    h.send(b"\x01\x7fbb-profile-alia")?;
    h.wait_text("bb-profile-alias", TIMEOUT)?;
    h.send(b"\x01\x7f")?;
    h.send(b"[Console]::WriteLine(('PROFILE:{0}:{1}' -f $global:BlueberryProfileCount, $LASTEXITCODE))\r")?;
    wait_output_line(&mut h, "PROFILE:1:37")?;
    h.send_text("if ($global:BlueberryProfileModuleCount -ge 2) { Write-Output 'MULTIPLE-MODULE-RECORDS' }\r")?;
    wait_output_line(&mut h, "MULTIPLE-MODULE-RECORDS")?;
    h.send(b"\x07")?;
    h.wait_text("PROFILE_BINDING", TIMEOUT)?;
    h.send(b"\x01\x7f")?;
    h.finish(TIMEOUT)
}

#[test]
fn direct_preserves_command_status_across_prompt_lifecycle() -> Result<()> {
    let dir = direct_fixture()?;
    let mut h = start(dir.path())?;
    h.wait_text("PS ", TIMEOUT)?;
    h.send(b"cmd.exe /d /c exit 7\r")?;
    // Status is expanded by the first next command, after ReadLine has returned.
    h.send(b"[Console]::WriteLine(('STATUS:{0}:{1}' -f $?, $LASTEXITCODE))\r")?;
    h.wait_line("STATUS:False:7", TIMEOUT)?;
    h.finish(TIMEOUT)
}

#[test]
fn direct_preserves_standard_module_autoload_across_shell_editions() -> Result<()> {
    let dir = direct_fixture()?;
    let mut h = start(dir.path())?;
    h.send(b"if ((Get-Command Get-FileHash -ErrorAction Stop).Source -eq 'Microsoft.PowerShell.Utility' -and (Get-Command ConvertTo-Json -ErrorAction Stop)) { Write-Output 'MODULE-AUTOLOAD-OK' }\r")?;
    wait_output_line(&mut h, "MODULE-AUTOLOAD-OK")?;
    h.finish(TIMEOUT)
}

#[test]
fn direct_autoloads_a_module_under_a_sql_style_powershell_directory() -> Result<()> {
    let dir = direct_fixture()?;
    let modules = dir.path().join("SQL/Tools/PowerShell/Modules");
    let module = modules.join("BlueberrySqlRootFixture");
    std::fs::create_dir_all(&module)?;
    std::fs::write(
        module.join("BlueberrySqlRootFixture.psm1"),
        "function Get-BlueberrySqlRootFixture { 'SQL-MODULE-AUTOLOAD-OK' }; Export-ModuleMember -Function Get-BlueberrySqlRootFixture",
    )?;
    let executable = std::env::var_os("BLUEBERRY_TEST_PRODUCT_EXE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_blueberry").into());
    let mut paths = vec![modules];
    if let Some(inherited) = std::env::var_os("PSModulePath") {
        paths.extend(std::env::split_paths(&inherited));
    }
    let paths = std::env::join_paths(paths)?.to_string_lossy().into_owned();
    let mut h = Harness::start(
        &executable,
        &[
            "run".into(),
            "--host-mode".into(),
            "direct".into(),
            "--no-profile".into(),
            "--shell".into(),
            blueberry::pty::default_shell()
                .to_string_lossy()
                .into_owned(),
            "--data-dir".into(),
            dir.path().to_string_lossy().into_owned(),
        ],
        dir.path(),
        &BTreeMap::from([
            ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
            ("PSModulePath".into(), paths.clone()),
            ("WinPSModulePath".into(), paths),
        ]),
        String::new(),
    )?;
    h.wait_text("PS ", TIMEOUT)?;
    h.send_text("Get-BlueberrySqlRootFixture\r")?;
    wait_output_line(&mut h, "SQL-MODULE-AUTOLOAD-OK")?;
    h.finish(TIMEOUT)
}

#[test]
fn direct_without_inherited_module_path_keeps_editor_and_menu_above_input() -> Result<()> {
    let dir = direct_fixture()?;
    let mut h = Harness::start(
        std::path::Path::new(env!("CARGO_BIN_EXE_blueberry")),
        &[
            "run".into(),
            "--host-mode".into(),
            "direct".into(),
            "--shell".into(),
            blueberry::pty::default_shell()
                .to_string_lossy()
                .into_owned(),
            "--no-profile".into(),
            "--data-dir".into(),
            dir.path().to_string_lossy().into_owned(),
        ],
        dir.path(),
        &BTreeMap::from([
            ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
            ("PSModulePath".into(), String::new()),
            (
                "WinPSModulePath".into(),
                dir.path()
                    .join("stale-user/Modules")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ("TERM".into(), "xterm-256color".into()),
        ]),
        String::new(),
    )?;
    h.wait_text("PS ", TIMEOUT)?;
    let status = std::fs::read_dir(dir.path())?
        .filter_map(Result::ok)
        .find_map(|entry| std::fs::read(entry.path().join("adapter.json")).ok())
        .context("missing actual editor status")?;
    let status: serde_json::Value = serde_json::from_slice(&status)?;
    ensure!(
        status["editor_mode"] == "editor_hooks_v1",
        "GUI startup lost private editor: {status}"
    );
    let expected = std::process::Command::new(blueberry::pty::default_shell())
        .env_remove("PSModulePath")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
            "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); [Console]::WriteLine((ConvertTo-Json -InputObject @($env:PSModulePath -split ';') -Compress))"])
        .output()?;
    ensure!(
        expected.status.success(),
        "plain module path control failed"
    );
    let expected: Vec<String> = serde_json::from_slice(&expected.stdout)?;
    let paths_file = dir.path().join("actual-module-paths.json");
    h.send_text(&format!("[IO.File]::WriteAllText('{}',(ConvertTo-Json -InputObject @($env:PSModulePath -split ';') -Compress)); [Console]::WriteLine('MODULE-PATHS-CAPTURED')\r",
        paths_file.to_string_lossy().replace('\'', "''")))?;
    wait_output_line(&mut h, "MODULE-PATHS-CAPTURED")?;
    let actual: Vec<String> = serde_json::from_slice(&std::fs::read(paths_file)?)?;
    let normalized = |path: &str| {
        path.replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    for path in expected.iter().filter(|path| !path.is_empty()) {
        ensure!(
            actual
                .iter()
                .any(|entry| normalized(entry) == normalized(path)),
            "GUI startup lost default module directory {path}: {actual:?}"
        );
    }
    // Fill the viewport: a menu below the last prompt has no room. Only a
    // verified editor can provide the input bounds for upward placement.
    h.send(b"1..40 | ForEach-Object { 'HISTORY-ROW' }; 'BOTTOM-READY'\r")?;
    wait_output_line(&mut h, "BOTTOM-READY")?;
    h.send(b"git sw")?;
    // The same scenario can also be run with the frozen native observer.
    // Private companion frames intentionally never enter the legacy VT model.
    let deadline = std::time::Instant::now() + TIMEOUT;
    let screen = loop {
        let screen = h.observed_viewport()?;
        if screen.contains("switch") {
            break screen;
        }
        ensure!(
            std::time::Instant::now() < deadline,
            "menu missing above bottom input: {screen}"
        );
        let _ = h.pump(Duration::from_millis(10));
    };
    let menu = screen
        .lines()
        .position(|line| line.contains("switch"))
        .context("menu missing")?;
    let input = screen
        .lines()
        .position(|line| line.trim_end().ends_with("git sw"))
        .context("input missing")?;
    ensure!(menu > input, "menu was not below input: {screen}");
    h.send(b"\x01\x7f")?;
    h.finish(TIMEOUT)
}

#[test]
fn direct_shows_powershell_commands_before_execution_and_accepts_visible_menu() -> Result<()> {
    let dir = direct_fixture()?;
    let mut h = start(dir.path())?;
    // No command execution/autoload is allowed before these first-prompt checks.
    for prefix in ["get", "GET", "get-"] {
        h.send_text(prefix)?;
        h.wait_text("PowerShell 命令", TIMEOUT)?; // The reserved page may not include Get-Help yet.
        h.send(b"\x01\x7f")?;
    }
    for command in ["Get-ChildItem", "Get-Process", "Get-FileHash"] {
        h.send_text(&command[..command.len() - 1])?;
        h.wait_text(command, TIMEOUT)?; // Full name can only come from the menu.
        h.send(b"\t")?;
        h.send_text("X")?;
        h.wait_text(&format!("> {command} X"), TIMEOUT)?;
        h.send(b"\x01\x7f")?;
    }
    h.finish(TIMEOUT)
}

#[test]
fn direct_refreshes_large_runtime_snapshots_and_removals() -> Result<()> {
    let dir = direct_fixture()?;
    let mut h = start(dir.path())?;
    h.send_text("1..705 | ForEach-Object { Set-Alias -Name ('bb-session-'+$_) -Value Get-Help }; function bb-runtime { 'old' }; 'SNAPSHOT-READY'\r")?;
    wait_output_line(&mut h, "SNAPSHOT-READY")?;
    wait_editor_begin(&mut h, dir.path(), 2)?;
    h.send(b"bb-session-705")?;
    h.wait_text("Get-Help", TIMEOUT)?;
    h.send(b"\x01\x7fRemove-Item Alias:bb-session-705; function bb-runtime { 'new' }; 'REMOVAL-READY'\r")?;
    wait_output_line(&mut h, "REMOVAL-READY")?;
    wait_editor_begin(&mut h, dir.path(), 3)?;
    h.send(b"bb-runtim")?;
    h.wait_text("bb-runtime", TIMEOUT)?;
    h.send(b"\t\r")?;
    wait_output_line(&mut h, "new")?;
    wait_editor_begin(&mut h, dir.path(), 4)?;
    h.send(b"\x0cbb-session-705")?;
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        let _ = h.pump(Duration::from_millis(20));
    }
    ensure!(
        h.viewport_contents().matches("bb-session-705").count() == 1,
        "deleted alias still in menu: {}",
        h.viewport_contents()
    );
    h.send(b"\x01\x7f")?;
    h.finish(TIMEOUT)
}

#[test]
fn direct_commands_are_isolated_between_live_sessions() -> Result<()> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    let first = direct_fixture()?;
    let second = direct_fixture()?;
    let stop = Arc::new(AtomicBool::new(false));
    let observer_stop = stop.clone();
    let (ready, ready_rx) = mpsc::channel();
    // Each terminal must keep answering its own cursor queries. Driving two
    // Harnesses serially starves the first while waiting on the second; a late
    // CPR then becomes F3 or literal input after ConPTY's response deadline.
    let observer = std::thread::spawn(move || -> Result<()> {
        let mut a = start(first.path())?;
        a.send_text("function bb-only-one { 'ONE' }; 'ISOLATION-READY'\r")?;
        wait_output_line(&mut a, "ISOLATION-READY")?;
        wait_editor_begin(&mut a, first.path(), 2)?;
        a.send_text("bb-only-on")?;
        a.wait_text("bb-only-one", TIMEOUT)?;
        ready.send(())?;
        let deadline = std::time::Instant::now() + TIMEOUT * 3;
        while !observer_stop.load(Ordering::Acquire) {
            ensure!(
                std::time::Instant::now() < deadline,
                "second session did not finish"
            );
            let _ = a.pump(Duration::from_millis(20));
        }
        a.send(b"\x01\x7f")?;
        a.finish(TIMEOUT)
    });
    let result = (|| -> Result<()> {
        ready_rx.recv_timeout(TIMEOUT * 2)?;
        let mut b = start(second.path())?;
        b.send_text("bb-only-on")?;
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < deadline {
            let _ = b.pump(Duration::from_millis(20));
        }
        ensure!(
            !b.viewport_contents().contains("bb-only-one"),
            "other session commands leaked"
        );
        b.send(b"\x01\x7f")?;
        b.finish(TIMEOUT)
    })();
    stop.store(true, Ordering::Release);
    let first_result = observer
        .join()
        .map_err(|_| anyhow::anyhow!("first observer panicked"))?;
    result?;
    first_result
}

#[test]
fn direct_tracks_imported_and_removed_module_exports() -> Result<()> {
    let dir = direct_fixture()?;
    std::fs::write(
        dir.path().join("Commands.psm1"),
        "function Get-BlueberryModuleOnly { 'MODULE-RESULT' }; Export-ModuleMember -Function Get-BlueberryModuleOnly",
    )?;
    let mut h = start(dir.path())?;
    h.send_text("Import-Module ./Commands.psm1; 'IMPORT-READY'\r")?;
    wait_output_line(&mut h, "IMPORT-READY")?;
    wait_editor_begin(&mut h, dir.path(), 2)?;
    h.send(b"Get-BlueberryModuleOnl")?;
    h.wait_text("Get-BlueberryModuleOnly", TIMEOUT)?;
    h.send(b"\t\r")?;
    wait_output_line(&mut h, "MODULE-RESULT")?;
    h.send(b"Remove-Module Commands; 'REMOVE-READY'\r")?;
    wait_output_line(&mut h, "REMOVE-READY")?;
    wait_editor_begin(&mut h, dir.path(), 4)?;
    h.send(b"\x0cGet-BlueberryModuleOnl")?;
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        let _ = h.pump(Duration::from_millis(20));
    }
    ensure!(
        // PSReadLine 2.4.5 may display its own history prediction on the
        // prompt row. Check the menu below it, not that native ghost text.
        !h.viewport_contents()
            .lines()
            .skip(1)
            .any(|line| line.contains("Get-BlueberryModuleOnly")),
        "removed module still in menu: {}",
        h.viewport_contents()
    );
    h.send(b"\x01\x7f")?;
    h.finish(TIMEOUT)
}

#[test]
fn direct_and_original_editor_agree_on_bursts_cursor_selection_undo_and_modal_keys() -> Result<()> {
    editor_equivalence(false)
}

#[test]
fn direct_and_original_editor_agree_on_vi_command_and_insert_transitions() -> Result<()> {
    editor_equivalence(true)
}

fn editor_equivalence(vi: bool) -> Result<()> {
    let cases: &[&str] = if vi {
        &["abc\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_hxiZ"]
    } else {
        &[
            "中文😀e\u{301}👩‍💻",
            "abc def\x1b[D\x1b[DZ",
            "abcdef\x1b[1;2D\x1b[1;2DX",
            "undo-redo\x1a\x19",
            "\x18\x01",
            "\x1b2x",
        ]
    };
    let mut results = Vec::new();
    for direct in [false, true] {
        let dir = direct_fixture()?;
        let snapshot = dir.path().join("snapshot.txt");
        let setup = dir.path().join("setup.ps1");
        std::fs::write(&setup, include_str!("editor-equivalence.ps1"))?;
        let mut h = if direct {
            start(dir.path())?
        } else {
            let module = std::env::var("BLUEBERRY_TEST_PSREADLINE_MODULE")
                .unwrap_or("PSReadLine".into())
                .replace('\'', "''");
            Harness::start(
                &blueberry::pty::default_shell(),
                &[
                    "-NoLogo".into(),
                    "-NoProfile".into(),
                    "-NoExit".into(),
                    "-Command".into(),
                    format!(
                        "Import-Module '{module}'; Set-PSReadLineOption -HistorySaveStyle SaveNothing"
                    ),
                ],
                dir.path(),
                &BTreeMap::from([("BLUEBERRY_NO_HISTORY".into(), "1".into())]),
                String::new(),
            )?
        };
        h.wait_text("PS ", TIMEOUT)?;
        h.send(
            format!(
                ". '{}' -Snapshot '{}'{}\r",
                setup.display().to_string().replace('\'', "''"),
                snapshot.display().to_string().replace('\'', "''"),
                if vi { " -Vi" } else { "" }
            )
            .as_bytes(),
        )?;
        wait_output_line(&mut h, "EQUIVALENCE-READY")?;
        if direct {
            wait_editor_begin(&mut h, dir.path(), 2)?;
        } else {
            // The setup marker is emitted before ConsoleHost returns to the
            // editor. Observe the following prompt, then acknowledge the F12
            // binding before sending Unicode (cooked-mode input can drop it).
            let deadline = std::time::Instant::now() + TIMEOUT;
            while !h
                .viewport_contents()
                .lines()
                .last()
                .is_some_and(|line| line.trim_end().ends_with('>'))
            {
                ensure!(
                    std::time::Instant::now() < deadline,
                    "next plain prompt missing"
                );
                let _ = h.pump(Duration::from_millis(5));
            }
        }
        h.send(b"\x1b[24~")?;
        let deadline = std::time::Instant::now() + TIMEOUT;
        while !std::fs::read_to_string(&snapshot).is_ok_and(|state| state.contains('\n')) {
            ensure!(
                std::time::Instant::now() < deadline,
                "editor readiness acknowledgement missing"
            );
            let _ = h.pump(Duration::from_millis(5));
        }
        let mut states = Vec::new();
        for input in cases {
            if snapshot.exists() {
                std::fs::remove_file(&snapshot)?;
            }
            send_unicode_input(&mut h, input)?;
            h.send(b"\x1b[24~")?;
            let deadline = std::time::Instant::now() + TIMEOUT;
            loop {
                if let Ok(state) = std::fs::read_to_string(&snapshot)
                    && state.contains('\n')
                {
                    states.push(state);
                    break;
                }
                ensure!(
                    std::time::Instant::now() < deadline,
                    "editor snapshot failed for {input:?}: {}",
                    h.viewport_contents()
                );
                let _ = h.pump(Duration::from_millis(10));
            }
        }
        h.finish(TIMEOUT)?;
        results.push(states);
    }
    ensure!(
        results[0] == results[1],
        "native editing diverged: {results:?}"
    );
    if vi {
        ensure!(
            results[1][0].ends_with("aZc"),
            "Vi transitions were not exercised: {results:?}"
        );
        return Ok(());
    }
    ensure!(
        results[1][0].contains("中文😀e\u{301}👩‍💻"),
        "Unicode input lost in plain/direct snapshots: {results:?}"
    );
    ensure!(
        results[1][4].ends_with("CHORD") && results[1][5].ends_with("xx"),
        "modal action not exercised: {:?}",
        results[1]
    );
    Ok(())
}

#[test]
fn editor_callback_exception_unregisters_without_losing_native_input() -> Result<()> {
    let dir = direct_fixture()?;
    let script = dir.path().join("inject-callback.ps1");
    let fault = dir.path().join("fault.txt");
    std::fs::write(
        &script,
        r#"param([string]$FaultPath)
$global:BlueberryFaultPath=$FaultPath
$field=[Blueberry.Direct.Bridge].GetField('editorRegistration',[Reflection.BindingFlags]'NonPublic,Static')
$field.GetValue($null).Dispose()
$global:BlueberryTestRegistration=[Microsoft.PowerShell.PSConsoleReadLine]::RegisterEditorIntegration(1,$null,[Action]{throw 'injected editor callback fault'},$null,$null,$null,$null,[Action[string]]{param($message) [IO.File]::WriteAllText($global:BlueberryFaultPath,$message)})
[Console]::WriteLine('CALLBACK-INJECTED')
"#,
    )?;
    let mut h = start(dir.path())?;
    h.send(
        format!(
            ". '{}' -FaultPath '{}'\r",
            script.display(),
            fault.display()
        )
        .as_bytes(),
    )?;
    wait_output_line(&mut h, "CALLBACK-INJECTED")?;
    h.send(b"Write-Output 'CALLBACK-EDIT-OK'\r")?;
    wait_output_line(&mut h, "CALLBACK-EDIT-OK")?;
    ensure!(
        std::fs::read_to_string(&fault)?.contains("injected editor callback fault"),
        "fault callback missing"
    );
    h.send(b"Write-Output 'NEXT-PROMPT-OK'\r")?;
    wait_output_line(&mut h, "NEXT-PROMPT-OK")?;
    h.finish(TIMEOUT)
}

#[test]
#[ignore = "native foreground child for the Ctrl+C lifecycle regression"]
fn external_interrupt_helper() {
    println!(
        "BB-EXTERNAL-INTERRUPT-READY-{}",
        std::env::var("BLUEBERRY_INTERRUPT_ROUND").unwrap()
    );
    std::thread::sleep(Duration::from_secs(60));
}

#[test]
fn direct_ctrl_c_during_native_process_preserves_session_and_prompt() -> Result<()> {
    check_native_interrupt(false)?;
    check_native_interrupt(true)
}

fn check_native_interrupt(parent_shell: bool) -> Result<()> {
    let dir = direct_fixture()?;
    let mut h = if parent_shell {
        let executable = std::env::var_os("BLUEBERRY_TEST_PRODUCT_EXE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_blueberry").into());
        let shell = blueberry::pty::default_shell();
        let quote = |path: &std::path::Path| path.to_string_lossy().replace('\'', "''");
        let command = format!(
            "function global:prompt {{ 'BB-PARENT-RESTORED> ' }}; & '{}' run --host-mode direct --no-profile --shell '{}' --data-dir '{}' --trace '{}'",
            quote(&executable),
            quote(&shell),
            quote(dir.path()),
            quote(&dir.path().join("trace.jsonl"))
        );
        let mut h = Harness::start(
            &shell,
            &[
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NoExit".into(),
                "-Command".into(),
                command,
            ],
            dir.path(),
            &BTreeMap::from([("BLUEBERRY_NO_HISTORY".into(), "1".into())]),
            String::new(),
        )?;
        wait_editor_begin(&mut h, dir.path(), 1)?;
        h
    } else {
        start(dir.path())?
    };
    let initialize = if let Some(starship) = std::env::var_os("BLUEBERRY_TEST_STARSHIP") {
        format!(
            "Invoke-Expression (& '{}' init powershell)",
            starship.to_string_lossy().replace('\'', "''")
        )
    } else {
        "function global:prompt { 'BB-PLUGIN-PROMPT> ' }".into()
    };
    h.send_text(&format!("$global:BBInterruptPid=$PID; {initialize}; $global:BBInterruptPrompt=(Get-Command prompt).ScriptBlock.ToString(); Write-Output 'BB-PLUGIN-READY'\r"))?;
    wait_output_line(&mut h, "BB-PLUGIN-READY")?;
    let helper = std::env::current_exe()?
        .to_string_lossy()
        .replace('\'', "''");
    for iteration in 0..3 {
        h.send_text(&format!("$env:BLUEBERRY_INTERRUPT_ROUND='{iteration}'; & '{helper}' external_interrupt_helper --exact --ignored --nocapture\r"))?;
        h.wait_text(&format!("BB-EXTERNAL-INTERRUPT-READY-{iteration}"), TIMEOUT)?;
        let begins = std::fs::read_to_string(dir.path().join("trace.jsonl"))?
            .lines()
            .filter(|line| line.contains("direct_readline_begin"))
            .count();
        h.send(b"\x03")?;
        wait_editor_begin(&mut h, dir.path(), begins + 1)?;
        let marker = format!("BB-SESSION-PRESERVED-{iteration}");
        h.send_text(&format!("if ($global:BBInterruptPid -eq $PID -and (Get-Command prompt).ScriptBlock.ToString() -eq $global:BBInterruptPrompt) {{ Write-Output '{marker}' }}\r"))?;
        wait_output_line(&mut h, &marker)?;
    }
    h.send(b"git sw")?;
    h.wait_text("switch", TIMEOUT)?;
    h.send(b"\x03")?;
    if parent_shell {
        h.send_text("exit\r")?;
        // Windows may expand an 8.3 TEMP path in its stock prompt.
        wait_output_line(&mut h, "BB-PARENT-RESTORED>")?;
    }
    h.finish(TIMEOUT)
}

#[test]
fn direct_dismiss_external_program_and_ctrl_c_restore_shell() -> Result<()> {
    let dir = direct_fixture()?;
    let mut h = start(dir.path())?;
    h.wait_text("PS ", TIMEOUT)?;
    h.send(b"git sw")?;
    h.wait_text("switch", TIMEOUT)?;
    h.send(b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_")?;
    let until = std::time::Instant::now() + TIMEOUT;
    while h.viewport_contents().contains("› ") {
        ensure!(
            std::time::Instant::now() < until,
            "dismiss left menu visible"
        );
        let _ = h.pump(Duration::from_millis(25));
    }
    ensure!(
        h.viewport_contents().contains("git sw"),
        "dismiss changed line"
    );
    h.send(b"\x01")?;
    h.send(b"cmd.exe /d /c echo external-direct\r")?;
    h.wait_line("external-direct", TIMEOUT)?;
    h.send(b"git sw")?;
    h.wait_text("switch", TIMEOUT)?;
    h.send(b"\x03")?;
    h.wait_text("^C", TIMEOUT)?;
    h.send(b"Write-Output 'ctrl-c-restored'\r")?;
    h.wait_line("ctrl-c-restored", TIMEOUT)?;
    h.finish(TIMEOUT)
}

#[test]
fn direct_selected_frame_navigation_survives_resize_and_disconnect() -> Result<()> {
    let dir = direct_fixture()?;
    let mut harness = start(dir.path())?;
    harness.wait_text("PS ", TIMEOUT)?;
    harness.send(b"git sw")?;
    harness.wait_text("switch", TIMEOUT)?;
    harness.send(b"\x1b[B")?;
    harness.wait_text("›", TIMEOUT)?;
    // Wait until the selected candidate really changes; acceptance binds the
    // displayed frame rather than the newest asynchronously computed result.
    let deadline = std::time::Instant::now() + TIMEOUT;
    while !harness
        .viewport_contents()
        .lines()
        .any(|line| line.contains("›") && line.contains("show"))
    {
        ensure!(
            std::time::Instant::now() < deadline,
            "selection did not advance"
        );
        let _ = harness.pump(Duration::from_millis(50));
    }
    harness.send(b"\t")?;
    harness.wait_text("git show", TIMEOUT)?;
    harness.resize(10, 50)?;
    harness.send(b"\x01\x7f")?;
    harness.send(b"[Blueberry.Direct.Bridge]::Shutdown(); Write-Output 'disconnected'\r")?;
    harness.wait_text("disconnected", TIMEOUT)?;
    harness.send(b"Write-Output 'editing-restored'\r")?;
    harness.wait_text("editing-restored", TIMEOUT)?;
    harness.finish(TIMEOUT)
}

fn selected_path(h: &Harness, name: &str) -> bool {
    h.viewport_contents()
        .lines()
        .any(|line| line.contains('›') && line.contains(name))
}

fn wait_selected_path(h: &mut Harness, name: &str) -> Result<()> {
    let deadline = std::time::Instant::now() + TIMEOUT;
    while !selected_path(h, name) {
        ensure!(
            std::time::Instant::now() < deadline,
            "selection never reached {name}: {}",
            h.viewport_contents()
        );
        let _ = h.pump(Duration::from_millis(10));
    }
    Ok(())
}

fn path_fixture(root: &std::path::Path) -> Result<()> {
    for index in 0..16 {
        std::fs::write(root.join(format!("nav-{index:02}.txt")), "fixture")?;
    }
    Ok(())
}

#[test]
fn direct_path_navigation_stays_selected_and_orders_burst_acceptance() -> Result<()> {
    for delay in [0, 40] {
        let dir = direct_fixture()?;
        path_fixture(dir.path())?;
        let mut h = start_with_delay(dir.path(), delay)?;
        let result = (|| -> Result<()> {
            h.send_text("Get-ChildItem .\\nav-")?;
            wait_selected_path(&mut h, "nav-00.txt")?;
            h.send(b"\x1b[B")?;
            wait_selected_path(&mut h, "nav-01.txt")?;
            // Observe beyond injected delays: the old bug briefly showed the
            // next item, then an empty worker result reset it to the first.
            let until = std::time::Instant::now() + Duration::from_millis(600);
            while std::time::Instant::now() < until {
                let _ = h.pump(Duration::from_millis(10));
            }
            ensure!(
                selected_path(&h, "nav-01.txt"),
                "provider reset the selection: {}",
                h.viewport_contents()
            );
            h.send(b"\x1b[A")?;
            wait_selected_path(&mut h, "nav-00.txt")?;
            let steps = if delay == 0 { 10 } else { 3 };
            let mut burst = b"\x1b[B".repeat(steps);
            burst.push(b'\t');
            h.send(&burst)?;
            h.wait_text(&format!("Get-ChildItem .\\nav-{steps:02}.txt"), TIMEOUT)?;
            let trace = std::fs::read_to_string(dir.path().join("trace.jsonl"))?;
            ensure!(
                trace
                    .lines()
                    .filter(|line| line.contains("\"direct_readline_begin\""))
                    .count()
                    == 1,
                "acceptance executed the command"
            );
            h.finish(TIMEOUT)
        })();
        if let Err(error) = result {
            let _ = h.save_evidence(dir.path());
            let _ = h.stop();
            return Err(error).with_context(|| {
                format!("path navigation delay {delay}: {}", dir.keep().display())
            });
        }
    }
    Ok(())
}

#[test]
fn direct_bottom_menu_reserves_space_and_survives_small_viewport_navigation() -> Result<()> {
    let dir = direct_fixture()?;
    path_fixture(dir.path())?;
    let mut h = start(dir.path())?;
    let result = (|| -> Result<()> {
        h.send_text("1..40 | ForEach-Object { 'BOTTOM-SENTINEL' }")?;
        h.send(b"\r")?;
        wait_editor_begin(&mut h, dir.path(), 2)?;
        h.send_text("Get-ChildItem .\\nav-")?;
        wait_selected_path(&mut h, "nav-00.txt")?;
        let screen = h.viewport_contents();
        let rows: Vec<_> = screen.lines().collect();
        let input = rows
            .iter()
            .position(|row| row.contains("Get-ChildItem .\\nav-"))
            .context("input missing")?;
        let menu = rows
            .iter()
            .position(|row| row.contains('›'))
            .context("menu missing")?;
        ensure!(
            menu > input,
            "bottom menu did not reserve rows below input: {screen}"
        );
        h.resize(10, 90)?;
        h.send(b"\x1b[B")?;
        wait_selected_path(&mut h, "nav-01.txt")?;
        h.send(b"\x1b[B")?;
        wait_selected_path(&mut h, "nav-02.txt")?;
        h.send(b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_")?;
        let until = std::time::Instant::now() + Duration::from_millis(400);
        while std::time::Instant::now() < until {
            let _ = h.pump(Duration::from_millis(10));
        }
        ensure!(
            !h.viewport_contents().contains('›'),
            "Escape left the overlay visible"
        );
        // Reserving rows can legitimately scroll history out of the viewport.
        // Verify that the observer retained it in scrollback, not that the
        // product copied old cells back over the current editor.
        let mut history = h.screen_snapshot();
        let mut retained = history.contents().contains("BOTTOM-SENTINEL");
        for offset in 1..=1024 {
            history.set_scrollback(offset);
            retained |= history.contents().contains("BOTTOM-SENTINEL");
            if history.scrollback() < offset {
                break;
            }
        }
        ensure!(retained, "reserved menu destroyed output history");
        h.send_text("x")?;
        h.wait_text("Get-ChildItem .\\nav-x", TIMEOUT)?;
        h.finish(TIMEOUT)
    })();
    if let Err(error) = result {
        let _ = h.save_evidence(dir.path());
        let _ = console_snapshot::capture(&mut h, &dir.path().join("native-screen.json"));
        let _ = h.stop();
        return Err(error).with_context(|| format!("bottom menu: {}", dir.keep().display()));
    }
    Ok(())
}

#[test]
fn direct_pending_path_navigation_yields_to_typing_and_discards_late_frames() -> Result<()> {
    let dir = direct_fixture()?;
    path_fixture(dir.path())?;
    let mut h = start_with_delay(dir.path(), 100)?;
    let result = (|| -> Result<()> {
        h.send_text("Get-ChildItem .\\nav-")?;
        wait_selected_path(&mut h, "nav-00.txt")?;
        h.send(b"\x1b[B\x1b[B")?;
        h.send_text("x")?;
        h.wait_text("Get-ChildItem .\\nav-x", TIMEOUT)?;
        let until = std::time::Instant::now() + Duration::from_millis(800);
        while std::time::Instant::now() < until {
            let _ = h.pump(Duration::from_millis(10));
        }
        ensure!(
            !h.viewport_contents().contains('›'),
            "late navigation resurrected the old menu"
        );
        ensure!(
            h.viewport_contents().contains("Get-ChildItem .\\nav-x"),
            "late navigation overwrote typing"
        );
        h.finish(TIMEOUT)
    })();
    if let Err(error) = result {
        let _ = h.save_evidence(dir.path());
        let _ = h.stop();
        return Err(error).with_context(|| format!("pending navigation: {}", dir.keep().display()));
    }
    Ok(())
}

#[test]
fn direct_above_menu_preserves_wrapped_unicode_input_and_right_text() -> Result<()> {
    exercise_multiline_resize(false, false)
}

#[test]
fn direct_resize_reflows_input_when_menu_ownership_readback_failed() -> Result<()> {
    exercise_multiline_resize(true, false)
}

#[test]
fn direct_resize_after_menu_clear_reflows_input() -> Result<()> {
    exercise_multiline_resize(false, true)
}

fn exercise_multiline_resize(omit_capture: bool, resize_after_clear: bool) -> Result<()> {
    // Keep failed native snapshots and traces inside the uploaded CI evidence.
    let evidence =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/input-protocol-evidence");
    std::fs::create_dir_all(&evidence)?;
    let dir = tempfile::Builder::new()
        .prefix("direct-multiline-")
        .tempdir_in(evidence)?;
    if omit_capture {
        std::fs::write(dir.path().join("omit-menu-ownership"), "1")?;
    }
    path_fixture(dir.path())?;
    let suffix = " ; Write-Output 'RIGHT'";
    let expected = format!(
        "Write-Output 'HEAD{}'\nGet-ChildItem .\\nav-{suffix}",
        "中".repeat(30)
    );
    std::fs::write(dir.path().join("input.txt"), &expected)?;
    let quote = |name: &str| {
        dir.path()
            .join(name)
            .display()
            .to_string()
            .replace('\'', "''")
    };
    let script = format!(
        r#"
function global:prompt {{ 'BB> ' }}
Set-PSReadLineKeyHandler -Chord F7 -ScriptBlock {{
    # The editor's Before callback has already cleared the visible menu.
    [IO.File]::WriteAllText('{cleared}', 'ready')
    $deadline=[DateTime]::UtcNow.AddSeconds(5)
    while (-not [IO.File]::Exists('{resized}')) {{
        if ([DateTime]::UtcNow -gt $deadline) {{ throw 'Resize gate timed out' }}
        [Threading.Thread]::Sleep(5)
    }}
}}
Set-PSReadLineKeyHandler -Chord F6 -ScriptBlock {{
    [Microsoft.PowerShell.PSConsoleReadLine]::Insert([IO.File]::ReadAllText('{input}'))
    [Microsoft.PowerShell.PSConsoleReadLine]::BackwardChar($null,{back})
}}
Set-PSReadLineKeyHandler -Chord F12 -ScriptBlock {{
    $line=''; $cursor=0
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line,[ref]$cursor)
    [IO.File]::WriteAllText('{snapshot}.pending',$line)
    [IO.File]::Move('{snapshot}.pending','{snapshot}')
}}
1..40 | ForEach-Object {{ 'MULTILINE-SENTINEL' }}
"#,
        input = quote("input.txt"),
        cleared = quote("menu-cleared.txt"),
        resized = quote("resize-complete.txt"),
        back = suffix.encode_utf16().count(),
        snapshot = quote("buffer.txt")
    );
    std::fs::write(dir.path().join("setup.ps1"), script)?;
    let mut h = start(dir.path())?;
    let result = (|| -> Result<()> {
        h.resize(12, 70)?;
        h.send_text(&format!(". '{}'", quote("setup.ps1")))?;
        h.send(b"\r")?;
        wait_editor_begin(&mut h, dir.path(), 2)?;
        h.send(b"\x1b[17~")?;
        wait_selected_path(&mut h, "nav-00.txt")?;
        let snapshot = dir.path().join("native-screen.json");
        let screen = wait_native_selected_path(&mut h, &snapshot, "nav-00.txt")?;
        let rows: Vec<_> = screen.lines().collect();
        let input = rows
            .iter()
            .position(|row| row.contains("HEAD"))
            .context("wrapped input start missing")?;
        let menu = rows
            .iter()
            .position(|row| row.contains('›'))
            .context("native menu missing")?;
        ensure!(
            menu > input && screen.contains("RIGHT"),
            "menu covered multiline input: {screen}"
        );
        if resize_after_clear {
            h.send(b"\x1b[18~")?;
            let deadline = std::time::Instant::now() + TIMEOUT;
            while !dir.path().join("menu-cleared.txt").is_file() {
                ensure!(
                    std::time::Instant::now() < deadline,
                    "menu clear gate missing"
                );
                let _ = h.pump(Duration::from_millis(10));
            }
        }
        h.resize(10, 60)?;
        if resize_after_clear {
            std::fs::write(dir.path().join("resize-complete.txt"), "ready")?;
            // F7 is a user handler: it revokes the displayed frame before
            // dispatch. A queued arrow before a replacement is visible belongs
            // to native editing, not to an invisible menu. Observe the new
            // native frame before exercising its navigation contract.
            let screen = wait_native_selected_path(
                &mut h,
                &dir.path().join("native-after-clear-resize.json"),
                "nav-00.txt",
            )?;
            ensure!(
                screen.contains("HEAD") && screen.contains("RIGHT"),
                "clear/resize covered the input before navigation: {screen}"
            );
        }
        h.send(b"\x1b[B")?;
        wait_selected_path(&mut h, "nav-01.txt")?;
        let resized_screen = wait_native_selected_path(
            &mut h,
            &dir.path().join("native-resized.json"),
            "nav-01.txt",
        )?;
        ensure!(
            resized_screen.contains("HEAD") && resized_screen.contains("RIGHT"),
            "resize covered the input or its suffix: {resized_screen}"
        );
        h.send(b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_")?;
        h.send(b"\x1b[24~")?;
        let deadline = std::time::Instant::now() + TIMEOUT;
        while !dir.path().join("buffer.txt").is_file() {
            ensure!(
                std::time::Instant::now() < deadline,
                "native buffer snapshot missing"
            );
            let _ = h.pump(Duration::from_millis(10));
        }
        ensure!(
            std::fs::read_to_string(dir.path().join("buffer.txt"))? == expected,
            "navigation/resize modified the editor buffer"
        );
        h.finish(TIMEOUT)
    })();
    if let Err(error) = result {
        let _ = h.save_evidence(dir.path());
        let _ = console_snapshot::capture(&mut h, &dir.path().join("native-failure.json"));
        let _ = h.stop();
        return Err(error).with_context(|| format!("multiline overlay: {}", dir.keep().display()));
    }
    Ok(())
}

#[test]
fn direct_above_menu_preserves_truecolor_prompt_cells() -> Result<()> {
    let evidence =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/input-protocol-evidence");
    std::fs::create_dir_all(&evidence)?;
    let dir = tempfile::Builder::new()
        .prefix("direct-prompt-colors-")
        .tempdir_in(evidence)?;
    let setup = dir.path().join("setup.ps1");
    std::fs::write(
        &setup,
        "\u{feff}1..40 | ForEach-Object { 'COLOR-SENTINEL' }; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $e=[char]27; [Console]::Write(\"${e}[38;2;123;45;67m${e}[48;2;23;89;145mRGB-PROMPT-中🚀é${e}[0m`r`n\"); function global:prompt { 'BB> ' }\r\n",
    )?;
    let executable = std::env::var_os("BLUEBERRY_TEST_PRODUCT_EXE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_blueberry").into());
    let mut h = start_delayed_executable_with_pages(dir.path(), 0, &executable, true)?;
    let result = (|| -> Result<()> {
        h.send_text(&format!(
            ". '{}'\r",
            setup.display().to_string().replace('\'', "''")
        ))?;
        wait_editor_begin(&mut h, dir.path(), 2)?;
        h.wait_text("RGB-PROMPT-中🚀é", TIMEOUT)?;
        let before = h.screen_snapshot();
        let row = (0..before.size().0)
            .find(|row| {
                before
                    .rows(0, before.size().1)
                    .nth(*row as usize)
                    .is_some_and(|line| line.contains("RGB-PROMPT-"))
            })
            .context("prompt row missing")?;
        ensure!(
            before.cell(row, 0).unwrap().fgcolor() == vt100::Color::Rgb(123, 45, 67),
            "fixture lacks truecolor"
        );
        h.send(b"git sw")?;
        h.wait_text("switch", TIMEOUT)?;
        h.send(b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_")?;
        let until = std::time::Instant::now() + Duration::from_millis(400);
        while std::time::Instant::now() < until {
            let _ = h.pump(Duration::from_millis(10));
        }
        let after = h.screen_snapshot();
        let after_row = (0..after.size().0)
            .find(|row| {
                after
                    .rows(0, after.size().1)
                    .nth(*row as usize)
                    .is_some_and(|line| line.contains("RGB-PROMPT-"))
            })
            .context("scrolled prompt row missing")?;
        for column in 0..before.size().1 {
            let old = before.cell(row, column).unwrap();
            let new = after.cell(after_row, column).unwrap();
            ensure!(
                old == new,
                "prompt cell changed at {row},{column}: {old:?} -> {new:?}"
            );
        }
        h.finish(TIMEOUT)
    })();
    if let Err(error) = result {
        let _ = h.save_evidence(dir.path());
        let _ = h.stop();
        return Err(error).with_context(|| format!("prompt colors: {}", dir.keep().display()));
    }
    Ok(())
}

#[test]
#[ignore = "requires frozen CI EXE and native history observer DLL"]
fn direct_above_menu_resize_preserves_truecolor_history() -> Result<()> {
    check_above_menu_resized_history(true)
}

#[test]
#[ignore = "requires frozen CI EXE and native history observer DLL"]
fn direct_above_menu_resize_preserves_unicode_and_wrapped_history() -> Result<()> {
    // Keep this independent of RGB: a colour assertion must not conceal a
    // second regression in the character/soft-wrap restoration path.
    check_above_menu_resized_history(false)
}

fn check_above_menu_resized_history(check_rgb: bool) -> Result<()> {
    let evidence =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/input-protocol-evidence");
    std::fs::create_dir_all(&evidence)?;
    let mut dir = tempfile::Builder::new()
        .prefix("direct-resize-prompt-colors-")
        .tempdir_in(evidence)?;
    dir.disable_cleanup(true);
    let setup = dir.path().join("setup.ps1");
    std::fs::write(
        &setup,
        format!(
            "\u{feff}1..40 | ForEach-Object {{ 'COLOR-SENTINEL' }}; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $e=[char]27; [Console]::Write(\"${{e}}[38;2;123;45;67m${{e}}[48;2;23;89;145mRGB-PROMPT-中🚀é{}${{e}}[0m`r`n\"); function global:prompt {{ 'BB> ' }}\r\n",
            "x".repeat(55)
        ),
    )?;
    let executable = std::env::var_os("BLUEBERRY_TEST_PRODUCT_EXE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_blueberry").into());
    let mut h = start_delayed_executable_with_pages(dir.path(), 0, &executable, true)?;
    let result = (|| -> Result<()> {
        h.send_text(&format!(
            ". '{}'\r",
            setup.display().to_string().replace('\'', "''")
        ))?;
        wait_editor_begin(&mut h, dir.path(), 2)?;
        h.wait_text("RGB-PROMPT-中🚀é", TIMEOUT)?;
        let before = h.native_history()?;
        std::fs::write(dir.path().join("native-history-before.txt"), &before)?;
        let row = before
            .lines()
            .position(|line| line.starts_with("RGB-PROMPT-"))
            .context("original native history row missing")?;
        ensure!(
            h.native_cell_colors(row as u32, 0)?
                == [
                    3,
                    123 | (45 << 8) | (67 << 16),
                    3,
                    23 | (89 << 8) | (145 << 16)
                ],
            "fixture lacks original RGB attributes"
        );
        h.send(b"git sw")?;
        h.wait_text("switch", TIMEOUT)?;
        h.resize(12, 40)?;
        let until = std::time::Instant::now() + Duration::from_millis(500);
        while std::time::Instant::now() < until {
            let _ = h.pump(Duration::from_millis(10));
        }
        h.send(b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_")?;
        let until = std::time::Instant::now() + Duration::from_millis(400);
        while std::time::Instant::now() < until {
            let _ = h.pump(Duration::from_millis(10));
        }
        // Native TerminalCore retains and reflows logical scrollback. Read it
        // directly; replaying or growing the lossy VT model is not evidence.
        let after = h.native_history()?;
        std::fs::write(dir.path().join("native-history-after.txt"), &after)?;
        std::fs::write(
            dir.path().join("native-observer.json"),
            serde_json::to_vec_pretty(&h.native_observer_identity())?,
        )?;
        let row = after
            .lines()
            .position(|line| line.starts_with("RGB-PROMPT-"))
            .context("native history disappeared after menu resize and Esc")?;
        let colors = h.native_cell_colors(row as u32, 0)?;
        std::fs::write(
            dir.path().join("native-history-colors.json"),
            serde_json::to_vec(&colors)?,
        )?;
        if check_rgb {
            ensure!(
                colors
                    == [
                        3,
                        123 | (45 << 8) | (67 << 16),
                        3,
                        23 | (89 << 8) | (145 << 16)
                    ],
                "resized native history lost RGB: {colors:?}"
            );
        }
        let expected = format!("RGB-PROMPT-中🚀é{}", "x".repeat(55));
        let logical_text = after.lines().map(str::trim_end).collect::<String>();
        ensure!(
            logical_text.contains(&expected),
            "resized native history clipped or changed the wrapped Unicode fixture"
        );
        h.finish(TIMEOUT)
    })();
    if let Err(error) = result {
        let _ = h.save_evidence(dir.path());
        let _ = h.stop();
        return Err(error)
            .with_context(|| format!("resize prompt colors: {}", dir.keep().display()));
    }
    Ok(())
}

#[test]
fn direct_guided_form_cancel_preserves_right_text_and_confirm_does_not_execute() -> Result<()> {
    let dir = direct_fixture()?;
    let mut harness = start(dir.path())?;
    harness.wait_text("PS ", TIMEOUT)?;
    harness.send(b"git switch -c  --track")?;
    harness.send(&b"\x1b[D".repeat(" --track".len()))?;
    harness.send(b"\x1b[80;25;16;1;10;1_\x1b[80;25;16;0;10;1_")?;
    harness.wait_text("继续填写", TIMEOUT)?;
    harness.send(b"\r")?;
    harness.wait_text("填写参数", TIMEOUT)?;
    harness.send("分支😀".as_bytes())?;
    harness.wait_text("当前值", TIMEOUT)?;
    harness.send(b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_")?;
    // Escape is a terminal prefix. Confirm cancellation before sending a
    // new modified chord, so the observer does not synthesize an Alt chord.
    let deadline = std::time::Instant::now() + TIMEOUT;
    while harness.viewport_contents().contains("填写参数") {
        ensure!(
            std::time::Instant::now() < deadline,
            "cancel did not restore original line"
        );
        let _ = harness.pump(Duration::from_millis(50));
    }
    harness.send(b"\x1b[80;25;16;1;10;1_\x1b[80;25;16;0;10;1_")?;
    harness.wait_text("继续填写", TIMEOUT)?;
    harness.send(b"\r")?;
    harness.wait_text("填写参数", TIMEOUT)?;
    harness.send(b"new-branch")?;
    harness.send(b"\r")?;
    harness.wait_text("git switch -c new-branch", TIMEOUT)?;
    ensure!(
        harness.viewport_contents().contains("--track"),
        "right text lost"
    );
    ensure!(
        !harness.viewport_contents().contains("fatal:"),
        "form executed git"
    );
    harness.send(b"\x1a")?;
    harness.wait_text("git switch -c  --track", TIMEOUT)?;
    harness.finish(TIMEOUT)
}

#[test]
fn direct_malformed_cargo_form_keeps_completion_and_native_history_alive() -> Result<()> {
    let dir = direct_fixture()?;
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace\nbroken")?;
    let snapshot = dir.path().join("error-buffer.json");
    let mut h = start(dir.path())?;
    h.send_text(&format!(
        "Set-PSReadLineKeyHandler -Chord F12 -ScriptBlock {{ $line=''; $cursor=0; [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line,[ref]$cursor); [IO.File]::WriteAllText('{}', (@{{line=$line;cursor=$cursor}} | ConvertTo-Json -Compress)) }}\r",
        snapshot.display().to_string().replace('\'', "''")
    ))?;
    wait_editor_begin(&mut h, dir.path(), 2)?;
    h.send_text("cargo test -p --release")?;
    h.send(&b"\x1b[D".repeat(" --release".len()))?;
    h.send(b"\x1b[80;25;16;1;10;1_\x1b[80;25;16;0;10;1_")?;
    h.wait_text("继续填写", TIMEOUT)?;
    h.send(b"\r")?;
    h.wait_text("填写参数：工作区包", TIMEOUT)?;
    h.wait_text("Cargo.toml", TIMEOUT)?;
    h.send(b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_")?;
    let deadline = std::time::Instant::now() + TIMEOUT;
    while h.viewport_contents().contains("填写参数") {
        ensure!(
            std::time::Instant::now() < deadline,
            "form error could not be canceled: {}",
            h.viewport_contents()
        );
        let _ = h.pump(Duration::from_millis(25));
    }
    for (expected, cursor) in [
        ("cargo test -p --release", Some(13)),
        ("Write-Output BB_DIRECT_FORM_RECOVERED", None),
    ] {
        h.send(b"\x1b[24~")?;
        let deadline = std::time::Instant::now() + TIMEOUT;
        let value = loop {
            if let Ok(bytes) = std::fs::read(&snapshot)
                && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
                && value["line"] == expected
            {
                break value;
            }
            ensure!(
                std::time::Instant::now() < deadline,
                "native buffer unavailable after form error"
            );
            let _ = h.pump(Duration::from_millis(25));
        };
        if let Some(cursor) = cursor {
            ensure!(
                value["cursor"] == cursor,
                "cancel changed original cursor: {value}"
            );
            h.send(b"\x01\x7f")?;
            h.send_text("git ")?;
            h.wait_text("archive", TIMEOUT)?;
            h.send(b"\x01\x7f")?;
            h.send_text("Write-Output BB_DIRECT_FORM_RECOVERED\r")?;
            h.wait_line("BB_DIRECT_FORM_RECOVERED", TIMEOUT)?;
            wait_editor_begin(&mut h, dir.path(), 3)?;
            h.send(b"\x1b[A")?;
        }
    }
    h.finish(TIMEOUT)
}

#[test]
fn direct_form_confirmation_precedes_immediately_following_typing() -> Result<()> {
    let dir = direct_fixture()?;
    let mut h = start(dir.path())?;
    h.wait_text("PS ", TIMEOUT)?;
    h.send(b"git switch -c  --track")?;
    h.send(&b"\x1b[D".repeat(" --track".len()))?;
    h.send(b"\x1b[80;25;16;1;10;1_\x1b[80;25;16;0;10;1_")?;
    h.wait_text("继续填写", TIMEOUT)?;
    h.send(b"\r")?;
    h.wait_text("填写参数", TIMEOUT)?;
    h.send(b"burst\rX")?;
    h.wait_text("git switch -c burst --trackX", TIMEOUT)?;
    ensure!(
        !h.viewport_contents().contains("fatal:"),
        "form executed command"
    );
    h.send(b"\x01\x7f")?;
    h.finish(TIMEOUT)
}

#[test]
fn unmatched_editor_manual_completion_preserves_native_editing() -> Result<()> {
    let dir = direct_fixture()?;
    let discovery = dir.path().join("unmatched.psd1");
    std::fs::write(&discovery, "ModuleVersion = '2.3.6'")?;
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        "[keys]\ntrigger = 'F6'\nhub = 'F7'\nsearch = 'F8'\n",
    )?;
    let mut h = Harness::start(
        std::path::Path::new(env!("CARGO_BIN_EXE_blueberry")),
        &[
            "run".into(),
            "--host-mode".into(),
            "direct".into(),
            "--no-profile".into(),
            "--shell".into(),
            blueberry::pty::default_shell()
                .to_string_lossy()
                .into_owned(),
            "--data-dir".into(),
            dir.path().to_string_lossy().into_owned(),
            "--config".into(),
            config.to_string_lossy().into_owned(),
        ],
        dir.path(),
        &BTreeMap::from([
            ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
            (
                "BLUEBERRY_TEST_PSREADLINE_MODULE".into(),
                discovery.to_string_lossy().into_owned(),
            ),
        ]),
        String::new(),
    )?;
    h.wait_text("PS ", TIMEOUT)?;
    h.send_text("git sw")?;
    h.pump(Duration::from_millis(500))?;
    ensure!(
        !h.contents().contains("switch"),
        "unmatched editor popped up automatically"
    );
    h.send(b"\x1b[17~")?;
    h.wait_text("switch", TIMEOUT)?;
    h.send(b"\t")?;
    h.wait_text("git switch", TIMEOUT)?;
    h.send(b"\x01\x7f")?;
    h.send_text("Write-Output 'MANUAL-EDIT-OK'")?;
    h.send(b"\r")?;
    h.wait_line("MANUAL-EDIT-OK", TIMEOUT)
        .context("post acceptance edit")?;
    h.send(b"\x1b[18~")?;
    h.wait_text("工作台", TIMEOUT)
        .context("open manual workbench")?;
    h.send_text("\x1b")?;
    h.send_text("Write-Output 'MANUAL-CANCEL-OK'")?;
    h.send(b"\r")?;
    h.wait_line("MANUAL-CANCEL-OK", TIMEOUT)
        .context("post cancel edit")?;
    h.finish(TIMEOUT)
}

#[test]
fn builtin_workflows_fill_native_editor_without_executing() -> Result<()> {
    exercise_builtin_workflows(false)
}

#[test]
fn queued_workflow_responses_survive_a_late_editor_wake() -> Result<()> {
    exercise_builtin_workflows(true)
}

#[test]
fn queued_completion_acceptance_survives_a_late_editor_wake() -> Result<()> {
    let dir = direct_fixture()?;
    std::fs::write(dir.path().join("delay-ui-wake"), "600")?;
    let mut h = start(dir.path())?;
    h.send_text("git sw")?;
    h.wait_text("switch", TIMEOUT)?;
    h.send(b"\tX")?;
    h.wait_text("git switch X", TIMEOUT)?;
    h.finish(TIMEOUT)
}

fn exercise_builtin_workflows(delay_wake: bool) -> Result<()> {
    let dir = direct_fixture()?;
    if delay_wake {
        std::fs::write(dir.path().join("delay-ui-wake"), "600")?;
    }
    let snapshot = dir.path().join("workflow-buffer.txt");
    let setup = dir.path().join("setup.ps1");
    let mode = dir.path().join("argument-mode.txt");
    std::fs::write(
        &setup,
        format!(
            "Set-PSReadLineKeyHandler -Chord F12 -ScriptBlock {{ $line=''; $cursor=0; [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line,[ref]$cursor); $snapshotPath='{}'; [IO.File]::WriteAllText($snapshotPath+'.pending',$line); [IO.File]::Move($snapshotPath+'.pending',$snapshotPath) }}; [IO.File]::WriteAllText('{}', [string]($PSVersionTable.PSVersion -lt [version]'7.3' -or $PSNativeCommandArgumentPassing -eq 'Legacy'))",
            snapshot.display().to_string().replace('\'', "''"),
            mode.display().to_string().replace('\'', "''")
        ),
    )?;
    let mut h = start(dir.path())?;
    let result = (|| -> Result<()> {
        // The fixture is already the Shell's working directory. Avoid sending
        // a long temporary absolute path as hundreds of synchronous native
        // key records before the observer can service cursor queries.
        workflow_text(&mut h, ". .\\setup.ps1\r")?;
        wait_editor_begin(&mut h, dir.path(), 2)?;
        let legacy = std::fs::read_to_string(&mode)? == "True";
        let cases = workflow_cases::CASES;
        let mut trace = incremental_trace::Trace::open(&dir.path().join("trace.jsonl"))?;
        for (name, values, expected) in cases {
            let expected = if legacy && expected.starts_with("git worktree add") {
                expected.replace("'../work tree'", "'\"../work tree\"'")
            } else {
                (*expected).to_owned()
            };
            h.send(b"\x1b[80;25;16;1;10;1_\x1b[80;25;16;0;10;1_")?;
            h.wait_text("工作台", TIMEOUT)?;
            let before = workflow_trace(&mut h, &mut trace)?.keys;
            workflow_text(&mut h, name)?;
            let deadline = std::time::Instant::now() + TIMEOUT;
            loop {
                let state = workflow_trace(&mut h, &mut trace)?;
                if state.keys >= before + name.encode_utf16().count()
                    && state.painted_revision >= state.key_revision.unwrap_or(u64::MAX)
                    && h.viewport_contents()
                        .lines()
                        .any(|row| row.contains('›') && row.contains(name))
                {
                    break;
                }
                ensure!(
                    std::time::Instant::now() < deadline,
                    "workflow not selected: {name}: {}",
                    h.viewport_contents()
                );
                let _ = h.pump(Duration::from_millis(10));
            }
            let state = workflow_trace(&mut h, &mut trace)?;
            let applied_before = state.applied;
            let mut form_keys_before = state.keys;
            h.send(b"\r")?;
            for (index, value) in values.iter().enumerate() {
                let deadline = std::time::Instant::now() + TIMEOUT;
                loop {
                    let state = workflow_trace(&mut h, &mut trace)?;
                    let screen = h.viewport_contents();
                    if state.keys > form_keys_before
                        && state.painted_revision >= state.key_revision.unwrap_or(u64::MAX)
                        && screen.contains("填写参数：")
                        && screen.contains(&format!("参数 {}/{}", index + 1, values.len()))
                    {
                        break;
                    }
                    ensure!(
                        std::time::Instant::now() < deadline,
                        "workflow field not painted: {name}/{}: {screen}",
                        index + 1
                    );
                    let _ = h.pump(Duration::from_millis(10));
                }
                if !value.is_empty() {
                    workflow_text(&mut h, value)?;
                }
                form_keys_before = workflow_trace(&mut h, &mut trace)?.keys;
                h.send(b"\r")?;
            }
            let deadline = std::time::Instant::now() + TIMEOUT;
            while workflow_trace(&mut h, &mut trace)?.applied <= applied_before {
                ensure!(
                    std::time::Instant::now() < deadline,
                    "workflow did not finish: {name}: {}",
                    h.viewport_contents()
                );
                let _ = h.pump(Duration::from_millis(10));
            }
            h.send(b"\x1b[24~")?;
            while !snapshot.exists() {
                ensure!(
                    std::time::Instant::now() < deadline,
                    "workflow buffer snapshot missing: {name}"
                );
                let _ = h.pump(Duration::from_millis(10));
            }
            // F12 publishes by rename only after its complete buffer has
            // reached disk. Existence must not race WriteAllText's create.
            let observed = std::fs::read_to_string(&snapshot)?;
            ensure!(
                observed == expected,
                "workflow filled unexpected command: {name}: {observed:?}"
            );
            std::fs::remove_file(&snapshot)?;
            h.send(b"\x01\x7f")?;
        }
        h.send_text("Write-Output 'WORKFLOWS-STILL-EDITABLE'\r")?;
        let deadline = std::time::Instant::now() + TIMEOUT;
        while !h
            .viewport_contents()
            .lines()
            .any(|line| line.trim() == "WORKFLOWS-STILL-EDITABLE")
        {
            ensure!(
                std::time::Instant::now() < deadline,
                "post-workflow command output missing: {}",
                h.viewport_contents()
            );
            let _ = h.pump(Duration::from_millis(10));
        }
        h.finish(TIMEOUT)
    })();
    if let Err(error) = result {
        let _ = h.save_evidence(dir.path());
        let _ = h.stop();
        return Err(error).with_context(|| format!("workflow evidence: {}", dir.keep().display()));
    }
    Ok(())
}

#![cfg(all(windows, debug_assertions))]
//! Inherited-console tests run in an outer observing ConPTY, never a second
//! product ConPTY. Every session uses an isolated directory and no history.
use anyhow::{Context, Result, ensure};
use blueberry::probe::Harness;
use std::{collections::BTreeMap, time::Duration};
const TIMEOUT: Duration = Duration::from_secs(20);

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
        let dir = tempfile::tempdir()?;
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
    let dir = tempfile::tempdir()?;
    let bootstrap = blueberry::host::direct::ensure_bootstrap(dir.path())?;
    let script = dir.path().join("codec.ps1");
    std::fs::write(
        &script,
        format!("\u{feff}{}", include_str!("direct-bridge.tests.ps1")),
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
    let dir = tempfile::tempdir()?;
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
    start_delayed_executable(
        directory,
        delay,
        std::path::Path::new(env!("CARGO_BIN_EXE_blueberry")),
    )
}

fn start_delayed_executable(
    directory: &std::path::Path,
    delay: u32,
    executable: &std::path::Path,
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
        ]),
        String::new(),
    )?;
    wait_editor_begin(&mut harness, directory, 1)?;
    Ok(harness)
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
        ensure!(
            std::time::Instant::now() < deadline,
            "editor did not enter its input loop"
        );
        let _ = h.pump(Duration::from_millis(5));
    }
}

#[test]
fn direct_late_frames_wake_editor_without_another_key_or_character_bindings() -> Result<()> {
    for delay in [0, 3, 5, 10, 40, 100] {
        let dir = tempfile::tempdir()?;
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
        ensure!(
            !waits.is_empty(),
            "missing receive-to-paint spans for delay {delay}"
        );
        // This is a liveness regression with generous CI scheduling tolerance.
        // The stricter 5 ms P95 belongs to the isolated performance measurement.
        ensure!(
            waits.iter().all(|wait| *wait < 200.0),
            "late frame waited on idle polling: {waits:?}"
        );
        eprintln!("frame delay {delay} ms: receive-to-paint {waits:?}");
    }
    Ok(())
}

#[test]
fn direct_empty_line_cancels_menu_and_disabled_autotrigger_preserves_typing() -> Result<()> {
    for automatic in [true, false] {
        let dir = tempfile::tempdir()?;
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
    let dir = tempfile::tempdir()?;
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
    let dir = tempfile::tempdir()?;
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
    let dir = tempfile::tempdir()?;
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
fn direct_preserves_command_status_across_prompt_lifecycle() -> Result<()> {
    let dir = tempfile::tempdir()?;
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
    let dir = tempfile::tempdir()?;
    let mut h = start(dir.path())?;
    h.send(b"if ((Get-Command Get-FileHash -ErrorAction Stop).Source -eq 'Microsoft.PowerShell.Utility' -and (Get-Command ConvertTo-Json -ErrorAction Stop)) { Write-Output 'MODULE-AUTOLOAD-OK' }\r")?;
    wait_output_line(&mut h, "MODULE-AUTOLOAD-OK")?;
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
        let dir = tempfile::tempdir()?;
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
    let dir = tempfile::tempdir()?;
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
fn direct_dismiss_external_program_and_ctrl_c_restore_shell() -> Result<()> {
    let dir = tempfile::tempdir()?;
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
    let dir = tempfile::tempdir()?;
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

#[test]
fn direct_guided_form_cancel_preserves_right_text_and_confirm_does_not_execute() -> Result<()> {
    let dir = tempfile::tempdir()?;
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
fn direct_form_confirmation_precedes_immediately_following_typing() -> Result<()> {
    let dir = tempfile::tempdir()?;
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

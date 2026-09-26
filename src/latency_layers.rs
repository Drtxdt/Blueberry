//! Diagnostic input-to-visible-echo probe. This is not release acceptance:
//! it isolates the cost of each nested ConPTY layer before optimizing menus.

use crate::{
    host, input, metrics,
    probe::Harness,
    protocol::{Decoder, Part},
    pty,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(20);
const QUERY_BATCH: usize = 10;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn QueryPerformanceCounter(value: *mut i64) -> i32;
    fn QueryPerformanceFrequency(value: *mut i64) -> i32;
}

pub(crate) fn qpc() -> Result<i64> {
    let mut value = 0i64;
    ensure!(
        unsafe { QueryPerformanceCounter(&mut value) } != 0,
        "QueryPerformanceCounter failed"
    );
    Ok(value)
}

pub(crate) fn qpc_frequency() -> Result<f64> {
    let mut value = 0i64;
    ensure!(
        unsafe { QueryPerformanceFrequency(&mut value) } != 0 && value > 0,
        "QueryPerformanceFrequency failed"
    );
    Ok(value as f64)
}

#[derive(Clone, Copy)]
enum Mode {
    Plain,
    DirectAdapter,
    DirectService,
    Nested,
    Blueberry,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::DirectAdapter => "direct_adapter",
            Self::DirectService => "direct_service",
            Self::Nested => "nested_passthrough",
            Self::Blueberry => "blueberry",
        }
    }
}

const MODES: [Mode; 5] = [
    Mode::Plain,
    Mode::DirectAdapter,
    Mode::DirectService,
    Mode::Nested,
    Mode::Blueberry,
];

fn stats(values: &[f64]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({
        "samples": values,
        "median": (sorted[(sorted.len()-1)/2] + sorted[sorted.len()/2]) / 2.0,
        "p95": sorted[(sorted.len()*95).div_ceil(100)-1],
        "unit": "ms"
    })
}

fn shell_args() -> Vec<String> {
    vec![
        "-NoLogo".into(),
        "-NoExit".into(),
        "-NoProfile".into(),
        "-Command".into(),
        "Import-Module PSReadLine; Set-PSReadLineOption -HistorySaveStyle SaveNothing".into(),
    ]
}

fn wait_until_gone(harness: &mut Harness, marker: &str) -> Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    while marker_visible(harness, marker) {
        ensure!(
            Instant::now() < deadline,
            "old input remains visible: {marker}"
        );
        harness.pump(Duration::from_millis(25))?;
    }
    Ok(())
}

fn marker_visible(harness: &Harness, marker: &str) -> bool {
    harness.contents().contains(marker)
        || harness
            .viewport_contents()
            .replace('\n', "")
            .contains(marker)
}

fn measure_session(
    mode: Mode,
    executable: &Path,
    shell: &Path,
    session: usize,
    start: usize,
    count: usize,
) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    let directory = std::env::temp_dir().join(format!(
        "blueberry-layers-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&directory)?;
    let token = uuid::Uuid::new_v4().to_string();
    let mut env = BTreeMap::from([
        ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
        ("TERM".into(), "xterm-256color".into()),
    ]);
    let (program, args) = match mode {
        Mode::Plain => (shell, shell_args()),
        Mode::DirectAdapter => {
            let integration = pty::ensure_integration(&directory)?;
            env.insert("BLUEBERRY_TOKEN".into(), token.clone());
            env.insert("BLUEBERRY_ACTIVE".into(), "1".into());
            env.insert(
                "BLUEBERRY_EDIT_PATH".into(),
                directory.join("edit.json").to_string_lossy().into_owned(),
            );
            env.extend(pty::key_environment(&crate::config::KeyBindings::default()));
            (shell, pty::shell_args(&integration, true))
        }
        Mode::Nested => (
            executable,
            vec![
                "layer-passthrough".into(),
                "--shell".into(),
                shell.to_string_lossy().into_owned(),
            ],
        ),
        Mode::DirectService => (
            executable,
            vec![
                "layer-direct-service".into(),
                "--shell".into(),
                shell.to_string_lossy().into_owned(),
                "--report".into(),
                directory
                    .join("direct-events.json")
                    .to_string_lossy()
                    .into_owned(),
            ],
        ),
        Mode::Blueberry => {
            env.insert(
                "BLUEBERRY_DATA_DIR".into(),
                directory.to_string_lossy().into_owned(),
            );
            (
                executable,
                vec![
                    "run".into(),
                    "--shell".into(),
                    shell.to_string_lossy().into_owned(),
                    "--no-profile".into(),
                    "--data-dir".into(),
                    directory.to_string_lossy().into_owned(),
                ],
            )
        }
    };
    let mut harness = Harness::start(program, &args, &directory, &env, token)
        .with_context(|| format!("start {} session {session}", mode.name()))?;
    harness.wait_text("PS ", TIMEOUT)?;
    if matches!(mode, Mode::DirectService) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let ready = std::fs::read(directory.join("direct-events.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|report| report["ready"] == true && report["prompt_end"] == true);
            if ready {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "direct service did not reach a ready prompt"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    let mut values = Vec::with_capacity(count);
    let mut confirmed = Vec::new();
    let mut delivered = Vec::new();
    let frequency = qpc_frequency()?;
    for offset in 0..count {
        let marker = format!("sslat{session}x{}x{}", start + offset, mode.name());
        ensure!(
            !marker_visible(&harness, &marker),
            "stale probe marker before send"
        );
        let started = Instant::now();
        harness.send(marker.as_bytes())?;
        if matches!(mode, Mode::DirectService) {
            let deadline = Instant::now() + TIMEOUT;
            while !marker_visible(&harness, &marker) {
                ensure!(
                    Instant::now() < deadline,
                    "direct-service echo did not become visible"
                );
                harness.pump(Duration::from_millis(25))?;
            }
        } else {
            harness.wait_text(&marker, TIMEOUT)?;
        }
        values.push(started.elapsed().as_secs_f64() * 1000.0);
        if matches!(mode, Mode::DirectService) {
            let sent_qpc = qpc()?;
            harness.send(&input::protocol_chord("F12", 's'))?;
            let deadline = Instant::now() + TIMEOUT;
            let report_path = directory.join("direct-events.json");
            loop {
                let report = std::fs::read(&report_path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
                if report
                    .as_ref()
                    .and_then(|value| value["confirmed_buffer_events"].as_u64())
                    .unwrap_or(0)
                    >= (offset + 1) as u64
                {
                    let report = report.unwrap();
                    let confirmed_qpc = report["psreadline_confirmed_qpc"]
                        .as_i64()
                        .context("missing PSReadLine confirmation timestamp")?;
                    let delivered_qpc = report["pipe_received_qpc"]
                        .as_i64()
                        .context("missing sidecar delivery timestamp")?;
                    ensure!(
                        confirmed_qpc >= sent_qpc && delivered_qpc >= confirmed_qpc,
                        "direct-service counter order invalid"
                    );
                    confirmed.push((confirmed_qpc - sent_qpc) as f64 * 1000.0 / frequency);
                    delivered.push((delivered_qpc - confirmed_qpc) as f64 * 1000.0 / frequency);
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "direct service did not receive PSReadLine buffer event"
                );
                thread::sleep(Duration::from_millis(1));
            }
        }
        harness.send(b"\x01\x7f")?;
        wait_until_gone(&mut harness, &marker)?;
    }
    if matches!(mode, Mode::DirectService) {
        harness.send(b"g")?;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let report = std::fs::read(directory.join("direct-events.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            if report
                .as_ref()
                .and_then(|value| value["confirmed_buffer_events"].as_u64())
                .unwrap_or(0)
                >= (count + 1) as u64
            {
                ensure!(
                    report.unwrap()["last_line_utf16"] == 1,
                    "delegated SelfInsert did not leave exactly one confirmed character"
                );
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "direct key hook did not report a confirmed PSReadLine buffer"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    harness.stop()?;
    let _ = std::fs::remove_dir_all(&directory);
    Ok((values, confirmed, delivered))
}

/// Alternate all five paths by session so thermal and OS-cache drift does
/// not systematically favor one mode. The report deliberately retains raw
/// samples and labels the absence of a menu in the plain/nested paths.
pub fn run(executable: &Path, shell: &Path, samples: u16) -> Result<Value> {
    ensure!(samples > 0, "need at least one sample per mode");
    let mut by_mode: BTreeMap<&str, Vec<f64>> =
        MODES.iter().map(|mode| (mode.name(), Vec::new())).collect();
    let mut first_by_mode: BTreeMap<&str, Vec<f64>> =
        MODES.iter().map(|mode| (mode.name(), Vec::new())).collect();
    let mut warm_by_mode: BTreeMap<&str, Vec<f64>> =
        MODES.iter().map(|mode| (mode.name(), Vec::new())).collect();
    let mut direct_confirmed = Vec::new();
    let mut direct_delivered = Vec::new();
    let count = usize::from(samples);
    let sessions = count.div_ceil(QUERY_BATCH);
    for session in 0..sessions {
        let start = session * QUERY_BATCH;
        let batch = QUERY_BATCH.min(count - start);
        for offset in 0..MODES.len() {
            let mode = MODES[(session + offset) % MODES.len()];
            let (values, confirmed, delivered) =
                measure_session(mode, executable, shell, session, start, batch)?;
            direct_confirmed.extend(confirmed);
            direct_delivered.extend(delivered);
            first_by_mode.get_mut(mode.name()).unwrap().push(values[0]);
            warm_by_mode
                .get_mut(mode.name())
                .unwrap()
                .extend_from_slice(&values[1..]);
            by_mode.get_mut(mode.name()).unwrap().extend(values);
        }
    }
    let plain = &by_mode["plain"];
    let direct = &by_mode["direct_adapter"];
    let direct_service = &by_mode["direct_service"];
    let nested = &by_mode["nested_passthrough"];
    let blueberry = &by_mode["blueberry"];
    ensure!(
        [plain, direct, direct_service, nested, blueberry]
            .iter()
            .all(|samples| samples.len() == count),
        "layer probe did not collect all samples"
    );
    Ok(json!({
        "schema": 3,
        "diagnostic_only": true,
        "platform": "windows",
        "shell": shell,
        "executable_sha256": metrics::executable_sha256(executable)?,
        "samples_per_mode": count,
        "direct_service_confirmed_buffer_events": count + sessions,
        "direct_service_delegated_self_insert_sessions": sessions,
        "direct_service_send_to_confirmed_buffer": stats(&direct_confirmed),
        "direct_service_confirmed_buffer_to_pipe": stats(&direct_delivered),
        "terminal_rows": 30,
        "terminal_columns": 120,
        "profile_mode": "no_profile",
        "method": "Alternating fresh sessions: plain PowerShell, direct adapter, direct PowerShell with Rust pipe service, minimal nested ConPTY forwarder, and full Blueberry. The outer Harness sends a unique ASCII marker and records when it first appears in viewport rows. In direct-service mode, a separate private PSReadLine chord must subsequently deliver the confirmed buffer to Rust before the next sample. First query is separate from warm queries. No candidate is required; this is not release acceptance. OS file caches are retained.",
        "plain": stats(plain),
        "direct_adapter": stats(direct),
        "direct_service": stats(direct_service),
        "nested_passthrough": stats(nested),
        "blueberry": stats(blueberry),
        "first_query": first_by_mode.iter().map(|(mode, samples)| (*mode, stats(samples))).collect::<BTreeMap<_, _>>(),
        "warm_query": warm_by_mode.iter().map(|(mode, samples)| (*mode, if samples.is_empty() { Value::Null } else { stats(samples) })).collect::<BTreeMap<_, _>>(),
    }))
}

/// A deliberately minimal host with the same Windows input reader and inner
/// ConPTY as Blueberry, but no adapter, completion, menu or tracing.
pub fn passthrough(shell: &Path) -> Result<u32> {
    let _raw = host::RawMode::enable()?;
    let (columns, rows) = crossterm::terminal::size().unwrap_or((120, 30));
    let mut session = pty::spawn(
        shell,
        &shell_args(),
        &std::env::current_dir()?,
        &BTreeMap::from([("BLUEBERRY_NO_HISTORY".into(), "1".into())]),
        rows,
        columns,
    )?;
    let mut reader = std::mem::replace(&mut session.reader, Box::new(std::io::empty()));
    let writer = Arc::new(Mutex::new(std::mem::replace(
        &mut session.writer,
        Box::new(std::io::sink()),
    )));
    let output_writer = writer.clone();
    thread::spawn(move || {
        let mut out = std::io::stdout().lock();
        let mut decoder = Decoder::new(String::new());
        let mut screen = vt100::Parser::new(rows, columns, 0);
        let mut buffer = [0u8; 16_384];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            for part in decoder.feed(&buffer[..count]) {
                match part {
                    Part::Data(data) => {
                        screen.process(&data);
                        if out.write_all(&data).is_err() || out.flush().is_err() {
                            return;
                        }
                    }
                    Part::CursorQuery(private) => {
                        let (row, col) = screen.screen().cursor_position();
                        let reply = format!(
                            "\x1b[{}{};{}R",
                            if private { "?" } else { "" },
                            row + 1,
                            col + 1
                        );
                        let Ok(mut input) = output_writer.lock() else {
                            return;
                        };
                        if input.write_all(reply.as_bytes()).is_err() || input.flush().is_err() {
                            return;
                        }
                    }
                    Part::Message(_) => {}
                }
            }
        }
    });
    let mut input_reader = crate::windows_input::Reader::new()?;
    loop {
        let mut bytes = Vec::new();
        for event in input_reader.read_batch(65_536)? {
            if let Some(action) = input::translate(event, false) {
                match action {
                    input::Input::Bytes(value) => bytes.extend(value),
                    input::Input::Resize(width, height) => {
                        session.master.resize(portable_pty::PtySize {
                            rows: height,
                            cols: width,
                            pixel_width: 0,
                            pixel_height: 0,
                        })?;
                    }
                    input::Input::Tab => bytes.push(b'\t'),
                    input::Input::Dismiss => bytes.push(0x1b),
                    _ => {}
                }
            }
        }
        if !bytes.is_empty() {
            let mut input = writer
                .lock()
                .map_err(|_| anyhow::anyhow!("input writer poisoned"))?;
            input.write_all(&bytes)?;
            input.flush()?;
        }
        if session.child.try_wait()?.is_some() {
            return Ok(0);
        }
    }
}

/// Experimental sidecar: PowerShell owns the inherited console, while Rust
/// receives authenticated adapter events on the existing session pipe. There
/// is no automatic key hook or menu here; layer-probe tests only the plumbing.
pub fn direct_service(shell: &Path, report: &Path) -> Result<u32> {
    let directory = report.parent().context("direct report needs a directory")?;
    std::fs::create_dir_all(directory)?;
    let integration = pty::ensure_integration(directory)?;
    let mut pipe = crate::pipe::PipeServer::new()?;
    let mut child = Command::new(shell)
        .args(pty::shell_args(&integration, true))
        .env("BLUEBERRY_ACTIVE", "1")
        .env("BLUEBERRY_DIRECT_PIPE", "1")
        .env(
            "BLUEBERRY_DIRECT_STATUS_PATH",
            directory.join("adapter-direct-status.txt"),
        )
        .env("BLUEBERRY_PIPE_NAME", pipe.name())
        .env("BLUEBERRY_TOKEN", uuid::Uuid::new_v4().to_string())
        .env("BLUEBERRY_EDIT_PATH", directory.join("edit.json"))
        .env("BLUEBERRY_NO_HISTORY", "1")
        .envs(pty::key_environment(&crate::config::KeyBindings::default()))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("start direct PowerShell {}", shell.display()))?;
    let _job = match DirectChildJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            return Err(error);
        }
    };
    pipe.set_client_pid(child.id());
    let mut confirmed_buffer_events = 0u64;
    let mut received_events = 0u64;
    let mut last_confirmed_qpc = None;
    let mut last_received_qpc = None;
    let mut last_line_utf16 = None;
    let mut ready = false;
    let mut prompt_end = false;
    std::fs::write(
        report,
        serde_json::to_vec(&json!({
            "confirmed_buffer_events": 0,
            "received_events": 0,
            "status": "waiting",
        }))?,
    )?;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.code().unwrap_or(1) as u32);
        }
        match pipe.read_json() {
            Ok(envelope) => {
                received_events += 1;
                if envelope["payload"]["event"] == "capabilities"
                    && envelope["payload"]["ready"] == true
                {
                    ready = true;
                }
                if envelope["payload"]["event"] == "prompt_end" {
                    prompt_end = true;
                }
                if envelope["payload"]["event"] == "buffer" {
                    confirmed_buffer_events += 1;
                    last_confirmed_qpc = envelope["payload"]["diagnostic_qpc"].as_i64();
                    last_received_qpc = Some(qpc()?);
                    last_line_utf16 = envelope["payload"]["line"]
                        .as_str()
                        .map(|line| line.encode_utf16().count());
                }
                std::fs::write(
                    report,
                    serde_json::to_vec(&json!({
                        "confirmed_buffer_events": confirmed_buffer_events,
                        "received_events": received_events,
                        "ready": ready,
                        "prompt_end": prompt_end,
                        "last_event": envelope["payload"]["event"],
                        "psreadline_confirmed_qpc": last_confirmed_qpc,
                        "pipe_received_qpc": last_received_qpc,
                        "last_line_utf16": last_line_utf16,
                        "source": "PSReadLine adapter via session pipe",
                    }))?,
                )?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub(crate) struct DirectChildJob(windows_sys::Win32::Foundation::HANDLE);

impl DirectChildJob {
    pub(crate) fn attach(child: &std::process::Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            ensure!(
                !handle.is_null(),
                "create direct-service child job: {}",
                std::io::Error::last_os_error()
            );
            let job = Self(handle);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            ensure!(
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as _,
                    std::mem::size_of_val(&info) as u32,
                ) != 0,
                "configure direct-service child job: {}",
                std::io::Error::last_os_error()
            );
            ensure!(
                AssignProcessToJobObject(handle, child.as_raw_handle() as _) != 0,
                "attach direct-service child: {}",
                std::io::Error::last_os_error()
            );
            Ok(job)
        }
    }
}

impl Drop for DirectChildJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

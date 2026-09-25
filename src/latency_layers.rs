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
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(20);
const QUERY_BATCH: usize = 10;

#[derive(Clone, Copy)]
enum Mode {
    Plain,
    DirectAdapter,
    Nested,
    Blueberry,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::DirectAdapter => "direct_adapter",
            Self::Nested => "nested_passthrough",
            Self::Blueberry => "blueberry",
        }
    }
}

const MODES: [Mode; 4] = [
    Mode::Plain,
    Mode::DirectAdapter,
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
    while harness.viewport_contents().contains(marker) {
        ensure!(
            Instant::now() < deadline,
            "old input remains visible: {marker}"
        );
        harness.pump(Duration::from_millis(25))?;
    }
    Ok(())
}

fn measure_session(
    mode: Mode,
    executable: &Path,
    shell: &Path,
    session: usize,
    start: usize,
    count: usize,
) -> Result<Vec<f64>> {
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
    let mut values = Vec::with_capacity(count);
    for offset in 0..count {
        let marker = format!("sslat{session}x{}x{}", start + offset, mode.name());
        ensure!(
            !harness.viewport_contents().contains(&marker),
            "stale probe marker before send"
        );
        let started = Instant::now();
        harness.send(marker.as_bytes())?;
        harness.wait_text(&marker, TIMEOUT)?;
        values.push(started.elapsed().as_secs_f64() * 1000.0);
        harness.send(b"\x01\x7f")?;
        wait_until_gone(&mut harness, &marker)?;
    }
    harness.stop()?;
    let _ = std::fs::remove_dir_all(&directory);
    Ok(values)
}

/// Alternate all three paths by session so thermal and OS-cache drift does
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
    let count = usize::from(samples);
    let sessions = count.div_ceil(QUERY_BATCH);
    for session in 0..sessions {
        let start = session * QUERY_BATCH;
        let batch = QUERY_BATCH.min(count - start);
        for offset in 0..MODES.len() {
            let mode = MODES[(session + offset) % MODES.len()];
            let values = measure_session(mode, executable, shell, session, start, batch)?;
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
    let nested = &by_mode["nested_passthrough"];
    let blueberry = &by_mode["blueberry"];
    ensure!(
        [plain, direct, nested, blueberry]
            .iter()
            .all(|samples| samples.len() == count),
        "layer probe did not collect all samples"
    );
    Ok(json!({
        "schema": 2,
        "diagnostic_only": true,
        "platform": "windows",
        "shell": shell,
        "executable_sha256": metrics::executable_sha256(executable)?,
        "samples_per_mode": count,
        "terminal_rows": 30,
        "terminal_columns": 120,
        "profile_mode": "no_profile",
        "method": "Alternating fresh sessions: plain PowerShell, direct PowerShell with Blueberry adapter but no Rust host, minimal Rust forwarder with inner ConPTY, and full Blueberry host. The same outer Harness sends a unique ASCII marker and records when it first appears in viewport rows. First query of each session is reported separately from warm queries. No candidate is required; this is not release acceptance. OS file caches are retained.",
        "plain": stats(plain),
        "direct_adapter": stats(direct),
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

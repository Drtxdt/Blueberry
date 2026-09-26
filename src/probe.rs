//! Reproducible real-PTY adapter validation. No user profile or terminal settings are edited.
use crate::{
    engine::CommandIndex,
    protocol::{Decoder, Part},
    pty,
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub struct Harness {
    session: pty::Session,
    receive: mpsc::Receiver<(Vec<u8>, Instant)>,
    last_output_arrival: Option<Instant>,
    decoder: Decoder,
    messages: VecDeque<Value>,
    screen: vt100::Parser,
    trace: Vec<u8>,
    protocol_seen: bool,
    known_empty_buffer: bool,
    last_capabilities: Option<Value>,
}

impl Harness {
    pub fn start(
        program: &Path,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<String, String>,
        token: String,
    ) -> Result<Self> {
        let mut session = pty::spawn(program, args, cwd, env, 30, 120)?;
        let mut reader = std::mem::replace(&mut session.reader, Box::new(std::io::empty()));
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0u8; 16384];
            while let Ok(n) = reader.read(&mut buffer) {
                let arrived = Instant::now();
                if n == 0 || send.send((buffer[..n].to_vec(), arrived)).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            session,
            receive,
            decoder: Decoder::new(token),
            messages: VecDeque::new(),
            screen: vt100::Parser::new(30, 120, 0),
            trace: Vec::new(),
            protocol_seen: false,
            known_empty_buffer: false,
            last_capabilities: None,
            last_output_arrival: None,
        })
    }
    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.known_empty_buffer = false;
        self.session.writer.write_all(bytes)?;
        self.session.writer.flush()?;
        Ok(())
    }
    pub fn pump(&mut self, timeout: Duration) -> Result<()> {
        let (bytes, arrived) = self.receive.recv_timeout(timeout).with_context(|| {
            format!(
                "PTY timeout/closed. Screen: {}; raw tail: {:?}",
                self.screen.screen().contents(),
                String::from_utf8_lossy(&self.trace)
            )
        })?;
        for part in self.decoder.feed(&bytes) {
            match part {
                Part::Data(data) => {
                    self.last_output_arrival = Some(arrived);
                    self.trace.extend_from_slice(&data);
                    if self.trace.len() > 4096 {
                        self.trace.drain(..self.trace.len() - 4096);
                    }
                    self.screen.process(&data);
                }
                Part::Message(value) => {
                    self.protocol_seen = true;
                    if value["event"] == "capabilities" {
                        self.last_capabilities = Some(value.clone());
                    }
                    match value["event"].as_str() {
                        Some("prompt_end") => self.known_empty_buffer = true,
                        Some("buffer") => self.known_empty_buffer = value["line"] == "",
                        Some("execute" | "editing") => self.known_empty_buffer = false,
                        _ => {}
                    }
                    self.messages.push_back(value);
                }
                Part::CursorQuery(private) => {
                    let (row, col) = self.screen.screen().cursor_position();
                    // A terminal cursor response is not an editor keystroke.
                    // Preserve the last confirmed empty-buffer state, especially
                    // when ConPTY asks for its position after prompt_end.
                    self.session.writer.write_all(
                        format!(
                            "\x1b[{}{};{}R",
                            if private { "?" } else { "" },
                            row + 1,
                            col + 1
                        )
                        .as_bytes(),
                    )?;
                    self.session.writer.flush()?;
                }
            }
        }
        Ok(())
    }
    pub fn event(&mut self, name: &str, timeout: Duration) -> Result<Value> {
        let until = Instant::now() + timeout;
        loop {
            while let Some(value) = self.messages.pop_front() {
                if value["event"] == "error" && name != "error" {
                    bail!("Adapter: {value}");
                }
                if value["event"] == name {
                    return Ok(value);
                }
            }
            self.pump(until.saturating_duration_since(Instant::now()))
                .with_context(|| format!("waiting for adapter event {name}"))?;
        }
    }
    pub fn wait_text(&mut self, text: &str, timeout: Duration) -> Result<()> {
        let until = Instant::now() + timeout;
        while !self.screen.screen().contents().contains(text) {
            self.pump(until.saturating_duration_since(Instant::now()))?;
        }
        Ok(())
    }
    pub fn contents(&self) -> String {
        self.screen.screen().contents()
    }
    /// Arrival of the bytes that most recently changed the VT observation.
    /// This excludes test-thread scheduling and is not a physical pixel time.
    pub fn last_output_arrival(&self) -> Option<Instant> {
        self.last_output_arrival
    }
    /// Physical rows for visual assertions. `contents()` joins soft-wrapped
    /// lines, including historical rows later covered by a menu after resize.
    pub fn viewport_contents(&self) -> String {
        self.screen
            .screen()
            .rows(0, self.screen.screen().size().1)
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_owned()
    }
    pub fn process_id(&self) -> Option<u32> {
        self.session.child.process_id()
    }
    pub fn capabilities(&self) -> Option<&Value> {
        self.last_capabilities.as_ref()
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        self.session.master.resize(portable_pty::PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        self.screen.screen_mut().set_size(rows, cols);
        Ok(())
    }
    pub fn wait_line(&mut self, text: &str, timeout: Duration) -> Result<()> {
        let until = Instant::now() + timeout;
        while !self.contents().lines().any(|line| line.trim() == text) {
            self.pump(until.saturating_duration_since(Instant::now()))?;
        }
        Ok(())
    }
    pub fn stop(&mut self) -> Result<()> {
        self.session.child.kill()?;
        self.session.child.wait()?;
        Ok(())
    }
    pub fn finish(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let needs_cancel = !self.known_empty_buffer;
        self.messages.clear();
        self.trace.clear();
        if needs_cancel {
            if self.protocol_seen {
                // A cancelled empty line need not emit another prompt. Clear
                // the edit and confirm the real buffer before typing exit.
                self.send(b"\x01\x7f")?;
                self.send(b"\x1b[32;5u")?;
                loop {
                    let buffer =
                        self.event("buffer", deadline.saturating_duration_since(Instant::now()))?;
                    if buffer["line"] == "" {
                        break;
                    }
                }
            } else {
                self.send(b"\x01\x7f")?;
                // Release probes use the Windows PSReadLine editing mode.
                // SelectAll + Backspace clears our synthetic input without
                // relying on ETX, which an outer ConPTY can consume as a
                // console control event. This is outside every timed sample.
                let settle = Instant::now() + Duration::from_millis(250).min(timeout);
                while Instant::now() < settle {
                    let _ = self.pump(Duration::from_millis(25));
                }
            }
        }
        self.send(b"exit\r")?;
        loop {
            if let Some(status) = self.session.child.try_wait()? {
                ensure!(
                    status.success(),
                    "Host exited with code {}",
                    status.exit_code()
                );
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "Host did not exit normally before timeout; screen:\n{}",
                self.viewport_contents()
            );
            let _ = self.pump(Duration::from_millis(50));
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.session.child.kill();
    }
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                if entry.path().is_file() {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn stats(values: &[f64]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({"samples":values,"median":(sorted[(sorted.len()-1)/2]+sorted[sorted.len()/2])/2.0,"p95":sorted[((sorted.len() as f64*0.95).ceil() as usize).saturating_sub(1)],"unit":"ms"})
}

pub fn run(shell: &Path, iterations: u16, no_profile: bool) -> Result<Value> {
    run_with_adapter(shell, iterations, no_profile, None)
}

pub fn run_with_adapter(
    shell: &Path,
    iterations: u16,
    no_profile: bool,
    adapter_script: Option<&Path>,
) -> Result<Value> {
    let directory = std::env::temp_dir().join(format!("blueberry-probe-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory)?;
    let temporary = Temporary(directory);
    let integration = match adapter_script {
        Some(path) => path.to_path_buf(),
        None => pty::ensure_integration(&temporary.0)?,
    };
    let adapter_source = adapter_script
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "embedded".to_owned());
    let profile_mode = if no_profile {
        "no_profile"
    } else {
        "with_profile"
    };
    let edit_path = temporary.0.join("edit.json");
    let cwd = std::env::current_dir()?;
    let token = uuid::Uuid::new_v4().to_string();
    let mut env = BTreeMap::from([
        ("BLUEBERRY_TOKEN".into(), token.clone()),
        (
            "BLUEBERRY_EDIT_PATH".into(),
            edit_path.to_string_lossy().into(),
        ),
        ("BLUEBERRY_ACTIVE".into(), "1".into()),
        ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
        ("ISTERM".into(), "1".into()),
        ("TERM".into(), "xterm-256color".into()),
    ]);
    env.extend(pty::key_environment(&crate::config::KeyBindings::default()));
    let mut baseline = Vec::new();
    let mut integrated = Vec::new();
    let mut queries = Vec::new();
    let mut baseline_input = Vec::new();
    let mut integrated_input = Vec::new();
    let mut psreadline_versions = Vec::new();
    let mut shell_versions = Vec::new();
    let mut json_initialization = Vec::new();
    let baseline_module_import = std::env::var("BLUEBERRY_TEST_PSREADLINE_MODULE")
        .ok()
        .filter(|path| !path.is_empty())
        .map(|path| {
            let quoted = path.replace('\'', "''");
            format!(
                "Import-Module '{quoted}' -Force -ErrorAction Stop; if ((Get-Module PSReadLine).ModuleBase -ine (Split-Path -LiteralPath '{quoted}')) {{ throw 'Unexpected baseline PSReadLine version' }}; "
            )
        })
        .unwrap_or_default();
    let timeout = Duration::from_secs(20);
    for iteration in 0..iterations {
        // Alternate order to reduce systematic warm-cache ordering bias.
        for adapted in if iteration % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let args = if adapted {
                pty::shell_args(&integration, no_profile)
            } else {
                let frame = format!("\x1b]7776;{token};{{\"event\":\"prompt_end\"}}\x07");
                let mut args = vec!["-NoLogo".into(), "-NoExit".into()];
                if no_profile {
                    args.push("-NoProfile".into());
                }
                args.extend([
                    "-Command".into(),
                    format!(
                        "[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); [Console]::InputEncoding = [Text.UTF8Encoding]::new($false); {baseline_module_import}Import-Module PSReadLine; Set-PSReadLineOption -HistorySaveStyle SaveNothing; $global:__blueberry_original_prompt = $ExecutionContext.InvokeCommand.GetCommand('Prompt', [System.Management.Automation.CommandTypes]::Function); $global:__blueberry_original_prompt = if ($null -eq $global:__blueberry_original_prompt) {{ {{ 'PS> ' }} }} else {{ $global:__blueberry_original_prompt.ScriptBlock }}; function global:prompt {{ $promptState = & {{ param($savedLastExitCode) try {{ $originalOutput = @(& $global:__blueberry_original_prompt); [pscustomobject]@{{ output = $originalOutput; error = $null; lastExitCode = $savedLastExitCode }} }} catch {{ [pscustomobject]@{{ output = @(); error = $_; lastExitCode = $savedLastExitCode }} }} }} ($ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE')); [Console]::Write('{}'); $global:LASTEXITCODE = $promptState.lastExitCode; if ($null -ne $promptState.error) {{ throw $promptState.error }}; return $promptState.output }}",
                        frame.replace('\'', "''")
                    ),
                ]);
                args
            };
            let started = Instant::now();
            let mut harness = Harness::start(shell, &args, &cwd, &env, token.clone())?;
            harness.event("prompt_end", timeout)?;
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            harness.send(b"BLUEBERRY_INPUT_READY")?;
            harness.wait_text("BLUEBERRY_INPUT_READY", timeout)?;
            let input_elapsed = started.elapsed().as_secs_f64() * 1000.0;
            if !adapted {
                baseline.push(elapsed);
                baseline_input.push(input_elapsed);
                harness.stop()?;
                continue;
            }
            integrated.push(elapsed);
            integrated_input.push(input_elapsed);
            let capabilities = harness
                .capabilities()
                .context("missing adapter capabilities")?;
            psreadline_versions.push(
                capabilities["psreadline_version"]
                    .as_str()
                    .context("missing PSReadLine version")?
                    .to_owned(),
            );
            shell_versions.push(
                capabilities["shell_version"]
                    .as_str()
                    .context("missing PowerShell version")?
                    .to_owned(),
            );
            json_initialization.push(
                capabilities["json_initialization_ms"]
                    .as_f64()
                    .context("missing adapter JSON initialization timing")?,
            );
            harness.send(b"\x1b[24~s")?;
            let initial = harness.event("buffer", timeout)?;
            let initial_line = initial["line"].as_str().context("missing initial line")?;
            std::fs::write(
                &edit_path,
                serde_json::to_vec(
                    &json!({"expectedLine":initial_line,"expectedCursor":initial["cursor"],"start":0,"length":initial_line.encode_utf16().count(),"text":"gi"}),
                )?,
            )?;
            harness.send(b"\x1b[24~a")?;
            let buffer = harness.event("buffer", timeout)?;
            ensure!(
                buffer["line"] == "gi" && buffer["cursor"] == 2,
                "Unexpected buffer: {buffer}"
            );
            for _ in 0..10 {
                let started = Instant::now();
                harness.send(b"\x1b[24~s")?;
                harness.event("buffer", timeout)?;
                queries.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            std::fs::write(
                &edit_path,
                serde_json::to_vec(
                    &json!({"expectedLine":"gi","expectedCursor":2,"start":0,"length":2,"text":"git"}),
                )?,
            )?;
            harness.send(b"\x1b[24~a")?;
            let buffer = harness.event("buffer", timeout)?;
            ensure!(
                buffer["line"] == "git",
                "Acceptance did not replace the buffer: {buffer}"
            );
            let mut command_count = 0;
            loop {
                harness.send(b"\x1b[24~c")?;
                let commands = harness.event("commands", timeout)?;
                command_count += commands["commands"].as_array().map_or(0, Vec::len);
                if commands["complete"] == true || commands["snapshot"].is_null() {
                    break;
                }
            }
            ensure!(command_count > 0, "No loaded commands");
            // Clearing and executing proves protocol keys did not become command text.
            std::fs::write(
                &edit_path,
                serde_json::to_vec(
                    &json!({"expectedLine":"git","expectedCursor":3,"start":0,"length":3,"text":"Write-Output BLUEBERRY_PROBE_OK"}),
                )?,
            )?;
            harness.send(b"\x1b[24~a")?;
            harness.event("buffer", timeout)?;
            harness.send(b"\r")?;
            harness.event("execute", timeout)?;
            harness.event("prompt_end", timeout)?;
            harness.wait_line("BLUEBERRY_PROBE_OK", timeout)?;
            harness.stop()?;
        }
    }
    let index_started = Instant::now();
    let index = CommandIndex::discover();
    let index_ms = index_started.elapsed().as_secs_f64() * 1000.0;
    let mut lookups = Vec::new();
    for _ in 0..100 {
        let started = Instant::now();
        let result = index.complete("gi", 2, &cwd, 100);
        std::hint::black_box(result);
        lookups.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let deltas: Vec<f64> = integrated
        .iter()
        .zip(&baseline)
        .map(|(a, b)| a - b)
        .collect();
    let input_deltas: Vec<f64> = integrated_input
        .iter()
        .zip(&baseline_input)
        .map(|(a, b)| a - b)
        .collect();
    let executable_sha256 = crate::metrics::executable_sha256(&std::env::current_exe()?)?;
    Ok(json!({
        "schema":2,"build":if cfg!(debug_assertions) {"debug"} else {"release"},
        "executable_sha256":executable_sha256,
        "platform":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "shell":shell,"adapter_source":adapter_source,"profile_mode":profile_mode,
        "shell_versions":shell_versions,"psreadline_versions":psreadline_versions,
        "json_initialization_ms":stats(&json_initialization),
        "no_profile":no_profile,"iterations":iterations,
        "public_keys": "default keys, identical JSON and startup environment as the host",
        "method":"Alternating fresh ConPTY pwsh processes; UTF-8 console and history saving disabled in both cases. The baseline wraps the profile's existing prompt and emits a controlled marker after its output; the adapter source is reported above. Prompt marker timestamp, not first visible frame. OS caches are not cleared. Query timings include PSReadLine + OSC + ConPTY roundtrip. This does not measure outer-host rendering or RSS.",
        "baseline_prompt":stats(&baseline),"adapter_prompt":stats(&integrated),
        "baseline_first_input_echo":stats(&baseline_input),"adapter_first_input_echo":stats(&integrated_input),
        "paired_first_input_delta":stats(&input_deltas),
        "paired_adapter_delta":stats(&deltas),"buffer_query_roundtrip":stats(&queries),
        "command_index_build_ms":index_ms,"command_count":index.len(),"root_lookup":stats(&lookups),
        "verified":["PSReadLine real buffer","validated replacement gi -> git","loaded shell commands","execute -> next prompt"]
    }))
}

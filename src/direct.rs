//! Experimental inherited-console host. PSReadLine is the sole input reader
//! and console writer. This service reuses the nested host's completion worker.
use super::*;
use anyhow::{bail, ensure};
use std::{
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// PIPE_NOWAIT avoids duplex serialization of synchronous named-pipe handles.
/// Use a kernel high resolution timer instead of the scheduler's coarse Sleep.
struct ReceiveTimer(std::os::windows::io::OwnedHandle);
impl ReceiveTimer {
    fn new() -> Result<Self> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::System::Threading::{CreateWaitableTimerExW, SetWaitableTimer};
        let handle =
            unsafe { CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), 2, 0x1f0003) };
        ensure!(
            !handle.is_null(),
            "high resolution pipe receiver timer unavailable: {}",
            std::io::Error::last_os_error()
        );
        let owned = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) };
        let due = -10_000i64;
        ensure!(
            unsafe { SetWaitableTimer(handle, &due, 1, None, std::ptr::null(), 0) } != 0,
            "cannot arm pipe receiver timer: {}",
            std::io::Error::last_os_error()
        );
        Ok(Self(owned))
    }
    fn wait(&self) {
        use std::os::windows::io::AsRawHandle;
        unsafe {
            windows_sys::Win32::System::Threading::WaitForSingleObject(self.0.as_raw_handle(), 100);
        }
    }
}

/// Keep a query and its visible frame separate: asynchronous results must not
/// change which candidate an acceptance request identifies.
struct Session {
    revision: u64,
    line: String,
    cursor: usize,
    width: u16,
    rows: usize,
    frame: u64,
    visible: std::collections::VecDeque<(u64, String, Candidate, Completion)>,
    accepting: bool,
    selected: usize,
    selected_id: Option<String>,
    cwd: PathBuf,
    ui: Option<Workbench>,
    values_requested: u64,
    history_pending: bool,
    history_partial: bool,
    details: bool,
}
struct Workbench {
    input: String,
    high_surrogate: Option<u16>,
    form: Option<crate::hub::TemplateForm>,
    project: bool,
    templates: BTreeMap<String, crate::hub::CommandTemplate>,
    error: Option<String>,
    generation: u64,
}

impl Session {
    fn new() -> Self {
        Self {
            revision: 0,
            line: String::new(),
            cursor: 0,
            width: 120,
            rows: 30,
            frame: 0,
            visible: Default::default(),
            accepting: false,
            selected: 0,
            selected_id: None,
            cwd: PathBuf::new(),
            ui: None,
            values_requested: 0,
            history_pending: false,
            history_partial: false,
            details: false,
        }
    }
    fn query(&mut self, value: &Value) -> Result<()> {
        let revision = value["revision"].as_u64().context("missing revision")?;
        ensure!(revision > self.revision, "stale query revision");
        let line = value["line"].as_str().context("missing confirmed line")?;
        let cursor16 = value["cursor"]
            .as_u64()
            .context("missing confirmed cursor")?;
        let cursor =
            protocol::utf16_to_byte(line, cursor16 as usize).context("invalid UTF-16 cursor")?;
        self.revision = revision;
        self.line = line.into();
        self.cursor = cursor;
        self.width = value["width"].as_u64().unwrap_or(120).clamp(1, 512) as u16;
        self.rows = value["rows"].as_u64().unwrap_or(30).clamp(1, 512) as usize;
        self.visible.clear();
        self.accepting = false;
        self.selected = 0;
        self.selected_id = None;
        Ok(())
    }
    fn render(&mut self, completion: Completion, settings: &Config, token: &str) -> Value {
        self.frame += 1;
        if let Some(id) = &self.selected_id
            && let Some(index) = completion
                .candidates
                .iter()
                .position(|c| c.identity() == id)
        {
            self.selected = index;
        }
        self.selected = self
            .selected
            .min(completion.candidates.len().saturating_sub(1));
        let candidate = completion.candidates.get(self.selected);
        let id = candidate
            .map(|c| c.identity().to_owned())
            .unwrap_or_default();
        let tail_rows = tail_rows(&self.line[self.cursor..], self.width);
        let menu = crate::menu::render_with_state(
            &completion.candidates,
            self.selected,
            &self.line[..self.cursor],
            self.width.saturating_sub(1),
            settings,
            crate::menu::MenuState {
                incomplete: completion.incomplete,
                details: self.details,
                searching: false,
                diagnostic: self.ui.as_ref().and_then(|ui| ui.error.as_deref()),
                argument_hint: Some(&completion.argument_hint),
                ..Default::default()
            },
            self.rows.saturating_sub(tail_rows + 2).min(12),
        );
        if let Some(candidate) = candidate.cloned() {
            self.visible
                .push_back((self.frame, id.clone(), candidate, completion.clone()));
            while self.visible.len() > 32 {
                self.visible.pop_front();
            }
        }
        self.selected_id = (!id.is_empty()).then_some(id.clone());
        json!({"kind":"frame", "token":token, "revision":self.revision, "frame_id":self.frame,
            "candidate_id":id, "expected_line":self.line, "expected_cursor":protocol::byte_to_utf16(&self.line,self.cursor),
            "lines":menu.lines, "width":menu.width, "tail_rows":tail_rows, "incomplete":completion.incomplete,
            "interaction":if self.ui.as_ref().is_some_and(|ui|ui.form.is_some()) {"form"} else if self.ui.is_some() {"hub"} else {"completion"}})
    }
    fn accept(&self, value: &Value, token: &str) -> Option<Value> {
        let requested_frame = value["frame_id"].as_u64()?;
        let (frame, identity, candidate, completion) = self
            .visible
            .iter()
            .find(|(frame, _, _, _)| *frame == requested_frame)?;
        if value["revision"].as_u64()? != self.revision
            || value["frame_id"].as_u64()? != *frame
            || value["candidate_id"].as_str()? != identity
            || value["line"].as_str()? != self.line
            || value["cursor"].as_u64()? as usize
                != protocol::byte_to_utf16(&self.line, self.cursor)?
        {
            return None;
        }
        let range = candidate.replacement.unwrap_or(crate::model::Replacement {
            start: completion.replace_start,
            end: completion.replace_end,
        });
        if range.end < range.start {
            return None;
        }
        let start = protocol::byte_to_utf16(&self.line, range.start)?;
        let end = protocol::byte_to_utf16(&self.line, range.end)?;
        let mut text = candidate.insert_text.clone();
        if candidate.append_space && range.end == self.line.len() && !text.ends_with(' ') {
            text.push(' ');
        }
        Some(
            json!({"kind":"edit", "token":token, "revision":self.revision, "frame_id":frame,
            "candidate_id":identity,"expected_line":self.line,"expected_cursor":value["cursor"],
            "start":start,"length":end-start,"text":text}),
        )
    }
    fn clear_frame(&self, token: &str) -> Value {
        json!({"kind":"clear","token":token,"revision":self.revision,"expected_line":self.line,
            "expected_cursor":protocol::byte_to_utf16(&self.line,self.cursor)})
    }
    fn edit_command(&mut self, text: String, token: &str) -> Value {
        self.accepting = true;
        self.ui = None;
        json!({"kind":"edit","token":token,"revision":self.revision,"expected_line":self.line,
            "expected_cursor":protocol::byte_to_utf16(&self.line,self.cursor),"start":0,"length":self.line.encode_utf16().count(),"text":text})
    }
    fn refresh_hub(&mut self, settings: &Config, token: &str, history: &[String]) -> Value {
        let ui = self.ui.as_mut().expect("active workbench");
        let completion = if let Some(form) = &ui.form {
            crate::hub::form_completion(form, &ui.input, self.line.len())
        } else {
            let (candidates, templates) = crate::hub::search_with_history_guided(
                settings,
                &ui.input,
                history,
                None,
                crate::hub::guided_template(&self.line, self.cursor),
            );
            ui.templates = templates;
            Completion {
                replace_start: 0,
                replace_end: self.line.len(),
                candidates,
                incomplete: self.history_pending,
                argument_hint: if self.history_partial {
                    "工作台 · 部分历史 · Enter 填回 · Esc 取消"
                } else {
                    "工作台 · Enter 填回 · Esc 取消"
                }
                .into(),
            }
        };
        self.render(completion, settings, token)
    }
    fn request_values(
        &mut self,
        tx: SyncSender<HostEvent>,
        environment: Arc<BTreeMap<String, String>>,
    ) {
        let Some(ui) = &self.ui else {
            return;
        };
        let Some(field) = ui.form.as_ref().and_then(|form| form.current()).cloned() else {
            return;
        };
        let generation = ui.generation;
        let cwd = self.cwd.clone();
        if generation == self.values_requested {
            return;
        }
        self.values_requested = generation;
        thread::spawn(move || {
            let started = Instant::now();
            let values = crate::hub::parameter_suggestions(&field, "", &cwd, &environment);
            let _ = tx.send(HostEvent::FormValues(
                generation,
                field.name,
                values,
                started.elapsed(),
            ));
        });
    }
    fn ui_key(
        &mut self,
        value: &Value,
        settings: &Config,
        token: &str,
        history: &[String],
    ) -> Result<Value> {
        let next = value["revision"].as_u64().context("missing UI revision")?;
        ensure!(
            next > self.revision
                && value["line"].as_str() == Some(&self.line)
                && value["cursor"].as_u64().map(|v| v as usize)
                    == protocol::byte_to_utf16(&self.line, self.cursor),
            "stale workbench edit"
        );
        let seen = value["frame_id"]
            .as_u64()
            .and_then(|id| {
                self.visible.iter().find(|(frame, identity, _, _)| {
                    *frame == id && value["candidate_id"].as_str() == Some(identity)
                })
            })
            .cloned();
        self.revision = next;
        self.selected_id = None;
        let key = value["key"].as_str().unwrap_or("");
        if key == "F1" {
            self.details = !self.details;
            return Ok(self.refresh_hub(settings, token, history));
        }
        if key == "Escape" {
            self.ui = None;
            self.visible.clear();
            return Ok(self.clear_frame(token));
        }
        let ui = self.ui.as_mut().context("no active workbench")?;
        ui.error = None;
        match key {
            "Enter" => {
                if let Some(form) = ui.form.as_mut() {
                    match form.submit(&ui.input) {
                        Ok(true) => {
                            if ui.project {
                                crate::packs::verify_approval(&self.cwd)?;
                            }
                            let command = form.finish()?;
                            return Ok(self.edit_command(command, token));
                        }
                        Ok(false) => {
                            ui.input.clear();
                            ui.generation += 1;
                        }
                        Err(error) => ui.error = Some(format!("{error:#}")),
                    }
                } else if let Some((_, _, candidate, _)) = seen {
                    ui.project = candidate.id.starts_with("hub:project:");
                    if ui.project {
                        crate::packs::verify_approval(&self.cwd)?;
                    }
                    if let Some(template) = ui.templates.get(&candidate.id).cloned() {
                        let form = crate::hub::TemplateForm::new(template)?;
                        if form.is_empty() {
                            return Ok(self.edit_command(form.finish()?, token));
                        }
                        ui.form = Some(form);
                        ui.input.clear();
                        ui.generation += 1;
                    } else {
                        return Ok(self.edit_command(candidate.insert_text, token));
                    }
                }
            }
            "Backspace" => {
                ui.high_surrogate = None;
                ui.input.pop();
                self.selected = 0;
            }
            "UpArrow" | "DownArrow" => {
                if let Some(field) = ui.form.as_ref().and_then(|form| form.current()) {
                    if !field.values.is_empty() {
                        let forward = key == "DownArrow";
                        let index = field.values.iter().position(|v| v == &ui.input);
                        let next = match index {
                            Some(index) if forward => (index + 1) % field.values.len(),
                            Some(0) => field.values.len() - 1,
                            Some(index) => index - 1,
                            None => 0,
                        };
                        ui.input = field.values[next].clone();
                    }
                } else if let Some((_, _, _, completion)) = seen {
                    let count = completion.candidates.len();
                    if count > 0 {
                        self.selected = if key == "DownArrow" {
                            (self.selected + 1) % count
                        } else {
                            (self.selected + count - 1) % count
                        };
                    }
                }
            }
            _ if value["modifiers"].as_u64().unwrap_or(0) & 6 == 0 => {
                let unit = value["character_unit"].as_u64().unwrap_or(0) as u16;
                if (0xd800..=0xdbff).contains(&unit) {
                    ui.high_surrogate = Some(unit);
                } else if (0xdc00..=0xdfff).contains(&unit) {
                    if let Some(high) = ui.high_surrogate.take() {
                        ui.input.push(
                            char::from_u32(
                                0x10000 + (u32::from(high) - 0xd800) * 0x400 + u32::from(unit)
                                    - 0xdc00,
                            )
                            .unwrap(),
                        );
                    }
                } else if let Some(ch) =
                    char::from_u32(u32::from(unit)).filter(|ch| !ch.is_control())
                {
                    ui.high_surrogate = None;
                    ui.input.push(ch);
                }
                self.selected = 0;
            }
            _ => {}
        }
        Ok(self.refresh_hub(settings, token, history))
    }
    fn navigate(&mut self, value: &Value, settings: &Config, token: &str) -> Result<Value> {
        let next = value["revision"]
            .as_u64()
            .context("missing menu revision")?;
        ensure!(
            next > self.revision
                && value["line"].as_str() == Some(&self.line)
                && value["cursor"].as_u64().map(|v| v as usize)
                    == protocol::byte_to_utf16(&self.line, self.cursor),
            "stale menu navigation"
        );
        let id = value["frame_id"].as_u64().context("missing seen frame")?;
        let (_, identity, _, completion) = self
            .visible
            .iter()
            .find(|(frame, identity, _, _)| {
                *frame == id && value["candidate_id"].as_str() == Some(identity)
            })
            .cloned()
            .context("unseen frame")?;
        let current = completion
            .candidates
            .iter()
            .position(|c| c.identity() == identity)
            .context("unseen candidate")?;
        let count = completion.candidates.len();
        self.revision = next;
        self.selected = if value["key"] == "F1" {
            self.details = !self.details;
            current
        } else if value["key"] == "DownArrow" {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        self.selected_id = None;
        Ok(self.render(completion, settings, token))
    }
}

// Conservative geometry: start at the actual cursor, allow one wrapping row
// for its unknown column. Newlines and grapheme widths cannot cover right text.
fn tail_rows(tail: &str, width: u16) -> usize {
    let width = usize::from(width).max(1);
    tail.split('\n')
        .map(|line| {
            line.graphemes(true)
                .map(UnicodeWidthStr::width)
                .sum::<usize>()
                .div_ceil(width)
        })
        .sum::<usize>()
        + tail.bytes().filter(|b| *b == b'\n').count()
}

pub fn ensure_bootstrap(directory: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let assembly = include_bytes!(concat!(env!("OUT_DIR"), "/direct-bridge.dll"));
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(assembly));
    let directory = directory.join(format!("direct-{}", &hash[..16]));
    std::fs::create_dir_all(&directory)?;
    let dll = directory.join("direct-bridge.dll");
    if std::fs::read(&dll).ok().as_deref() != Some(assembly.as_slice()) {
        std::fs::write(&dll, assembly)?;
    }
    let script = directory.join("direct.ps1");
    std::fs::write(
        &script,
        format!("\u{feff}{}", include_str!("../shell/direct.ps1")),
    )?;
    Ok(script)
}

pub fn run(options: RunOptions) -> Result<u32> {
    ensure!(
        matches!(options.transport, Transport::Pipe),
        "direct host requires pipe transport; OSC is only supported by the nested host"
    );
    let settings = config::load(options.config_path.as_deref())?;
    let trace = Trace::open(options.trace_path.as_deref())?;
    let token = uuid::Uuid::new_v4().to_string();
    let directory = options.data_dir.join(&token);
    let bootstrap = ensure_bootstrap(&directory)?;
    let mut pipe = crate::pipe::PipeServer::new()?;
    let mut child = Command::new(&options.shell)
        .args(pty::shell_args(&bootstrap, options.no_profile))
        .env("BLUEBERRY_ACTIVE", "1")
        .env("BLUEBERRY_HOST_MODE", "direct")
        .env("BLUEBERRY_PIPE_NAME", pipe.name())
        .env("BLUEBERRY_TOKEN", &token)
        .env("BLUEBERRY_SESSION_DIR", &directory)
        .env(
            "BLUEBERRY_DIRECT_TRACE",
            if trace.enabled() { "1" } else { "0" },
        )
        .envs(pty::key_environment(&settings.keys))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("start inherited-console PowerShell")?;
    let _job = crate::latency_layers::DirectChildJob::attach(&child)?;
    pipe.set_client_pid(child.id());
    let (tx, rx) = mpsc::sync_channel(256);
    let worker = Worker::new(
        options.data_dir.join("commands.json"),
        config::specs_dir(&settings, options.config_path.as_deref()),
        directory.join("data-sources.json"),
        tx.clone(),
        trace.clone(),
    );
    let changed = tx.clone();
    crate::hub::on_commands_change(move || {
        let _ = changed.try_send(HostEvent::CommandsChanged);
    });
    let changed = tx.clone();
    crate::packs::on_change(move || {
        let _ = changed.try_send(HostEvent::CommandsChanged);
    });
    let descriptions = Arc::new(settings.descriptions.clone());
    let mut environment = Arc::new(std::env::vars().collect::<BTreeMap<_, _>>());
    let mut session = Session::new();
    let mut ready = false;
    let mut automatic = false;
    let mut history: Vec<String> = Vec::new();
    let mut history_requested = false;
    let status_writer = crate::status::StatusWriter::new(directory.join("adapter.json"));
    let mut adapter_status = Value::Null;
    let mut active_query: Option<Query> = None;
    worker.update(|w| w.started = true);
    let served = (|| -> Result<u32> {
        loop {
            if let Some(status) = child.try_wait()? {
                trace.flush();
                return Ok(status.code().unwrap_or(1) as u32);
            }
            let event = if ready {
                rx.recv_timeout(Duration::from_millis(100)).ok()
            } else {
                match pipe.read_json() {
                    Ok(value) => Some(HostEvent::DirectMessage(value, Instant::now())),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => None,
                    Err(_) => Some(HostEvent::Eof),
                }
            };
            let mut completion_event = None;
            match event {
                Some(HostEvent::DirectMessage(value, arrived)) => {
                    ensure!(
                        value["token"].as_str() == Some(&token),
                        "invalid direct session token"
                    );
                    trace.event(
                        "direct_pipe_received",
                        value["revision"].as_u64(),
                        None,
                        None,
                    );
                    trace.flush();
                    trace.event(
                        "direct_pipe_queue",
                        value["revision"].as_u64(),
                        Some(arrived.elapsed()),
                        None,
                    );
                    if trace.enabled()
                        && let Some(confirmed) = value["confirmed_qpc"].as_i64()
                    {
                        let duration = (crate::latency_layers::qpc()? - confirmed) as f64
                            / crate::latency_layers::qpc_frequency()?;
                        if duration >= 0.0 {
                            trace.event(
                                "direct_confirmed_to_dispatch",
                                value["revision"].as_u64(),
                                Some(Duration::from_secs_f64(duration)),
                                None,
                            );
                        }
                    }
                    match value["event"].as_str() {
                        Some("trace") if ready && trace.enabled() => {
                            let stage = match value["stage"].as_str() {
                                Some("direct_binding_snapshot") => "direct_binding_snapshot",
                                Some("direct_binding_registration") => {
                                    "direct_binding_registration"
                                }
                                Some("direct_bridge_initialization") => {
                                    "direct_bridge_initialization"
                                }
                                Some("direct_readline_begin") => "direct_readline_begin",
                                Some("direct_binding_audit") => "direct_binding_audit",
                                Some("direct_response_wait") => "direct_response_wait",
                                Some("direct_original_edit") => "direct_original_edit",
                                Some("direct_confirmed_query_send") => {
                                    "direct_confirmed_query_send"
                                }
                                Some("direct_console_menu_output") => "direct_console_menu_output",
                                _ => bail!("unknown numeric direct trace phase"),
                            };
                            trace.event(
                                stage,
                                value["revision"].as_u64(),
                                value["duration_us"].as_u64().map(Duration::from_micros),
                                value["count"].as_u64().map(|count| count as usize),
                            );
                        }
                        Some("hello") => {
                            ensure!(
                                !ready
                                    && value["protocol"] == 1
                                    && value["transport"] == "pipe"
                                    && value["host_mode"] == "direct",
                                "invalid capability negotiation"
                            );
                            ready = true;
                            let mut reader = pipe.receiver()?;
                            let timer = ReceiveTimer::new()?;
                            let received_tx = tx.clone();
                            thread::spawn(move || {
                                loop {
                                    match reader.read_json() {
                                        Ok((value, arrived)) => {
                                            if received_tx
                                                .send(HostEvent::DirectMessage(value, arrived))
                                                .is_err()
                                            {
                                                return;
                                            }
                                        }
                                        Err(error)
                                            if error.kind() == std::io::ErrorKind::WouldBlock =>
                                        {
                                            timer.wait()
                                        }
                                        Err(_) => break,
                                    }
                                }
                                let _ = received_tx.send(HostEvent::Eof);
                            });
                            let capabilities = value["capabilities"]
                                .as_array()
                                .context("missing direct capabilities")?;
                            ensure!(
                                [
                                    "revision",
                                    "frame_identity",
                                    "accept_identity",
                                    "utf16_edit"
                                ]
                                .iter()
                                .all(|name| capabilities.iter().any(|item| item == name)),
                                "incompatible direct capabilities"
                            );
                            pipe.write_json(&json!({"kind":"hello","token":token,"protocol":1,"host_mode":"direct","transport":"pipe","capabilities":capabilities}))?;
                            automatic = value["automatic_menu"] == true;
                            adapter_status = json!({"shell_version":value["shell_version"],"psreadline_version":value["psreadline"],
                            "transport":"pipe","host_mode":"direct","automatic_menu":automatic && settings.completion.auto_trigger,
                            "disabled_reason":if automatic && !settings.completion.auto_trigger {json!("completion.auto_trigger is disabled")} else {value["disabled_reason"].clone()}});
                            status_writer.update(adapter_status.clone());
                        }
                        Some("begin" | "end") if ready => {
                            let revision = value["revision"]
                                .as_u64()
                                .context("missing lifecycle revision")?;
                            ensure!(revision > session.revision, "stale lifecycle revision");
                            session.revision = revision;
                            session.visible.clear();
                            session.accepting = false;
                            session.ui = None;
                            active_query = None;
                            if value["event"] == "begin" {
                                if !history_requested {
                                    history_requested = true;
                                    if let Some(path) = value["history_path"]
                                        .as_str()
                                        .filter(|path| !path.is_empty())
                                    {
                                        session.history_pending = true;
                                        let path = PathBuf::from(path);
                                        let history_tx = tx.clone();
                                        let limit = settings.workbench.history_limit;
                                        thread::spawn(move || {
                                            let started = Instant::now();
                                            let (values, partial) =
                                                crate::hub::merged_history_with_status(
                                                    limit,
                                                    &[],
                                                    Some(&path),
                                                );
                                            let _ = history_tx.send(HostEvent::HistoryLoaded(
                                                0,
                                                values,
                                                partial,
                                                started.elapsed(),
                                            ));
                                        });
                                    }
                                }
                                automatic = value["automatic_menu"] == true;
                                adapter_status["automatic_menu"] =
                                    json!(automatic && settings.completion.auto_trigger);
                                adapter_status["disabled_reason"] =
                                    if automatic && !settings.completion.auto_trigger {
                                        json!("completion.auto_trigger is disabled")
                                    } else {
                                        value["disabled_reason"].clone()
                                    };
                                status_writer.update(adapter_status.clone());
                                if let Some(cwd) = value["cwd"].as_str() {
                                    session.cwd = cwd.into();
                                    crate::packs::set_cwd(session.cwd.clone());
                                }
                                if let Ok(snapshot) =
                                    serde_json::from_value::<BTreeMap<String, String>>(
                                        value["environment"].clone(),
                                    )
                                {
                                    if let (Some(path), Some(pathext)) =
                                        (snapshot.get("PATH"), snapshot.get("PATHEXT"))
                                    {
                                        worker.update(|w| {
                                            w.environment = Some((path.clone(), pathext.clone()))
                                        });
                                    }
                                    environment = Arc::new(snapshot);
                                }
                            } else if let Some(line) = value["line"]
                                .as_str()
                                .filter(|line| !line.trim().is_empty())
                            {
                                history.retain(|previous| previous != line);
                                history.insert(0, line.to_owned());
                                history.truncate(settings.workbench.history_limit);
                            }
                            worker.update(|w| {
                                w.cancel = true;
                                w.query = None;
                            });
                        }
                        Some("hub" | "query" | "cancel") if ready && automatic => {
                            session.query(&value)?;
                            let cwd =
                                PathBuf::from(value["cwd"].as_str().context("missing shell cwd")?);
                            session.cwd = cwd.clone();
                            crate::packs::set_cwd(cwd.clone());
                            if value["event"] == "cancel"
                                || (value["event"] == "query"
                                    && (!settings.completion.auto_trigger
                                        || session.line.trim().is_empty()))
                            {
                                session.ui = None;
                                active_query = None;
                                worker.update(|w| {
                                    w.cancel = true;
                                    w.query = None;
                                });
                                pipe.write_json(&session.clear_frame(&token))?;
                                continue;
                            }
                            if value["event"] == "hub" {
                                trace.event(
                                    "direct_hub_opened",
                                    Some(session.revision),
                                    None,
                                    None,
                                );
                                trace.flush();
                                worker.update(|w| {
                                    w.cancel = true;
                                    w.query = None;
                                });
                                session.values_requested = 0;
                                session.ui = Some(Workbench {
                                    input: String::new(),
                                    high_surrogate: None,
                                    form: None,
                                    project: false,
                                    templates: Default::default(),
                                    error: None,
                                    generation: 0,
                                });
                                pipe.write_json(&session.refresh_hub(&settings, &token, &history))?;
                                continue;
                            }
                            session.ui = None;
                            let tool = session.line.split_whitespace().next().unwrap_or("");
                            if settings.resources_for(tool) == "automatic" {
                                Arc::make_mut(&mut environment)
                                    .insert("BLUEBERRY_REMOTE_REQUESTED".into(), "1".into());
                            }
                            let query = Query {
                                revision: session.revision,
                                line: session.line.clone(),
                                cursor: session.cursor,
                                cwd,
                                limit: settings.completion.max_results,
                                descriptions: descriptions.clone(),
                                context: None,
                                fuzzy: settings.completion.fuzzy,
                                usage: None,
                                environment: environment.clone(),
                                dynamic: settings.dynamic_for(tool),
                                searching: false,
                                help_enabled: settings.help_for(tool),
                                queued_at: arrived,
                            };
                            active_query = Some(query.clone());
                            Arc::make_mut(&mut environment).remove("BLUEBERRY_REMOTE_REQUESTED");
                            worker.update(|w| w.query = Some(query));
                        }
                        Some("accept") if ready && automatic => {
                            if let Some(edit) = session.accept(&value, &token) {
                                session.accepting = true;
                                worker.update(|w| {
                                    w.cancel = true;
                                    w.query = None;
                                });
                                pipe.write_json(&edit)?;
                            }
                        }
                        Some("ui_key") if ready && automatic => {
                            trace.event("direct_ui_key", value["revision"].as_u64(), None, None);
                            trace.flush();
                            let frame = session.ui_key(&value, &settings, &token, &history)?;
                            pipe.write_json(&frame)?;
                            session.request_values(tx.clone(), environment.clone());
                        }
                        Some("menu_key") if ready && automatic => {
                            let frame = session.navigate(&value, &settings, &token)?;
                            pipe.write_json(&frame)?;
                            if let Some(query) = active_query.as_mut() {
                                query.revision = session.revision;
                                query.queued_at = Instant::now();
                                worker.update(|w| w.query = Some(query.clone()));
                            }
                        }
                        _ => bail!("unexpected direct protocol event"),
                    }
                }
                // A sidecar failure leaves the inherited shell and its original
                // editing actions running. Do not restart or kill the user's shell.
                Some(HostEvent::Eof) => {
                    adapter_status["automatic_menu"] = json!(false);
                    adapter_status["disabled_reason"] = json!("session pipe disconnected");
                    status_writer.update(adapter_status.clone());
                    return Ok(child.wait()?.code().unwrap_or(1) as u32);
                }
                other => completion_event = other,
            }
            if let Some(event) = completion_event {
                if let HostEvent::Completion(revision, completion, _) = &event
                    && ready
                    && automatic
                    && session.ui.is_none()
                    && !session.accepting
                    && *revision == session.revision
                {
                    let rendered = Instant::now();
                    let frame = session.render(completion.clone(), &settings, &token);
                    trace.event(
                        "direct_menu_render",
                        Some(*revision),
                        Some(rendered.elapsed()),
                        frame["lines"].as_array().map(Vec::len),
                    );
                    let output = Instant::now();
                    pipe.write_json(&frame)?;
                    trace.event(
                        "direct_frame_written",
                        Some(*revision),
                        Some(output.elapsed()),
                        frame["lines"].as_array().map(Vec::len),
                    );
                } else if let HostEvent::HistoryLoaded(_, values, partial, elapsed) = event {
                    trace.event("history_load", None, Some(elapsed), Some(values.len()));
                    let mut merged = history.clone();
                    merged.extend(values);
                    let mut seen = std::collections::HashSet::new();
                    history = merged
                        .into_iter()
                        .filter(|line| seen.insert(line.clone()))
                        .take(settings.workbench.history_limit)
                        .collect();
                    session.history_pending = false;
                    session.history_partial = partial;
                    if session.ui.is_some() {
                        pipe.write_json(&session.refresh_hub(&settings, &token, &history))?;
                    }
                } else if let HostEvent::FormValues(generation, name, values, _) = event {
                    if let Some(ui) = session.ui.as_mut()
                        && ui.generation == generation
                        && let Some(form) = ui.form.as_mut()
                        && form.current().is_some_and(|field| field.name == name)
                    {
                        form.set_values(&name, values);
                        pipe.write_json(&session.refresh_hub(&settings, &token, &history))?;
                    }
                } else if matches!(event, HostEvent::CommandsChanged) && session.ui.is_some() {
                    pipe.write_json(&session.refresh_hub(&settings, &token, &history))?;
                }
            }
            if !ready {
                thread::sleep(Duration::from_millis(1));
            }
        }
    })();
    if let Err(error) = &served {
        adapter_status["automatic_menu"] = json!(false);
        adapter_status["disabled_reason"] = json!(format!("direct service disabled: {error:#}"));
        status_writer.update(adapter_status);
        pipe.disable();
        drop(worker);
        trace.event("direct_service_disabled", None, None, None);
        trace.flush();
        return Ok(child.wait()?.code().unwrap_or(1) as u32);
    }
    served
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accept_requires_visible_identity_revision_and_utf16_buffer() {
        let mut session = Session::new();
        session
            .query(&json!({"revision":1,"line":"😀 gi right","cursor":5}))
            .unwrap();
        let completion = Completion {
            replace_start: 5,
            replace_end: 7,
            candidates: vec![Candidate {
                label: "git".into(),
                insert_text: "git".into(),
                id: "git-stable".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let frame = session.render(completion, &Config::default(), "token");
        let mut accept = json!({"revision":1,"frame_id":frame["frame_id"],"candidate_id":"git-stable","line":"😀 gi right","cursor":5});
        let edit = session.accept(&accept, "token").unwrap();
        assert_eq!(edit["start"], 3);
        assert_eq!(edit["length"], 2);
        accept["candidate_id"] = json!("other");
        assert!(session.accept(&accept, "token").is_none());
        accept["candidate_id"] = json!("git-stable");
        accept["cursor"] = json!(4);
        assert!(session.accept(&accept, "token").is_none());
        accept["cursor"] = json!(5);
        accept["revision"] = json!(0);
        assert!(session.accept(&accept, "token").is_none());
    }
    #[test]
    fn stale_queries_and_surrogate_middle_are_rejected() {
        let mut session = Session::new();
        assert!(
            session
                .query(&json!({"revision":1,"line":"😀","cursor":1}))
                .is_err()
        );
        session
            .query(&json!({"revision":1,"line":"😀","cursor":2}))
            .unwrap();
        assert!(
            session
                .query(&json!({"revision":1,"line":"wrong","cursor":0}))
                .is_err()
        );
        assert_eq!(session.line, "😀");
    }
}

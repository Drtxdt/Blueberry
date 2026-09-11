use crate::{
    config::{self, Config},
    engine::CommandIndex,
    input::{self, Input},
    model::{Completion, ShellCommand},
    overlay::Overlay,
    protocol::{self, Decoder, Part},
    pty,
};
use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event},
    terminal,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, SyncSender},
    },
    thread,
};

const QUERY: &[u8] = b"\x1b[24~s";
const ACCEPT: &[u8] = b"\x1b[24~a";
const COMMANDS: &[u8] = b"\x1b[24~c";

pub struct RunOptions {
    pub shell: PathBuf,
    pub no_profile: bool,
    pub config_path: Option<PathBuf>,
    pub data_dir: PathBuf,
}

enum HostEvent {
    Output(Vec<u8>),
    Input(Event),
    Eof,
    Exit(u32),
    Error(String),
    Completion(u64, Completion),
}

#[derive(Clone)]
struct Query {
    revision: u64,
    line: String,
    cursor: usize,
    cwd: PathBuf,
    limit: usize,
}

#[derive(Default)]
struct Work {
    started: bool,
    refresh: bool,
    stop: bool,
    commands: Option<Vec<ShellCommand>>,
    query: Option<Query>,
    environment: Option<(String, String)>,
}

struct Worker(Arc<(Mutex<Work>, Condvar)>);

impl Worker {
    fn new(cache: PathBuf, output: SyncSender<HostEvent>) -> Self {
        let shared = Arc::new((Mutex::new(Work::default()), Condvar::new()));
        let thread_shared = shared.clone();
        thread::spawn(move || {
            let mut index: Option<CommandIndex> = None;
            let mut environment: Option<(String, String)> = None;
            let mut shell_commands = Vec::new();
            loop {
                let (lock, condition) = &*thread_shared;
                let mut work = lock.lock().unwrap();
                while !work.stop
                    && (!work.started
                        || (!work.refresh
                            && work.commands.is_none()
                            && work.query.is_none()
                            && index.is_some()))
                {
                    work = condition.wait(work).unwrap();
                }
                if work.stop {
                    break;
                }
                let refresh = std::mem::take(&mut work.refresh);
                let commands = work.commands.take();
                let commands_changed = commands.is_some();
                let query = work.query.take();
                if let Some(env) = work.environment.take() {
                    environment = Some(env);
                }
                drop(work);
                if let Some(commands) = commands {
                    shell_commands = commands;
                }
                let rebuilt = index.is_none() || refresh;
                if rebuilt {
                    let discovered = if let Some((path, pathext)) = &environment {
                        if !refresh {
                            CommandIndex::load_with_env(
                                &cache,
                                OsStr::new(path),
                                OsStr::new(pathext),
                            )
                            .unwrap_or_else(|_| {
                                CommandIndex::discover_with_env(
                                    OsStr::new(path),
                                    OsStr::new(pathext),
                                )
                            })
                        } else {
                            CommandIndex::discover_with_env(OsStr::new(path), OsStr::new(pathext))
                        }
                    } else if !refresh {
                        CommandIndex::load(&cache).unwrap_or_else(|_| CommandIndex::discover())
                    } else {
                        CommandIndex::discover()
                    };
                    // A missing/unwritable cache must never prevent completion.
                    let _ = discovered.save(&cache);
                    index = Some(discovered);
                }
                let index = index.as_mut().unwrap();
                if rebuilt || commands_changed {
                    index.replace_shell_commands(shell_commands.clone());
                }
                if let Some(q) = query {
                    let result = index.complete(&q.line, q.cursor, &q.cwd, q.limit);
                    if output
                        .send(HostEvent::Completion(q.revision, result))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });
        Self(shared)
    }
    fn update(&self, change: impl FnOnce(&mut Work)) {
        let (lock, condition) = &*self.0;
        change(&mut lock.lock().unwrap());
        condition.notify_one();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.update(|w| w.stop = true);
    }
}

struct RawMode {
    #[cfg(windows)]
    original: u32,
}
impl RawMode {
    fn enable() -> std::io::Result<Self> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Console::*;
            let mut original = 0;
            // Borrow the process console handle; the OS retains ownership.
            // Window events are opt-in and crossterm raw mode alone omits them.
            unsafe {
                let handle = GetStdHandle(STD_INPUT_HANDLE);
                if GetConsoleMode(handle, &mut original) == 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let mode = (original
                    & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT))
                    | ENABLE_WINDOW_INPUT;
                if SetConsoleMode(handle, mode) == 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(Self { original })
        }
        #[cfg(not(windows))]
        {
            terminal::enable_raw_mode()?;
            Ok(Self {})
        }
    }
}
impl Drop for RawMode {
    fn drop(&mut self) {
        #[cfg(windows)]
        // Restore exactly the input mode inherited from the launching terminal.
        unsafe {
            use windows_sys::Win32::System::Console::*;
            SetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), self.original);
        }
        #[cfg(not(windows))]
        let _ = terminal::disable_raw_mode();
        let _ = std::io::stdout().write_all(b"\x1b[0m\x1b[?25h");
    }
}

struct ChildGuard(Box<dyn portable_pty::ChildKiller + Send + Sync>);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

struct SessionFiles(PathBuf);
impl Drop for SessionFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("edit.json"));
        let _ = std::fs::remove_dir(&self.0);
    }
}

struct State {
    parser: vt100::Parser,
    decoder: Decoder,
    overlay: Overlay,
    config: Config,
    config_path: Option<PathBuf>,
    cwd: PathBuf,
    edit_path: PathBuf,
    revision: u64,
    prompt: bool,
    ready: bool,
    dirty: bool,
    explicit: bool,
    pending_query: Option<u64>,
    line: String,
    cursor: usize,
    completion: Completion,
    selected: usize,
    dismissed: bool,
    indexed: bool,
    environment: Option<(String, String)>,
    diagnostic: Option<String>,
    nested_edit: bool,
    bell: bool,
    adapter_diagnostic_shown: bool,
    repaint: bool,
}

impl State {
    fn invalidate(&mut self) {
        self.revision += 1;
        self.completion = Completion::default();
        self.selected = 0;
    }
    fn message(&mut self, value: Value, writer: &mut impl Write, worker: &Worker) -> Result<()> {
        match value["event"].as_str().unwrap_or("") {
            "capabilities" => {
                self.ready = value["ready"] == true
                    && value["psreadline"] == true
                    && value["key_handlers"]["buffer"] == true
                    && value["key_handlers"]["apply"] == true
                    && value["key_handlers"]["commands"] == true;
                if self.ready {
                    self.diagnostic = None;
                }
            }
            "prompt_start" => {
                self.prompt = false;
                self.invalidate();
                self.pending_query = None;
                self.dirty = false;
            }
            "prompt_end" => {
                self.prompt = true;
                if !self.ready && !self.adapter_diagnostic_shown {
                    self.diagnostic = Some("PowerShell adapter unavailable (PSReadLine or reserved key binding). Completion is disabled for this session.".into());
                    self.adapter_diagnostic_shown = true;
                }
                self.line.clear();
                if let Some(cwd) = value["cwd"].as_str() {
                    self.cwd = cwd.into();
                }
                let environment = value["path"]
                    .as_str()
                    .zip(value["pathext"].as_str())
                    .map(|(p, e)| (p.to_owned(), e.to_owned()));
                if environment != self.environment {
                    self.environment = environment.clone();
                    let was_indexed = self.indexed;
                    worker.update(|w| {
                        w.environment = environment;
                        w.refresh = was_indexed;
                    });
                }
                if !self.indexed && self.ready {
                    worker.update(|w| {
                        w.started = true;
                    });
                    writer.write_all(COMMANDS)?;
                    writer.flush()?;
                    self.indexed = true;
                }
            }
            "execute" => {
                self.prompt = false;
                self.pending_query = None;
                self.dirty = false;
                self.invalidate();
            }
            "commands" => {
                if let Ok(commands) = serde_json::from_value(value["commands"].clone()) {
                    worker.update(|w| w.commands = Some(commands));
                }
            }
            "buffer" => {
                let revision = self.pending_query.take().unwrap_or(self.revision);
                if revision != self.revision || !self.prompt {
                    return Ok(());
                }
                let Some(line) = value["line"].as_str() else {
                    return Ok(());
                };
                let Some(cursor16) = value["cursor"].as_u64() else {
                    return Ok(());
                };
                let Some(cursor) = protocol::utf16_to_byte(line, cursor16 as usize) else {
                    return Ok(());
                };
                self.line = line.into();
                self.cursor = cursor;
                if !self.dismissed
                    && (self.explicit
                        || (self.config.completion.auto_trigger && !line.trim().is_empty()))
                {
                    let query = Query {
                        revision,
                        line: line.into(),
                        cursor,
                        cwd: self.cwd.clone(),
                        limit: self.config.completion.max_results,
                    };
                    worker.update(|w| w.query = Some(query));
                }
                self.explicit = false;
            }
            "error" => {
                self.pending_query = None;
                self.dirty = false;
                self.dismissed = true;
                self.invalidate();
                match value["code"].as_str().unwrap_or("") {
                    "buffer_unavailable"
                    | "key_chord_collision"
                    | "key_handler_registration_failed" => {
                        self.ready = false;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn query(&mut self, writer: &mut impl Write) -> Result<()> {
        if self.dirty && self.ready && self.prompt && self.pending_query.is_none() {
            writer.write_all(QUERY)?;
            writer.flush()?;
            self.pending_query = Some(self.revision);
            self.dirty = false;
        }
        Ok(())
    }

    fn accept(&mut self, writer: &mut impl Write) -> Result<()> {
        let Some(candidate) = self.completion.candidates.get(self.selected) else {
            return Ok(());
        };
        let start = protocol::byte_to_utf16(&self.line, self.completion.replace_start);
        let end = protocol::byte_to_utf16(&self.line, self.completion.replace_end);
        let cursor = protocol::byte_to_utf16(&self.line, self.cursor);
        if let (Some(start), Some(end), Some(cursor)) = (start, end, cursor) {
            if end < start {
                return Ok(());
            }
            let edit = json!({"expectedLine":self.line,"expectedCursor":cursor,"start":start,"length":end-start,"text":candidate.insert_text});
            // Only the child receives the apply chord, after this write is closed.
            std::fs::write(&self.edit_path, serde_json::to_vec(&edit)?)?;
            self.invalidate();
            self.pending_query = Some(self.revision);
            self.dismissed = true;
            writer.write_all(ACCEPT)?;
            writer.flush()?;
        }
        Ok(())
    }

    fn input(
        &mut self,
        event: Event,
        writer: &mut impl Write,
        master: &dyn portable_pty::MasterPty,
        worker: &Worker,
    ) -> Result<()> {
        // A custom PSReadLine key handler can end native selection/search.
        // Never query while those editing modes are active.
        let mut query_after = true;
        if let Event::Key(key) = &event {
            use crossterm::event::{KeyCode, KeyModifiers};
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let alt = key.modifiers.contains(KeyModifiers::ALT);
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);
            if ctrl && matches!(key.code, KeyCode::Char('r' | 's')) && !alt {
                self.nested_edit = true;
            }
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter)
                || (ctrl && matches!(key.code, KeyCode::Char('c' | 'g')))
            {
                self.nested_edit = false;
            }
            if (ctrl || alt) && !matches!(key.code, KeyCode::Backspace | KeyCode::Delete) {
                query_after = false;
            }
            if shift && !matches!(key.code, KeyCode::Char(_)) {
                query_after = false;
            }
        }
        let paste = matches!(event, Event::Paste(_));
        let Some(input) = input::translate(event, self.parser.screen().application_cursor()) else {
            return Ok(());
        };
        let visible = !self.completion.candidates.is_empty() && !self.dismissed && self.prompt;
        let bytes = match input {
            Input::Resize(cols, rows) => {
                #[cfg(windows)]
                let (cols, rows) = terminal::size().unwrap_or((cols, rows));
                if rows > 0 && cols > 0 && self.parser.screen().size() != (rows, cols) {
                    master.resize(portable_pty::PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    })?;
                    self.parser.screen_mut().set_size(rows, cols);
                    // The outer terminal can reflow old overlay cells during a
                    // resize. Repaint the shell model instead of erasing only
                    // the overlay's obsolete row coordinates.
                    self.overlay = Overlay::default();
                    self.repaint = true;
                }
                return Ok(());
            }
            Input::Tab if visible => return self.accept(writer),
            Input::Previous if visible => {
                self.selected = self
                    .selected
                    .checked_sub(1)
                    .unwrap_or(self.completion.candidates.len() - 1);
                return Ok(());
            }
            Input::BackTab if visible => {
                self.selected = self
                    .selected
                    .checked_sub(1)
                    .unwrap_or(self.completion.candidates.len() - 1);
                return Ok(());
            }
            Input::Next if visible => {
                self.selected = (self.selected + 1) % self.completion.candidates.len();
                return Ok(());
            }
            Input::Dismiss if visible => {
                self.dismissed = true;
                self.invalidate();
                return Ok(());
            }
            Input::Trigger if self.prompt && self.ready => {
                self.explicit = true;
                self.dismissed = false;
                self.dirty = true;
                return Ok(());
            }
            Input::Refresh if self.prompt && self.ready => {
                worker.update(|w| w.refresh = true);
                writer.write_all(COMMANDS)?;
                writer.flush()?;
                return Ok(());
            }
            Input::Reload if self.prompt => {
                match config::load(self.config_path.as_deref()) {
                    Ok(config) => self.config = config,
                    Err(_) => self.bell = true,
                }
                return Ok(());
            }
            Input::Tab => vec![b'\t'],
            Input::Previous => if self.parser.screen().application_cursor() {
                b"\x1bOA"
            } else {
                b"\x1b[A"
            }
            .to_vec(),
            Input::BackTab => b"\x1b[Z".to_vec(),
            Input::Next => if self.parser.screen().application_cursor() {
                b"\x1bOB"
            } else {
                b"\x1b[B"
            }
            .to_vec(),
            Input::Dismiss => vec![0x1b],
            Input::Trigger => vec![0],
            Input::Refresh => b"\x1b\x03".to_vec(),
            Input::Reload => b"\x1b\x12".to_vec(),
            Input::Bytes(bytes) => bytes,
        };
        self.invalidate();
        self.dismissed = false;
        // After submitting a line the next input may belong to a native program.
        // Do not inject a PSReadLine chord until the next prompt marker.
        if bytes.iter().any(|b| matches!(b, b'\r' | b'\n' | 3 | 4)) {
            self.prompt = false;
            self.dirty = false;
        } else {
            self.dirty = self.prompt && query_after && !self.nested_edit;
        }
        if paste && self.parser.screen().bracketed_paste() {
            writer.write_all(b"\x1b[200~")?;
            writer.write_all(&bytes)?;
            writer.write_all(b"\x1b[201~")?;
        } else {
            writer.write_all(&bytes)?;
        }
        writer.flush()?;
        Ok(())
    }
}

pub fn run(options: RunOptions) -> Result<u32> {
    let config = config::load(options.config_path.as_deref())?;
    let integration = pty::ensure_integration(&options.data_dir)?;
    let session_directory = options
        .data_dir
        .join(format!("session-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&session_directory)?;
    let _files = SessionFiles(session_directory.clone());
    let edit_path = session_directory.join("edit.json");
    let token = uuid::Uuid::new_v4().to_string();
    let env = BTreeMap::from([
        ("SHELLSENSE_TOKEN".into(), token.clone()),
        (
            "SHELLSENSE_EDIT_PATH".into(),
            edit_path.to_string_lossy().into(),
        ),
        ("SHELLSENSE_ACTIVE".into(), "1".into()),
        ("ISTERM".into(), "1".into()),
        ("TERM".into(), "xterm-256color".into()),
    ]);
    let cwd = std::env::current_dir()?;
    let (cols, rows) = terminal::size().unwrap_or((120, 30));
    let _raw = RawMode::enable()
        .context("ShellSense run requires an interactive terminal. Use 'complete' for scripts.")?;
    #[cfg(windows)]
    let _ = crossterm::ansi_support::supports_ansi();
    // The screen model owns a fresh viewport. CSI 2J preserves scrollback.
    std::io::stdout().write_all(b"\x1b[2J\x1b[H")?;
    std::io::stdout().flush()?;
    let pty::Session {
        master,
        mut reader,
        mut writer,
        mut child,
    } = pty::spawn(
        &options.shell,
        &pty::shell_args(&integration, options.no_profile),
        &cwd,
        &env,
        rows,
        cols,
    )?;
    let _child_guard = ChildGuard(child.clone_killer());
    let (tx, rx) = mpsc::sync_channel(256);
    let read_tx = tx.clone();
    thread::spawn(move || {
        let mut buffer = [0u8; 16_384];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = read_tx.send(HostEvent::Eof);
                    break;
                }
                Ok(n) => {
                    if read_tx
                        .send(HostEvent::Output(buffer[..n].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    let _ = read_tx.send(HostEvent::Error(error.to_string()));
                    break;
                }
            }
        }
    });
    let input_tx = tx.clone();
    thread::spawn(move || {
        loop {
            match event::read() {
                Ok(event) => {
                    if input_tx.send(HostEvent::Input(event)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = input_tx.send(HostEvent::Error(error.to_string()));
                    break;
                }
            }
        }
    });
    let wait_tx = tx.clone();
    thread::spawn(move || {
        let code = child.wait().map(|s| s.exit_code()).unwrap_or(1);
        let _ = wait_tx.send(HostEvent::Exit(code));
    });
    let worker = Worker::new(options.data_dir.join("commands.json"), tx);
    let mut state = State {
        parser: vt100::Parser::new(rows, cols, 0),
        decoder: Decoder::new(token),
        overlay: Overlay::default(),
        config,
        config_path: options.config_path,
        cwd,
        edit_path,
        revision: 0,
        prompt: false,
        ready: false,
        dirty: false,
        explicit: false,
        pending_query: None,
        repaint: false,
        line: String::new(),
        cursor: 0,
        completion: Completion::default(),
        selected: 0,
        dismissed: false,
        indexed: false,
        environment: None,
        diagnostic: None,
        nested_edit: false,
        bell: false,
        adapter_diagnostic_shown: false,
    };
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    let mut exit_code = None;
    'events: loop {
        let event = if exit_code.is_some() {
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(e) => e,
                Err(_) => break,
            }
        } else {
            match rx.recv() {
                Ok(e) => e,
                Err(_) => break,
            }
        };
        state.overlay.erase(state.parser.screen(), &mut output)?;
        let mut batch = vec![event];
        batch.extend(rx.try_iter().take(63));
        for event in batch {
            match event {
                HostEvent::Output(bytes) => {
                    for part in state.decoder.feed(&bytes) {
                        match part {
                            Part::Data(data) => {
                                state.parser.process(&data);
                                output.write_all(&data)?;
                            }
                            Part::Message(value) => state.message(value, &mut writer, &worker)?,
                            Part::CursorQuery(private) => {
                                let (row, col) = state.parser.screen().cursor_position();
                                writer.write_all(
                                    format!(
                                        "\x1b[{}{};{}R",
                                        if private { "?" } else { "" },
                                        row + 1,
                                        col + 1
                                    )
                                    .as_bytes(),
                                )?;
                                writer.flush()?;
                            }
                        }
                    }
                }
                HostEvent::Input(input) => {
                    state.input(input, &mut writer, master.as_ref(), &worker)?
                }
                HostEvent::Completion(revision, completion)
                    if revision == state.revision && state.prompt && !state.dismissed =>
                {
                    state.completion = completion;
                    state.selected = 0;
                }
                HostEvent::Completion(_, _) => {}
                HostEvent::Exit(code) => exit_code = Some(code),
                HostEvent::Eof => break 'events,
                HostEvent::Error(error) => {
                    if exit_code.is_none() {
                        return Err(anyhow::anyhow!(error));
                    }
                }
            }
        }
        state.query(&mut writer)?;
        if std::mem::take(&mut state.repaint) {
            output.write_all(b"\x1b[2J\x1b[H")?;
            output.write_all(&state.parser.screen().state_formatted())?;
        }
        if std::mem::take(&mut state.bell) {
            output.write_all(b"\x07")?;
        }
        if let Some(message) = state.diagnostic.take() {
            let bytes = format!("\r\nShellSense: {message}\r\n");
            state.parser.process(bytes.as_bytes());
            output.write_all(bytes.as_bytes())?;
        }
        if state.prompt && !state.dismissed {
            let query = state
                .line
                .get(state.completion.replace_start..state.cursor)
                .unwrap_or("");
            state.overlay.draw(
                state.parser.screen(),
                &mut output,
                &state.completion.candidates,
                state.selected,
                query,
                &state.config,
            )?;
        }
        output.flush()?;
    }
    state.overlay.erase(state.parser.screen(), &mut output)?;
    output.write_all(&state.decoder.finish())?;
    output.flush()?;
    Ok(exit_code.unwrap_or(0))
}

use crate::{
    cache_writer::CacheWriter,
    config::{self, Config},
    engine::{CommandIndex, Discovery},
    input::{self, Input},
    model::{Candidate, Completion, InputContext, ShellCommand},
    overlay::Overlay,
    protocol::{self, Decoder, Part},
    pty,
    ranking::{Learning, UsageSnapshot},
    trace::Trace,
};
use anyhow::{Context, Result};
#[cfg(not(windows))]
use crossterm::event;
use crossterm::{event::Event, terminal};
use serde_json::{Value, json};
#[cfg(not(windows))]
use std::time::Duration;
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
    time::Instant,
};

pub struct RunOptions {
    pub shell: PathBuf,
    pub no_profile: bool,
    pub config_path: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub trace_path: Option<PathBuf>,
    pub transport: Transport,
}

#[derive(Clone, Copy, Default, clap::ValueEnum)]
pub enum Transport {
    #[default]
    Osc,
    Pipe,
}

enum HostEvent {
    Output(Vec<u8>),
    Input(Vec<Event>),
    Eof,
    Exit(u32),
    Error(String),
    Completion(u64, Completion),
    Diagnostic(String),
    Reload,
}

#[derive(Clone)]
struct Query {
    revision: u64,
    line: String,
    cursor: usize,
    cwd: PathBuf,
    limit: usize,
    descriptions: Arc<BTreeMap<String, String>>,
    context: Option<InputContext>,
    fuzzy: bool,
    usage: Option<Arc<UsageSnapshot>>,
    environment: Arc<BTreeMap<String, String>>,
    dynamic: bool,
    searching: bool,
    help_enabled: bool,
}

#[derive(Default)]
struct Work {
    started: bool,
    refresh: bool,
    stop: bool,
    commands: Option<Vec<ShellCommand>>,
    query: Option<Query>,
    environment: Option<(String, String)>,
    source_updates: Vec<crate::sources::SourceUpdate>,
    cancel: bool,
    invalidate_sources: bool,
    help_updates: Vec<crate::knowledge::Record>,
    catalog_path: Option<PathBuf>,
    catalog_snapshot: Option<Arc<crate::spec_catalog::Catalog>>,
}

struct Worker(Arc<(Mutex<Work>, Condvar)>);

impl Worker {
    fn new(
        cache: PathBuf,
        catalog_path: PathBuf,
        source_status_path: PathBuf,
        output: SyncSender<HostEvent>,
        trace: Trace,
    ) -> Self {
        let shared = Arc::new((Mutex::new(Work::default()), Condvar::new()));
        let thread_shared = shared.clone();
        thread::spawn(move || {
            let help_shared = thread_shared.clone();
            let learner = crate::knowledge::Learner::new(
                cache
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("help"),
                move |record| {
                    let mut work = help_shared.0.lock().unwrap();
                    work.help_updates.push(record);
                    help_shared.1.notify_one();
                },
            );
            let mut learned = Vec::<crate::knowledge::Record>::new();
            let mut help_catalog: Option<(String, Arc<crate::spec_catalog::Catalog>)> = None;
            let mut entry_cache = std::collections::HashMap::<
                (String, PathBuf),
                (Instant, Option<crate::knowledge::Entry>),
            >::new();
            let mut catalog = Arc::new(crate::spec_catalog::Catalog::builtin().clone());
            match crate::spec_catalog::Catalog::load_user_dir(&catalog_path) {
                Ok(loaded) if loaded.diagnostics().is_empty() => catalog = Arc::new(loaded),
                Ok(_) => {
                    let _ = output.send(HostEvent::Diagnostic(
                        "用户规格无效，暂时使用内置规格；运行 specs check 查看原因".into(),
                    ));
                }
                Err(error) => {
                    let _ =
                        output.send(HostEvent::Diagnostic(format!("加载用户规格失败：{error}")));
                }
            }
            thread_shared.0.lock().unwrap().catalog_snapshot = Some(catalog.clone());
            let source_shared = thread_shared.clone();
            let mut sources = crate::sources::Sources::new(move |update| {
                let (lock, condition) = &*source_shared;
                let mut work = lock.lock().unwrap();
                if update.kind == crate::sources::SourceKind::Invalidated {
                    work.invalidate_sources = true;
                } else {
                    work.source_updates.push(update);
                }
                condition.notify_one();
            });
            let mut request: Option<crate::sources::SourceRequest> = None;
            let mut parts: [Option<crate::sources::SourceUpdate>; 2] = [None, None];
            let cache_writer = CacheWriter::new(cache.clone(), trace.clone());
            let source_status = crate::status::StatusWriter::new(source_status_path);
            let mut last_source_status = Value::Null;
            let mut index: Option<CommandIndex> = None;
            let mut discovery: Option<Discovery> = None;
            let mut environment: Option<(String, String)> = None;
            let mut shell_commands = Vec::new();
            let mut latest_query: Option<Query> = None;
            let mut prepared: Option<(u64, Completion, crate::sources::SourceRequest)> = None;
            loop {
                let (lock, condition) = &*thread_shared;
                let mut work = lock.lock().unwrap();
                while !work.stop
                    && (!work.started
                        || (!work.refresh
                            && work.commands.is_none()
                            && work.query.is_none()
                            && work.source_updates.is_empty()
                            && work.help_updates.is_empty()
                            && !work.invalidate_sources
                            && !work.cancel
                            && work.catalog_path.is_none()
                            && index.is_some()
                            && discovery.is_none()))
                {
                    work = condition.wait(work).unwrap();
                }
                if work.stop {
                    break;
                }
                let refresh = std::mem::take(&mut work.refresh);
                let cancel = std::mem::take(&mut work.cancel);
                let invalidate_sources = std::mem::take(&mut work.invalidate_sources);
                let updates = std::mem::take(&mut work.source_updates);
                let help_updates = std::mem::take(&mut work.help_updates);
                let help_changed = !help_updates.is_empty();
                let reload_catalog = work.catalog_path.take();
                let commands = work.commands.take();
                let commands_changed = commands.is_some();
                let query = work.query.take();
                let query_changed = query.is_some();
                if let Some(env) = work.environment.take() {
                    environment = Some(env);
                }
                drop(work);
                if cancel {
                    learner.cancel();
                    prepared = None;
                    latest_query = None;
                    request = None;
                    parts = [None, None];
                    sources.cancel();
                }
                if help_changed || reload_catalog.is_some() || refresh {
                    help_catalog = None;
                    prepared = None;
                }
                if reload_catalog.is_some() || refresh {
                    learner.invalidate();
                    learned.retain(|r| r.entry.fingerprint == "session");
                }
                for record in help_updates {
                    learned.retain(|r| {
                        r.entry.fingerprint != record.entry.fingerprint
                            || r.entry.command != record.entry.command
                            || r.context != record.context
                    });
                    if learned.len() >= 512 {
                        learned.remove(0);
                    }
                    learned.push(record);
                }
                if refresh {
                    entry_cache.clear();
                }
                if let Some(path) = reload_catalog.as_ref() {
                    match crate::spec_catalog::Catalog::load_user_dir(path) {
                        Ok(loaded) if loaded.diagnostics().is_empty() => {
                            catalog = Arc::new(loaded);
                            thread_shared.0.lock().unwrap().catalog_snapshot =
                                Some(catalog.clone());
                        }
                        Ok(_) => {
                            let _ = output.send(HostEvent::Diagnostic(
                                "用户规格校验失败，保留上次有效规格；运行 specs check 查看原因"
                                    .into(),
                            ));
                        }
                        Err(error) => {
                            let _ = output
                                .send(HostEvent::Diagnostic(format!("规格重载失败：{error}")));
                        }
                    }
                }
                if let Some(commands) = commands {
                    shell_commands = commands;
                }
                if let Some(query) = query {
                    latest_query = Some(query);
                }
                let mut changed = commands_changed;
                if (index.is_none() && discovery.is_none()) || refresh {
                    let (path, pathext) = environment.clone().unwrap_or_else(|| {
                        (
                            std::env::var("PATH").unwrap_or_default(),
                            std::env::var("PATHEXT")
                                .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into()),
                        )
                    });
                    let started = Instant::now();
                    let cached = if refresh {
                        None
                    } else {
                        CommandIndex::load_with_env(&cache, OsStr::new(&path), OsStr::new(&pathext))
                            .ok()
                    };
                    trace.event(
                        if cached.is_some() {
                            "cache_hit"
                        } else {
                            "cache_miss"
                        },
                        None,
                        Some(started.elapsed()),
                        None,
                    );
                    if let Some(cached) = cached {
                        index = Some(cached);
                    } else {
                        discovery = Some(Discovery::new(OsStr::new(&path), OsStr::new(&pathext)));
                    }
                    changed = true;
                }
                let mut finished = false;
                if let Some(scan) = discovery.as_mut() {
                    let started = Instant::now();
                    finished = scan.step();
                    index = Some(scan.snapshot());
                    trace.event(
                        "index_batch",
                        None,
                        Some(started.elapsed()),
                        Some(usize::from(finished)),
                    );
                    changed = true;
                }
                if finished {
                    discovery = None;
                }
                let index = index.as_mut().unwrap();
                if changed {
                    index.replace_shell_commands(shell_commands.clone());
                }
                if let Some(q) = latest_query.as_ref().filter(|_| {
                    changed
                        || help_changed
                        || query_changed
                        || !updates.is_empty()
                        || invalidate_sources
                        || refresh
                        || reload_catalog.is_some()
                }) {
                    let derived = index.input_context(&q.line, q.cursor);
                    let context = q.context.as_ref().unwrap_or(&derived);
                    let command = if context.command_position {
                        context.prefix.as_str()
                    } else {
                        context.command.as_str()
                    };
                    let entry_key = (command.to_owned(), q.cwd.clone());
                    if entry_cache
                        .get(&entry_key)
                        .is_none_or(|(time, _)| time.elapsed() > std::time::Duration::from_secs(1))
                    {
                        let found = index
                            .executable(command)
                            .and_then(|path| crate::knowledge::entry(command, &path, &q.cwd));
                        if entry_cache.len() >= 256 {
                            entry_cache.clear();
                        }
                        entry_cache.insert(entry_key.clone(), (Instant::now(), found));
                    }
                    let executable = entry_cache.get(&entry_key).and_then(|(_, e)| e.clone());
                    let key = executable
                        .as_ref()
                        .map(|e| e.fingerprint.clone())
                        .unwrap_or_default();
                    if help_catalog.as_ref().is_none_or(|(old, _)| old != &key) {
                        let mut effective = (*catalog).clone();
                        for record in learned.iter().filter(|r| {
                            r.entry.fingerprint == key || r.entry.fingerprint == "session"
                        }) {
                            effective.apply_help(record);
                        }
                        help_catalog = Some((key, Arc::new(effective)));
                    }
                    let catalog = help_catalog.as_ref().unwrap().1.clone();
                    thread_shared.0.lock().unwrap().catalog_snapshot = Some(catalog.clone());
                    if (query_changed || changed || refresh || reload_catalog.is_some())
                        && let Some(mut entry) = executable
                    {
                        if !q.help_enabled {
                            entry.trusted = false;
                        }
                        let context = catalog
                            .lookup_unfiltered(command, &context.arguments, "")
                            .map(|r| {
                                r.context
                                    .split_whitespace()
                                    .skip(1)
                                    .map(str::to_owned)
                                    .collect()
                            })
                            .unwrap_or_default();
                        learner.submit(entry, context);
                    }
                    if q.searching {
                        // Root purpose search can use knowledge from other installed tools,
                        // but only after checking that their active entry is still the same.
                        let mut search_catalog = (*catalog).clone();
                        if context.command_position {
                            for record in
                                learned.iter().filter(|r| r.entry.fingerprint != "session")
                            {
                                if index
                                    .executable(&record.entry.command)
                                    .and_then(|p| std::fs::canonicalize(p).ok())
                                    .is_some_and(|p| p == record.entry.path)
                                    && crate::knowledge::current(record)
                                {
                                    search_catalog.apply_help(record);
                                }
                            }
                        }
                        let mut result = index.purpose_search(
                            &search_catalog,
                            &q.line,
                            q.cursor,
                            q.context.as_ref(),
                            q.limit,
                            &q.descriptions,
                        );
                        if context.command_position {
                            for action in crate::providers::project_actions(&q.cwd,&q.line) {
                                if result.candidates.len()>=q.limit { break; }
                                result.candidates.push(Candidate{label:action.value.clone(),insert_text:action.value,description:action.description,kind:action.kind,id:action.source.clone(),source:action.source,match_reason:"当前项目".into(),append_space:true,..Default::default()});
                            }
                        }
                        let _ = output.send(HostEvent::Completion(q.revision, result));
                        continue;
                    }
                    let started = Instant::now();
                    let (base, next) = if let Some((_, base, next)) =
                        prepared.as_ref().filter(|(revision, _, _)| {
                            *revision == q.revision
                                && !changed
                                && !query_changed
                                && reload_catalog.is_none()
                        }) {
                        (base.clone(), next.clone())
                    } else {
                        let result = crate::completion::plan(
                            index,
                            &catalog,
                            &q.line,
                            q.cursor,
                            &q.cwd,
                            q.context.as_ref(),
                            q.revision,
                            q.environment.clone(),
                            q.fuzzy,
                            q.dynamic,
                            &q.descriptions,
                        );
                        prepared = Some((q.revision, result.0.clone(), result.1.clone()));
                        result
                    };
                    let force = invalidate_sources || refresh || reload_catalog.is_some();
                    if request.as_ref() != Some(&next) || force {
                        parts = [None, None];
                        sources.submit(next.clone(), force);
                        request = Some(next);
                    }
                    for update in updates {
                        if update.revision != q.revision
                            || update.generation != sources.generation()
                        {
                            continue;
                        }
                        if let Some(root) = update.project_root.as_ref() {
                            sources.watch_project(root);
                        }
                        sources.watch_paths(&update.watch_paths);
                        sources.watch_recursive_paths(&update.recursive_watch_paths);
                        let slot = usize::from(update.kind == crate::sources::SourceKind::Paths);
                        trace.event(
                            "dynamic_ready",
                            Some(q.revision),
                            Some(update.elapsed),
                            Some(update.candidates.len()),
                        );
                        parts[slot] = Some(update);
                    }
                    // Report the most recently returned provider snapshots.
                    // Unchanged hot queries do not cause another disk write.
                    let provider_state = json!({
                        "provider": request.as_ref().and_then(|r| r.provider.as_deref()),
                        "lanes": parts.iter().enumerate().map(|(slot, part)| {
                            part.as_ref().map(|p| json!({
                                "kind": if slot == 0 { "project" } else { "paths" },
                                "complete": !p.incomplete,
                                "candidates": p.candidates.len(),
                                "diagnostics": p.diagnostics,
                            }))
                        }).collect::<Vec<_>>()
                    });
                    if parts.iter().all(Option::is_some) && provider_state != last_source_status {
                        source_status.update(provider_state.clone());
                        last_source_status = provider_state;
                    }
                    let result = crate::completion::merge(
                        base,
                        request.as_ref().unwrap(),
                        [parts[0].as_ref(), parts[1].as_ref()],
                        q.usage.as_deref(),
                        q.limit,
                        &q.descriptions,
                    );
                    trace.event(
                        "completion",
                        Some(q.revision),
                        Some(started.elapsed()),
                        Some(result.candidates.len()),
                    );
                    if output
                        .send(HostEvent::Completion(q.revision, result))
                        .is_err()
                    {
                        break;
                    }
                }
                // A partial index is never published as a complete cache.
                if finished {
                    cache_writer.submit(index.clone());
                }
                // Scanning is active work, not a timer loop. Yield at each
                // bounded batch and check the latest request before continuing.
                if discovery.is_some() {
                    thread::yield_now();
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

const MOUSE_OFF: &[u8] = b"\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l";

pub(crate) struct RawMode {
    #[cfg(windows)]
    original: u32,
    mouse: bool,
}
impl RawMode {
    pub(crate) fn enable() -> std::io::Result<Self> {
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
                    & !(ENABLE_LINE_INPUT
                        | ENABLE_ECHO_INPUT
                        | ENABLE_PROCESSED_INPUT
                        | ENABLE_QUICK_EDIT_MODE
                        | ENABLE_MOUSE_INPUT))
                    | ENABLE_WINDOW_INPUT
                    | ENABLE_VIRTUAL_TERMINAL_INPUT
                    | ENABLE_EXTENDED_FLAGS;
                if SetConsoleMode(handle, mode) == 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(Self {
                original,
                mouse: false,
            })
        }
        #[cfg(not(windows))]
        {
            terminal::enable_raw_mode()?;
            Ok(Self { mouse: false })
        }
    }

    pub(crate) fn mouse(&mut self, enabled: bool) -> std::io::Result<()> {
        if self.mouse == enabled {
            return Ok(());
        }
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Console::*;
            let handle = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode = 0;
            if GetConsoleMode(handle, &mut mode) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            mode &= !ENABLE_MOUSE_INPUT;
            if enabled {
                mode |= ENABLE_MOUSE_INPUT;
            }
            if SetConsoleMode(handle, mode) == 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        self.mouse = enabled;
        Ok(())
    }
}
impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = std::io::stdout().write_all(MOUSE_OFF);
        let _ = std::io::stdout().flush();
        #[cfg(windows)]
        // Restore exactly the input mode inherited from the launching terminal.
        unsafe {
            use windows_sys::Win32::System::Console::*;
            SetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), self.original);
        }
        #[cfg(not(windows))]
        let _ = terminal::disable_raw_mode();
        let _ = std::io::stdout().write_all(b"\x1b[0m\x1b[?25h\x1b[?2004l");
        let _ = std::io::stdout().flush();
    }
}

struct ChildGuard(Box<dyn portable_pty::ChildKiller + Send + Sync>);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn plain_directory(path: &std::path::Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}

struct SessionFiles(PathBuf);
impl Drop for SessionFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("edit.json"));
        let _ = std::fs::remove_file(self.0.join("request.json"));
        let _ = std::fs::remove_file(self.0.join("adapter.json"));
        let _ = std::fs::remove_file(self.0.join("adapter.tmp"));
        let _ = std::fs::remove_file(self.0.join("data-sources.json"));
        let _ = std::fs::remove_file(self.0.join("data-sources.tmp"));
        let paste = self.0.join("paste");
        if plain_directory(&paste)
            && let Ok(entries) = std::fs::read_dir(&paste)
        {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_file()) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let _ = std::fs::remove_dir(paste);
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[derive(Default)]
struct CommandSnapshot {
    id: Option<String>,
    pending: Vec<ShellCommand>,
    previous: Vec<ShellCommand>,
}
impl CommandSnapshot {
    fn receive(&mut self, value: &Value) -> Option<(Vec<ShellCommand>, bool)> {
        let commands: Vec<ShellCommand> = serde_json::from_value(value["commands"].clone()).ok()?;
        let Some(id) = value["snapshot"].as_str() else {
            self.previous = commands.clone();
            return Some((commands, true));
        };
        if self.id.as_deref() != Some(id) {
            self.id = Some(id.into());
            self.pending.clear();
        }
        self.pending.extend(commands);
        let complete = value["complete"] == true;
        if complete {
            self.previous = std::mem::take(&mut self.pending);
            self.id = None;
            Some((self.previous.clone(), true))
        } else {
            // Keep old commands available until the replacement snapshot is
            // complete; a partial batch must not delete aliases/functions.
            let mut merged = self.previous.clone();
            merged.extend(self.pending.clone());
            Some((merged, false))
        }
    }
}

struct HubForm { template:String, fields:Vec<String>, index:usize, values:BTreeMap<String,String> }

struct State {
    pipe: Option<crate::pipe::PipeServer>,
    parser: vt100::Parser,
    decoder: Decoder,
    overlay: Overlay,
    config: Config,
    descriptions: Arc<BTreeMap<String, String>>,
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
    selection_touched: bool,
    dismissed: bool,
    indexed: bool,
    environment: Option<(String, String)>,
    diagnostic: Option<String>,
    nested_edit: bool,
    bell: bool,
    adapter_diagnostic_shown: bool,
    repaint: bool,
    commands_snapshot: CommandSnapshot,
    commands_pending: bool,
    commands_inflight: bool,
    commands_allowed: bool,
    trace: Trace,
    query_started: Option<Instant>,
    request_path: PathBuf,
    protocol_prefix: String,
    context: Option<InputContext>,
    shell_environment: Arc<BTreeMap<String, String>>,
    learning: Arc<Learning>,
    pending_accept: Option<(u64, String, PathBuf)>,
    native_request: Option<u64>,
    native_ready: bool,
    metadata_ready: bool,
    metadata_active: bool,
    metadata_pending: Option<String>,
    metadata_seen: std::collections::HashSet<String>,
    paste_ready: bool,
    paste_sequence: u64,
    native_menu: bool,
    menu_focus: bool,
    details: bool,
    detail_page: usize,
    searching: bool,
    enter_ready: bool,
    shift_enter_ready: bool,
    native_queued: bool,
    monitor: crate::reload::Monitor,
    notification: Option<String>,
    public_keys: BTreeMap<String, bool>,
    reset_pending: Option<String>,
    status_writer: crate::status::StatusWriter,
    hub_query: Option<String>,
    hub_history: Vec<String>,
    hub_history_path: Option<PathBuf>,
    history_ready: bool,
    history_pending: bool,
    hub_form: Option<HubForm>,
}

impl State {
    fn refresh_hub(&mut self) {
        let Some(query)=self.hub_query.as_deref() else{return};
        if let Some(form)=self.hub_form.as_ref(){
            let name=form.fields.get(form.index).cloned().unwrap_or_default();
            let preview=crate::hub::fill_placeholders(&form.template,&form.values);
            self.completion=Completion{replace_start:0,replace_end:self.line.len(),candidates:vec![Candidate{label:format!("填写参数：{name}"),insert_text:preview,description:if query.is_empty(){"输入参数值后按 Enter".into()}else{format!("当前值：{query}")},kind:crate::model::CandidateKind::Value,id:format!("hub-form:{name}"),source:"Blueberry 工作台/参数表单".into(),append_space:false,..Default::default()}],incomplete:false,argument_hint:format!("参数 {}/{} · {}",form.index+1,form.fields.len(),name)};self.selected=0;self.dismissed=false;self.searching=true;return
        }
        let selected=self.completion.candidates.get(self.selected).map(|c|c.identity().to_owned());
        let candidates=crate::hub::candidates_with_history(&self.config,query,&self.hub_history,None);
        self.selected=selected.and_then(|id|candidates.iter().position(|c|c.identity()==id)).unwrap_or(0);
        self.completion=Completion{replace_start:0,replace_end:self.line.len(),candidates,incomplete:self.history_pending,argument_hint:"输入关键词搜索；Enter 填回，Tab/F1 预览，Esc 返回".into()};
        self.menu_focus=true;self.explicit=true;self.dismissed=false;self.searching=true;
    }

    fn write_payload(&mut self, kind: &str, payload: &Value) -> Result<()> {
        if let Some(pipe) = self.pipe.as_mut() {
            if pipe
                .write_json(&json!({"kind":kind,"payload":payload}))
                .is_ok()
            {
                return Ok(());
            }
            pipe.disable();
            self.pipe = None;
            self.diagnostic = Some("管道不可用，已回退 OSC 和本地编辑载荷".into());
        }
        let path = if kind == "edit" {
            &self.edit_path
        } else {
            &self.request_path
        };
        std::fs::write(path, serde_json::to_vec(payload)?)?;
        Ok(())
    }
    fn reload(&mut self, worker: &Worker) {
        match config::load(self.config_path.as_deref()) {
            Ok(mut config) => {
                if serde_json::to_string(&self.config.keys).ok()
                    != serde_json::to_string(&config.keys).ok()
                {
                    self.reset_pending = serde_json::to_string(&config.keys).ok();
                }
                self.descriptions = Arc::new(std::mem::take(&mut config.descriptions));
                self.config = config;
                self.monitor
                    .update(&self.config, self.config_path.as_deref());
                worker.update(|w| {
                    w.catalog_path =
                        Some(config::specs_dir(&self.config, self.config_path.as_deref()))
                });
                if self.config.keys.protocol_prefix != self.protocol_prefix {
                    self.diagnostic = Some("内部快捷键前缀将在下次启动生效".into());
                }
                self.invalidate();
                self.dirty = self.prompt;
                self.explicit = true;
                self.dismissed = false;
            }
            Err(error) => {
                self.diagnostic = Some(format!("配置无效，保留上次有效设置：{error}"));
            }
        }
    }
    fn invalidate(&mut self) {
        self.revision += 1;
        self.completion = Completion::default();
        self.selected = 0;
        self.selection_touched = false;
        self.menu_focus = false;
        self.details = false;
        self.native_menu = false;
    }
    fn schedule_metadata(&mut self) {
        if !self.metadata_ready || !self.prompt {
            return;
        }
        let derived = CommandIndex::default().input_context(&self.line, self.cursor);
        let context = self.context.as_ref().unwrap_or(&derived);
        if context.suppressed || self.line.contains(['\n', '\r']) {
            return;
        }
        let name = if context.command_position {
            &context.prefix
        } else {
            &context.command
        };
        let confirmed_name = !context.command_position
            && !context.suppressed
            && !name.is_empty()
            && name.len() <= 128
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
        if (confirmed_name
            || self
                .commands_snapshot
                .pending
                .iter()
                .chain(self.commands_snapshot.previous.iter())
                .any(|c| {
                    c.name.eq_ignore_ascii_case(name)
                        && matches!(c.kind.as_str(), "function" | "cmdlet")
                }))
            && self.metadata_seen.insert(name.clone())
        {
            self.metadata_pending = Some(name.clone());
        }
    }
    fn message(&mut self, value: Value, _writer: &mut impl Write, worker: &Worker) -> Result<()> {
        match value["event"].as_str().unwrap_or("") {
            "capabilities" => {
                self.status_writer.update(value.clone());
                self.public_keys = serde_json::from_value(
                    value
                        .get("public_keys")
                        .unwrap_or(&value["capabilities"]["public_keys"])
                        .clone(),
                )
                .unwrap_or_default();
                self.enter_ready = value["key_handlers"]["enter"] == true;
                self.shift_enter_ready = value["key_handlers"]["shift_enter"] == true;
                self.native_ready = value["key_handlers"]["native"] == true;
                self.metadata_ready = value["capabilities"]["command_metadata"] == true;
                self.history_ready = value["capabilities"]["history"] == true;
                self.paste_ready = value["key_handlers"]["paste"] == true;
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
                self.notification = None;
                worker.update(|w| {
                    w.cancel = true;
                    w.query = None;
                });
                self.prompt = false;
                self.invalidate();
                self.pending_query = None;
                self.dirty = false;
            }
            "prompt_end" => {
                self.learning.refresh();
                worker.update(|w| w.invalidate_sources = true);
                self.native_queued = false;
                self.trace.event("prompt_end", None, None, None);
                self.prompt = true;
                self.commands_allowed = true;
                self.metadata_seen.clear();
                // ReadLine disposed the previous enumerator when execution
                // began. Refresh the session snapshot after every new prompt.
                self.commands_pending = self.ready;
                self.commands_inflight = false;
                self.metadata_pending = None;
                if !self.ready && !self.adapter_diagnostic_shown {
                    self.diagnostic = Some("PowerShell adapter unavailable (PSReadLine or reserved key binding). Completion is disabled for this session.".into());
                    self.adapter_diagnostic_shown = true;
                }
                self.line.clear();
                self.context = None;
                self.native_request = None;
                if let Ok(environment) =
                    serde_json::from_value::<BTreeMap<String, String>>(value["environment"].clone())
                {
                    self.shell_environment = Arc::new(environment);
                }
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
                    self.commands_pending = true;
                    self.indexed = true;
                }
            }
            "execute" => {
                worker.update(|w| {
                    w.cancel = true;
                    w.query = None;
                });
                self.prompt = false;
                self.pending_query = None;
                self.dirty = false;
                self.invalidate();
                self.native_request = None;
                self.commands_pending = false;
                self.commands_inflight = false;
                self.metadata_pending = None;
            }
            "editing" if value["state"] == "continuation" => {
                self.prompt = true;
                self.native_request = None;
                self.pending_query = None;
                self.dirty = true;
                self.dismissed = false;
            }
            "commands" => {
                self.commands_inflight = false;
                if let Some((commands, complete)) = self.commands_snapshot.receive(&value) {
                    worker.update(|w| w.commands = Some(commands));
                    self.commands_pending = !complete;
                    self.schedule_metadata();
                }
            }
            "buffer" => {
                let revision = self.pending_query.take().unwrap_or(self.revision);
                self.trace.event(
                    "query_response",
                    Some(revision),
                    self.query_started.take().map(|s| s.elapsed()),
                    None,
                );
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
                self.context = decode_context(line, &value["context"]);
                if !self.dismissed
                    && (self.explicit
                        || (self.config.completion.auto_trigger && !line.trim().is_empty()))
                {
                    let tool_name=self.context.as_ref().map(|context|context.command.as_str()).filter(|name|!name.is_empty()).unwrap_or_else(||line.split_whitespace().next().unwrap_or(""));
                    if self.config.resources_for(tool_name)=="automatic"{Arc::make_mut(&mut self.shell_environment).insert("BLUEBERRY_REMOTE_REQUESTED".into(),"1".into());}
                    let query = Query {
                        revision,
                        line: line.into(),
                        cursor,
                        cwd: self.cwd.clone(),
                        limit: self.config.completion.max_results,
                        descriptions: self.descriptions.clone(),
                        context: self.context.clone(),
                        fuzzy: self.config.completion.fuzzy,
                        usage: self
                            .config
                            .learning
                            .enabled
                            .then(|| self.learning.snapshot()),
                        environment: self.shell_environment.clone(),
                        dynamic: self.config.dynamic_for(tool_name),
                        searching: self.searching,
                        help_enabled: self.config.help_for(tool_name),
                    };
                    Arc::make_mut(&mut self.shell_environment)
                        .remove("BLUEBERRY_REMOTE_REQUESTED");
                    self.schedule_metadata();
                    worker.update(|w| w.query = Some(query));
                }
                self.explicit = false;
            }
            "command_metadata" => {
                if request_number(&value["request_id"]) == self.native_request {
                    self.native_request = None;
                    self.metadata_active = false;
                }
                // Static metadata is session-scoped, independent of the edit revision.
                if let (Some(command), Ok(page)) = (
                    value["command"].as_str(),
                    serde_json::from_value::<crate::knowledge::HelpPage>(value["page"].clone()),
                ) {
                    let record = crate::knowledge::Record {
                        parser_version: 1,
                        entry: crate::knowledge::Entry {
                            command: command.into(),
                            path: PathBuf::from("PowerShell 会话"),
                            target: PathBuf::new(),
                            fingerprint: "session".into(),
                            trusted: false,
                            script: None,
                        },
                        context: Vec::new(),
                        page,
                        fetched_at: 0,
                        error: None,
                    };
                    worker.update(|w| w.help_updates.push(record));
                }
            }
            "history" => {
                if request_number(&value["request_id"]) == self.native_request {
                    self.native_request=None;
                }
                let commands=serde_json::from_value::<Vec<String>>(value["commands"].clone()).unwrap_or_default();
                self.hub_history_path=value["path"].as_str().filter(|p|!p.is_empty()).map(PathBuf::from);
                self.hub_history=crate::hub::merged_history(self.config.workbench.history_limit,&commands,self.hub_history_path.as_deref());
                self.history_pending=false;
                if self.hub_query.is_some(){self.refresh_hub();}
            }
            "edit_result" => {
                let id = request_number(&value["request_id"]);
                if self
                    .pending_accept
                    .as_ref()
                    .is_some_and(|pending| Some(pending.0) == id)
                {
                    let (_, key, project) = self.pending_accept.take().unwrap();
                    if value["applied"] == true {
                        if self.config.learning.enabled {
                            self.learning.accepted(&key, &project);
                        }
                        self.dismissed = false;
                        self.explicit = true;
                    } else {
                        self.pending_query = None;
                        self.dirty = true;
                    }
                }
            }
            "native_completion" => {
                let id = request_number(&value["request_id"]);
                if id != self.native_request {
                    return Ok(());
                }
                self.native_request = None;
                self.pending_query = None;
                if id != Some(self.revision) || !self.prompt {
                    self.dirty = self.prompt;
                    return Ok(());
                }
                if value["status"] != "ok" {
                    self.diagnostic = Some("PowerShell 原生补全没有返回结果".into());
                    return Ok(());
                }
                let Some(line) = value["line"].as_str() else {
                    return Ok(());
                };
                let Some(cursor) = value["cursor"]
                    .as_u64()
                    .and_then(|c| protocol::utf16_to_byte(line, c as usize))
                else {
                    return Ok(());
                };
                let Some(start) = value["replace_start"]
                    .as_u64()
                    .and_then(|c| protocol::utf16_to_byte(line, c as usize))
                else {
                    return Ok(());
                };
                let Some(end) = value["replace_end"]
                    .as_u64()
                    .and_then(|c| protocol::utf16_to_byte(line, c as usize))
                else {
                    return Ok(());
                };
                if start > end {
                    return Ok(());
                }
                let Ok(mut candidates) =
                    serde_json::from_value::<Vec<Candidate>>(value["candidates"].clone())
                else {
                    return Ok(());
                };
                candidates.truncate(self.config.completion.max_results);
                let native_context = decode_context(line, &value["context"]).unwrap_or_else(|| {
                    CommandIndex::discover_with_env(OsStr::new(""), OsStr::new(".EXE"))
                        .input_context(line, cursor)
                });
                let catalog = worker.0.0.lock().unwrap().catalog_snapshot.clone();
                let catalog = catalog
                    .as_deref()
                    .unwrap_or_else(|| crate::spec_catalog::Catalog::builtin());
                let known = catalog.lookup_unfiltered(
                    &native_context.command,
                    &native_context.arguments,
                    "",
                );
                for candidate in &mut candidates {
                    candidate.id = format!("native:{}", candidate.insert_text);
                    candidate.source = "PowerShell 原生补全".into();
                    crate::knowledge::native_description(candidate);
                    candidate.description = catalog
                        .describe_command(&candidate.label)
                        .unwrap_or(&candidate.description)
                        .into();
                    let mut description_key = candidate.label.clone();
                    if let Some(spec) = known.as_ref().and_then(|result| {
                        result.candidates.iter().find(|spec| {
                            spec.name == candidate.label || spec.name == candidate.insert_text
                        })
                    }) {
                        candidate.description = spec.description.clone();
                        candidate.description_source = spec.source.clone();
                        description_key = spec.key.clone();
                    }
                    if let Some(custom) = self.descriptions.get(&description_key) {
                        candidate.description = custom.clone();
                        candidate.description_source = "user".into();
                    }
                }
                self.line = line.into();
                self.cursor = cursor;
                self.completion = Completion {
                    replace_start: start,
                    replace_end: end,
                    candidates,
                    incomplete: false,
                    argument_hint: String::new(),
                };
                self.native_menu = true;
                self.menu_focus = true;
                self.dismissed = false;
                self.selected = 0;
            }
            "error" => {
                self.notification = Some(format!(
                    "{}：{}",
                    value["code"].as_str().unwrap_or("adapter"),
                    value["message"].as_str().unwrap_or("适配器错误")
                ));
                self.pending_query = None;
                self.dirty = false;
                self.dismissed = false;
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
            "trace" => {
                let stage = match value["stage"].as_str() {
                    Some("adapter_bootstrap") => "adapter_bootstrap",
                    Some("readline_init") => "readline_init",
                    Some("command_snapshot") => "command_snapshot",
                    Some("serialize") => "serialize",
                    Some("key_snapshot") => "key_snapshot",
                    Some("key_register") => "key_register",
                    Some("public_keys") => "public_keys",
                    Some("readline_wrap") => "readline_wrap",
                    _ => return Ok(()),
                };
                if let Some(ms) = value["duration_ms"]
                    .as_f64()
                    .filter(|v| v.is_finite() && *v >= 0.0 && *v < 86_400_000.0)
                {
                    self.trace.event(
                        stage,
                        None,
                        Some(std::time::Duration::from_secs_f64(ms / 1000.0)),
                        None,
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn query(&mut self, writer: &mut impl Write) -> Result<()> {
        if self.history_pending
            && self.history_ready
            && self.ready
            && self.prompt
            && self.native_request.is_none()
            && self.pending_query.is_none()
            && !self.commands_inflight
        {
            self.native_request=Some(self.revision);
            self.write_payload("request",&json!({"id":self.revision.to_string(),"kind":"history","limit":self.config.workbench.history_limit}))?;
            writer.write_all(&input::protocol_chord(&self.protocol_prefix,'n'))?;writer.flush()?;
        } else if self.native_queued
            && !self.commands_inflight
            && self.ready
            && self.prompt
            && self.pending_query.is_none()
            && self.native_request.is_none()
        {
            self.native_queued = false;
            self.native_request = Some(self.revision);
            self.dirty = false;
            self.write_payload(
                "request",
                &json!({"id":self.revision.to_string(),"kind":"native"}),
            )?;
            writer.write_all(&input::protocol_chord(&self.protocol_prefix, 'n'))?;
            writer.flush()?;
        } else if self.dirty
            && self.ready
            && self.prompt
            && self.pending_query.is_none()
            && self.native_request.is_none()
        {
            writer.write_all(&input::protocol_chord(&self.protocol_prefix, 's'))?;
            writer.flush()?;
            self.pending_query = Some(self.revision);
            if self.trace.enabled() {
                self.query_started = Some(Instant::now());
            }
            self.trace
                .event("query_sent", Some(self.revision), None, None);
            self.dirty = false;
        } else if self.reset_pending.is_some()
            && !self.commands_inflight
            && self.ready
            && self.prompt
            && !self.nested_edit
            && self.pending_query.is_none()
            && self.native_request.is_none()
        {
            self.commands_inflight = true;
            let public_keys = self.reset_pending.take().unwrap();
            self.write_payload("request", &json!({"id":self.revision.to_string(),"kind":"commands_reset","public_keys_json":public_keys}))?;
            writer.write_all(&input::protocol_chord(&self.protocol_prefix, 'c'))?;
            writer.flush()?;
            self.commands_pending = false;
        } else if self.metadata_pending.is_some()
            && !self.commands_inflight
            && self.metadata_ready
            && self.prompt
            && !self.nested_edit
            && self.pending_query.is_none()
            && self.native_request.is_none()
        {
            let name = self.metadata_pending.take().unwrap();
            self.metadata_active = true;
            // Share the adapter request gate with manual native completion so
            // another request cannot overwrite the payload before it is consumed.
            self.native_request = Some(self.revision);
            self.write_payload(
                "request",
                &json!({"id":self.revision.to_string(),"kind":"command_metadata","command":name}),
            )?;
            writer.write_all(&input::protocol_chord(&self.protocol_prefix, 'n'))?;
            writer.flush()?;
        } else if self.commands_pending
            && self.commands_allowed
            && self.ready
            && self.prompt
            && !self.nested_edit
            && self.pending_query.is_none()
            && self.native_request.is_none()
        {
            self.commands_inflight = true;
            if self.pipe.is_some() {
                self.write_payload(
                    "request",
                    &json!({"id":self.revision.to_string(),"kind":"commands_next"}),
                )?;
            }
            writer.write_all(&input::protocol_chord(&self.protocol_prefix, 'c'))?;
            writer.flush()?;
            self.commands_pending = false;
        }
        Ok(())
    }

    fn accept(&mut self, writer: &mut impl Write) -> Result<()> {
        let Some(candidate) = self.completion.candidates.get(self.selected) else {
            return Ok(());
        };
        self.searching = false;
        let replacement=candidate.replacement.unwrap_or(crate::model::Replacement{start:self.completion.replace_start,end:self.completion.replace_end});
        let start = protocol::byte_to_utf16(&self.line, replacement.start);
        let end = protocol::byte_to_utf16(&self.line, replacement.end);
        let cursor = protocol::byte_to_utf16(&self.line, self.cursor);
        if let (Some(start), Some(end), Some(cursor)) = (start, end, cursor) {
            if end < start {
                return Ok(());
            }
            let id = self.revision + 1;
            let mut text = candidate.insert_text.clone();
            if self.config.completion.append_space
                && candidate.append_space
                && replacement.end == self.line.len()
                && !text.ends_with(char::is_whitespace)
                && crate::engine::space_after(&text)
            {
                text.push(' ');
            }
            self.pending_accept = Some((id, candidate.identity().into(), self.cwd.clone()));
            let edit = json!({"id":id.to_string(),"expectedLine":self.line,"expectedCursor":cursor,"start":start,"length":end-start,"text":text});
            // Only the child receives the apply chord, after this write is closed.
            self.write_payload("edit", &edit)?;
            self.invalidate();
            self.pending_query = Some(self.revision);
            self.dismissed = true;
            writer.write_all(&input::protocol_chord(&self.protocol_prefix, 'a'))?;
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
        if self.hub_query.is_some() {
            if matches!(&event,Event::Key(key) if key.kind==crossterm::event::KeyEventKind::Release){return Ok(())}
            use crossterm::event::{KeyCode,KeyModifiers};
            match event {
                Event::Paste(text)=>{if let Some(query)=self.hub_query.as_mut(){query.push_str(&text.replace(['\r','\n']," "));}self.refresh_hub();return Ok(())}
                Event::Resize(cols,rows)=>{
                    #[cfg(windows)] let (cols,rows)=terminal::size().unwrap_or((cols,rows));
                    if rows>0&&cols>0&&self.parser.screen().size()!=(rows,cols){master.resize(portable_pty::PtySize{rows,cols,pixel_width:0,pixel_height:0})?;self.parser.screen_mut().set_size(rows,cols);self.overlay=Overlay::default();self.repaint=true;}return Ok(())
                }
                Event::Key(key)=>{
                    if input::configured(&Event::Key(key),&self.config.keys).is_some_and(|i|matches!(i,Input::Hub))||key.code==KeyCode::Esc {
                        self.hub_query=None;self.hub_form=None;self.searching=false;self.invalidate();self.dismissed=true;return Ok(())
                    }
                    match key.code {
                        KeyCode::Char(ch) if !key.modifiers.intersects(KeyModifiers::CONTROL|KeyModifiers::ALT)=>{self.hub_query.as_mut().unwrap().push(ch);self.refresh_hub();return Ok(())}
                        KeyCode::Backspace=>{self.hub_query.as_mut().unwrap().pop();self.refresh_hub();return Ok(())}
                        KeyCode::Up=>{if !self.completion.candidates.is_empty(){self.selected=self.selected.checked_sub(1).unwrap_or(self.completion.candidates.len()-1);}return Ok(())}
                        KeyCode::Down=>{if !self.completion.candidates.is_empty(){self.selected=(self.selected+1)%self.completion.candidates.len();}return Ok(())}
                        KeyCode::PageUp=>{self.selected=self.selected.saturating_sub(self.config.ui.max_rows.max(1));return Ok(())}
                        KeyCode::PageDown=>{self.selected=(self.selected+self.config.ui.max_rows.max(1)).min(self.completion.candidates.len().saturating_sub(1));return Ok(())}
                        KeyCode::Tab|KeyCode::F(1)=>{self.details=!self.details;self.detail_page=0;return Ok(())}
                        KeyCode::Enter=>{
                            if self.hub_form.is_some(){
                                let value=self.hub_query.as_ref().cloned().unwrap_or_default();if value.trim().is_empty(){self.notification=Some("参数值不能为空".into());return Ok(())}
                                let form=self.hub_form.as_mut().unwrap();let name=form.fields[form.index].clone();form.values.insert(name,value);form.index+=1;
                                if form.index<form.fields.len(){*self.hub_query.as_mut().unwrap()=String::new();self.refresh_hub();return Ok(())}
                                let form=self.hub_form.take().unwrap();let command=crate::hub::fill_placeholders(&form.template,&form.values);self.hub_query=None;self.searching=false;self.completion=Completion{replace_start:0,replace_end:self.line.len(),candidates:vec![Candidate{label:command.clone(),insert_text:command,description:"已填写命令模板".into(),kind:crate::model::CandidateKind::Command,id:"hub:resolved-template".into(),source:"Blueberry 工作台/模板".into(),append_space:false,..Default::default()}],incomplete:false,argument_hint:String::new()};self.selected=0;return self.accept(writer)
                            }
                            if let Some(candidate)=self.completion.candidates.get(self.selected){let fields=crate::hub::placeholder_names(&candidate.insert_text);if !fields.is_empty(){self.hub_form=Some(HubForm{template:candidate.insert_text.clone(),fields,index:0,values:BTreeMap::new()});*self.hub_query.as_mut().unwrap()=String::new();self.refresh_hub();return Ok(())}}
                            self.hub_query=None;self.searching=false;return self.accept(writer)
                        }
                        _=>return Ok(())
                    }
                }
                _=>return Ok(())
            }
        }
        if let Event::Mouse(mouse) = event {
            if self.prompt {
                return Ok(());
            }
            if let Some(bytes) = input::mouse_bytes(mouse, self.parser.screen()) {
                writer.write_all(&bytes)?;
                writer.flush()?;
            }
            return Ok(());
        }
        if matches!(&event, Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Release)
        {
            return Ok(());
        }
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
        // PSReadLine 2.0 drops surrogate key events. Insert supplementary
        // characters through the acknowledged literal-text bridge instead.
        let event = match event {
            Event::Key(key)
                if self.ready
                    && self.prompt
                    && !self.nested_edit
                    && self.paste_ready
                    && key.modifiers.is_empty()
                    && matches!(key.code, crossterm::event::KeyCode::Char(c) if c as u32 > 0xffff) =>
            {
                if let crossterm::event::KeyCode::Char(c) = key.code {
                    Event::Paste(c.to_string())
                } else {
                    unreachable!()
                }
            }
            event => event,
        };
        if let Event::Paste(text) = &event
            && self.prompt
        {
            if !self.ready || !self.paste_ready {
                self.diagnostic = Some("安全粘贴不可用：PSReadLine 或内部粘贴快捷键存在冲突，请检查 doctor 并更换内部快捷键前缀".into());
                return Ok(());
            }
            // The shell processes this private key in the same input stream
            // as surrounding keystrokes. Insert(string) treats newlines as
            // editable text instead of AcceptLine/execute key events.
            let directory = self
                .edit_path
                .parent()
                .context("session directory")?
                .join("paste");
            self.paste_sequence = self
                .paste_sequence
                .checked_add(1)
                .context("paste sequence exhausted")?;
            let id = self.paste_sequence.to_string();
            let payload = serde_json::to_vec(&json!({"id": id, "text": text}))?;
            if payload.len() > 1024 * 1024 {
                self.diagnostic = Some("粘贴内容超过单次 1 MiB 的协议上限，请分段粘贴".into());
                return Ok(());
            }
            std::fs::create_dir_all(&directory)?;
            anyhow::ensure!(
                plain_directory(&directory),
                "paste directory must not be a link or reparse point"
            );
            let temporary = directory.join(format!("{:020}.tmp", self.paste_sequence));
            let pending = directory.join(format!("{:020}.json", self.paste_sequence));
            std::fs::write(&temporary, payload)?;
            std::fs::rename(&temporary, &pending)?;
            self.invalidate();
            self.dismissed = false;
            self.dirty = true;
            writer.write_all(&input::protocol_chord(&self.protocol_prefix, 'p'))?;
            writer.flush()?;
            return Ok(());
        }
        let paste = matches!(event, Event::Paste(_));
        let edit_enter = if let Event::Key(key) = &event {
            use crossterm::event::{KeyCode, KeyModifiers};
            if self.prompt
                && self.ready
                && key.code == KeyCode::Enter
                && key.modifiers.is_empty()
                && self.enter_ready
            {
                Some('e')
            } else if self.prompt
                && self.ready
                && key.code == KeyCode::Enter
                && key.modifiers == KeyModifiers::SHIFT
                && self.shift_enter_ready
            {
                Some('l')
            } else {
                None
            }
        } else {
            None
        };
        if let Some(suffix) = edit_enter {
            self.notification = None;
            self.invalidate();
            self.prompt = false;
            self.searching = false;
            self.dirty = false;
            self.native_queued = false;
            worker.update(|w| {
                w.cancel = true;
                w.query = None;
            });
            writer.write_all(&input::protocol_chord(&self.protocol_prefix, suffix))?;
            writer.flush()?;
            return Ok(());
        }
        if matches!(event, Event::Key(_) | Event::Paste(_)) {
            self.notification = None;
            self.commands_allowed = query_after && !self.nested_edit;
        }
        if self.details
            && self.prompt
            && let Event::Key(key) = &event
        {
            use crossterm::event::KeyCode;
            if matches!(key.code, KeyCode::PageDown | KeyCode::PageUp) {
                let max = self
                    .completion
                    .candidates
                    .get(self.selected)
                    .map(|c| {
                        crate::menu::detail_lines(
                            c,
                            (self.parser.screen().size().1 as usize).min(
                                if self.config.ui.width == 0 {
                                    100
                                } else {
                                    self.config.ui.width
                                },
                            ),
                        )
                        .len()
                        .saturating_sub(1)
                            / 3
                    })
                    .unwrap_or(0);
                self.detail_page = if key.code == KeyCode::PageDown {
                    (self.detail_page + 1).min(max)
                } else {
                    self.detail_page.saturating_sub(1)
                };
                return Ok(());
            }
        }
        let configured = if self.prompt && self.ready {
            input::configured(&event, &self.config.keys).filter(|input| {
                let name = match input {
                    Input::Trigger => "trigger",
                    Input::Native => "native",
                    Input::Search => "search",
                    Input::Details => "details",
                    Input::Refresh => "refresh",
                    Input::Reload => "reload",
                    Input::Resources => "resources",
                    Input::Hub => "hub",
                    _ => return true,
                };
                self.public_keys.get(name).copied().unwrap_or(true)
            })
        } else {
            None
        };
        let Some(input) = configured
            .or_else(|| input::translate(event, self.parser.screen().application_cursor()))
        else {
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
            Input::Previous
                if visible && (self.menu_focus || !self.config.completion.up_arrow_history) =>
            {
                self.selection_touched = true;
                self.detail_page = 0;
                self.selected = self
                    .selected
                    .checked_sub(1)
                    .unwrap_or(self.completion.candidates.len() - 1);
                return Ok(());
            }
            Input::BackTab if visible => {
                self.menu_focus = true;
                self.selection_touched = true;
                self.detail_page = 0;
                self.selected = self
                    .selected
                    .checked_sub(1)
                    .unwrap_or(self.completion.candidates.len() - 1);
                return Ok(());
            }
            Input::Next if visible => {
                self.menu_focus = true;
                self.selection_touched = true;
                self.detail_page = 0;
                self.selected = (self.selected + 1) % self.completion.candidates.len();
                return Ok(());
            }
            Input::Search if self.prompt && self.ready => {
                self.searching = !self.searching;
                self.explicit = true;
                self.dismissed = false;
                self.dirty = true;
                self.menu_focus = true;
                return Ok(());
            }
            Input::Dismiss if self.searching => {
                self.searching = false;
                self.dismissed = true;
                self.invalidate();
                return Ok(());
            }
            Input::Dismiss if visible => {
                self.dismissed = true;
                self.invalidate();
                return Ok(());
            }
            Input::Trigger if self.prompt && self.ready => {
                self.menu_focus = true;
                self.explicit = true;
                self.dismissed = false;
                self.dirty = true;
                return Ok(());
            }
            Input::Refresh if self.prompt && self.ready => {
                self.commands_allowed = true;
                worker.update(|w| w.refresh = true);
                self.reset_pending = Some(serde_json::to_string(&self.config.keys)?);
                return Ok(());
            }
            Input::Details if visible => {
                self.details = !self.details;
                self.detail_page = 0;
                return Ok(());
            }
            Input::Native if self.prompt && self.ready => {
                if !self.native_ready {
                    self.diagnostic = Some("原生补全不可用；检查适配器版本和内部快捷键冲突".into());
                    return Ok(());
                }
                if self.native_request.is_some() && !self.metadata_active {
                    return Ok(());
                }
                self.metadata_pending = None;
                self.invalidate();
                worker.update(|w| {
                    w.cancel = true;
                    w.query = None;
                });
                self.native_queued = true;
                self.dirty = false;
                self.dismissed = false;
                return Ok(());
            }
            Input::Reload if self.prompt => {
                self.reload(worker);
                return Ok(());
            }
            Input::Resources if self.prompt && self.ready => {
                let tool=self.context.as_ref().map(|context|context.command.as_str()).filter(|name|!name.is_empty()).unwrap_or_else(||self.line.split_whitespace().next().unwrap_or(""));
                if self.config.resources_for(tool)=="off"{self.diagnostic=Some(format!("已在 tools.{tool}.resources 中关闭资源读取"));return Ok(())}
                Arc::make_mut(&mut self.shell_environment)
                    .insert("BLUEBERRY_REMOTE_REQUESTED".into(), "1".into());
                worker.update(|work| work.refresh=true);
                self.explicit=true; self.dismissed=false; self.dirty=true;
                self.diagnostic=Some("正在刷新当前本机资源；远程资源仍需在对应工具中明确读取。".into());
                return Ok(());
            }
            Input::Hub if self.prompt && self.ready => {
                self.hub_query=Some(String::new());self.hub_form=None;self.history_pending=self.history_ready;self.details=false;self.selected=0;self.refresh_hub();
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
            Input::Native => vec![0],
            Input::Search => b"\x1b\x06".to_vec(),
            Input::Details => b"\x1bOP".to_vec(),
            Input::Resources => Vec::new(),
            Input::Hub => Vec::new(),
            Input::Bytes(bytes) => bytes,
        };
        self.invalidate();
        self.dismissed = false;
        // After submitting a line the next input may belong to a native program.
        // Do not inject a PSReadLine chord until the next prompt marker.
        if bytes.iter().any(|b| matches!(b, b'\r' | b'\n' | 3 | 4)) {
            if paste && self.parser.screen().bracketed_paste() {
                self.dirty = self.prompt;
            } else {
                self.prompt = false;
                self.searching = false;
                self.dirty = false;
                worker.update(|w| {
                    w.cancel = true;
                    w.query = None;
                });
            }
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
    let trace = Trace::open(options.trace_path.as_deref())?;
    trace.event("host_start", None, None, None);
    let mut config = config::load(options.config_path.as_deref())?;
    let descriptions = Arc::new(std::mem::take(&mut config.descriptions));
    let protocol_prefix = config.keys.protocol_prefix.clone();
    let usage_path = if std::env::var("BLUEBERRY_NO_HISTORY").as_deref() == Ok("1") {
        options.data_dir.join("probe-usage.json")
    } else {
        config::statistics_path()
    };
    let learning = Arc::new(Learning::new(usage_path));
    let integration = pty::ensure_integration(&options.data_dir)?;
    let session_directory = options
        .data_dir
        .join(format!("session-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&session_directory)?;
    let _files = SessionFiles(session_directory.clone());
    let edit_path = session_directory.join("edit.json");
    let request_path = session_directory.join("request.json");
    let token = uuid::Uuid::new_v4().to_string();
    let mut pipe = if matches!(options.transport, Transport::Pipe) {
        crate::pipe::PipeServer::new().ok()
    } else {
        None
    };
    // Explicit test-only transport tap. Release builds cannot emit editing
    // payloads to an outer terminal, even if this environment variable is set.
    #[cfg(debug_assertions)]
    let probe_token = (std::env::var("BLUEBERRY_NO_HISTORY").as_deref() == Ok("1"))
        .then(|| std::env::var("BLUEBERRY_PROBE_TOKEN").ok())
        .flatten()
        .filter(|token| {
            !token.is_empty()
                && token
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        });
    let mut env = BTreeMap::from([
        ("BLUEBERRY_TOKEN".into(), token.clone()),
        (
            "BLUEBERRY_PIPE_NAME".into(),
            pipe.as_ref()
                .map(|p| p.name().to_owned())
                .unwrap_or_default(),
        ),
        (
            "BLUEBERRY_EDIT_PATH".into(),
            edit_path.to_string_lossy().into(),
        ),
        ("BLUEBERRY_ACTIVE".into(), "1".into()),
        (
            "BLUEBERRY_SESSION_DIR".into(),
            session_directory.to_string_lossy().into(),
        ),
        (
            "BLUEBERRY_REQUEST_PATH".into(),
            request_path.to_string_lossy().into(),
        ),
        ("BLUEBERRY_KEY_PREFIX".into(), protocol_prefix.clone()),
        (
            "BLUEBERRY_PUBLIC_KEYS".into(),
            serde_json::to_string(&config.keys)?,
        ),
        ("ISTERM".into(), "1".into()),
        ("TERM".into(), "xterm-256color".into()),
    ]);
    env.insert(
        "BLUEBERRY_TRACE".into(),
        if trace.enabled() { "1" } else { "0" }.into(),
    );
    let cwd = std::env::current_dir()?;
    env.extend(pty::key_environment(&config.keys));
    let (cols, rows) = terminal::size().unwrap_or((120, 30));
    let mut raw = RawMode::enable()
        .context("Blueberry run requires an interactive terminal. Use 'complete' for scripts.")?;
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
    trace.event("pty_started", None, None, None);
    if let Some(server) = pipe.as_mut() {
        if let Some(pid) = child.process_id() {
            server.set_client_pid(pid);
        } else {
            server.disable();
            pipe = None;
        }
    }
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
        #[cfg(windows)]
        let mut input_reader = match crate::windows_input::Reader::new() {
            Ok(reader) => reader,
            Err(error) => {
                let _ = input_tx.send(HostEvent::Error(error.to_string()));
                return;
            }
        };
        loop {
            #[cfg(windows)]
            let result = input_reader.read_batch(64);
            #[cfg(windows)]
            if input_reader.take_paste_rejection().is_some() {
                let _ = input_tx.send(HostEvent::Diagnostic(
                    "粘贴内容超过单次 1 MiB 上限，已完整丢弃；请分段粘贴".into(),
                ));
            }
            #[cfg(not(windows))]
            let result = event::read().map(|event| {
                let mut events = vec![event];
                while events.len() < 64 && event::poll(Duration::ZERO).unwrap_or(false) {
                    match event::read() {
                        Ok(event) => events.push(event),
                        Err(_) => break,
                    }
                }
                events
            });
            match result {
                Ok(events) => {
                    if input_tx.send(HostEvent::Input(events)).is_err() {
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
    let reload_tx = tx.clone();
    let monitor = crate::reload::Monitor::new(&config, options.config_path.as_deref(), move || {
        let _ = reload_tx.try_send(HostEvent::Reload);
    });
    let worker = Worker::new(
        options.data_dir.join("commands.json"),
        config::specs_dir(&config, options.config_path.as_deref()),
        session_directory.join("data-sources.json"),
        tx,
        trace.clone(),
    );
    let mut state = State {
        pipe,
        descriptions,
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
        selection_touched: false,
        dismissed: false,
        indexed: false,
        environment: None,
        diagnostic: None,
        nested_edit: false,
        bell: false,
        adapter_diagnostic_shown: false,
        commands_snapshot: CommandSnapshot::default(),
        commands_pending: false,
        commands_inflight: false,
        commands_allowed: false,
        trace: trace.clone(),
        query_started: None,
        request_path,
        protocol_prefix,
        context: None,
        shell_environment: Arc::new(std::env::vars().collect()),
        learning,
        pending_accept: None,
        native_request: None,
        native_ready: false,
        metadata_ready: false,
        metadata_active: false,
        metadata_pending: None,
        metadata_seen: Default::default(),
        paste_ready: false,
        paste_sequence: 0,
        native_menu: false,
        menu_focus: false,
        details: false,
        detail_page: 0,
        searching: false,
        enter_ready: false,
        shift_enter_ready: false,
        native_queued: false,
        monitor,
        notification: None,
        public_keys: BTreeMap::new(),
        reset_pending: None,
        status_writer: crate::status::StatusWriter::new(session_directory.join("adapter.json")),
        hub_query: None,
        hub_history: Vec::new(),
        hub_history_path: None,
        history_ready: false,
        history_pending: false,
        hub_form: None,
    };
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    let mut exit_code = None;
    let mut frame = Vec::with_capacity(32_768);
    loop {
        frame.clear();
        let mut ui_dirty = false;
        let mut eof = false;
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
        let mut batch = vec![event];
        batch.extend(rx.try_iter().take(63));
        for event in batch {
            match event {
                HostEvent::Output(bytes) => {
                    for part in state.decoder.feed(&bytes) {
                        match part {
                            Part::Data(data) => {
                                state.overlay.erase(state.parser.screen(), &mut frame)?;
                                state.parser.process(&data);
                                frame.extend_from_slice(&data);
                                ui_dirty = true;
                            }
                            Part::Message(mut value) => {
                                if value["event"] == "pipe" {
                                    let envelope =
                                        state.pipe.as_mut().and_then(|p| p.read_json().ok());
                                    match envelope {
                                        Some(envelope)
                                            if envelope["sequence"] == value["sequence"]
                                                && envelope["payload"].is_object() =>
                                        {
                                            value = envelope["payload"].clone()
                                        }
                                        _ => {
                                            if let Some(mut server) = state.pipe.take() {
                                                server.disable();
                                            }
                                            state.pending_query = None;
                                            state.native_request = None;
                                            state.dirty = state.prompt;
                                            state.diagnostic = Some("管道帧校验失败，已回退 OSC；请重新触发原生补全或插入".into());
                                            continue;
                                        }
                                    }
                                }
                                #[cfg(debug_assertions)]
                                if let Some(token) = &probe_token {
                                    frame.extend_from_slice(
                                        format!("\x1b]7776;{token};{value}\x07").as_bytes(),
                                    );
                                }
                                let before = (state.revision, state.prompt, state.dismissed);
                                let changes_menu = value["event"] == "native_completion";
                                state.message(value, &mut writer, &worker)?;
                                ui_dirty |= changes_menu
                                    || before != (state.revision, state.prompt, state.dismissed);
                            }
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
                HostEvent::Input(inputs) => {
                    // One native read can contain an entire paste marker or
                    // several key records. Preserve their order in one write
                    // to the child instead of flushing after every key. Query
                    // injection stays after the outer event batch so queued
                    // protocol replies are handled before a new request.
                    let mut input_frame = Vec::with_capacity(inputs.len() * 8);
                    for input in inputs {
                        ui_dirty |= matches!(&input, Event::Resize(..) | Event::Paste(_))
                            || matches!(&input, Event::Key(key) if key.kind != crossterm::event::KeyEventKind::Release);
                        // Restore the old coordinates before changing the model size.
                        if matches!(&input, Event::Resize(..)) {
                            state.overlay.erase(state.parser.screen(), &mut frame)?;
                        }
                        state.input(input, &mut input_frame, master.as_ref(), &worker)?;
                    }
                    if !input_frame.is_empty() {
                        writer.write_all(&input_frame)?;
                        writer.flush()?;
                    }
                }
                HostEvent::Completion(revision, mut completion)
                    if revision == state.revision
                        && state.prompt
                        && !state.dismissed
                        && !state.native_menu =>
                {
                    if state.hub_query.is_none()&&state.cursor==state.line.len()&&!state.line.contains(['\n','\r']) {
                        let suggestions=crate::hub::suggestions(&state.config,&state.line,&state.hub_history);
                        let mut seen=completion.candidates.iter().map(|candidate|candidate.insert_text.to_lowercase()).collect::<std::collections::HashSet<_>>();
                        completion.candidates.extend(suggestions.into_iter().filter(|candidate|seen.insert(candidate.insert_text.to_lowercase())));
                    }
                    ui_dirty |= state.completion != completion;
                    let selected_label = state
                        .completion
                        .candidates
                        .get(state.selected)
                        .map(|candidate| candidate.identity().to_owned());
                    state.selected = selected_label
                        .and_then(|label| {
                            completion
                                .candidates
                                .iter()
                                .position(|candidate| candidate.identity() == label)
                        })
                        .unwrap_or(0);
                    state.completion = completion;
                }
                HostEvent::Completion(revision, _) => {
                    trace.event("completion_discarded", Some(revision), None, None)
                }
                HostEvent::Exit(code) => exit_code = Some(code),
                HostEvent::Diagnostic(message) => state.diagnostic = Some(message),
                HostEvent::Reload => {}
                HostEvent::Eof => eof = true,
                HostEvent::Error(error) => {
                    if exit_code.is_none() {
                        return Err(anyhow::anyhow!(error));
                    }
                }
            }
        }
        if state.monitor.take_changed() {
            state.reload(&worker);
            ui_dirty = true;
        }
        if state.prompt {
            // A child can exit without resetting its modes. Do not resurrect
            // its old capture request on resize or the next command's execution.
            state.parser.process(MOUSE_OFF);
        }
        state.query(&mut writer)?;
        if std::mem::take(&mut state.repaint) {
            frame.extend_from_slice(b"\x1b[2J\x1b[H");
            frame.extend_from_slice(&state.parser.screen().state_formatted());
            ui_dirty = true;
        }
        if std::mem::take(&mut state.bell) {
            frame.push(7);
        }
        if let Some(message) = state.diagnostic.take() {
            state.notification = Some(message);
            ui_dirty = true;
        }
        let repaint_start = Instant::now();
        let bytes_before = frame.len();
        if ui_dirty && state.prompt && !state.dismissed {
            let notice = state
                .notification
                .as_ref()
                .map(|message| Candidate {
                    label: "Blueberry".into(),
                    description: message.clone(),
                    ..Default::default()
                })
                .or_else(|| {
                    if !state.completion.argument_hint.is_empty() {
                        Some(Candidate {
                            label: state.completion.argument_hint.clone(),
                            description: "请输入参数值".into(),
                            ..Default::default()
                        })
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    state.completion.incomplete.then(|| Candidate {
                        label: "正在加载候选…".into(),
                        ..Default::default()
                    })
                });
            let displayed = if state.completion.candidates.is_empty() {
                notice.as_slice()
            } else {
                &state.completion.candidates
            };
            let query = state.hub_query.as_deref().unwrap_or_else(||state.line.get(state.completion.replace_start..state.cursor).unwrap_or(""));
            state.overlay.draw_with_state(
                state.parser.screen(),
                &mut frame,
                displayed,
                state.selected,
                query,
                &state.config,
                crate::menu::MenuState {
                    incomplete: state.completion.incomplete,
                    details: state.details,
                    detail_page: state.detail_page,
                    argument_hint: (!state.completion.candidates.is_empty())
                        .then_some(state.completion.argument_hint.as_str()),
                    searching: state.searching,
                    diagnostic: state.notification.as_deref().or_else(|| {
                        state
                            .completion
                            .candidates
                            .is_empty()
                            .then_some("提示 · 请继续输入")
                    }),
                },
            )?;
        } else if ui_dirty {
            state.overlay.erase(state.parser.screen(), &mut frame)?;
        }
        if frame.len() > bytes_before {
            trace.event(
                "redraw",
                Some(state.revision),
                Some(repaint_start.elapsed()),
                Some(frame.len() - bytes_before),
            );
        }
        if ui_dirty {
            // Keep the outer terminal in paste-aware mode while editing,
            // even when this PSReadLine version does not advertise it. For
            // external applications retain their own requested mode.
            frame.extend_from_slice(if state.prompt || state.parser.screen().bracketed_paste() {
                b"\x1b[?2004h"
            } else {
                b"\x1b[?2004l"
            });
        }
        // Ordinary editing belongs to the terminal (selection/right-click paste).
        // Only applications requesting mouse reports may capture the outer mouse.
        raw.mouse(
            !state.prompt
                && state.parser.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None,
        )?;
        if state.prompt && !frame.is_empty() {
            frame.extend_from_slice(MOUSE_OFF);
        }
        if !frame.is_empty() {
            let started = Instant::now();
            output.write_all(&frame)?;
            output.flush()?;
            trace.event(
                "output",
                Some(state.revision),
                Some(started.elapsed()),
                Some(frame.len()),
            );
        }
        if eof {
            break;
        }
    }
    frame.clear();
    state.overlay.erase(state.parser.screen(), &mut frame)?;
    frame.extend_from_slice(&state.decoder.finish());
    output.write_all(&frame)?;
    output.flush()?;
    trace.event("host_exit", None, None, None);
    trace.flush();
    Ok(exit_code.unwrap_or(0))
}

fn request_number(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn decode_context(line: &str, value: &Value) -> Option<InputContext> {
    if !value.is_object() {
        return None;
    }
    let mut context: InputContext = serde_json::from_value(value.clone()).ok()?;
    match (
        protocol::utf16_to_byte(line, context.replace_start),
        protocol::utf16_to_byte(line, context.replace_end),
    ) {
        (Some(start), Some(end)) if start <= end => {
            context.replace_start = start;
            context.replace_end = end;
        }
        _ => {
            context.suppressed = true;
            context.replace_start = 0;
            context.replace_end = 0;
        }
    }
    Some(context)
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn batches_keep_previous_commands_until_complete_and_do_not_truncate() {
        let mut snapshot = CommandSnapshot::default();
        snapshot
            .receive(&json!({"commands":[{"name":"obsolete","kind":"function"}]}))
            .unwrap();
        for batch in 0..6 {
            let commands: Vec<_> = (0..128)
                .map(|n| json!({"name":format!("f{}",batch*128+n),"kind":"function"}))
                .collect();
            let (merged, complete) = snapshot
                .receive(&json!({"snapshot":"first","complete":false,"commands":commands}))
                .unwrap();
            assert!(!complete);
            assert!(merged.iter().any(|c| c.name == "obsolete"));
        }
        let (merged, complete) = snapshot
            .receive(&json!({"snapshot":"first","complete":true,"commands":[]}))
            .unwrap();
        assert!(complete);
        assert_eq!(merged.len(), 768);
        assert!(!merged.iter().any(|c| c.name == "obsolete"));
        let (merged,complete) = snapshot.receive(&json!({"snapshot":"next","complete":true,"commands":[{"name":"new","kind":"alias","definition":"git"}]})).unwrap();
        assert!(complete);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name, "new");
    }
}

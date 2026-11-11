//! Shared completion worker and its synchronized mailbox.
use super::*;

type EntryKey = (String, PathBuf, PathBuf, u64);

#[derive(Default)]
pub(super) struct Work {
    pub(super) started: bool,
    pub(super) refresh: bool,
    pub(super) stop: bool,
    pub(super) commands: Option<Vec<ShellCommand>>,
    pub(super) system_commands: Option<Vec<ShellCommand>>,
    pub(super) query: Option<Query>,
    pub(super) environment: Option<(String, String)>,
    pub(super) source_updates: Vec<crate::sources::SourceUpdate>,
    pub(super) cancel: bool,
    pub(super) invalidate_sources: bool,
    pub(super) help_updates: Vec<crate::knowledge::Record>,
    pub(super) entry_updates: Vec<(
        u64,
        EntryKey,
        Option<crate::knowledge::Entry>,
        std::time::Duration,
    )>,
    pub(super) catalog_path: Option<PathBuf>,
    pub(super) catalog_snapshot: Option<Arc<crate::spec_catalog::Catalog>>,
}

pub(super) struct Worker(pub(super) Arc<(Mutex<Work>, Condvar)>);

impl Worker {
    pub(super) fn new(
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
                EntryKey,
                (Instant, Option<crate::knowledge::Entry>),
            >::new();
            let mut entry_pending = std::collections::HashSet::<EntryKey>::new();
            let mut entry_generation = 0u64;
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
            let mut last_batch_end: Option<Instant> = None;
            let mut index_diagnostics = Vec::new();
            let mut environment: Option<(String, String)> = None;
            let mut shell_commands = Vec::new();
            let mut system_commands = Vec::new();
            let mut latest_query: Option<Query> = None;
            let mut prepared: Option<(u64, Completion, crate::sources::SourceRequest)> = None;
            let mut published = crate::completion::PublishedCompletion::default();
            loop {
                let (lock, condition) = &*thread_shared;
                let mut work = lock.lock().unwrap();
                while !work.stop
                    && (!work.started
                        || (!work.refresh
                            && work.commands.is_none()
                            && work.system_commands.is_none()
                            && work.query.is_none()
                            && work.environment.is_none()
                            && work.source_updates.is_empty()
                            && work.help_updates.is_empty()
                            && work.entry_updates.is_empty()
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
                let mut refresh = std::mem::take(&mut work.refresh);
                let cancel = std::mem::take(&mut work.cancel);
                let invalidate_sources = std::mem::take(&mut work.invalidate_sources);
                let updates = std::mem::take(&mut work.source_updates);
                let help_updates = std::mem::take(&mut work.help_updates);
                let help_changed = !help_updates.is_empty();
                let entry_updates = std::mem::take(&mut work.entry_updates);
                let entry_changed = !entry_updates.is_empty();
                let reload_catalog = work.catalog_path.take();
                let commands = work.commands.take();
                let declarations = work.system_commands.take();
                let commands_changed = commands.is_some() || declarations.is_some();
                if let Some(commands) = declarations {
                    system_commands = commands;
                }
                let query = work.query.take();
                let query_changed = query.is_some();
                if let Some(query) = query.as_ref() {
                    trace.event(
                        "worker_queue",
                        Some(query.revision),
                        Some(query.queued_at.elapsed()),
                        None,
                    );
                }
                let mut environment_changed = false;
                if let Some(env) = work.environment.take() {
                    if environment.as_ref() != Some(&env)
                        && (index.is_some() || discovery.is_some())
                    {
                        environment_changed = true;
                        refresh = true;
                        discovery = None;
                        index = None;
                        published.clear();
                        prepared = None;
                        sources.cancel();
                        request = None;
                        parts = [None, None];
                    }
                    environment = Some(env);
                }
                drop(work);
                if environment_changed {
                    cache_writer.invalidate();
                }
                if cancel {
                    published.clear();
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
                    entry_pending.clear();
                    entry_generation = entry_generation.wrapping_add(1);
                }
                for (generation, key, entry, elapsed) in entry_updates {
                    if generation != entry_generation {
                        continue;
                    }
                    trace.event(
                        "entry_resolve",
                        None,
                        Some(elapsed),
                        Some(usize::from(entry.is_some())),
                    );
                    entry_pending.remove(&key);
                    if entry_cache.len() >= 256 {
                        entry_cache.clear();
                    }
                    entry_cache.insert(key, (Instant::now(), entry));
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
                    last_batch_end = None;
                    trace.event(
                        "index_begin",
                        None,
                        None,
                        Some(usize::from(environment.is_some())),
                    );
                    index_diagnostics.clear();
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
                        if refresh {
                            "cache_bypassed_refresh"
                        } else if cached.is_some() {
                            "cache_hit"
                        } else {
                            "cache_miss"
                        },
                        None,
                        Some(started.elapsed()),
                        None,
                    );
                    if let Some(cached) = cached {
                        trace.event("index_complete", None, None, Some(cached.len()));
                        index = Some(cached);
                    } else {
                        let preparing = trace.enabled().then(Instant::now);
                        discovery = Some(Discovery::new(OsStr::new(&path), OsStr::new(&pathext)));
                        if let Some(start) = preparing {
                            trace.event("index_prepare", None, Some(start.elapsed()), None);
                        }
                    }
                    changed = true;
                }
                let mut finished = false;
                if let Some(scan) = discovery.as_mut() {
                    let started = Instant::now();
                    if let Some(end) = last_batch_end {
                        trace.event("index_between_batches", None, Some(started - end), None);
                    }
                    finished = scan.step();
                    let scanned = trace.enabled().then(Instant::now);
                    if finished {
                        index_diagnostics = scan.diagnostics().to_vec();
                    }
                    let snapshot = scan.snapshot();
                    if let Some(scanned) = scanned {
                        let end = Instant::now();
                        trace.event("index_scan", None, Some(scanned - started), None);
                        trace.event(
                            "index_snapshot",
                            None,
                            Some(end - scanned),
                            Some(snapshot.len()),
                        );
                        last_batch_end = Some(end);
                        if finished {
                            trace.event(
                                if snapshot.is_complete() {
                                    "index_complete"
                                } else {
                                    "index_exhausted_with_errors"
                                },
                                None,
                                None,
                                Some(snapshot.len()),
                            );
                        }
                    }
                    index = Some(snapshot);
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
                    index.replace_system_commands(system_commands.clone());
                    index.replace_shell_commands(shell_commands.clone());
                }
                if let Some(q) = latest_query.as_ref().filter(|_| {
                    changed
                        || help_changed
                        || entry_changed
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
                    let mut env_hasher = std::collections::hash_map::DefaultHasher::new();
                    q.environment.hash(&mut env_hasher);
                    let resolved_path = index.executable(command);
                    let entry_key = resolved_path.as_ref().map(|path| {
                        (
                            command.to_owned(),
                            q.cwd.clone(),
                            path.clone(),
                            env_hasher.finish(),
                        )
                    });
                    if q.help_enabled
                        && let (Some(path), Some(entry_key)) = (resolved_path, entry_key.as_ref())
                        && entry_cache.get(entry_key).is_none_or(|(time, _)| {
                            time.elapsed() > std::time::Duration::from_secs(1)
                        })
                        && entry_pending.len() < 8
                        && entry_pending.insert(entry_key.clone())
                    {
                        let shared = thread_shared.clone();
                        let entry_key = entry_key.clone();
                        let command = command.to_owned();
                        let cwd = q.cwd.clone();
                        let environment = q.environment.clone();
                        let generation = entry_generation;
                        thread::spawn(move || {
                            let started = Instant::now();
                            let entry = crate::knowledge::entry_with_environment(
                                &command,
                                &path,
                                &cwd,
                                &environment,
                            );
                            let (lock, wake) = &*shared;
                            lock.lock().unwrap().entry_updates.push((
                                generation,
                                entry_key,
                                entry,
                                started.elapsed(),
                            ));
                            wake.notify_one();
                        });
                    }
                    let executable = entry_key
                        .as_ref()
                        .and_then(|key| entry_cache.get(key))
                        .and_then(|(_, entry)| entry.clone());
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
                            for action in crate::providers::project_actions(&q.cwd, &q.line) {
                                if result.candidates.len() >= q.limit {
                                    break;
                                }
                                let command = match action.invocation.words.first() {
                                    Some(crate::invocation::Word::Syntax(command)) => {
                                        command.as_str()
                                    }
                                    _ => "",
                                };
                                let rendered =
                                    action.invocation.render(crate::invocation::shell_mode(
                                        command,
                                        &q.environment,
                                        &shell_commands,
                                        &q.cwd,
                                    ));
                                result.candidates.push(Candidate {
                                    label: rendered.clone(),
                                    insert_text: rendered,
                                    description: action.candidate.description,
                                    kind: action.candidate.kind,
                                    id: action.id,
                                    source: action.candidate.source,
                                    match_reason: "当前项目".into(),
                                    append_space: true,
                                    ..Default::default()
                                });
                            }
                        }
                        if thread_shared
                            .0
                            .lock()
                            .unwrap()
                            .environment
                            .as_ref()
                            .is_some_and(|next| Some(next) != environment.as_ref())
                        {
                            continue;
                        }
                        result.pending |= discovery.is_some() && context.command_position;
                        let _ =
                            output.send(HostEvent::Completion(q.revision, result, Instant::now()));
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
                        trace.event(
                            "completion_plan",
                            Some(q.revision),
                            Some(started.elapsed()),
                            Some(result.0.candidates.len()),
                        );
                        prepared = Some((q.revision, result.0.clone(), result.1.clone()));
                        result
                    };
                    let force = invalidate_sources || refresh || reload_catalog.is_some();
                    if request.as_ref() != Some(&next) || force {
                        trace.event(
                            "source_submit",
                            Some(q.revision),
                            None,
                            Some(
                                usize::from(next.paths)
                                    | (usize::from(next.project) << 1)
                                    | (usize::from(force) << 2)
                                    | (usize::from(help_changed) << 3)
                                    | (usize::from(entry_changed) << 4),
                            ),
                        );
                        parts = [None, None];
                        sources.submit(next.clone(), force);
                        request = Some(next);
                    }
                    let received_source_update = !updates.is_empty();
                    for update in updates {
                        if update.revision != q.revision
                            || update.generation != sources.generation()
                        {
                            continue;
                        }
                        let slot = usize::from(update.kind == crate::sources::SourceKind::Paths);
                        trace.event(
                            "dynamic_ready",
                            Some(q.revision),
                            Some(update.elapsed),
                            Some(update.candidates.len()),
                        );
                        parts[slot] = Some(update);
                    }
                    if received_source_update && parts.iter().all(Option::is_some) {
                        let root = parts
                            .iter()
                            .filter_map(|part| part.as_ref()?.project_root.as_deref())
                            .next();
                        let watch_paths = parts
                            .iter()
                            .filter_map(Option::as_ref)
                            .flat_map(|part| part.watch_paths.iter().cloned())
                            .collect::<Vec<_>>();
                        let recursive_watch_paths = parts
                            .iter()
                            .filter_map(Option::as_ref)
                            .flat_map(|part| part.recursive_watch_paths.iter().cloned())
                            .collect::<Vec<_>>();
                        sources.set_provider_watches(root, &watch_paths, &recursive_watch_paths);
                    }
                    // Report the most recently returned provider snapshots.
                    // Unchanged hot queries do not cause another disk write.
                    let provider_state = json!({
                        "command_index": { "complete":index.is_complete(), "pending":discovery.is_some(), "diagnostics":index_diagnostics },
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
                    let merging = Instant::now();
                    let mut result = crate::completion::merge(
                        base,
                        request.as_ref().unwrap(),
                        [parts[0].as_ref(), parts[1].as_ref()],
                        q.usage.as_deref(),
                        q.limit,
                        &q.descriptions,
                    );
                    result.pending |= discovery.is_some() && context.command_position;
                    if discovery.is_none() && !index.is_complete() && context.command_position {
                        result
                            .error
                            .get_or_insert_with(|| "部分命令目录读取失败；可使用原生补全".into());
                    }
                    let result = published.update(request.as_ref().unwrap(), result);
                    trace.event(
                        "completion_merge",
                        Some(q.revision),
                        Some(merging.elapsed()),
                        Some(result.candidates.len()),
                    );
                    trace.event(
                        "completion_incomplete",
                        Some(q.revision),
                        None,
                        Some(usize::from(result.incomplete)),
                    );
                    trace.event(
                        "completion",
                        Some(q.revision),
                        Some(started.elapsed()),
                        Some(result.candidates.len()),
                    );
                    // A newer authoritative environment can arrive during disk I/O.
                    if thread_shared
                        .0
                        .lock()
                        .unwrap()
                        .environment
                        .as_ref()
                        .is_some_and(|next| Some(next) != environment.as_ref())
                    {
                        continue;
                    }
                    if output
                        .send(HostEvent::Completion(q.revision, result, Instant::now()))
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
    pub(super) fn update(&self, change: impl FnOnce(&mut Work)) {
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

#[cfg(test)]
mod environment_tests {
    use super::*;
    fn query(root: &std::path::Path, revision: u64) -> Query {
        Query {
            revision,
            line: "custom".into(),
            cursor: 6,
            cwd: root.into(),
            limit: 20,
            descriptions: Arc::new(BTreeMap::new()),
            context: None,
            fuzzy: false,
            usage: None,
            environment: Arc::new(BTreeMap::new()),
            dynamic: false,
            searching: false,
            help_enabled: false,
            queued_at: Instant::now(),
        }
    }
    #[test]
    fn environment_updates_wake_idle_worker_and_replace_commands() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let b = root.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        std::fs::write(a.join("custom-old.exe"), b"").unwrap();
        std::fs::write(b.join("custom-new.cmd"), b"").unwrap();
        let (tx, rx) = mpsc::sync_channel(256);
        let worker = Worker::new(
            root.path().join("commands.json"),
            root.path().join("specs"),
            root.path().join("sources.json"),
            tx,
            Trace::default(),
        );
        for (revision, path, ext, wanted, unwanted) in [
            (1, &a, ".EXE", "custom-old", "custom-new"),
            (2, &b, ".CMD", "custom-new", "custom-old"),
            (3, &a, ".CMD", "", "custom-old"),
            (4, &a, "", "", "custom-old"),
        ] {
            worker.update(|w| {
                w.started = true;
                w.environment = Some((path.to_string_lossy().into_owned(), ext.into()));
                w.query = Some(query(root.path(), revision));
            });
            loop {
                if let HostEvent::Completion(r, completion, _) =
                    rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap()
                {
                    if r != revision || completion.pending {
                        continue;
                    }
                    assert!(!completion.candidates.iter().any(|c| c.label == unwanted));
                    if !wanted.is_empty() {
                        assert!(completion.candidates.iter().any(|c| c.label == wanted));
                    }
                    break;
                }
            }
        }
    }
}

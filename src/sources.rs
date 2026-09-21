//! Two latest-request workers for project and directory snapshots.
use crate::{
    engine,
    model::{Candidate, InputContext},
    paths::PathCache,
    providers::{self, ProjectQuery, ProviderCache},
    ranking,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    thread,
};

#[derive(Clone, PartialEq, Eq)]
pub struct SourceRequest {
    pub revision: u64,
    pub line: String,
    pub cursor: usize,
    pub context: InputContext,
    pub cwd: PathBuf,
    pub environment: Arc<BTreeMap<String, String>>,
    pub paths: bool,
    pub directories_only: bool,
    pub project: bool,
    pub provider: Option<String>,
    pub fuzzy: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Project,
    Paths,
    Invalidated,
}
pub struct SourceUpdate {
    pub revision: u64,
    pub kind: SourceKind,
    pub candidates: Vec<Candidate>,
    pub incomplete: bool,
    pub scan_pending: bool,
    pub diagnostics: Vec<String>,
    pub project_root: Option<PathBuf>,
    pub elapsed: std::time::Duration,
    pub generation: u64,
    pub watch_paths: Vec<PathBuf>,
    pub recursive_watch_paths: Vec<PathBuf>,
}
impl SourceUpdate {
    fn empty(revision: u64, kind: SourceKind) -> Self {
        Self {
            revision,
            kind,
            candidates: Vec::new(),
            incomplete: false,
            scan_pending: false,
            diagnostics: Vec::new(),
            project_root: None,
            elapsed: std::time::Duration::ZERO,
            generation: 0,
            watch_paths: Vec::new(),
            recursive_watch_paths: Vec::new(),
        }
    }
}
#[derive(Default)]
struct Queue {
    pending: Option<(SourceRequest, Arc<AtomicBool>, u64)>,
    running: Option<Arc<AtomicBool>>,
    stop: bool,
}
type Mailbox = Arc<(Mutex<Queue>, Condvar)>;
type Callback = Arc<dyn Fn(SourceUpdate) + Send + Sync>;
pub struct Sources {
    lanes: [Mailbox; 2],
    epochs: [Arc<AtomicU64>; 2],
    invalidated_domains: Arc<AtomicU8>,
    last: Option<SourceRequest>,
    _watcher: Option<RecommendedWatcher>,
    watched_cwd: Option<PathBuf>,
    /// Paths currently registered with notify and whether each registration
    /// is recursive. Keeping the mode lets a later provider upgrade a
    /// non-recursive watch (for example the request cwd) without being
    /// incorrectly skipped as an already-watched path.
    watch_modes: HashMap<PathBuf, bool>,
    watch_domains: Arc<Mutex<HashMap<PathBuf, u8>>>,
    generation: u64,
}

impl Sources {
    pub fn new(callback: impl Fn(SourceUpdate) + Send + Sync + 'static) -> Self {
        let callback: Callback = Arc::new(callback);
        let epochs: [Arc<AtomicU64>; 2] = std::array::from_fn(|_| Arc::new(AtomicU64::new(0)));
        let lanes =
            std::array::from_fn(|_| Arc::new((Mutex::new(Queue::default()), Condvar::new())));
        for (index, lane) in lanes.iter().enumerate() {
            let lane = lane.clone();
            let callback = callback.clone();
            let epoch = epochs[index].clone();
            thread::spawn(move || run_lane(index, lane, epoch, callback));
        }
        let invalidated_domains = Arc::new(AtomicU8::new(0));
        let invalidated = invalidated_domains.clone();
        let watch_domains = Arc::new(Mutex::new(HashMap::<PathBuf, u8>::new()));
        let watched = watch_domains.clone();
        let notify = callback.clone();
        let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            let mut domain_mask = 0u8;
            if let Ok(event) = result {
                if event.kind.is_access() {
                    return;
                }
                if !event.paths.is_empty()
                    && event.paths.iter().all(|path| {
                        path.components().any(|part| {
                            matches!(
                                part.as_os_str().to_str(),
                                Some("target" | "node_modules" | ".blueberry")
                            )
                        })
                    })
                {
                    return;
                }
                if let Ok(domains) = watched.lock() {
                    for changed in &event.paths {
                        for (root, domain) in domains.iter() {
                            if changed.starts_with(root) || root.starts_with(changed) {
                                domain_mask |= *domain;
                            }
                        }
                    }
                }
            }
            // Unknown watcher errors and events outside the current map must
            // conservatively invalidate both lanes.
            invalidated.fetch_or(
                if domain_mask == 0 { 0b11 } else { domain_mask },
                Ordering::Relaxed,
            );
            notify(SourceUpdate::empty(0, SourceKind::Invalidated));
        })
        .ok();
        Self {
            lanes,
            epochs,
            invalidated_domains,
            last: None,
            _watcher: watcher,
            watched_cwd: None,
            watch_modes: HashMap::new(),
            watch_domains,
            generation: 0,
        }
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn submit(&mut self, request: SourceRequest, force: bool) {
        if !force && self.last.as_ref() == Some(&request) {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        if force {
            let domains = self.invalidated_domains.swap(0, Ordering::Relaxed);
            for (index, epoch) in self.epochs.iter().enumerate() {
                if domains == 0 || domains & (1 << index) != 0 {
                    epoch.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        if self.watched_cwd.as_ref() != Some(&request.cwd) {
            self.clear_watches();
            // Watch current directory non-recursively. Project roots and
            // Git directories are added when a provider identifies them.
            if self.watch_path(&request.cwd, false) {
                self.watched_cwd = Some(request.cwd.clone());
                self.watch_domains
                    .lock()
                    .unwrap()
                    .insert(request.cwd.clone(), 0b11);
            }
        }
        for lane in &self.lanes {
            let (lock, wake) = &**lane;
            let mut queue = lock.lock().unwrap();
            if let Some(cancel) = queue.running.take() {
                cancel.store(true, Ordering::Relaxed);
            }
            if let Some((_, cancel, _)) = queue.pending.take() {
                cancel.store(true, Ordering::Relaxed);
            }
            queue.pending = Some((
                request.clone(),
                Arc::new(AtomicBool::new(false)),
                self.generation,
            ));
            wake.notify_one();
        }
        self.last = Some(request);
    }
    pub fn set_provider_watches(
        &mut self,
        root: Option<&std::path::Path>,
        paths: &[PathBuf],
        recursive_paths: &[PathBuf],
    ) {
        let mut desired = HashMap::<PathBuf, bool>::new();
        let mut domains = HashMap::<PathBuf, u8>::new();
        if let Some(cwd) = self.watched_cwd.as_ref() {
            desired.insert(cwd.clone(), false);
            domains.insert(cwd.clone(), 0b11);
        }
        // Manifests are in the root; Git metadata is watched separately.
        // Do not recursively subscribe to build outputs or dependencies.
        if let Some(root) = root {
            desired.entry(root.to_path_buf()).or_insert(false);
            domains.entry(root.to_path_buf()).or_insert(0b01);
            let git = root.join(".git");
            let git = if git.is_file() {
                std::fs::read_to_string(&git).ok().and_then(|s| {
                    s.trim()
                        .strip_prefix("gitdir:")
                        .map(|p| root.join(p.trim()))
                })
            } else {
                Some(git)
            };
            if let Some(git) = git.filter(|p| p.is_dir()) {
                desired.insert(git.clone(), true);
                domains.insert(git.clone(), 0b01);
                if let Ok(common) = std::fs::read_to_string(git.join("commondir")) {
                    let common = git.join(common.trim());
                    desired.insert(common.clone(), true);
                    domains.insert(common, 0b01);
                }
            }
        }
        for path in paths {
            desired.entry(path.clone()).or_insert(false);
            domains.entry(path.clone()).or_insert(0b01);
        }
        for path in recursive_paths {
            desired.insert(path.clone(), true);
            domains.insert(path.clone(), 0b01);
        }

        let stale = self
            .watch_modes
            .iter()
            .filter(|(path, recursive)| desired.get(*path) != Some(*recursive))
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        if let Some(watcher) = self._watcher.as_mut() {
            for path in &stale {
                let _ = watcher.unwatch(path);
            }
        }
        for path in stale {
            self.watch_modes.remove(&path);
        }
        for (path, recursive) in desired {
            self.watch_path(&path, recursive);
        }
        *self.watch_domains.lock().unwrap() = domains;
    }

    fn watch_path(&mut self, path: &std::path::Path, recursive: bool) -> bool {
        if path.as_os_str().is_empty() {
            return false;
        }
        if let Some(existing) = self.watch_modes.get(path).copied() {
            if existing || !recursive {
                return true;
            }
            // notify does not reliably replace a registration in place. Drop
            // the old non-recursive registration before upgrading it.
            if let Some(watcher) = self._watcher.as_mut() {
                let _ = watcher.unwatch(path);
            }
            self.watch_modes.remove(path);
        }
        let Some(watcher) = self._watcher.as_mut() else {
            return false;
        };
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        if watcher.watch(path, mode).is_ok() {
            self.watch_modes.insert(path.to_path_buf(), recursive);
            true
        } else {
            false
        }
    }

    fn clear_watches(&mut self) {
        if let Some(watcher) = self._watcher.as_mut() {
            for path in self.watch_modes.keys() {
                let _ = watcher.unwatch(path);
            }
        }
        self.watch_modes.clear();
        self.watch_domains.lock().unwrap().clear();
        self.watched_cwd = None;
    }

    pub fn cancel(&mut self) {
        self.last = None;
        for lane in &self.lanes {
            let mut queue = lane.0.lock().unwrap();
            if let Some(cancel) = queue.running.take() {
                cancel.store(true, Ordering::Relaxed);
            }
            if let Some((_, cancel, _)) = queue.pending.take() {
                cancel.store(true, Ordering::Relaxed);
            }
        }
    }
}
impl Drop for Sources {
    fn drop(&mut self) {
        self.cancel();
        for lane in &self.lanes {
            lane.0.lock().unwrap().stop = true;
            lane.1.notify_all();
        }
    }
}

fn run_lane(index: usize, lane: Mailbox, epoch: Arc<AtomicU64>, callback: Callback) {
    let mut paths = PathCache::default();
    let mut projects = ProviderCache::new();
    let mut observed_epoch = 0;
    loop {
        let (request, cancel, generation) = {
            let (lock, wake) = &*lane;
            let mut queue = lock.lock().unwrap();
            while !queue.stop && queue.pending.is_none() {
                queue = wake.wait(queue).unwrap();
            }
            if queue.stop {
                return;
            }
            let pair = queue.pending.take().unwrap();
            queue.running = Some(pair.1.clone());
            pair
        };
        loop {
            let current_epoch = epoch.load(Ordering::Relaxed);
            if observed_epoch != current_epoch {
                paths.invalidate();
                projects.clear();
                observed_epoch = current_epoch;
            }
            let mut update = collect_lane(index, &request, &cancel, &mut paths, &mut projects);
            let continue_scan = update.scan_pending;
            update.generation = generation;
            if !cancel.load(Ordering::Relaxed) {
                callback(update);
            }
            if index != 1 || !continue_scan || cancel.load(Ordering::Relaxed) {
                break;
            }
            thread::yield_now();
        }
    }
}

/// Synchronous diagnostics use exactly the same sources as the live host.
pub fn collect_once(request: &SourceRequest) -> [SourceUpdate; 2] {
    let cancel = AtomicBool::new(false);
    let mut paths = PathCache::default();
    let mut projects = ProviderCache::new();
    let project = collect_lane(0, request, &cancel, &mut paths, &mut projects);
    let path = loop {
        let update = collect_lane(1, request, &cancel, &mut paths, &mut projects);
        if !update.scan_pending {
            break update;
        }
    };
    [project, path]
}

fn collect_lane(
    index: usize,
    request: &SourceRequest,
    cancel: &AtomicBool,
    paths: &mut PathCache,
    projects: &mut ProviderCache,
) -> SourceUpdate {
    let started = std::time::Instant::now();
    let mut update = SourceUpdate::empty(
        request.revision,
        if index == 0 {
            SourceKind::Project
        } else {
            SourceKind::Paths
        },
    );
    if index == 0 && request.project {
        let query = ProjectQuery {
            revision: request.revision,
            command: request.context.command.clone(),
            args: request.context.arguments.clone(),
            prefix: request.context.prefix.clone(),
            cwd: request.cwd.clone(),
            environment: (*request.environment).clone(),
            provider: request.provider.clone(),
        };
        let result = if let Some(cached) = projects.get_project(&query) {
            cached
        } else {
            let result = providers::collect_snapshot(&query, cancel);
            if !cancel.load(Ordering::Relaxed) {
                projects.insert_project(&query, result.clone());
            }
            result
        };
        let leading = providers::query_value_prefix(&query);
        let attached = if query.prefix.starts_with('-') {
            query
                .prefix
                .find('=')
                .map(|i| query.prefix[..i + 1].to_owned())
                .unwrap_or_default()
        } else {
            String::new()
        };
        for candidate in result.candidates {
            let mut value = candidate.value;
            if !leading.is_empty() && !value.starts_with(&leading) {
                value = format!("{leading}{value}");
            }
            if !attached.is_empty() && !value.starts_with(&attached) {
                value = format!("{attached}{value}");
            }
            if ranking::match_class(&value, &request.context.prefix, request.fuzzy).is_none() {
                continue;
            }
            update.candidates.push(Candidate {
                label: value.clone(),
                insert_text: engine::format_value(
                    &value,
                    &request.line,
                    &request.context,
                    request.cursor,
                ),
                description: candidate.description,
                kind: candidate.kind,
                id: format!("{}:{value}", candidate.source),
                source: candidate.source,
                append_space: false,
                ..Default::default()
            });
        }
        update.incomplete = result.incomplete;
        update.diagnostics = result.diagnostics;
        update.project_root = result.project_root;
        update.watch_paths = result.watch_paths;
        update.recursive_watch_paths = result.recursive_watch_paths;
    } else if index == 1 && request.paths {
        let result = paths.complete(
            &request.line,
            request.cursor,
            &request.context,
            &request.cwd,
            &request.environment,
            request.directories_only,
            cancel,
        );
        update.candidates = result.candidates;
        update.incomplete = result.incomplete;
        update.scan_pending = result.scan_pending;
        update.diagnostics.extend(result.diagnostic);
    }
    update.elapsed = started.elapsed();
    update
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::mpsc::{self, Receiver},
        time::{Duration, Instant},
    };

    fn wait_for_invalidated(receiver: &Receiver<SourceUpdate>) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "notify did not invalidate sources");
            if receiver
                .recv_timeout(remaining)
                .is_ok_and(|update| update.kind == SourceKind::Invalidated)
            {
                return;
            }
        }
    }

    fn drain_invalidations(receiver: &Receiver<SourceUpdate>) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            while receiver.try_recv().is_ok() {}
            std::thread::sleep(Duration::from_millis(10));
        }
        while receiver.try_recv().is_ok() {}
    }

    #[test]
    fn latest_request_gets_complete_directory_snapshot_and_cancel_stops_followups() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..1_200 {
            std::fs::write(root.path().join(format!("file{index:04}")), "").unwrap();
        }
        let (send, recv) = std::sync::mpsc::channel();
        let mut sources = Sources::new(move |update| {
            let _ = send.send(update);
        });
        let mut request = SourceRequest {
            revision: 0,
            line: String::new(),
            cursor: 0,
            context: InputContext::default(),
            cwd: root.path().into(),
            environment: Arc::new(BTreeMap::new()),
            paths: true,
            directories_only: false,
            project: false,
            provider: None,
            fuzzy: false,
        };
        for revision in 1..=50 {
            request.revision = revision;
            sources.submit(request.clone(), false);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut complete = false;
        let mut partial = false;
        while let Ok(update) =
            recv.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        {
            if update.revision == 50 && update.kind == SourceKind::Paths {
                if update.scan_pending {
                    partial |= !update.candidates.is_empty() && update.candidates.len() < 1_200;
                } else {
                    assert_eq!(update.candidates.len(), 1_200);
                    assert!(!update.incomplete);
                    complete = true;
                    break;
                }
            }
        }
        assert!(
            partial,
            "large directory did not publish a partial snapshot"
        );
        assert!(complete);
        sources.cancel();
        std::fs::write(root.path().join("file9999"), "").unwrap();
        request.revision = 51;
        sources.submit(request, true);
        loop {
            let update = recv
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            if update.revision == 51 && update.kind == SourceKind::Paths {
                if update.scan_pending {
                    continue;
                }
                assert_eq!(update.candidates.len(), 1_201);
                break;
            }
        }
    }

    #[test]
    fn recursive_provider_watch_invalidates_nested_files_and_prunes_old_roots() {
        let base = tempfile::tempdir().unwrap();
        let first = base.path().join("first");
        let second = base.path().join("second");
        let first_nested = first.join("nested");
        fs::create_dir_all(&first_nested).unwrap();
        fs::create_dir_all(&second).unwrap();
        let (send, receiver) = mpsc::channel();
        let mut sources = Sources::new(move |update| {
            if update.kind == SourceKind::Invalidated {
                let _ = send.send(update);
            }
        });
        let request = SourceRequest {
            revision: 1,
            line: String::new(),
            cursor: 0,
            context: InputContext::default(),
            cwd: base.path().to_path_buf(),
            environment: Arc::new(BTreeMap::new()),
            paths: false,
            directories_only: false,
            project: false,
            provider: None,
            fuzzy: false,
        };
        sources.submit(request, false);
        sources.set_provider_watches(Some(&first), &[], &[]);
        // watch_project starts with a precise root watch. The recursive
        // provider request must upgrade that registration for nested status
        // paths rather than being skipped as a duplicate.
        sources.set_provider_watches(Some(&first), &[], std::slice::from_ref(&first));

        fs::write(first_nested.join("changed.txt"), "changed").unwrap();
        wait_for_invalidated(&receiver);
        drain_invalidations(&receiver);

        sources.set_provider_watches(Some(&second), &[], &[]);
        fs::write(first_nested.join("after-prune.txt"), "stale").unwrap();
        assert!(
            receiver.recv_timeout(Duration::from_millis(400)).is_err(),
            "an old provider root remained watched after moving worktrees"
        );

        fs::write(second.join("manifest.toml"), "new").unwrap();
        wait_for_invalidated(&receiver);
    }

    #[test]
    fn edit_revisions_keep_the_same_watch_registrations() {
        let base = tempfile::tempdir().unwrap();
        let project = base.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let mut sources = Sources::new(|_| {});
        let mut request = SourceRequest {
            revision: 1,
            line: "git".into(),
            cursor: 3,
            context: InputContext::default(),
            cwd: project.clone(),
            environment: Arc::new(BTreeMap::new()),
            paths: false,
            directories_only: false,
            project: false,
            provider: None,
            fuzzy: false,
        };
        sources.submit(request.clone(), false);
        sources.set_provider_watches(Some(&project), std::slice::from_ref(&project), &[]);
        let before = sources.watch_modes.clone();
        request.revision = 2;
        request.line = "git s".into();
        request.cursor = request.line.len();
        sources.submit(request, false);
        assert_eq!(sources.watch_modes, before);
    }
}

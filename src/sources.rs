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
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
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
    pub diagnostics: Vec<String>,
    pub project_root: Option<PathBuf>,
    pub elapsed: std::time::Duration,
    pub generation: u64,
    pub watch_paths: Vec<PathBuf>,
}
impl SourceUpdate {
    fn empty(revision: u64, kind: SourceKind) -> Self {
        Self {
            revision,
            kind,
            candidates: Vec::new(),
            incomplete: false,
            diagnostics: Vec::new(),
            project_root: None,
            elapsed: std::time::Duration::ZERO,
            generation: 0,
            watch_paths: Vec::new(),
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
    epoch: Arc<AtomicU64>,
    last: Option<SourceRequest>,
    _watcher: Option<RecommendedWatcher>,
    watched: Option<PathBuf>,
    project_watches: HashSet<PathBuf>,
    generation: u64,
}

impl Sources {
    pub fn new(callback: impl Fn(SourceUpdate) + Send + Sync + 'static) -> Self {
        let callback: Callback = Arc::new(callback);
        let epoch = Arc::new(AtomicU64::new(0));
        let lanes =
            std::array::from_fn(|_| Arc::new((Mutex::new(Queue::default()), Condvar::new())));
        for (index, lane) in lanes.iter().enumerate() {
            let lane = lane.clone();
            let callback = callback.clone();
            let epoch = epoch.clone();
            thread::spawn(move || run_lane(index, lane, epoch, callback));
        }
        let invalidated = epoch.clone();
        let notify = callback.clone();
        let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            if let Ok(event) = result {
                if event.kind.is_access() {
                    return;
                }
                if !event.paths.is_empty()
                    && event.paths.iter().all(|path| {
                        path.components().any(|part| {
                            matches!(
                                part.as_os_str().to_str(),
                                Some("target" | "node_modules" | ".shellsense")
                            )
                        })
                    })
                {
                    return;
                }
            }
            invalidated.fetch_add(1, Ordering::Relaxed);
            notify(SourceUpdate::empty(0, SourceKind::Invalidated));
        })
        .ok();
        Self {
            lanes,
            epoch,
            last: None,
            _watcher: watcher,
            watched: None,
            project_watches: HashSet::new(),
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
            self.epoch.fetch_add(1, Ordering::Relaxed);
        }
        if self.watched.as_ref() != Some(&request.cwd)
            && let Some(watcher) = self._watcher.as_mut()
        {
            if let Some(old) = self.watched.take() {
                let _ = watcher.unwatch(&old);
            }
            for old in self.project_watches.drain() {
                let _ = watcher.unwatch(&old);
            }
            // Watch current directory non-recursively. Project roots and
            // Git directories are added when a provider identifies them.
            if watcher
                .watch(&request.cwd, RecursiveMode::NonRecursive)
                .is_ok()
            {
                self.watched = Some(request.cwd.clone());
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
    pub fn watch_project(&mut self, root: &std::path::Path) {
        if let Some(watcher) = self._watcher.as_mut() {
            // Manifests are in the root; Git metadata is watched separately.
            // Do not recursively subscribe to build outputs or dependencies.
            if self.watched.as_deref() != Some(root)
                && !self.project_watches.contains(root)
                && watcher.watch(root, RecursiveMode::NonRecursive).is_ok()
            {
                self.project_watches.insert(root.into());
            }
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
                if !self.project_watches.contains(&git)
                    && watcher.watch(&git, RecursiveMode::Recursive).is_ok()
                {
                    self.project_watches.insert(git.clone());
                }
                if let Ok(common) = std::fs::read_to_string(git.join("commondir")) {
                    let common = git.join(common.trim());
                    if !self.project_watches.contains(&common)
                        && watcher.watch(&common, RecursiveMode::Recursive).is_ok()
                    {
                        self.project_watches.insert(common);
                    }
                }
            }
        }
    }
    pub fn watch_paths(&mut self, paths: &[PathBuf]) {
        let Some(watcher) = self._watcher.as_mut() else {
            return;
        };
        for path in paths {
            if self.watched.as_ref() == Some(path) || self.project_watches.contains(path) {
                continue;
            }
            if watcher.watch(path, RecursiveMode::NonRecursive).is_ok() {
                self.project_watches.insert(path.clone());
            }
        }
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
        let current_epoch = epoch.load(Ordering::Relaxed);
        if observed_epoch != current_epoch {
            paths.invalidate();
            projects.clear();
            observed_epoch = current_epoch;
        }
        let mut update = collect_lane(index, &request, &cancel, &mut paths, &mut projects);
        update.generation = generation;
        if !cancel.load(Ordering::Relaxed) {
            callback(update);
        }
    }
}

/// Synchronous diagnostics use exactly the same sources as the live host.
pub fn collect_once(request: &SourceRequest) -> [SourceUpdate; 2] {
    let cancel = AtomicBool::new(false);
    let mut paths = PathCache::default();
    let mut projects = ProviderCache::new();
    std::array::from_fn(|index| collect_lane(index, request, &cancel, &mut paths, &mut projects))
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
        update.diagnostics.extend(result.diagnostic);
    }
    update.elapsed = started.elapsed();
    update
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latest_request_gets_complete_directory_snapshot_and_cancel_stops_followups() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..300 {
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
        while let Ok(update) =
            recv.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        {
            if update.revision == 50 && update.kind == SourceKind::Paths {
                assert_eq!(update.candidates.len(), 300);
                assert!(!update.incomplete);
                complete = true;
                break;
            }
        }
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
                assert_eq!(update.candidates.len(), 301);
                break;
            }
        }
    }
}

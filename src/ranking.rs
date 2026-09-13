//! Deterministic match classes and local, hashed acceptance statistics.
use crate::model::Candidate;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_ENTRIES: usize = 10_000;
const RETENTION_SECONDS: u64 = 90 * 24 * 60 * 60;

/// A smaller class always wins, independently of past selections.
pub fn match_class(label: &str, prefix: &str, fuzzy: bool) -> Option<u8> {
    let label = label.to_lowercase();
    let prefix = prefix.to_lowercase();
    if label == prefix {
        return Some(0);
    }
    if label.starts_with(&prefix) {
        return Some(1);
    }
    if !fuzzy || prefix.is_empty() {
        return None;
    }
    let mut chars = label.chars();
    if prefix
        .chars()
        .all(|needle| chars.by_ref().any(|c| c == needle))
    {
        return Some(2);
    }
    if prefix.chars().count() >= 3 && one_edit_apart(&label, &prefix) {
        return Some(3);
    }
    None
}

fn one_edit_apart(left: &str, right: &str) -> bool {
    let a: Vec<_> = left.chars().collect();
    let b: Vec<_> = right.chars().collect();
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        if a.len() == b.len() {
            if i + 1 < a.len() && a[i] == b[j + 1] && a[i + 1] == b[j] {
                i += 2;
                j += 2;
            } else {
                i += 1;
                j += 1;
            }
        } else if a.len() > b.len() {
            i += 1;
        } else {
            j += 1;
        }
    }
    edits + usize::from(i < a.len() || j < b.len()) <= 1
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Selection {
    count: u64,
    last: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSnapshot {
    schema: u32,
    salt: String,
    entries: BTreeMap<String, Selection>,
}
impl Default for UsageSnapshot {
    fn default() -> Self {
        Self {
            schema: 1,
            salt: uuid::Uuid::new_v4().to_string(),
            entries: BTreeMap::new(),
        }
    }
}
impl UsageSnapshot {
    pub fn load(path: &Path) -> Self {
        use std::io::Read;
        let mut bytes = Vec::new();
        let mut store = fs::File::open(path)
            .ok()
            .and_then(|file| file.take(2_000_001).read_to_end(&mut bytes).ok())
            .filter(|_| bytes.len() <= 2_000_000)
            .and_then(|_| serde_json::from_slice::<Self>(&bytes).ok())
            .filter(|store| store.schema == 1 && !store.salt.is_empty())
            .unwrap_or_default();
        store.prune(now());
        store
    }
    fn key(&self, candidate: &str, project: &Path) -> String {
        let mut hash = Sha256::new();
        let project = project.to_string_lossy();
        for field in [self.salt.as_str(), project.as_ref(), candidate] {
            hash.update((field.len() as u64).to_le_bytes());
            hash.update(field.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
    fn record(&mut self, candidate: &str, project: &Path, now: u64) {
        let entry = self
            .entries
            .entry(self.key(candidate, project))
            .or_default();
        entry.count = entry.count.saturating_add(1);
        entry.last = now;
        self.prune(now);
    }
    fn prune(&mut self, now: u64) {
        self.entries.retain(|key, value| {
            key.len() == 64
                && key.bytes().all(|b| b.is_ascii_hexdigit())
                && now.saturating_sub(value.last) <= RETENTION_SECONDS
        });
        if self.entries.len() > MAX_ENTRIES {
            let mut order: Vec<_> = self
                .entries
                .iter()
                .map(|(key, value)| (value.last, key.clone()))
                .collect();
            order.sort();
            for (_, key) in order.into_iter().take(self.entries.len() - MAX_ENTRIES) {
                self.entries.remove(&key);
            }
        }
    }
    fn score(&self, candidate: &Candidate, project: &Path) -> (u64, u64) {
        self.entries
            .get(&self.key(candidate.identity(), project))
            .map(|e| (e.count, e.last))
            .unwrap_or_default()
    }
}

pub fn sort(
    candidates: &mut Vec<Candidate>,
    prefix: &str,
    fuzzy: bool,
    usage: Option<&UsageSnapshot>,
    project: &Path,
    limit: usize,
) {
    candidates.retain(|c| match_class(&c.label, prefix, fuzzy).is_some());
    candidates.sort_by_cached_key(|candidate| {
        let score = usage
            .map(|s| s.score(candidate, project))
            .unwrap_or_default();
        let command = matches!(
            candidate.kind,
            crate::model::CandidateKind::Command
                | crate::model::CandidateKind::Alias
                | crate::model::CandidateKind::Function
                | crate::model::CandidateKind::Cmdlet
        );
        // Stable sorting preserves the declared order of equal-quality spec
        // candidates. Executable names retain their familiar short-name order.
        (
            match_class(&candidate.label, prefix, fuzzy).unwrap_or(4),
            // Distinct native flags such as Git -C and -c must not let
            // case folding or learned frequency displace the typed spelling.
            candidate.kind == crate::model::CandidateKind::Option
                && !candidate.label.starts_with(prefix),
            std::cmp::Reverse(score),
            if command { candidate.label.len() } else { 0 },
        )
    });
    candidates.truncate(limit);
}

enum Event {
    Accepted(String, PathBuf),
    Clear(mpsc::Sender<()>),
    Stop,
}
pub struct Learning {
    sender: mpsc::Sender<Event>,
    snapshot: Arc<RwLock<Arc<UsageSnapshot>>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Learning {
    pub fn new(path: PathBuf) -> Self {
        let snapshot = Arc::new(RwLock::new(Arc::new(UsageSnapshot::default())));
        let (sender, receiver) = mpsc::channel();
        let shared = snapshot.clone();
        let thread = thread::spawn(move || {
            let mut store = UsageSnapshot::load(&path);
            let mut disk_stamp = stamp(&path);
            store.prune(now());
            *shared.write().unwrap() = Arc::new(store.clone());
            let mut dirty = false;
            loop {
                let event = if dirty {
                    receiver.recv_timeout(Duration::from_millis(500))
                } else {
                    receiver
                        .recv()
                        .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                };
                match event {
                    Ok(Event::Accepted(id, project)) => {
                        if disk_stamp.is_some() && stamp(&path).is_none() {
                            store = UsageSnapshot::default();
                            disk_stamp = None;
                        }
                        store.record(&id, &project, now());
                        dirty = true;
                        *shared.write().unwrap() = Arc::new(store.clone());
                    }
                    Ok(Event::Clear(done)) => {
                        store = UsageSnapshot::default();
                        *shared.write().unwrap() = Arc::new(store.clone());
                        let _ = fs::remove_file(&path);
                        disk_stamp = None;
                        dirty = false;
                        let _ = done.send(());
                    }
                    Ok(Event::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        if dirty && !(disk_stamp.is_some() && stamp(&path).is_none()) {
                            let _ = save(&path, &store);
                        }
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if disk_stamp.is_some() && stamp(&path).is_none() {
                            store = UsageSnapshot::default();
                            disk_stamp = None;
                            dirty = false;
                            *shared.write().unwrap() = Arc::new(store.clone());
                        }
                        if dirty && save(&path, &store).is_ok() {
                            dirty = false;
                            disk_stamp = stamp(&path);
                        }
                    }
                }
            }
        });
        Self {
            sender,
            snapshot,
            thread: Some(thread),
        }
    }
    pub fn snapshot(&self) -> Arc<UsageSnapshot> {
        self.snapshot.read().unwrap().clone()
    }
    pub fn accepted(&self, id: &str, project: &Path) {
        let _ = self.sender.send(Event::Accepted(id.into(), project.into()));
    }
    pub fn clear(&self) {
        let (send, recv) = mpsc::channel();
        let _ = self.sender.send(Event::Clear(send));
        let _ = recv.recv_timeout(Duration::from_secs(2));
    }
}
impl Drop for Learning {
    fn drop(&mut self) {
        let _ = self.sender.send(Event::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn stamp(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok().and_then(|m| m.modified().ok())
}
fn save(path: &Path, store: &UsageSnapshot) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temp, serde_json::to_vec(store)?)?;
    // Atomic replacement permits concurrent read-only diagnostics.
    if let Err(error) = crate::engine::replace_file(&temp, path) {
        let _ = fs::remove_file(temp);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn match_quality_dominates_frequency_and_typos_are_explicit_candidates() {
        assert_eq!(match_class("git", "gti", true), Some(3));
        assert_eq!(match_class("Get-ChildItem", "gci", true), Some(2));
        assert_eq!(match_class("git", "gti", false), None);
        let mut flags = vec![
            Candidate {
                label: "-c".into(),
                kind: crate::model::CandidateKind::Option,
                ..Default::default()
            },
            Candidate {
                label: "-C".into(),
                kind: crate::model::CandidateKind::Option,
                ..Default::default()
            },
        ];
        sort(&mut flags, "-C", true, None, Path::new("."), 10);
        assert_eq!(flags[0].label, "-C");
        let mut data = UsageSnapshot::default();
        data.record("git-gui", Path::new("project"), now());
        let mut values = vec![
            Candidate {
                label: "git-gui".into(),
                ..Default::default()
            },
            Candidate {
                label: "git".into(),
                ..Default::default()
            },
        ];
        sort(
            &mut values,
            "git",
            true,
            Some(&data),
            Path::new("project"),
            100,
        );
        assert_eq!(values[0].label, "git");
    }
    #[test]
    fn persisted_statistics_contain_only_hashes_counts_and_times() {
        let mut store = UsageSnapshot::default();
        store.record("private-branch", Path::new("C:/private/project"), 100);
        let text = serde_json::to_string(&store).unwrap();
        assert!(!text.contains("private"));
        store.prune(RETENTION_SECONDS + 101);
        assert!(store.entries.is_empty());
    }

    #[test]
    fn acknowledged_selections_flush_on_exit_and_clear_is_durable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("usage.json");
        {
            let learning = Learning::new(path.clone());
            learning.accepted("branch:private", Path::new("private-project"));
        }
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("private"));
        let data: UsageSnapshot = serde_json::from_str(&text).unwrap();
        assert_eq!(data.entries.len(), 1);
        {
            let learning = Learning::new(path.clone());
            learning.clear();
        }
        assert!(!path.exists());
    }
}

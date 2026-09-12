//! Asynchronous persistence for completed command discovery snapshots.
//!
//! Cache writes include filesystem metadata and an atomic replacement, so
//! they must not run on the query worker. `CacheWriter` keeps only the newest
//! submitted snapshot while one write is in progress and drains that slot
//! during shutdown when the bounded drop wait permits it.

use crate::{engine::CommandIndex, trace::Trace};
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const DROP_DRAIN_TIMEOUT: Duration = Duration::from_millis(250);

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

struct State {
    pending: Option<CommandIndex>,
    stopping: bool,
    busy: bool,
    #[cfg(test)]
    waiters: Vec<Sender<()>>,
}

/// Persists the latest complete command index on a dedicated thread.
pub struct CacheWriter {
    shared: Arc<Shared>,
    done: Option<Receiver<()>>,
    thread: Option<JoinHandle<()>>,
}

impl CacheWriter {
    /// Start a cache writer for `path`.
    pub fn new(path: PathBuf, trace: Trace) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                pending: None,
                stopping: false,
                busy: false,
                #[cfg(test)]
                waiters: Vec::new(),
            }),
            wake: Condvar::new(),
        });
        let thread_shared = Arc::clone(&shared);
        let (done_sender, done_receiver) = mpsc::channel();
        let thread = thread::spawn(move || writer_loop(path, trace, thread_shared, done_sender));

        Self {
            shared,
            done: Some(done_receiver),
            thread: Some(thread),
        }
    }

    /// Queue a complete snapshot for persistence.
    ///
    /// A snapshot submitted while another write is in progress replaces any
    /// older snapshot still waiting in the slot. Incomplete snapshots are
    /// ignored before they can wake or block the writer.
    pub fn submit(&self, index: CommandIndex) {
        if !index.is_complete() {
            return;
        }
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        if state.stopping {
            return;
        }
        state.pending = Some(index);
        self.shared.wake.notify_one();
    }

    #[cfg(test)]
    fn wait_for_idle(&self) {
        let (sender, receiver) = mpsc::channel();
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        if state.pending.is_none() && !state.busy {
            return;
        }
        state.waiters.push(sender);
        self.shared.wake.notify_one();
        drop(state);
        let _ = receiver.recv();
    }
}

impl Drop for CacheWriter {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.stopping = true;
            self.shared.wake.notify_one();
        }

        // The writer is asked to drain its latest slot, but shutdown must not
        // make shell exit wait indefinitely on a slow filesystem. Dropping a
        // still-running JoinHandle detaches it after the bounded wait.
        if let Some(done) = self.done.take() {
            let _ = done.recv_timeout(DROP_DRAIN_TIMEOUT);
        }
        let _ = self.thread.take();
    }
}

fn writer_loop(path: PathBuf, trace: Trace, shared: Arc<Shared>, done: Sender<()>) {
    loop {
        let index = {
            let Ok(mut state) = shared.state.lock() else {
                break;
            };
            loop {
                if let Some(index) = state.pending.take() {
                    state.busy = true;
                    break Some(index);
                }
                if state.stopping {
                    signal_idle(&mut state);
                    break None;
                }
                signal_idle(&mut state);
                state = match shared.wake.wait(state) {
                    Ok(state) => state,
                    Err(_) => break None,
                };
            }
        };

        let Some(index) = index else {
            break;
        };

        let started = Instant::now();
        let success = index.save(&path).is_ok();
        trace.event(
            "cache_write",
            None,
            Some(started.elapsed()),
            Some(if success { 1 } else { 0 }),
        );

        if let Ok(mut state) = shared.state.lock() {
            state.busy = false;
            signal_idle(&mut state);
        }
    }

    trace.flush();
    let _ = done.send(());
}

#[cfg(test)]
fn signal_idle(state: &mut State) {
    if state.pending.is_none() && !state.busy {
        for waiter in state.waiters.drain(..) {
            let _ = waiter.send(());
        }
    }
}

#[cfg(not(test))]
fn signal_idle(_state: &mut State) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
    };
    use tempfile::tempdir;

    fn completed_index(directory: &Path, command: &str) -> (CommandIndex, OsString) {
        fs::write(directory.join(format!("{command}.exe")), b"").unwrap();
        let path = std::env::join_paths([directory]).unwrap();
        let pathext = OsString::from(".EXE");
        (
            CommandIndex::discover_with_env(&path, pathext.as_os_str()),
            path,
        )
    }

    #[test]
    fn latest_complete_snapshot_wins_without_sleeping() {
        let root = tempdir().unwrap();
        let old_dir = root.path().join("old");
        let latest_dir = root.path().join("latest");
        fs::create_dir(&old_dir).unwrap();
        fs::create_dir(&latest_dir).unwrap();
        let (old, old_path) = completed_index(&old_dir, "old-command");
        let (latest, latest_path) = completed_index(&latest_dir, "latest-command");
        let cache = root.path().join("commands.json");

        let writer = CacheWriter::new(cache.clone(), Trace::default());
        writer.submit(old);
        writer.submit(latest);
        writer.wait_for_idle();
        drop(writer);

        let loaded =
            CommandIndex::load_with_env(&cache, &latest_path, OsString::from(".EXE").as_os_str())
                .unwrap();
        let completion = loaded.complete("latest", 6, Path::new("."), 10);
        assert!(
            completion
                .candidates
                .iter()
                .any(|candidate| candidate.label == "latest-command")
        );
        assert!(
            CommandIndex::load_with_env(&cache, &old_path, OsString::from(".EXE").as_os_str())
                .is_err()
        );
    }

    #[test]
    fn incomplete_snapshot_is_skipped() {
        let root = tempdir().unwrap();
        let directory = root.path().join("many");
        fs::create_dir(&directory).unwrap();
        for number in 0..300 {
            fs::write(directory.join(format!("command-{number:03}.exe")), b"").unwrap();
        }
        let path = std::env::join_paths([PathBuf::from(&directory)]).unwrap();
        let mut discovery =
            crate::engine::Discovery::new(&path, OsString::from(".EXE").as_os_str());
        assert!(!discovery.step());
        let partial = discovery.snapshot();
        assert!(!partial.is_complete());

        let cache = root.path().join("commands.json");
        let writer = CacheWriter::new(cache.clone(), Trace::default());
        writer.submit(partial);
        drop(writer);
        assert!(!cache.exists());
    }
}

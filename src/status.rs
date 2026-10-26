//! Small session diagnostics written away from the input and query path.
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
};
#[derive(Default)]
struct Pending {
    value: Option<Value>,
    stop: bool,
}
pub struct StatusWriter(
    Arc<(Mutex<Pending>, Condvar)>,
    Option<thread::JoinHandle<()>>,
);
impl StatusWriter {
    pub fn new(path: PathBuf) -> Self {
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let worker = shared.clone();
        let thread = thread::spawn(move || {
            loop {
                let value = {
                    let (lock, wake) = &*worker;
                    let mut pending = lock.lock().unwrap();
                    while pending.value.is_none() && !pending.stop {
                        pending = wake.wait(pending).unwrap();
                    }
                    if pending.stop {
                        return;
                    }
                    pending.value.take().unwrap()
                };
                if let Ok(bytes) = serde_json::to_vec(&value) {
                    let temporary = path.with_extension("tmp");
                    if std::fs::write(&temporary, bytes).is_ok() {
                        #[cfg(windows)]
                        if !publish_windows_status(&worker, &temporary, &path) {
                            return;
                        }
                        #[cfg(not(windows))]
                        let _ = crate::engine::replace_file(&temporary, &path);
                    }
                }
            }
        });
        Self(shared, Some(thread))
    }
    pub fn update(&self, value: Value) {
        #[cfg(windows)]
        let value = {
            let mut value = value;
            value["conpty"] = crate::conpty::loaded_identity();
            value
        };
        self.0.0.lock().unwrap().value = Some(value);
        self.0.1.notify_one();
    }
}

#[cfg(windows)]
fn publish_windows_status(
    shared: &(Mutex<Pending>, Condvar),
    temporary: &std::path::Path,
    path: &std::path::Path,
) -> bool {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        match crate::engine::replace_file(temporary, path) {
            Ok(()) => return true,
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {}
            Err(_) => return true,
        }
        let (lock, wake) = shared;
        let pending = lock.lock().unwrap();
        if pending.stop {
            return false;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if pending.value.is_some() || remaining.is_zero() {
            return true;
        }
        let (pending, _) = wake
            .wait_timeout(pending, remaining.min(Duration::from_millis(10)))
            .unwrap();
        if pending.stop {
            return false;
        }
        if pending.value.is_some() {
            return true;
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::windows::fs::OpenOptionsExt,
        time::{Duration, Instant},
    };

    fn wait_value(path: &std::path::Path, expected: u64) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|value| value["value"] == expected)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "status {} never reached {expected}",
                path.display()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn status_is_published_after_a_temporary_windows_handle_releases() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("adapter.json");
        fs::write(&path, br#"{"value":0}"#).unwrap();
        let blocker = fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();
        let writer = StatusWriter::new(path.clone());
        writer.update(serde_json::json!({"value":1}));
        wait_value(&path.with_extension("tmp"), 1);
        thread::sleep(Duration::from_millis(100));
        drop(blocker);
        wait_value(&path, 1);
    }

    #[test]
    fn a_newer_status_supersedes_a_blocked_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("adapter.json");
        fs::write(&path, br#"{"value":0}"#).unwrap();
        let blocker = fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();
        let writer = StatusWriter::new(path.clone());
        writer.update(serde_json::json!({"value":1}));
        wait_value(&path.with_extension("tmp"), 1);
        writer.update(serde_json::json!({"value":2}));
        wait_value(&path.with_extension("tmp"), 2);
        drop(blocker);
        wait_value(&path, 2);
    }
}
impl Drop for StatusWriter {
    fn drop(&mut self) {
        self.0.0.lock().unwrap().stop = true;
        self.0.1.notify_one();
        if let Some(thread) = self.1.take() {
            let _ = thread.join();
        }
    }
}

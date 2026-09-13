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
                        let _ = crate::engine::replace_file(&temporary, &path);
                    }
                }
            }
        });
        Self(shared, Some(thread))
    }
    pub fn update(&self, value: Value) {
        self.0.0.lock().unwrap().value = Some(value);
        self.0.1.notify_one();
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

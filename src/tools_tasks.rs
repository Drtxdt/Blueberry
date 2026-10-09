//! One latest-request worker; only the UI thread may publish learned records.
use super::ToolInfo;
use crate::knowledge;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Version,
    Learn,
}
pub(super) enum Output {
    Version(String),
    Learned(Box<knowledge::Record>),
}
pub(super) struct Completed {
    pub tool: ToolInfo,
    pub output: Result<Output, String>,
}
struct Request {
    id: u64,
    tool: ToolInfo,
    kind: Kind,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct Pending {
    request: Option<Request>,
    stop: bool,
}
struct Current {
    id: u64,
    command: String,
    paths: Vec<std::path::PathBuf>,
    kind: Kind,
    cancel: Arc<AtomicBool>,
}
pub(super) struct Worker {
    shared: Arc<(Mutex<Pending>, Condvar)>,
    results: mpsc::Receiver<(u64, Completed)>,
    current: Option<Current>,
    generation: u64,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    pub fn new() -> Self {
        Self::with_handler(|tool, kind, cancel| match kind {
            Kind::Version => super::query_version(tool, cancel).map(Output::Version),
            Kind::Learn => {
                super::learn(tool, cancel).map(|record| Output::Learned(Box::new(record)))
            }
        })
    }
    fn with_handler(
        handler: impl Fn(&ToolInfo, Kind, &AtomicBool) -> Result<Output, String> + Send + 'static,
    ) -> Self {
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let background = shared.clone();
        let (tx, results) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            loop {
                let (lock, wake) = &*background;
                let mut pending = lock.lock().unwrap();
                while pending.request.is_none() && !pending.stop {
                    pending = wake.wait(pending).unwrap();
                }
                if pending.stop {
                    break;
                }
                let request = pending.request.take().unwrap();
                drop(pending);
                let output = handler(&request.tool, request.kind, &request.cancel);
                if tx
                    .send((
                        request.id,
                        Completed {
                            tool: request.tool,
                            output,
                        },
                    ))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            shared,
            results,
            current: None,
            generation: 0,
            thread: Some(thread),
        }
    }
    pub fn submit(&mut self, tool: &ToolInfo, kind: Kind) -> bool {
        if self.current.as_ref().is_some_and(|current| {
            current.command == tool.command && current.paths == tool.paths && current.kind == kind
        }) {
            return false;
        }
        self.cancel();
        self.generation += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.current = Some(Current {
            id: self.generation,
            command: tool.command.clone(),
            paths: tool.paths.clone(),
            kind,
            cancel: cancel.clone(),
        });
        self.shared.0.lock().unwrap().request = Some(Request {
            id: self.generation,
            tool: tool.clone(),
            kind,
            cancel,
        });
        self.shared.1.notify_one();
        true
    }
    pub fn cancel(&mut self) {
        if let Some(current) = self.current.take() {
            current.cancel.store(true, Ordering::Relaxed);
        }
        self.shared.0.lock().unwrap().request = None;
    }
    pub fn poll(&mut self) -> Option<Completed> {
        while let Ok((id, completed)) = self.results.try_recv() {
            if self
                .current
                .as_ref()
                .is_some_and(|current| current.id == id)
            {
                self.current = None;
                return Some(completed);
            }
        }
        None
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        self.shared.0.lock().unwrap().stop = true;
        self.shared.1.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn tool(name: &str) -> ToolInfo {
        ToolInfo {
            command: name.into(),
            paths: vec![name.into()],
            installed: true,
            contexts: 0,
            dynamic: false,
            help: String::new(),
            help_adapter: String::new(),
            providers: vec![],
            resources: String::new(),
            error: None,
            version: None,
        }
    }
    #[test]
    fn duplicates_coalesce_and_cancelled_completion_cannot_publish() {
        let (started_tx, started) = mpsc::channel();
        let mut worker = Worker::with_handler(move |tool, _, cancel| {
            started_tx.send(tool.command.clone()).unwrap();
            if tool.command == "old" {
                while !cancel.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            Ok(Output::Version(tool.command.clone()))
        });
        assert!(worker.submit(&tool("old"), Kind::Learn));
        assert_eq!(started.recv_timeout(Duration::from_secs(2)).unwrap(), "old");
        assert!(!worker.submit(&tool("old"), Kind::Learn));
        assert!(worker.submit(&tool("new"), Kind::Version));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = worker.poll() {
                assert_eq!(result.tool.command, "new");
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        worker.submit(&tool("old"), Kind::Learn);
        worker.cancel();
        assert!(worker.poll().is_none());
    }
    #[test]
    fn dropping_worker_cancels_active_task() {
        let (tx, rx) = mpsc::channel();
        let mut worker = Worker::with_handler(move |_, _, cancel| {
            tx.send(()).unwrap();
            while !cancel.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err("cancelled".into())
        });
        worker.submit(&tool("test"), Kind::Version);
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let start = Instant::now();
        drop(worker);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}

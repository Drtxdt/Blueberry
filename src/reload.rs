//! File notifications for user configuration and declarative specifications.
use crate::config::{self, Config};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct Monitor {
    watcher: Option<RecommendedWatcher>,
    watched: Vec<PathBuf>,
    paths: Arc<Mutex<(PathBuf, PathBuf)>>,
    dirty: Arc<AtomicBool>,
}
impl Monitor {
    pub fn new(config: &Config, path: Option<&Path>, changed: impl Fn() + Send + 'static) -> Self {
        let paths = Arc::new(Mutex::new((
            path.map(PathBuf::from).unwrap_or_else(config::default_path),
            config::specs_dir(config, path),
        )));
        let checked = paths.clone();
        let dirty = Arc::new(AtomicBool::new(false));
        let notified = dirty.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else {
                return;
            };
            if event.kind.is_access() {
                return;
            }
            let (config, specs) = &*checked.lock().unwrap();
            if event.paths.iter().any(|p| {
                p == config
                    || p == specs
                    || (p.starts_with(specs)
                        && p.extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("toml")))
            }) {
                notified.store(true, Ordering::Release);
                changed();
            }
        })
        .ok();
        let mut monitor = Self {
            watcher,
            watched: Vec::new(),
            paths,
            dirty,
        };
        monitor.update(config, path);
        monitor
    }
    pub fn take_changed(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }
    pub fn update(&mut self, config: &Config, path: Option<&Path>) {
        let config_path = path.map(PathBuf::from).unwrap_or_else(config::default_path);
        let specs = config::specs_dir(config, path);
        *self.paths.lock().unwrap() = (config_path.clone(), specs.clone());
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        for old in self.watched.drain(..) {
            let _ = watcher.unwatch(&old);
        }
        if let Some(parent) = config_path.parent().filter(|p| p.is_dir())
            && watcher.watch(parent, RecursiveMode::NonRecursive).is_ok()
        {
            self.watched.push(parent.into());
        }
        if specs.is_dir() && watcher.watch(&specs, RecursiveMode::Recursive).is_ok() {
            self.watched.push(specs);
        }
    }
}

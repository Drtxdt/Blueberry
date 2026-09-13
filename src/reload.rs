//! File notifications for user configuration and declarative specifications.
use crate::config::{self, Config};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct Monitor {
    watcher: Option<RecommendedWatcher>,
    /// Paths currently registered with notify and whether each is recursive.
    /// The mode is tracked so a specs directory can upgrade a parent watch
    /// after it is created without being hidden by a duplicate-path check.
    watched: HashMap<PathBuf, bool>,
    paths: Arc<Mutex<ReloadPaths>>,
    dirty: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
struct ReloadPaths {
    config: PathBuf,
    specs: PathBuf,
    /// The nearest currently existing ancestor used while the config parent
    /// is absent.  Each directory creation under this ancestor is relevant
    /// only when it is on the path to `config`.
    config_pending_parent: Option<PathBuf>,
    /// The nearest currently existing ancestor used while the specs
    /// directory is absent.  This is updated on every notification so a
    /// multi-level path can be followed as each component is created.
    specs_pending_parent: Option<PathBuf>,
}

impl ReloadPaths {
    fn new(config: PathBuf, specs: PathBuf) -> Self {
        let config = watch_path(&config);
        let specs = watch_path(&specs);
        let config_pending_parent = config
            .parent()
            .filter(|parent| !parent.is_dir())
            .and_then(|parent| nearest_existing_directory(Some(parent)));
        let specs_pending_parent = (!specs.is_dir())
            .then(|| nearest_existing_directory(Some(&specs)))
            .flatten();
        Self {
            config,
            specs,
            config_pending_parent,
            specs_pending_parent,
        }
    }
}

fn event_targets_path(event: &Path, target: &Path, pending_parent: Option<&Path>) -> bool {
    event == target
        || pending_parent.is_some_and(|parent| {
            event != parent
                && event.starts_with(parent)
                && (target.starts_with(event) || event.starts_with(target))
        })
}

impl Monitor {
    pub fn new(config: &Config, path: Option<&Path>, changed: impl Fn() + Send + 'static) -> Self {
        let paths = Arc::new(Mutex::new(ReloadPaths::new(
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
            let paths = checked.lock().unwrap();
            if event.paths.iter().any(|p| {
                let p = watch_path(p);
                event_targets_path(&p, &paths.config, paths.config_pending_parent.as_deref())
                    || event_targets_path(&p, &paths.specs, paths.specs_pending_parent.as_deref())
                    || (p.starts_with(&paths.specs)
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
            watched: HashMap::new(),
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
        let paths = ReloadPaths::new(
            path.map(PathBuf::from).unwrap_or_else(config::default_path),
            config::specs_dir(config, path),
        );
        let config_path = paths.config.clone();
        let specs = paths.specs.clone();
        *self.paths.lock().unwrap() = paths;
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        let old = self.watched.keys().cloned().collect::<Vec<_>>();
        for path in old {
            let _ = watcher.unwatch(&path);
        }
        self.watched.clear();

        // Keep watching the nearest existing parent when the config file's
        // parent has not been created yet. This lets a later config creation
        // notify the host instead of requiring a restart.
        if let Some(parent) = nearest_existing_directory(config_path.parent()) {
            self.watch(&parent, false);
        }

        if specs.is_dir() {
            self.watch(&specs, true);
        } else if let Some(parent) = nearest_existing_directory(Some(&specs)) {
            // A missing or replaced specs directory is noticed through its
            // nearest existing parent. The host calls update again after the
            // notification, which reattaches a recursive watch once the
            // directory exists.
            self.watch(&parent, false);
        }
    }

    fn watch(&mut self, path: &Path, recursive: bool) -> bool {
        if path.as_os_str().is_empty() {
            return false;
        }
        if let Some(existing) = self.watched.get(path).copied() {
            if existing || !recursive {
                return true;
            }
            let _ = self.watcher.as_mut().unwrap().unwatch(path);
            self.watched.remove(path);
        }
        let Some(watcher) = self.watcher.as_mut() else {
            return false;
        };
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        if watcher.watch(path, mode).is_ok() {
            self.watched.insert(path.to_path_buf(), recursive);
            true
        } else {
            false
        }
    }
}

/// FSEvents reports real paths, including `/private/var` for `/var` on macOS.
/// Resolve the existing prefix even when the configured file does not exist.
fn watch_path(path: &Path) -> PathBuf {
    let mut prefix = path;
    let mut missing = Vec::new();
    loop {
        if let Ok(mut resolved) = prefix.canonicalize() {
            for component in missing.iter().rev() {
                resolved.push(component);
            }
            return resolved;
        }
        let Some(name) = prefix.file_name() else {
            return path.to_path_buf();
        };
        missing.push(name.to_os_string());
        let Some(parent) = prefix.parent() else {
            return path.to_path_buf();
        };
        prefix = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
    }
}

fn nearest_existing_directory(path: Option<&Path>) -> Option<PathBuf> {
    let mut current = path?.to_path_buf();
    if current.as_os_str().is_empty() {
        current.push(".");
    }
    loop {
        if current.is_dir() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
        if current.as_os_str().is_empty() {
            current.push(".");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::mpsc::{self, Receiver},
        time::{Duration, Instant},
    };

    fn wait_for_change(monitor: &Monitor, receiver: &Receiver<()>) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if monitor.take_changed() {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "notify did not report the expected change"
            );
            let _ = receiver.recv_timeout(remaining);
        }
    }

    // A directory watch receives more than one event for some filesystem
    // operations. Drain those events before the next mutation so each wait
    // below proves that the current path was reattached after the update.
    fn settle_events(monitor: &Monitor, receiver: &Receiver<()>) {
        let deadline = Instant::now() + Duration::from_millis(150);
        while Instant::now() < deadline {
            monitor.take_changed();
            while receiver.try_recv().is_ok() {}
            std::thread::sleep(Duration::from_millis(10));
        }
        monitor.take_changed();
        while receiver.try_recv().is_ok() {}
    }

    #[test]
    fn missing_nested_specs_directory_is_followed_by_real_notify_events() {
        let root = tempfile::tempdir().unwrap();
        let specs = root.path().join("one").join("two").join("specs");
        let config_path = root.path().join("config.toml");
        let mut config = Config::default();
        config.specs.directory = Some(specs.clone());
        let (send, receiver) = mpsc::channel();
        let mut monitor = Monitor::new(&config, Some(&config_path), move || {
            let _ = send.send(());
        });

        fs::create_dir(root.path().join("one")).unwrap();
        wait_for_change(&monitor, &receiver);
        monitor.update(&config, Some(&config_path));
        settle_events(&monitor, &receiver);

        fs::create_dir(root.path().join("one").join("two")).unwrap();
        wait_for_change(&monitor, &receiver);
        monitor.update(&config, Some(&config_path));
        settle_events(&monitor, &receiver);

        fs::create_dir(&specs).unwrap();
        wait_for_change(&monitor, &receiver);
        monitor.update(&config, Some(&config_path));
        settle_events(&monitor, &receiver);

        fs::write(specs.join("custom.toml"), "[completion]\n").unwrap();
        wait_for_change(&monitor, &receiver);
    }

    #[test]
    fn missing_config_parent_is_followed_by_real_notify_events() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("late").join("config.toml");
        let specs = root.path().join("specs");
        fs::create_dir(&specs).unwrap();
        let mut config = Config::default();
        config.specs.directory = Some(specs);
        let (send, receiver) = mpsc::channel();
        let mut monitor = Monitor::new(&config, Some(&config_path), move || {
            let _ = send.send(());
        });

        fs::create_dir(root.path().join("late")).unwrap();
        wait_for_change(&monitor, &receiver);
        monitor.update(&config, Some(&config_path));
        settle_events(&monitor, &receiver);

        fs::write(&config_path, "[ui]\nwidth = 80\n").unwrap();
        wait_for_change(&monitor, &receiver);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_parent_matches_real_event_paths_for_missing_targets() {
        let root = tempfile::tempdir().unwrap();
        let actual = root.path().join("actual");
        fs::create_dir(&actual).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&actual, &alias).unwrap();
        let paths = ReloadPaths::new(alias.join("late/config.toml"), alias.join("specs"));
        let event = actual.canonicalize().unwrap().join("late");
        assert!(event_targets_path(
            &event,
            &paths.config,
            paths.config_pending_parent.as_deref()
        ));
        assert!(!event_targets_path(
            &actual.join("unrelated"),
            &paths.config,
            paths.config_pending_parent.as_deref()
        ));
    }
}

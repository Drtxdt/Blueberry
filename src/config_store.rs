//! Serialized, lossless configuration edits shared by all configuration UIs.
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
use toml_edit::{DocumentMut, Item};

/// Preserve a field's inline comment and spacing when a UI changes its value.
pub fn set_value(target: &mut Item, mut replacement: Item) {
    if let (Some(old), Some(new)) = (target.as_value(), replacement.as_value_mut()) {
        *new.decor_mut() = old.decor().clone();
    }
    *target = replacement;
}

fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("无法读取 {}", path.display())),
    }
}

fn document(bytes: &[u8]) -> Result<DocumentMut> {
    std::str::from_utf8(bytes)
        .context("配置需要 UTF-8 编码")?
        .trim_start_matches('\u{feff}')
        .parse()
        .context("配置 TOML 无法解析")
}

/// Re-read under the writer lock, preserving fields owned by other writers.
pub fn edit(path: &Path, change: impl FnOnce(&mut DocumentMut) -> Result<()>) -> Result<Vec<u8>> {
    edit_initial(path, crate::config::example().as_bytes(), |doc, _| {
        change(doc)
    })
}

fn acquire_lock(lock: &fs::File, timeout: std::time::Duration) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match lock.try_lock() {
            Ok(()) => return Ok(()),
            Err(fs::TryLockError::Error(error)) => return Err(error).context("无法锁定配置"),
            Err(fs::TryLockError::WouldBlock) => {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    bail!("配置正在由其他程序保存；等待超时，未修改文件，请重试");
                }
                std::thread::sleep(remaining.min(std::time::Duration::from_millis(10)));
            }
        }
    }
}

fn edit_initial(
    path: &Path,
    initial: &[u8],
    change: impl FnOnce(&mut DocumentMut, bool) -> Result<()>,
) -> Result<Vec<u8>> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("toml.lock"))?;
    // Several independent saves may each sync to a slow Windows volume. Give
    // their queue a separate bounded budget from atomic replacement retries.
    acquire_lock(&lock, std::time::Duration::from_secs(5))?;
    let original = read(path)?;
    let initial = original.as_deref().unwrap_or(initial);
    let mut doc = document(initial)?;
    change(&mut doc, original.is_some())?;
    let mut text = doc.to_string();
    if initial.starts_with(&[0xef, 0xbb, 0xbf]) {
        text.insert(0, '\u{feff}');
    }
    let bytes = text.into_bytes();
    if original.as_deref() == Some(bytes.as_slice()) {
        return Ok(bytes);
    }
    let temporary = parent.join(format!(".blueberry-config-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(test)]
        if let Some(marker) = std::env::var_os("BLUEBERRY_TEST_CONFIG_SYNC_MARKER") {
            fs::write(marker, b"synced")?;
            std::thread::sleep(std::time::Duration::from_secs(30));
        }
        let validate = || -> Result<()> {
            if read(path)? != original {
                bail!("配置在保存期间被外部修改；未覆盖原文件");
            }
            Ok(())
        };
        #[cfg(windows)]
        {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
            loop {
                validate()?;
                let result = crate::engine::replace_file(&temporary, path);
                if result
                    .as_ref()
                    .is_err_and(|error| matches!(error.raw_os_error(), Some(5 | 32 | 33)))
                    && std::time::Instant::now() < deadline
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }
                result.with_context(|| format!("无法替换 {}", path.display()))?;
                break;
            }
        }
        #[cfg(not(windows))]
        {
            validate()?;
            crate::engine::replace_file(&temporary, path)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(bytes)
}

fn same(a: Option<&Item>, b: Option<&Item>) -> bool {
    a.map(ToString::to_string) == b.map(ToString::to_string)
}

fn merge_table(
    disk: &mut dyn toml_edit::TableLike,
    base: &dyn toml_edit::TableLike,
    desired: &dyn toml_edit::TableLike,
    prefix: &str,
) -> Result<()> {
    let keys: std::collections::BTreeSet<_> = base
        .iter()
        .chain(desired.iter())
        .map(|(k, _)| k.to_owned())
        .collect();
    for key in keys {
        let before = base.get(&key);
        let after = desired.get(&key);
        if same(before, after) {
            continue;
        }
        let label = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let empty = toml_edit::Table::new();
        let previous_table = match before {
            None => Some(&empty as &dyn toml_edit::TableLike),
            Some(item) => item.as_table_like(),
        };
        if let (Some(before), Some(after), Some(current)) = (
            previous_table,
            after.and_then(Item::as_table_like),
            disk.get_mut(&key).and_then(Item::as_table_like_mut),
        ) {
            merge_table(current, before, after, &label)?;
            continue;
        }
        if !same(disk.get(&key), before) && !same(disk.get(&key), after) {
            bail!("配置字段 {label} 已被其他程序修改；请重新加载后再保存");
        }
        match after {
            Some(item) => {
                disk.insert(&key, item.clone());
            }
            None => {
                disk.remove(&key);
            }
        }
    }
    Ok(())
}

/// Three-way merge: unrelated edits survive; conflicting field edits abort.
pub fn merge(path: &Path, baseline: &[u8], desired: &[u8], existed: bool) -> Result<Vec<u8>> {
    let base = document(baseline)?;
    let desired = document(desired)?;
    edit_initial(path, baseline, |disk, exists| {
        if existed && !exists {
            bail!("配置已被其他程序删除；请重新加载后再保存");
        }
        merge_table(disk.as_table_mut(), base.as_table(), desired.as_table(), "")?;
        let settings: crate::config::Config = toml::from_str(&disk.to_string())?;
        settings.validate()?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_writer_waits_for_slow_save_and_lock_timeout_is_bounded() -> Result<()> {
        use std::{sync::mpsc, time::Duration};
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        let original = "# retained\n[ui]\nicons = true\n";
        fs::write(&path, original)?;
        let lock_path = path.with_extension("toml.lock");
        let held = fs::File::create(&lock_path)?;
        held.lock()?;
        let contender = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)?;
        assert!(acquire_lock(&contender, Duration::from_millis(40)).is_err());
        assert_eq!(fs::read_to_string(&path)?, original);
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = edit(&worker_path, |doc| {
                doc["ui"]["icons"] = toml_edit::value(false);
                Ok(())
            });
            done_tx.send(result).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(5))?;
        let early = done_rx.recv_timeout(Duration::from_millis(650));
        drop(held);
        assert!(matches!(early, Err(mpsc::RecvTimeoutError::Timeout)));
        let bytes = done_rx.recv_timeout(Duration::from_secs(5))??;
        worker.join().unwrap();
        assert!(String::from_utf8(bytes)?.contains("# retained"));
        assert_eq!(
            document(&fs::read(path)?)?["ui"]["icons"].as_bool(),
            Some(false)
        );
        Ok(())
    }

    #[test]
    fn newly_created_tables_merge_independent_fields() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.toml");
        let base = b"# shared baseline\n[ui]\nicons = true\n";
        fs::write(&path, base)?;
        let mut desired = document(base)?;
        desired["tools"]["git"]["help"] = toml_edit::value(false);
        edit(&path, |disk| {
            disk["tools"]["git"]["dynamic"] = toml_edit::value(false);
            Ok(())
        })?;
        let merged = merge(&path, base, desired.to_string().as_bytes(), true)?;
        let doc = document(&merged)?;
        assert_eq!(doc["tools"]["git"]["help"].as_bool(), Some(false));
        assert_eq!(doc["tools"]["git"]["dynamic"].as_bool(), Some(false));
        assert!(String::from_utf8(merged)?.contains("# shared baseline"));
        Ok(())
    }
    #[test]
    fn changing_a_value_preserves_its_inline_comment() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.toml");
        fs::write(&path, "[tools.git]\nhelp = true # keep this explanation\n")?;
        let bytes = edit(&path, |doc| {
            set_value(&mut doc["tools"]["git"]["help"], toml_edit::value(false));
            Ok(())
        })?;
        assert_eq!(
            String::from_utf8(bytes)?,
            "[tools.git]\nhelp = false # keep this explanation\n"
        );
        Ok(())
    }
    #[test]
    fn interrupted_config_writer_child() -> Result<()> {
        if let Some(path) = std::env::var_os("BLUEBERRY_TEST_CONFIG_CHILD") {
            edit(Path::new(&path), |doc| {
                doc["ui"]["icons"] = toml_edit::value(false);
                Ok(())
            })?;
        }
        Ok(())
    }

    #[test]
    fn killed_writer_preserves_config_and_releases_lock() -> Result<()> {
        use std::time::{Duration, Instant};
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        let marker = dir.path().join("synced");
        let original = "# user note\n[ui]\nicons = true\n";
        fs::write(&path, original)?;
        let mut child = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "config_store::tests::interrupted_config_writer_child",
            ])
            .env("BLUEBERRY_TEST_CONFIG_CHILD", &path)
            .env("BLUEBERRY_TEST_CONFIG_SYNC_MARKER", &marker)
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() {
            if let Some(status) = child.try_wait()? {
                bail!("writer exited before sync: {status}");
            }
            if Instant::now() >= deadline {
                child.kill()?;
                child.wait()?;
                bail!("writer failed to reach sync boundary");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        child.kill()?;
        child.wait()?;
        assert_eq!(fs::read_to_string(&path)?, original);
        edit(&path, |doc| {
            doc["ui"]["icons"] = toml_edit::value(false);
            Ok(())
        })?;
        assert!(fs::read_to_string(path)?.contains("# user note"));
        Ok(())
    }
    #[test]
    fn merge_initializes_from_editor_defaults_and_preserves_bom() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        let initial = "\u{feff}# editor defaults\n[ui]\nicons = false\n";
        assert_eq!(
            merge(&path, initial.as_bytes(), initial.as_bytes(), false)?,
            initial.as_bytes()
        );
        fs::remove_file(&path)?;
        assert!(merge(&path, initial.as_bytes(), initial.as_bytes(), true).is_err());
        assert!(!path.exists());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn sharing_failure_leaves_original_readable() -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        fs::write(&path, "# original\n")?;
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)?;
        assert!(
            edit(&path, |doc| {
                doc["ui"]["icons"] = toml_edit::value(false);
                Ok(())
            })
            .is_err()
        );
        assert_eq!(fs::read_to_string(&path)?, "# original\n");
        drop(held);
        edit(&path, |doc| {
            doc["ui"]["icons"] = toml_edit::value(false);
            Ok(())
        })?;
        Ok(())
    }
    #[test]
    fn concurrent_fields_preserve_comments_and_unknown_content() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        fs::write(&path, "# keep\n[custom]\nx = 9\n")?;
        let threads: Vec<_> = (0..12)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    edit(&path, |doc| {
                        doc["tools"][&format!("tool{i}")]["help"] = toml_edit::value(false);
                        Ok(())
                    })
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap()?;
        }
        let text = fs::read_to_string(path)?;
        assert!(text.contains("# keep"));
        let value: toml::Value = toml::from_str(&text)?;
        assert_eq!(value["custom"]["x"].as_integer(), Some(9));
        assert_eq!(value["tools"].as_table().unwrap().len(), 12);
        Ok(())
    }
    #[test]
    fn merge_keeps_independent_edits_and_rejects_conflicts() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        let base = b"[ui]\nicons = true\nmax_rows = 10\n";
        fs::write(&path, base)?;
        edit(&path, |doc| {
            doc["ui"]["icons"] = toml_edit::value(false);
            Ok(())
        })?;
        // Use a known independent field; no reliance on defaults in the merge.
        let base = fs::read(&path)?;
        let mut desired = document(&base)?;
        desired["ui"]["icons"] = toml_edit::value(true);
        edit(&path, |doc| {
            doc["ui"]["max_rows"] = toml_edit::value(12);
            Ok(())
        })?;
        // Exercise the merge directly, allowing this fixture's arbitrary fields.
        edit(&path, |disk| {
            merge_table(
                disk.as_table_mut(),
                document(&base)?.as_table(),
                desired.as_table(),
                "",
            )
        })?;
        assert!(fs::read_to_string(&path)?.contains("max_rows = 12"));
        let old = fs::read(&path)?;
        let mut next = document(&old)?;
        next["ui"]["max_rows"] = toml_edit::value(14);
        edit(&path, |doc| {
            doc["ui"]["max_rows"] = toml_edit::value(16);
            Ok(())
        })?;
        let external = fs::read(&path)?;
        assert!(
            edit(&path, |disk| merge_table(
                disk.as_table_mut(),
                document(&old)?.as_table(),
                next.as_table(),
                ""
            ))
            .is_err()
        );
        assert_eq!(fs::read(path)?, external);
        Ok(())
    }
    #[test]
    fn invalid_files_and_external_edits_are_not_overwritten() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        for bytes in [b"[broken".as_slice(), &[0xff, 0xfe]] {
            fs::write(&path, bytes)?;
            assert!(edit(&path, |_| Ok(())).is_err());
            assert_eq!(fs::read(&path)?, bytes);
        }
        fs::write(&path, "# original\n")?;
        assert!(
            edit(&path, |doc| {
                doc["ui"]["icons"] = toml_edit::value(false);
                fs::write(&path, "# external\n")?;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(fs::read_to_string(path)?, "# external\n");
        Ok(())
    }
}

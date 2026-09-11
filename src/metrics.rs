//! Small, repeatable measurements for the complete native host.
//!
//! The probe intentionally exercises the same executable that a user starts:
//! the host owns an outer ConPTY and starts PowerShell in the inner one.  The
//! timings therefore include host startup, ConPTY setup, and PowerShell
//! startup.  This module does not attempt to clear operating-system caches or
//! to attribute PowerShell's memory to the native host.

use crate::{config, probe::Harness};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const FIXTURE_COUNT: usize = 10;
const MAX_ITERATIONS: u16 = 100;
const WAIT_TIMEOUT: Duration = Duration::from_secs(20);
const FIXTURE_PREFIX: &str = "ss-ben";
const TEMP_PREFIX: &str = "shellsense-host-probe-";

/// Run a bounded end-to-end host measurement.
///
/// A fresh host process and fresh data directory are used for every sample.
/// The returned report contains the first visible PowerShell prompt, the
/// first completion menu, the warm completion menu, and working-set samples
/// for the native host process alone.  The first-prompt measurement includes
/// the outer ConPTY and the inner PowerShell startup.
pub fn host_probe(executable: &Path, shell: &Path, iterations: u16) -> Result<Value> {
    ensure!(
        (1..=MAX_ITERATIONS).contains(&iterations),
        "iterations must be between 1 and {MAX_ITERATIONS}"
    );

    let temporary = ProbeDirectory::new()?;
    let fixture_dir = temporary.path().join("fixtures");
    fs::create_dir_all(&fixture_dir).with_context(|| {
        format!(
            "unable to create probe fixture directory '{}'",
            fixture_dir.display()
        )
    })?;
    create_fixtures(&fixture_dir)?;

    // Keep this configuration fixed and local to the run.  In particular, a
    // user's normal config must not alter the menu size or auto-trigger mode.
    let config_path = temporary.path().join("defaults.toml");
    fs::write(&config_path, config::example())
        .with_context(|| format!("unable to write probe config '{}'", config_path.display()))?;

    let path = fixture_path(&fixture_dir)?;
    let pathext = probe_pathext();
    let cwd = env::current_dir().context("unable to determine probe working directory")?;

    let mut first_prompt = Vec::with_capacity(iterations as usize);
    let mut first_menu = Vec::with_capacity(iterations as usize);
    let mut warm_menu = Vec::with_capacity(iterations as usize);
    let mut prompt_working_set = Vec::with_capacity(iterations as usize);
    let mut first_menu_working_set = Vec::with_capacity(iterations as usize);
    let mut warm_menu_working_set = Vec::with_capacity(iterations as usize);

    for iteration in 0..iterations {
        let data_dir = temporary.path().join(format!("run-{iteration}"));
        fs::create_dir_all(&data_dir).with_context(|| {
            format!(
                "unable to create probe data directory '{}'",
                data_dir.display()
            )
        })?;
        let args = host_args(&config_path, shell, &data_dir);
        let environment = probe_environment(&path, &pathext);
        let token = uuid::Uuid::new_v4().to_string();

        let startup = Instant::now();
        let mut harness = Harness::start(executable, &args, &cwd, &environment, token)
            .with_context(|| format!("unable to start native host for iteration {iteration}"))?;
        harness.wait_text("PS ", WAIT_TIMEOUT).with_context(|| {
            format!("native host did not show the PowerShell prompt in iteration {iteration}")
        })?;
        first_prompt.push(milliseconds(startup.elapsed()));
        push_working_set(&mut prompt_working_set, harness.process_id());

        let first_started = Instant::now();
        harness.send(b"ss-ben0")?;
        wait_menu(&mut harness, "ss-ben0", "ss-ben0-complete").with_context(|| {
            format!("first completion menu did not show ss-ben0-complete in iteration {iteration}")
        })?;
        first_menu.push(milliseconds(first_started.elapsed()));
        push_working_set(&mut first_menu_working_set, harness.process_id());

        // Reuse the same host and change the prefix.  A different fixture
        // label makes wait_text immune to a stale first-menu frame.
        let warm_started = Instant::now();
        harness.send(b"\x7f1")?;
        wait_menu(&mut harness, "ss-ben1", "ss-ben1-complete").with_context(|| {
            format!("warm completion menu did not show ss-ben1-complete in iteration {iteration}")
        })?;
        warm_menu.push(milliseconds(warm_started.elapsed()));
        push_working_set(&mut warm_menu_working_set, harness.process_id());

        // Prefer a normal child wait.  If an intermediate operation failed,
        // Harness::Drop still kills the child before this function returns.
        harness
            .finish(Duration::from_secs(5))
            .with_context(|| format!("unable to stop native host in iteration {iteration}"))?;
    }

    Ok(json!({
        "schema": 1,
        "platform": env::consts::OS,
        "arch": env::consts::ARCH,
        "executable": executable.to_string_lossy(),
        "shell": shell.to_string_lossy(),
        "iterations": iterations,
        "fixtures": fixture_names(),
        "method": {
            "host": "fresh native host process through an outer ConPTY with PowerShell in the inner ConPTY",
            "native_host_first_prompt": "time from Harness::start until visible 'PS '; includes native host, ConPTY, and PowerShell startup",
            "first_menu": "time from sending ss-ben0 until a distinct ss-ben0-complete candidate is visible",
            "warm_menu": "same host after Backspace + 1, until distinct ss-ben1-complete is visible",
            "working_set": "native host process only, sampled with K32GetProcessMemoryInfo WorkingSetSize; pwsh is excluded",
            "cache": "operating-system caches are not cleared"
        },
        "native_host_first_prompt": timing_stats(&first_prompt),
        "first_menu": timing_stats(&first_menu),
        "warm_menu": timing_stats(&warm_menu),
        "native_host_working_set": {
            "process": "native host only",
            "unit": "bytes",
            "startup": working_set_stats(&prompt_working_set),
            "first_menu": working_set_stats(&first_menu_working_set),
            "warm_menu": working_set_stats(&warm_menu_working_set),
            "api": if cfg!(windows) { "K32GetProcessMemoryInfo" } else { "unavailable on this platform" }
        }
    }))
}

fn wait_menu(harness: &mut Harness, prefix: &str, label: &str) -> Result<()> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    let ending = format!("> {prefix}");
    loop {
        let screen = harness.contents();
        if screen.contains(label)
            && screen
                .lines()
                .any(|line| line.trim_end().ends_with(&ending) || line.trim() == prefix)
        {
            return Ok(());
        }
        harness.pump(deadline.saturating_duration_since(Instant::now()))?;
    }
}

fn create_fixtures(directory: &Path) -> Result<()> {
    for name in fixture_names() {
        let path = directory.join(name);
        // The host indexes the file name; it never executes these fixtures.
        fs::write(&path, b"@echo off\r\n")
            .with_context(|| format!("unable to write probe fixture '{}'", path.display()))?;
    }
    Ok(())
}

fn fixture_names() -> Vec<String> {
    (0..FIXTURE_COUNT)
        .map(|index| format!("{FIXTURE_PREFIX}{index}-complete.cmd"))
        .collect()
}

fn fixture_path(directory: &Path) -> Result<OsString> {
    let original = env::var_os("PATH").unwrap_or_default();
    let mut entries = vec![directory.to_path_buf()];
    entries.extend(env::split_paths(&original));
    env::join_paths(entries)
        .map_err(|error| anyhow::anyhow!("unable to construct probe PATH: {error}"))
}

fn probe_pathext() -> OsString {
    let original = env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".EXE;.BAT;.CMD"));
    let original_text = original.to_string_lossy();
    let has_cmd = original_text
        .split(';')
        .any(|extension| extension.trim().eq_ignore_ascii_case(".cmd"));
    if has_cmd {
        original
    } else if original_text.is_empty() {
        OsString::from(".CMD;.EXE;.BAT")
    } else {
        OsString::from(format!(".CMD;{original_text}"))
    }
}

fn probe_environment(path: &OsStr, pathext: &OsStr) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("PATH".to_owned(), path.to_string_lossy().into_owned()),
        ("PATHEXT".to_owned(), pathext.to_string_lossy().into_owned()),
        ("SHELLSENSE_NO_HISTORY".to_owned(), "1".to_owned()),
        ("SHELLSENSE_ACTIVE".to_owned(), "1".to_owned()),
        ("ISTERM".to_owned(), "1".to_owned()),
        ("TERM".to_owned(), "xterm-256color".to_owned()),
    ])
}

fn host_args(config_path: &Path, shell: &Path, data_dir: &Path) -> Vec<String> {
    vec![
        "--config".to_owned(),
        config_path.to_string_lossy().into_owned(),
        "run".to_owned(),
        "--shell".to_owned(),
        shell.to_string_lossy().into_owned(),
        "--no-profile".to_owned(),
        "--data-dir".to_owned(),
        data_dir.to_string_lossy().into_owned(),
    ]
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn timing_stats(values: &[f64]) -> Value {
    stats(values, "ms")
}

fn working_set_stats(values: &[u64]) -> Value {
    if values.is_empty() {
        return json!({
            "available": false,
            "samples": [],
            "unit": "bytes"
        });
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    json!({
        "available": true,
        "samples": values,
        "median": sorted[sorted.len() / 2],
        "p95": sorted[p95_index(sorted.len())],
        "unit": "bytes"
    })
}

fn stats(values: &[f64], unit: &str) -> Value {
    debug_assert!(!values.is_empty());
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({
        "samples": values,
        "median": sorted[sorted.len() / 2],
        "p95": sorted[p95_index(sorted.len())],
        "unit": unit
    })
}

fn p95_index(length: usize) -> usize {
    ((length as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(length.saturating_sub(1))
}

fn push_working_set(values: &mut Vec<u64>, process_id: Option<u32>) {
    if let Some(bytes) = process_id.and_then(working_set_bytes) {
        values.push(bytes);
    }
}

#[cfg(windows)]
fn working_set_bytes(process_id: u32) -> Option<u64> {
    use std::mem::size_of;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ},
        },
    };

    // PROCESS_QUERY_LIMITED_INFORMATION and PROCESS_VM_READ are sufficient
    // for the process's own working-set counters on supported Windows hosts.
    // SAFETY: OpenProcess is called with a valid PID supplied by the live
    // Harness child and no inherited handle is requested.
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            0,
            process_id,
        )
    };
    if handle.is_null() {
        return None;
    }

    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: `counters` is a writable PROCESS_MEMORY_COUNTERS value and its
    // declared byte size matches the structure passed to the Windows API.
    let succeeded = unsafe { K32GetProcessMemoryInfo(handle, &mut counters, counters.cb) != 0 };
    // SAFETY: `handle` was returned by OpenProcess and is closed exactly once.
    unsafe {
        CloseHandle(handle);
    }
    succeeded.then_some(counters.WorkingSetSize as u64)
}

#[cfg(not(windows))]
fn working_set_bytes(_process_id: u32) -> Option<u64> {
    None
}

struct ProbeDirectory {
    path: PathBuf,
}

impl ProbeDirectory {
    fn new() -> Result<Self> {
        let path = env::temp_dir().join(format!("{TEMP_PREFIX}{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).with_context(|| {
            format!(
                "unable to create probe temporary directory '{}'",
                path.display()
            )
        })?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ProbeDirectory {
    fn drop(&mut self) {
        // The UUID and fixed prefix make this guard's recursive cleanup
        // scoped to a directory created by this function.  Ignore cleanup
        // errors because the measurement result is already determined.
        let Ok(base) = env::temp_dir().canonicalize() else {
            return;
        };
        let Ok(resolved) = self.path.canonicalize() else {
            return;
        };
        if resolved.parent() == Some(base.as_path())
            && self
                .path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with(TEMP_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

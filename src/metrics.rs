//! Reproducible complete-host measurements with explicit application cache modes.
use crate::{engine::CommandIndex, probe::Harness};
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

#[derive(Default)]
struct Measurements {
    prompt: Vec<f64>,
    first: Vec<f64>,
    warm: Vec<f64>,
    prompt_memory: Vec<u64>,
    menu_memory: Vec<u64>,
}
impl Measurements {
    fn report(&self) -> Value {
        json!({
            "first_prompt":timing_stats(&self.prompt),
            "first_menu":timing_stats(&self.first),
            "warm_menu":timing_stats(&self.warm),
            "host_working_set_at_prompt":working_set_stats(&self.prompt_memory),
            "host_working_set_at_menu":working_set_stats(&self.menu_memory),
        })
    }
}

pub fn host_probe(executable: &Path, shell: &Path, iterations: u16) -> Result<Value> {
    host_probe_with_descriptions(executable, shell, iterations, true)
}

pub fn host_probe_with_descriptions(
    executable: &Path,
    shell: &Path,
    iterations: u16,
    descriptions: bool,
) -> Result<Value> {
    host_probe_traced(executable, shell, iterations, descriptions, None)
}

pub fn host_probe_traced(
    executable: &Path,
    shell: &Path,
    iterations: u16,
    descriptions: bool,
    trace_dir: Option<&Path>,
) -> Result<Value> {
    ensure!(
        (1..=MAX_ITERATIONS).contains(&iterations),
        "iterations must be between 1 and {MAX_ITERATIONS}"
    );
    let temporary = ProbeDirectory::new()?;
    let fixture_dir = temporary.path().join("fixtures");
    fs::create_dir_all(&fixture_dir)?;
    create_fixtures(&fixture_dir)?;
    let config_path = temporary.path().join("defaults.toml");
    // Keep this historical comparison fixture valid for the frozen 0.2 host.
    // New scenario probes separately exercise all current default features.
    fs::write(
        &config_path,
        format!(
            "[ui]\nwidth = 80\ndescriptions = {descriptions}\n[completion]\nmax_results = 100\nauto_trigger = true\n"
        ),
    )?;
    let path = fixture_path(&fixture_dir)?;
    let pathext = probe_pathext();
    let cwd = env::current_dir()?;
    let environment = probe_environment(&path, &pathext);
    let mut missing = Measurements::default();
    let mut cached = Measurements::default();
    for iteration in 0..iterations {
        // Each pair starts with an empty directory. The second run reuses only
        // a cache proven complete and valid for the exact fixture environment.
        let data_dir = temporary.path().join(format!("run-{iteration}"));
        fs::create_dir_all(&data_dir)?;
        for cache_hit in [false, true] {
            if cache_hit {
                let cache_path = data_dir.join("commands.json");
                let saved: Value = serde_json::from_slice(&fs::read(&cache_path)?)?;
                let child_path = saved["context"]["path"]
                    .as_str()
                    .context("cached child PATH")?;
                let child_pathext = saved["context"]["pathext"]
                    .as_str()
                    .context("cached child PATHEXT")?;
                ensure!(
                    env::split_paths(child_path).any(|p| p == fixture_dir),
                    "fixture directory must remain on the child PATH"
                );
                CommandIndex::load_with_env(
                    &cache_path,
                    OsStr::new(child_path),
                    OsStr::new(child_pathext),
                )
                .context("warm-cache sample requires a valid completed command index")?;
            }
            let cache_modified = if cache_hit {
                Some(fs::metadata(data_dir.join("commands.json"))?.modified()?)
            } else {
                None
            };
            let measured = if cache_hit { &mut cached } else { &mut missing };
            let mut args = host_args(&config_path, shell, &data_dir);
            if let Some(directory) = trace_dir {
                fs::create_dir_all(directory)?;
                let trace_path = directory.join(format!(
                    "{}-{iteration}.jsonl",
                    if cache_hit { "hit" } else { "miss" }
                ));
                args.extend(["--trace".into(), trace_path.to_string_lossy().into_owned()]);
            }
            let started = Instant::now();
            let mut harness = Harness::start(
                executable,
                &args,
                &cwd,
                &environment,
                uuid::Uuid::new_v4().to_string(),
            )?;
            harness.wait_text("PS ", WAIT_TIMEOUT)?;
            measured.prompt.push(milliseconds(started.elapsed()));
            push_working_set(&mut measured.prompt_memory, harness.process_id());
            let started = Instant::now();
            harness.send(b"ss-ben0")?;
            wait_menu(&mut harness, "ss-ben0", "ss-ben0-complete")?;
            measured.first.push(milliseconds(started.elapsed()));
            for index in 1..=10 {
                let suffix = index % FIXTURE_COUNT;
                let prefix = format!("{FIXTURE_PREFIX}{suffix}");
                let started = Instant::now();
                harness.send(format!("\x7f{suffix}").as_bytes())?;
                wait_menu(&mut harness, &prefix, &format!("{prefix}-complete"))?;
                measured.warm.push(milliseconds(started.elapsed()));
            }
            push_working_set(&mut measured.menu_memory, harness.process_id());
            harness.finish(Duration::from_secs(5))?;
            if let Some(before) = cache_modified {
                ensure!(
                    fs::metadata(data_dir.join("commands.json"))?.modified()? == before,
                    "cache-hit sample unexpectedly rewrote the command index"
                );
            }
        }
    }
    Ok(json!({
        "schema":2, "build":if cfg!(debug_assertions) {"debug"} else {"release"},
        "platform":env::consts::OS,"arch":env::consts::ARCH,"shell":shell,
        "executable":executable,"iterations_per_cache_mode":iterations,
        "warm_queries_per_session":10,"descriptions":descriptions,
        "cache_miss":missing.report(),"cache_hit":cached.report(),
        "method":{
            "host":"fresh native host via an outer ConPTY, pwsh --no-profile in its inner ConPTY",
            "first_prompt":"from process start to visible PS prompt; not yet a ready-to-type measurement",
            "first_menu":"send ss-ben0 and wait for a matching editable line and distinct candidate",
            "warm_menu":"ten prefix edits per session; wait for matching editable line and distinct candidate",
            "cache":"new application data directory for miss; validated complete index and integration reused for hit; OS caches are not cleared",
            "memory":"own-process WorkingSetSize, excluding pwsh/ConHost/Windows Terminal",
            "statistics":"median averages the middle pair for even samples; p95 uses nearest-rank",
            "trace":if trace_dir.is_some() {"enabled; diagnostic run, not an acceptance sample"} else {"disabled"}
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
        "median": (sorted[(sorted.len()-1)/2] as f64 + sorted[sorted.len()/2] as f64)/2.0,
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
        "median": (sorted[(sorted.len()-1)/2] + sorted[sorted.len()/2])/2.0,
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

#[cfg(test)]
mod statistics_tests {
    use super::*;
    #[test]
    fn even_samples_use_average_median_and_nearest_rank_p95() {
        let report = timing_stats(&[3.0, 1.0]);
        assert_eq!(report["median"], 2.0);
        assert_eq!(report["p95"], 3.0);
        assert_eq!(p95_index(100), 94);
        let memory = working_set_stats(&[4, 2]);
        assert_eq!(memory["median"], 3.0);
    }
}

#![cfg(windows)]

use anyhow::{Result, ensure};
use blueberry::{probe::Harness, pty};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};

// Explicit release-only measurement; ordinary regression runs never benchmark.
#[test]
#[ignore = "run explicitly with --release and BLUEBERRY_METRICS_OUTPUT"]
fn direct_and_profile_hook_startup() -> Result<()> {
    ensure!(!cfg!(debug_assertions), "Use cargo test --release");
    let output = PathBuf::from(std::env::var("BLUEBERRY_METRICS_OUTPUT")?);
    let shell = pty::default_shell();
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_blueberry"));
    let directory = tempfile::tempdir()?;
    let profile = directory.path().join("profile.ps1");
    let configured = std::process::Command::new(&executable)
        .args(["startup", "enable", "--profile"])
        .arg(&profile)
        .status()?;
    ensure!(configured.success(), "Prepare isolated startup hook");
    // Simulate ConsoleHost loading the hook without writing the user's actual
    // profile. -Command is solely the harness entry; all other guards remain.
    let hook = std::fs::read_to_string(&profile)?;
    let start = hook.find("$blueberryAutoScript =").expect("managed guard");
    let end = start + hook[start..].find("if ($Host.Name").expect("managed entry");
    let hook = format!(
        "{}$blueberryAutoScript = $false\n{}",
        &hook[..start],
        &hook[end..]
    );
    std::fs::write(&profile, hook)?;
    let mut json_initialization = Vec::new();
    let mut measured_sessions = BTreeSet::new();
    let mut direct = Vec::new();
    let mut automatic = Vec::new();
    for pair in 0..10 {
        for auto in if pair % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let token = format!("blueberry-metric-{}", uuid::Uuid::new_v4());
            let environment = BTreeMap::from([
                ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
                ("BLUEBERRY_ACTIVE".into(), "0".into()),
                ("BLUEBERRY_PROBE_TOKEN".into(), token.clone()),
                (
                    "APPDATA".into(),
                    directory
                        .path()
                        .join("config")
                        .to_string_lossy()
                        .into_owned(),
                ),
                (
                    "LOCALAPPDATA".into(),
                    directory
                        .path()
                        .join("local")
                        .to_string_lossy()
                        .into_owned(),
                ),
            ]);
            let args = if auto {
                vec![
                    "-NoLogo".into(),
                    "-NoProfile".into(),
                    "-NoExit".into(),
                    "-ExecutionPolicy".into(),
                    "Bypass".into(),
                    "-Command".into(),
                    format!(". '{}'", profile.display().to_string().replace('\'', "''")),
                ]
            } else {
                vec![
                    "run".into(),
                    "--shell".into(),
                    shell.to_string_lossy().into_owned(),
                ]
            };
            let started = Instant::now();
            let mut host = Harness::start(
                if auto { &shell } else { &executable },
                &args,
                directory.path(),
                &environment,
                token,
            )?;
            host.send(b"Write-Output BB_RELEASE_READY\r")?;
            let deadline = Instant::now() + Duration::from_secs(30);
            while !host
                .contents()
                .lines()
                .any(|line| line.trim() == "BB_RELEASE_READY")
            {
                host.pump(deadline.saturating_duration_since(Instant::now()))?;
            }
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            let cache = directory.path().join("local/Blueberry/cache");
            for entry in std::fs::read_dir(&cache)? {
                let status = entry?.path().join("adapter.json");
                if measured_sessions.contains(&status) {
                    continue;
                }
                if let Ok(bytes) = std::fs::read(&status) {
                    let metadata: serde_json::Value = serde_json::from_slice(&bytes)?;
                    if let Some(ms) = metadata["json_initialization_ms"].as_f64() {
                        json_initialization.push(ms);
                    }
                    measured_sessions.insert(status);
                }
            }
            host.stop()?;
            if auto {
                automatic.push(elapsed)
            } else {
                direct.push(elapsed)
            }
        }
    }
    fn summary(values: &[f64]) -> serde_json::Value {
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        json!({"samples_ms": values, "p50_ms": (sorted[4]+sorted[5])/2.0, "p95_ms": sorted[9]})
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&json!({
            "shell": shell, "version": env!("CARGO_PKG_VERSION"),
            "method": "10 alternating pairs; process spawn to first executed marker; isolated outer profile hook; child loads normal profile; OS caches retained; no history writes; exploratory, not formal performance acceptance",
            "direct": summary(&direct), "profile_hook": summary(&automatic), "json_initialization_samples_ms": json_initialization
        }))?,
    )?;
    Ok(())
}

#![cfg(windows)]

use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{fs, process::Command};

#[test]
fn startup_probe_distinguishes_isolated_cold_and_warm_metadata() -> Result<()> {
    let root = std::env::var_os("RUNNER_TEMP")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("blueberry-cache-probe-evidence");
    fs::create_dir_all(&root)?;
    let mut fixture = tempfile::Builder::new().prefix("probe-").tempdir_in(root)?;
    fixture.disable_cleanup(true);
    for (mode, expected) in [("cold", "rebuild"), ("warm", "hit")] {
        let report = fixture.path().join(format!("{mode}.json"));
        let mut probe = Command::new(env!("CARGO_BIN_EXE_blueberry"));
        let traces = fixture.path().join("traces");
        probe
            .args([
                "product-probe",
                "--cache-state",
                mode,
                "--iterations",
                "2",
                "--shell",
            ])
            .arg(blueberry::pty::default_shell())
            .arg("--output")
            .arg(&report)
            .env("POWERSHELL_UPDATECHECK", "Off");
        if mode == "cold" {
            probe.arg("--diagnostic-trace-directory").arg(&traces);
        }
        let output = probe.output()?;
        fs::write(
            fixture.path().join(format!("{mode}.stdout")),
            &output.stdout,
        )?;
        fs::write(
            fixture.path().join(format!("{mode}.stderr")),
            &output.stderr,
        )?;
        ensure!(
            output.status.success(),
            "{mode} probe failed; evidence: {}",
            fixture.path().display()
        );
        let value: Value = serde_json::from_slice(&fs::read(report)?)?;
        if mode == "cold" {
            ensure!(value["diagnostic_only"] == true);
            for (index, pair) in value["startup_qpc"]
                .as_array()
                .context("paired QPC observations")?
                .iter()
                .enumerate()
            {
                let events: Vec<Value> =
                    fs::read_to_string(traces.join(format!("direct-product-{index}.jsonl")))?
                        .lines()
                        .map(serde_json::from_str)
                        .collect::<Result<_, _>>()?;
                let tick = |name| {
                    events
                        .iter()
                        .find(|e| e["event"] == name)
                        .and_then(|e| e["qpc"].as_i64())
                        .with_context(|| format!("missing {name}"))
                };
                let ordered = [
                    "blueberry_process_entry",
                    "run_maintenance_begin",
                    "run_maintenance_end",
                    "run_options_ready",
                    "direct_start",
                    "direct_spawn_begin",
                    "child_environment_published",
                    "direct_environment_received",
                    "worker_index_started",
                    "worker_index_finished",
                ]
                .map(tick)
                .into_iter()
                .collect::<Result<Vec<_>>>()?;
                ensure!(
                    ordered.windows(2).all(|w| w[0] <= w[1]),
                    "unordered startup trace"
                );
                for side in ["plain", "candidate"] {
                    let points = &pair[side];
                    let observed = ["launch", "prompt", "key_sent", "echo"]
                        .map(|key| points[key].as_i64().context("missing VT/launch timestamp"))
                        .into_iter()
                        .collect::<Result<Vec<_>>>()?;
                    ensure!(observed.windows(2).all(|w| w[0] <= w[1]));
                }
                ensure!(
                    pair["candidate"]["complete_static"]
                        .as_i64()
                        .context("complete")?
                        >= pair["candidate"]["visible_static"]
                            .as_i64()
                            .context("visible")?
                );
            }
        } else {
            ensure!(value.get("startup_qpc").is_none());
        }
        ensure!(value["local_cache_state"] == mode && value["local_appdata_isolated"] == true);
        let samples = value["system_metadata_observations"]
            .as_array()
            .context("cache observations")?;
        ensure!(samples.len() == 2);
        ensure!(
            samples
                .iter()
                .all(|s| s["cache"] == expected && s["complete"] == true && s["error"].is_null())
        );
        if mode == "warm" {
            ensure!(value["local_cache_preparation"]["system_commands"]["cache"] == "rebuild");
            ensure!(value["local_cache_preparation"]["included_in_timed_pairs"] == false);
        } else {
            ensure!(value["local_cache_preparation"].is_null());
        }
    }
    Ok(())
}

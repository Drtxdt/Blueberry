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
        let output = Command::new(env!("CARGO_BIN_EXE_blueberry"))
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
            .env("POWERSHELL_UPDATECHECK", "Off")
            .output()?;
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

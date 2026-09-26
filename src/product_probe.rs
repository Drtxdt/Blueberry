//! Paired startup measurements of the actual product, not just its adapter.
//! The observation is a VT model fed by a timestamped ConPTY reader. It is
//! explicitly not the time at which physical display pixels change.
use crate::{beta_metrics, metrics, probe::Harness};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
const TIMEOUT: Duration = Duration::from_secs(20);

fn shell_metadata(screen: &str) -> Option<Value> {
    screen.lines().map(str::trim).find_map(|line| {
        let fields = line.split(':').collect::<Vec<_>>();
        if fields.len() != 4
            || fields[0] != "BB_PRODUCT_META"
            || fields[3] != "BB_PRODUCT_META_END"
            || !fields[1..3].iter().all(|version| {
                version.as_bytes().first().is_some_and(u8::is_ascii_digit)
                    && version
                        .bytes()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'-' | b'+'))
            })
        {
            return None;
        }
        Some(json!({"shell_version":fields[1],"psreadline_version":fields[2]}))
    })
}

pub(crate) fn environment(shell: &Path) -> Result<Value> {
    let output=Command::new(shell).args(["-NoProfile","-NonInteractive","-Command",
        "[Console]::WriteLine((@($PROFILE.AllUsersAllHosts,$PROFILE.AllUsersCurrentHost,$PROFILE.CurrentUserAllHosts,$PROFILE.CurrentUserCurrentHost) | ConvertTo-Json -Compress))"])
        .output().context("resolve actual ConsoleHost profile paths")?;
    ensure!(output.status.success(), "profile inventory failed");
    let paths: Vec<String> = serde_json::from_slice(&output.stdout)?;
    let mut hash = Sha256::new();
    let mut inventory = Vec::new();
    for path in paths {
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        match std::fs::read(&path) {
            Ok(bytes) => {
                hash.update([1]);
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(&bytes);
                inventory
                    .push(json!({"path":path,"sha256":format!("{:X}",Sha256::digest(&bytes))}));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hash.update([0]);
                inventory.push(json!({"path":path,"sha256":null}));
            }
            Err(error) => return Err(error).with_context(|| format!("read profile {path}")),
        }
    }
    let power = Command::new("powercfg.exe")
        .arg("/GETACTIVESCHEME")
        .output()
        .context("read power policy")?;
    ensure!(power.status.success(), "power policy query failed");
    // powercfg uses the console code page. Preserve the bytes as a digest as
    // well as a readable label; code-page replacement cannot hide a change.
    Ok(
        json!({"machine_id":std::env::var("COMPUTERNAME").context("missing machine ID")?,
        "power_policy":format!("{:X}",Sha256::digest(&power.stdout)),
        "power_policy_label":String::from_utf8_lossy(&power.stdout).trim(),
        "profile_sha256":format!("{:X}",hash.finalize()),"profile_inventory":inventory,
        "terminal_rows":30,"terminal_columns":120}),
    )
}

fn stats(values: &[f64]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({"samples":values,"median":(sorted[(sorted.len()-1)/2]+sorted[sorted.len()/2])/2.0,
        "p95":sorted[(sorted.len()*95).div_ceil(100)-1],"unit":"ms"})
}
fn elapsed(harness: &Harness, start: Instant) -> Result<f64> {
    let arrival = harness
        .last_output_arrival()
        .context("observer has no output arrival timestamp")?;
    ensure!(arrival >= start, "stale observer bytes precede sample");
    Ok(arrival.duration_since(start).as_secs_f64() * 1000.0)
}
fn wait(harness: &mut Harness, description: &str, predicate: impl Fn(&str) -> bool) -> Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let screen = harness.viewport_contents();
        if predicate(&screen) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("{description}: {screen}");
        }
        let _ = harness.pump(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(25)),
        );
    }
}
fn adapter(directory: &Path) -> Result<Value> {
    for entry in std::fs::read_dir(directory)?.flatten() {
        if let Ok(bytes) = std::fs::read(entry.path().join("adapter.json")) {
            return Ok(serde_json::from_slice(&bytes)?);
        }
    }
    bail!("actual product did not publish adapter status")
}
struct Sample {
    first_input: f64,
    first_key: f64,
    first_static: Option<f64>,
    first_dynamic: Option<f64>,
    status: Value,
}

fn measure(
    executable: &Path,
    shell: &Path,
    directory: &Path,
    candidate: bool,
    host_mode: &str,
    trace_path: Option<&Path>,
) -> Result<Sample> {
    let mut env = BTreeMap::from([
        ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
        // Suppress a profile's Blueberry autostart in both paired shells.
        // Otherwise "plain" could silently contain another nested host.
        ("BLUEBERRY_ACTIVE".into(), "1".into()),
        ("TERM".into(), "xterm-256color".into()),
    ]);
    env.insert(
        "APPDATA".into(),
        directory
            .join("isolated-appdata")
            .to_string_lossy()
            .into_owned(),
    );
    // Profiles are resolved by PowerShell, independently of the data/history
    // directory. Both modes receive the exact same environment overrides.
    let mut args = if candidate {
        vec![
            "run".into(),
            "--host-mode".into(),
            host_mode.into(),
            "--transport".into(),
            "pipe".into(),
            "--shell".into(),
            shell.to_string_lossy().into_owned(),
            "--data-dir".into(),
            directory.join("data").to_string_lossy().into_owned(),
        ]
    } else {
        let import = std::env::var("BLUEBERRY_TEST_PSREADLINE_MODULE")
            .ok()
            .map(|path| {
                format!(
                    "Import-Module '{}' -ErrorAction Stop; ",
                    path.replace('\'', "''")
                )
            })
            .unwrap_or_default();
        vec![
            "-NoLogo".into(),
            "-NoExit".into(),
            "-Command".into(),
            format!(
                "{import}Import-Module PSReadLine; Set-PSReadLineOption -HistorySaveStyle SaveNothing"
            ),
        ]
    };
    if candidate && let Some(path) = trace_path {
        args.extend(["--trace".into(), path.to_string_lossy().into_owned()]);
    }
    let start = Instant::now();
    let mut harness = Harness::start(
        if candidate { executable } else { shell },
        &args,
        directory,
        &env,
        String::new(),
    )?;
    beta_metrics::wait_for_prompt(&mut harness, TIMEOUT)?;
    let before = harness.viewport_contents();
    let prompt = before.lines().last().unwrap_or("").trim_end().to_owned();
    let key_start = Instant::now();
    harness.send(b"g")?;
    wait(&mut harness, "first PSReadLine-confirmed echo", |screen| {
        screen.lines().any(|line| {
            line.trim_end()
                .strip_prefix(&prompt)
                .is_some_and(|tail| tail.trim() == "g")
        })
    })?;
    let first_input = elapsed(&harness, start)?;
    let first_key = elapsed(&harness, key_start)?;
    let (first_static, first_dynamic, status) = if candidate {
        wait(&mut harness, "first complete static menu", |screen| {
            beta_metrics::has_complete_status(screen)
                && screen.lines().any(|line| line.contains("› "))
        })?;
        let first_static = elapsed(&harness, key_start)?;
        let status = adapter(&directory.join("data"))?;
        ensure!(
            status["transport"] == "pipe",
            "actual product transport degraded"
        );
        if host_mode == "direct" {
            ensure!(
                status["host_mode"] == "direct" && status["automatic_menu"] == true,
                "direct automatic menu unavailable: {status}"
            );
        }
        harness.send(b"\x01\x7f")?;
        wait(&mut harness, "clear first query", |screen| {
            !screen.contains("› ")
        })?;
        let dynamic_start = Instant::now();
        harness.send(b"cd bb-product-probe-dir")?;
        wait(&mut harness, "first complete dynamic menu", |screen| {
            beta_metrics::has_complete_status(screen)
                && screen
                    .lines()
                    .any(|line| line.contains("bb-product-probe-dir") && line.contains("› "))
        })?;
        let first_dynamic = elapsed(&harness, dynamic_start)?;
        harness.finish(TIMEOUT)?;
        (Some(first_static), Some(first_dynamic), status)
    } else {
        harness.send(b"\x01\x7f")?;
        harness.send(b"Write-Output ('BB_PRODUCT_META:' + $PSVersionTable.PSVersion.ToString() + ':' + (Get-Module PSReadLine).Version.ToString() + ':BB_PRODUCT_META_END')\r")?;
        wait(&mut harness, "plain version metadata", |screen| {
            shell_metadata(screen).is_some()
        })?;
        let contents = harness.viewport_contents();
        let status = shell_metadata(&contents).context("missing complete plain metadata")?;
        harness.finish(TIMEOUT)?;
        (None, None, status)
    };
    Ok(Sample {
        first_input,
        first_key,
        first_static,
        first_dynamic,
        status,
    })
}

pub fn run(executable: &Path, shell: &Path, iterations: u16, host_mode: &str) -> Result<Value> {
    run_with_trace(executable, shell, iterations, host_mode, None)
}
pub fn run_with_trace(
    executable: &Path,
    shell: &Path,
    iterations: u16,
    host_mode: &str,
    trace_directory: Option<&Path>,
) -> Result<Value> {
    ensure!(
        iterations > 0 && matches!(host_mode, "direct" | "nested"),
        "invalid product probe options"
    );
    let frozen = environment(shell)?;
    if let Some(path) = trace_directory {
        std::fs::create_dir_all(path)?;
    }
    let identity = metrics::build_identity(executable)?;
    let executable_hash = metrics::executable_sha256(executable)?;
    let root =
        std::env::temp_dir().join(format!("blueberry-product-probe-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root)?;
    let mut plain = Vec::new();
    let mut candidate = Vec::new();
    let mut delta = Vec::new();
    let mut keys = Vec::new();
    let mut static_menus = Vec::new();
    let mut dynamic_menus = Vec::new();
    let mut shells = Vec::new();
    let mut psreadline = Vec::new();
    let mut transports = Vec::new();
    let mut hosts = Vec::new();
    let mut automatic_menus = Vec::new();
    let mut orders = Vec::new();
    for index in 0..iterations {
        let directory = root.join(format!("pair-{index}"));
        std::fs::create_dir_all(directory.join("bb-product-probe-dir"))?;
        let order = if index % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        };
        orders.push(order.map(|candidate| if candidate { "candidate" } else { "plain" }));
        let mut baseline = None;
        let mut product = None;
        for mode in order {
            let trace_path =
                trace_directory.map(|root| root.join(format!("direct-product-{index}.jsonl")));
            let result = measure(
                executable,
                shell,
                &directory,
                mode,
                host_mode,
                trace_path.as_deref(),
            )
            .with_context(|| {
                format!("pair {index}, {}", if mode { "candidate" } else { "plain" })
            })?;
            if mode {
                product = Some(result);
            } else {
                baseline = Some(result);
            }
        }
        let baseline = baseline.unwrap();
        let product = product.unwrap();
        ensure!(
            baseline.status["shell_version"] == product.status["shell_version"],
            "paired Shell versions differ: plain={}, candidate={}",
            baseline.status["shell_version"],
            product.status["shell_version"]
        );
        ensure!(
            product.status["psreadline_version"]
                .as_str()
                .is_some_and(|version| version.trim_end_matches(".0")
                    == baseline.status["psreadline_version"]
                        .as_str()
                        .unwrap_or("")
                        .trim_end_matches(".0")),
            "paired PSReadLine versions differ"
        );
        delta.push(product.first_input - baseline.first_input);
        plain.push(baseline.first_input);
        candidate.push(product.first_input);
        keys.push(product.first_key);
        static_menus.push(product.first_static.unwrap());
        dynamic_menus.push(product.first_dynamic.unwrap());
        shells.push(product.status["shell_version"].clone());
        psreadline.push(product.status["psreadline_version"].clone());
        transports.push(product.status["transport"].clone());
        hosts.push(product.status["host_mode"].clone());
        automatic_menus.push(product.status["automatic_menu"].clone());
    }
    ensure!(
        environment(shell)? == frozen,
        "environment changed: whole batch invalidated; retain scratch at {}",
        root.display()
    );
    ensure!(
        metrics::executable_sha256(executable)? == executable_hash,
        "measured EXE changed during batch"
    );
    let mut report = json!({"schema":3,"measurement":"complete_product","adapter_source":"embedded","iterations":iterations,
        "source_commit":identity["commit"],"source_dirty":identity["dirty"],"build":identity["profile"],
        "platform":std::env::consts::OS,"arch":std::env::consts::ARCH,"shell":shell,"host_mode":host_mode,
        "executable_sha256":executable_hash,"profile_mode":"with_profile","no_profile":false,
        "trace":if trace_directory.is_some(){"enabled"}else{"disabled"},"trace_directory":trace_directory,
        "os_cache_cleared":false,"index_cache":"new directory per pair","scratch_directory":root,
        "profile_autostart":"suppressed identically with BLUEBERRY_ACTIVE=1",
        "observation":"VT observation formed from receive-thread timestamps; physical screen pixels are not measured",
        "startup_orders":orders,"plain_first_input":stats(&plain),"candidate_first_input":stats(&candidate),
        "paired_first_input_delta":stats(&delta),"first_key_echo":stats(&keys),"first_static_candidate":stats(&static_menus),
        "first_dynamic_candidate":stats(&dynamic_menus),"shell_versions":shells,"psreadline_versions":psreadline,
        "actual_transports":transports,"actual_host_modes":hosts,"actual_automatic_menu":automatic_menus});
    report
        .as_object_mut()
        .unwrap()
        .extend(frozen.as_object().unwrap().clone());
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_rejects_echo_fragments_and_incomplete_output() {
        for echo in [
            "BB_PRODUCT_META:5.1",
            "BB_PRODUCT_META:' + $PSVersionTable.PSVersion.ToString() + ':2.4.5:BB_PRODUCT_META_END",
        ] {
            assert!(shell_metadata(echo).is_none());
        }
        let screen = "BB_PRODUCT_META:' + $PSVersionTable.PSVersion.ToString() + ':2.4.5:BB_PRODUCT_META_END\nBB_PRODUCT_META:5.1.26100.9444:2.4.5:BB_PRODUCT_META_END\nPS >";
        assert_eq!(
            shell_metadata(screen).unwrap()["shell_version"],
            "5.1.26100.9444"
        );
    }
}

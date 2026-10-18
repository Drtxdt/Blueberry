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
        let screen = harness.observed_viewport()?;
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
    first_visible_static: Option<f64>,
    first_visible_dynamic: Option<f64>,
    status: Value,
}

fn first_key_echo(screen: &str, prompt: &str) -> bool {
    if prompt.is_empty() {
        return false;
    }
    let lines: Vec<_> = screen.lines().collect();
    lines.iter().enumerate().any(|(index, line)| {
        let Some(tail) = line.trim_end().strip_prefix(prompt) else {
            return false;
        };
        tail.trim() == "g"
            || (tail.trim().is_empty()
                && lines.get(index + 1).is_some_and(|next| next.trim() == "g"))
    })
}

fn measure(
    executable: &Path,
    shell: &Path,
    directory: &Path,
    candidate: bool,
    host_mode: &str,
    trace_path: Option<&Path>,
    profile_entry: bool,
) -> Result<Sample> {
    let mut env = BTreeMap::from([
        ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
        // Suppress a profile's Blueberry autostart in both paired shells.
        // Otherwise "plain" could silently contain another nested host.
        ("BLUEBERRY_ACTIVE".into(), "1".into()),
        ("TERM".into(), "xterm-256color".into()),
    ]);
    crate::probe::inherit_native_observer_environment(&mut env);
    if profile_entry && candidate {
        env.insert("BLUEBERRY_ACTIVE".into(), String::new());
        env.insert(
            "BLUEBERRY_PROFILE_PROBE_DATA".into(),
            directory.join("data").to_string_lossy().into_owned(),
        );
    }
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
    if candidate && profile_entry {
        args = vec!["-NoLogo".into()];
    }
    let start = Instant::now();
    let mut harness = Harness::start(
        if candidate && !profile_entry {
            executable
        } else {
            shell
        },
        &args,
        directory,
        &env,
        String::new(),
    )?;
    let start = harness.observation_start(start);
    beta_metrics::wait_for_prompt(&mut harness, TIMEOUT)?;
    let before = harness.observed_viewport()?;
    let prompt = before.lines().last().unwrap_or("").trim_end().to_owned();
    let key_start = Instant::now();
    harness.send_text("g")?;
    wait(&mut harness, "first PSReadLine-confirmed echo", |screen| {
        first_key_echo(screen, &prompt)
    })?;
    let first_input = elapsed(&harness, start)?;
    let first_key = elapsed(&harness, key_start)?;
    let mut first_visible_static = None;
    let mut first_visible_dynamic = None;
    let (first_static, first_dynamic, mut status) = if candidate {
        wait(&mut harness, "first visible static candidate", |screen| {
            screen.lines().any(|line| line.contains("› "))
        })?;
        first_visible_static = Some(elapsed(&harness, key_start)?);
        wait(&mut harness, "first complete static menu", |screen| {
            beta_metrics::has_complete_status(screen)
                && screen.lines().any(|line| line.contains("› "))
        })?;
        let first_static = elapsed(&harness, key_start)?;
        let mut status = adapter(&directory.join("data"))?;
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
        #[cfg(windows)]
        if host_mode == "nested" {
            let expected = crate::conpty::build_identity();
            ensure!(
                status["conpty"]["mode"] == "pinned"
                    && status["conpty"]["sha256"] == expected["sha256"]
                    && status["conpty"]["files"] == expected["files"],
                "Nested product ConPTY identity mismatch: {}",
                status["conpty"]
            );
        }
        harness.send(b"\x01\x7f")?;
        wait(&mut harness, "clear first query", |screen| {
            !screen.contains("› ")
        })?;
        let dynamic_start = Instant::now();
        harness.send_text("cd bb-product-probe-dir")?;
        wait(&mut harness, "first visible dynamic candidate", |screen| {
            screen
                .lines()
                .any(|line| line.contains("bb-product-probe-dir") && line.contains("› "))
        })?;
        first_visible_dynamic = Some(elapsed(&harness, dynamic_start)?);
        wait(&mut harness, "first complete dynamic menu", |screen| {
            beta_metrics::has_complete_status(screen)
                && screen
                    .lines()
                    .any(|line| line.contains("bb-product-probe-dir") && line.contains("› "))
        })?;
        let first_dynamic = elapsed(&harness, dynamic_start)?;
        status["process_tree_after_timing"] = json!(
            harness
                .process_id()
                .and_then(crate::process_metrics::snapshot_tree)
        );
        harness.finish(TIMEOUT)?;
        (Some(first_static), Some(first_dynamic), status)
    } else {
        harness.send(b"\x01\x7f")?;
        harness.send_text("Write-Output ('BB_PRODUCT_META:' + $PSVersionTable.PSVersion.ToString() + ':' + (Get-Module PSReadLine).Version.ToString() + ':BB_PRODUCT_META_END')\r")?;
        wait(&mut harness, "plain version metadata", |screen| {
            shell_metadata(screen).is_some()
        })?;
        let contents = harness.observed_viewport()?;
        let mut status = shell_metadata(&contents).context("missing complete plain metadata")?;
        // Collect the actually loaded assembly after all timed observations.
        // Windows inbox modules can share a version label with a different
        // upstream binary, so the label alone is not a reproducible baseline.
        let metadata_path = directory.join("plain-editor.txt");
        let command = format!(
            "[IO.File]::WriteAllLines('{}', [string[]]@((Get-Module PSReadLine).Path,[Microsoft.PowerShell.PSConsoleReadLine].Assembly.Location,[Microsoft.PowerShell.PSConsoleReadLine].Assembly.FullName,[BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([IO.File]::ReadAllBytes([Microsoft.PowerShell.PSConsoleReadLine].Assembly.Location))).Replace('-',''))); Write-Output 'BB_PLAIN_IDENTITY_DONE'\r",
            metadata_path.to_string_lossy().replace('\'', "''")
        );
        harness.send_text(&command)?;
        wait(&mut harness, "plain assembly identity", |screen| {
            screen
                .lines()
                .any(|line| line.trim() == "BB_PLAIN_IDENTITY_DONE")
        })?;
        let metadata = std::fs::read_to_string(&metadata_path).with_context(|| {
            format!(
                "read plain identity {}: {}",
                metadata_path.display(),
                harness.viewport_contents()
            )
        })?;
        let fields = metadata.lines().collect::<Vec<_>>();
        ensure!(
            fields.len() == 4
                && fields[3].len() == 64
                && fields[3].bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid plain DLL identity"
        );
        status["editor"] = json!({"module_path":fields[0],"dll_path":fields[1],"assembly":fields[2],"dll_sha256":fields[3]});
        status["process_tree_after_timing"] = json!(
            harness
                .process_id()
                .and_then(crate::process_metrics::snapshot_tree)
        );
        harness.finish(TIMEOUT)?;
        (None, None, status)
    };
    status["native_observer"] = json!(harness.native_observer_identity());
    Ok(Sample {
        first_input,
        first_key,
        first_static,
        first_dynamic,
        first_visible_static,
        first_visible_dynamic,
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
    run_with_entry(
        executable,
        shell,
        iterations,
        host_mode,
        trace_directory,
        false,
    )
}

pub fn run_with_entry(
    executable: &Path,
    shell: &Path,
    iterations: u16,
    host_mode: &str,
    trace_directory: Option<&Path>,
    profile_entry: bool,
) -> Result<Value> {
    ensure!(
        iterations > 0 && matches!(host_mode, "direct" | "nested"),
        "invalid product probe options"
    );
    let entry_fixture = if profile_entry {
        ensure!(
            host_mode == "direct" && trace_directory.is_none(),
            "profile entry requires Direct and cannot add diagnostic trace arguments"
        );
        let home = shell.parent().context("profile entry shell directory")?;
        let fixture: Value =
            serde_json::from_slice(&std::fs::read(home.join("blueberry-profile-probe.json"))?)?;
        ensure!(
            fixture["executable_sha256"] == metrics::executable_sha256(executable)?,
            "profile entry executable changed"
        );
        ensure!(
            fixture["shell_sha256"] == metrics::executable_sha256(shell)?,
            "profile entry shell changed"
        );
        ensure!(
            fixture["profile_sha256"]
                == metrics::executable_sha256(&home.join("Microsoft.PowerShell_profile.ps1"))?,
            "profile entry profile changed"
        );
        Some(fixture)
    } else {
        None
    };
    let frozen = environment(shell)?;
    // Probe setup is outside both members of every pair and reported separately.
    // Nested product setup remains inside the measured child process lifetime.
    let runtime_start = Instant::now();
    crate::conpty::ensure_loaded()?;
    let probe_runtime_prepare_ms = runtime_start.elapsed().as_secs_f64() * 1000.0;
    let trace_path = trace_directory
        .map(|path| {
            std::fs::create_dir_all(path)?;
            std::fs::canonicalize(path)
        })
        .transpose()?;
    let trace_directory = trace_path.as_deref();
    let identity = metrics::build_identity(executable)?;
    ensure!(
        trace_directory.is_some()
            || (!cfg!(feature = "allocation-diagnostics")
                && identity["allocation_diagnostics"] != true),
        "allocation diagnostics require a separate traced batch, not formal timing"
    );
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
    let mut visible_static_menus = Vec::new();
    let mut visible_dynamic_menus = Vec::new();
    let mut shells = Vec::new();
    let mut psreadline = Vec::new();
    let mut transports = Vec::new();
    let mut hosts = Vec::new();
    let mut automatic_menus = Vec::new();
    let mut editors = Vec::new();
    let mut plain_editors = Vec::new();
    let mut observers = Vec::new();
    let mut process_trees = Vec::new();
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
                profile_entry,
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
            baseline.status["native_observer"] == product.status["native_observer"],
            "paired observer identities differ"
        );
        observers.push(product.status["native_observer"].clone());
        process_trees.push(json!({"plain":baseline.status["process_tree_after_timing"], "candidate":product.status["process_tree_after_timing"]}));
        if let Some(first) = plain_editors.first() {
            ensure!(
                first == &baseline.status["editor"],
                "plain editor identity changed during batch"
            );
        }
        plain_editors.push(baseline.status["editor"].clone());
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
        visible_static_menus.push(product.first_visible_static.unwrap());
        visible_dynamic_menus.push(product.first_visible_dynamic.unwrap());
        shells.push(product.status["shell_version"].clone());
        psreadline.push(product.status["psreadline_version"].clone());
        transports.push(product.status["transport"].clone());
        hosts.push(product.status["host_mode"].clone());
        automatic_menus.push(product.status["automatic_menu"].clone());
        editors.push(json!({"mode":product.status["editor_mode"],"patch":product.status["editor_patch"],
            "dll_sha256":product.status["editor_dll_sha256"],"fallback_reason":product.status["editor_fallback_reason"]}));
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
        "actual_transports":transports,"actual_host_modes":hosts,"actual_automatic_menu":automatic_menus,
        "editors":editors,"private_editors":identity["private_editors"],
        "product_conpty":identity["conpty"],"probe_conpty":crate::conpty::loaded_identity(),"probe_runtime_prepare_ms":probe_runtime_prepare_ms,
        "editor_eligible":host_mode!="direct" || editors.iter().all(|editor|editor["mode"]=="editor_hooks_v1")});
    report
        .as_object_mut()
        .unwrap()
        .extend(frozen.as_object().unwrap().clone());
    report["plain_editors"] = json!(plain_editors);
    report["actual_entry"] = json!(if profile_entry {
        "isolated_automatic_profile"
    } else {
        "direct_executable"
    });
    report["entry_fixture"] = json!(entry_fixture);
    if profile_entry {
        report["profile_autostart"] = json!(
            "candidate executes the generated managed block during automatic profile loading; plain suppresses it with BLUEBERRY_ACTIVE=1"
        );
    }
    // Retain historical fields (which required a complete menu) for existing
    // evidence readers; expose unambiguous first/complete observations too.
    report["first_visible_static_candidate"] = stats(&visible_static_menus);
    report["first_visible_dynamic_candidate"] = stats(&visible_dynamic_menus);
    report["complete_static_candidates"] = stats(&static_menus);
    report["complete_dynamic_candidates"] = stats(&dynamic_menus);
    report["diagnostic_only"] = json!(cfg!(debug_assertions) || trace_directory.is_some());
    report["probe_build_profile"] = json!(env!("BLUEBERRY_BUILD_PROFILE"));
    report["probe_source_commit"] = json!(env!("BLUEBERRY_BUILD_COMMIT"));
    report["probe_source_dirty"] = json!(env!("BLUEBERRY_BUILD_DIRTY") == "true");
    report["probe_executable_sha256"] =
        json!(metrics::executable_sha256(&std::env::current_exe()?)?);
    report["native_observers"] = json!(observers);
    report["process_trees_after_timing"] = json!(process_trees);
    report["process_tree_observation"] = json!(
        "untimed snapshot after candidate measurements; transient startup processes are not captured"
    );
    report["screen_pixels"] = json!("not_measured");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_echo_can_wrap_but_must_immediately_follow_the_actual_prompt() {
        let prompt = "PS fixture-pair-10>";
        assert!(first_key_echo("PS fixture-pair-10> g", prompt));
        assert!(first_key_echo("PS fixture-pair-10>  \n g  ", prompt));
        for screen in [
            "PS fixture-pair-10>\n\ng",
            "PS other-pair>\ng",
            "PS fixture-pair-10>\ngit",
            "PS fixture-pair-10>\n│› g",
            "PS fixture-pair-10>",
        ] {
            assert!(!first_key_echo(screen, prompt), "{screen}");
        }
        assert!(!first_key_echo("g", ""));
    }
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

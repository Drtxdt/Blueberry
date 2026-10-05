//! One auditable ConPTY session for external comparator measurements.
//! The orchestrator supplies frozen programs, environments and literal queries.
use anyhow::{Context, Result, bail, ensure};
use blueberry::probe::Harness;
use clap::Parser;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Parser)]
struct Options {
    #[arg(long)]
    session: PathBuf,
    #[arg(long)]
    output: PathBuf,
}
#[derive(Deserialize)]
struct Session {
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    environment: BTreeMap<String, String>,
    style: String,
    queries: Vec<Query>,
    #[serde(default)]
    metadata_path: Option<PathBuf>,
    #[serde(default)]
    resource_sampler: Option<PathBuf>,
    #[serde(default)]
    resource_output: Option<PathBuf>,
    #[serde(default)]
    data_dir: Option<PathBuf>,
    #[serde(default)]
    expected_host_mode: Option<String>,
}
#[derive(Deserialize)]
struct Query {
    line: String,
    expected: String,
    #[serde(default)]
    warmup: bool,
    #[serde(default)]
    echo_only: bool,
}
const TIMEOUT: Duration = Duration::from_secs(20);

fn prompt(screen: &str) -> bool {
    screen
        .lines()
        .any(|line| line.trim_end().ends_with(['>', '$', '', '❯', '❱', 'λ']))
}
fn menu(screen: &str, query: &Query, style: &str) -> bool {
    if style == "plain" {
        return true;
    }
    if style == "blueberry" {
        return screen
            .lines()
            .any(|row| row.contains(&query.expected) && row.contains("› "))
            && screen
                .lines()
                .any(|row| row.contains(" · F1 详情") && !row.contains("正在加载"));
    }
    // The fixed 0.0.4 renderer has a boxed icon/name column. Neither its
    // separate description box nor the echoed command is a candidate row.
    screen.lines().any(|row| {
        row.contains('│')
            && row.contains(&query.expected)
            && ["📦", "🔗", "💲", "📄", "📁", "⭐", "📀", "🔥", "🏝"]
                .iter()
                .any(|icon| row.contains(icon))
    })
}
fn wait(h: &mut Harness, predicate: impl Fn(&str) -> bool) -> Result<()> {
    let until = Instant::now() + TIMEOUT;
    loop {
        if predicate(&h.viewport_contents()) {
            return Ok(());
        }
        ensure!(Instant::now() < until, "timeout: {}", h.viewport_contents());
        let _ = h.pump(Duration::from_millis(25));
    }
}
fn ms(h: &Harness, start: Instant) -> Result<f64> {
    let arrival = h.last_output_arrival().context("no output arrival")?;
    ensure!(arrival >= start, "stale output");
    Ok(arrival.duration_since(start).as_secs_f64() * 1000.0)
}
fn clear(h: &mut Harness, line: &str) -> Result<()> {
    h.send(b"\x01\x7f")?;
    wait(h, |screen| !screen.contains(line) && !screen.contains('│'))
}
fn run(session: &Session, result: &mut Value) -> Result<()> {
    ensure!(
        matches!(
            session.style.as_str(),
            "plain" | "blueberry" | "inshellisense"
        ),
        "invalid renderer"
    );
    let old_adapters: Vec<_> = session
        .data_dir
        .as_ref()
        .into_iter()
        .flat_map(|directory| fs::read_dir(directory).into_iter().flatten().flatten())
        .map(|entry| entry.path().join("adapter.json"))
        .collect();
    let started = Instant::now();
    let mut h = Harness::start(
        &session.program,
        &session.args,
        &session.cwd,
        &session.environment,
        String::new(),
    )?;
    let run = (|| -> Result<()> {
        wait(&mut h, prompt)?;
        let before = h.viewport_contents();
        let prefix = before.lines().last().unwrap_or("").trim_end().to_owned();
        let key_start = Instant::now();
        h.send(b"g")?;
        wait(&mut h, |screen| {
            screen.lines().any(|row| {
                row.trim_end()
                    .strip_prefix(&prefix)
                    .is_some_and(|tail| tail.trim() == "g")
            })
        })?;
        result["startup"] = json!(ms(&h, started)?);
        result["first_key"] = json!(ms(&h, key_start)?);
        clear(&mut h, &format!("{prefix}g"))?;
        // Clearing a one-character startup query can leave asynchronous menu
        // output queued; all settling is outside measured samples.
        let until = Instant::now() + Duration::from_millis(250);
        while Instant::now() < until {
            let _ = h.pump(Duration::from_millis(25));
        }
        if let Some(path) = &session.metadata_path {
            ensure!(!path.exists(), "metadata file must be new");
            let command = format!(
                "[IO.File]::WriteAllText('{}',(@{{shell=$PSVersionTable.PSVersion.ToString();psreadline=(Get-Module PSReadLine).Version.ToString();dll=[Microsoft.PowerShell.PSConsoleReadLine].Assembly.Location;profiles=@($PROFILE.AllUsersAllHosts,$PROFILE.AllUsersCurrentHost,$PROFILE.CurrentUserAllHosts,$PROFILE.CurrentUserCurrentHost)}}|ConvertTo-Json -Compress)); Write-Output 'BB_COMPARATOR_META_DONE'\r",
                path.to_string_lossy().replace('\'', "''")
            );
            h.send(command.as_bytes())?;
            h.wait_line("BB_COMPARATOR_META_DONE", TIMEOUT)?;
            wait(&mut h, prompt)?;
            result["actual_shell"] = serde_json::from_slice(&fs::read(path)?)?;
            let dll = result["actual_shell"]["dll"]
                .as_str()
                .context("loaded editor DLL path missing")?;
            let dll_hash = format!("{:X}", Sha256::digest(fs::read(dll)?));
            result["actual_shell"]["dll_sha256"] = json!(dll_hash);
        }
        let adapter_path = if let Some(directory) = &session.data_dir {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(path) = fs::read_dir(directory)?
                    .flatten()
                    .map(|entry| entry.path().join("adapter.json"))
                    .find(|path| path.is_file() && !old_adapters.contains(path))
                {
                    let adapter: Value = serde_json::from_slice(&fs::read(&path)?)?;
                    if session.expected_host_mode.as_deref() == Some("direct") {
                        ensure!(
                            adapter["host_mode"] == "direct"
                                && adapter["editor_mode"] == "editor_hooks_v1"
                                && adapter["automatic_menu"] == true
                                && adapter["transport"] == "pipe",
                            "direct fallback: {adapter}"
                        );
                    } else {
                        ensure!(
                            adapter["transport"] == "osc" && adapter["ready"] == true,
                            "nested adapter not ready: {adapter}"
                        );
                    }
                    result["adapter"] = adapter;
                    break Some(path);
                }
                ensure!(Instant::now() < deadline, "missing live adapter identity");
                let _ = h.pump(Duration::from_millis(25));
            }
        } else {
            None
        };
        if let Some(sampler) = &session.resource_sampler {
            let output = session
                .resource_output
                .as_ref()
                .context("resource output missing")?;
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                let _ = h.pump(Duration::from_millis(25));
            }
            let status = std::process::Command::new("pwsh.exe")
                .args(["-NoProfile", "-NonInteractive", "-File"])
                .arg(sampler)
                .arg("-RootProcessId")
                .arg(h.process_id().context("host PID missing")?.to_string())
                .arg("-OutputPath")
                .arg(output)
                .status()?;
            ensure!(status.success(), "resource sampler failed");
            result["resources"] = serde_json::from_slice(&fs::read(output)?)?;
        }
        for query in &session.queries {
            ensure!(
                !query.echo_only || session.style == "inshellisense",
                "echo-only capability comparison is restricted to inshellisense"
            );
            ensure!(
                !h.viewport_contents().contains(&query.line),
                "stale input before query"
            );
            let start = Instant::now();
            h.send(query.line.as_bytes())?;
            let mut echo = None;
            let mut observations = Vec::new();
            let mut last_observed = None;
            loop {
                let screen = h.viewport_contents();
                if h.last_output_arrival().is_some_and(|at| at >= start) {
                    let at = ms(&h, start)?;
                    if echo.is_none() && screen.contains(&query.line) {
                        echo = Some(at);
                    }
                    if last_observed != h.last_output_arrival() {
                        observations.push(json!({"ms":at,"screen":screen}));
                        last_observed = h.last_output_arrival();
                    }
                    if echo.is_some() && (query.echo_only || menu(&screen, query, &session.style)) {
                        result["queries"].as_array_mut().unwrap().push(json!({"line":query.line,"expected":query.expected,"warmup":query.warmup,"input_echo":echo,"menu":if session.style=="plain" || query.echo_only {None} else {Some(at)},"observation_mode":if query.echo_only {"echo_only"} else {"menu"},"observations":observations}));
                        break;
                    }
                }
                if start.elapsed() >= TIMEOUT {
                    result["queries"].as_array_mut().unwrap().push(json!({"line":query.line,"expected":query.expected,"warmup":query.warmup,"input_echo":echo,"menu":null,"failed":true,"elapsed_ms":start.elapsed().as_secs_f64()*1000.0,"observations":observations}));
                    bail!("menu timeout for {}", query.line);
                }
                let _ = h.pump(Duration::from_millis(25));
            }
            clear(&mut h, &query.line)?;
        }
        if let Some(path) = adapter_path {
            let adapter: Value = serde_json::from_slice(&fs::read(path)?)?;
            ensure!(
                adapter == result["adapter"],
                "adapter mode changed during comparison"
            );
        }
        h.finish(TIMEOUT)
    })();
    result["final_screen"] = json!(h.viewport_contents());
    run
}
fn main() -> Result<()> {
    let options = Options::parse();
    ensure!(!options.output.exists(), "refusing to overwrite evidence");
    let session: Session = serde_json::from_slice(&fs::read(&options.session)?)?;
    let hash = |path: &std::path::Path| -> Result<String> {
        Ok(format!("{:X}", Sha256::digest(fs::read(path)?)))
    };
    let mut result = json!({"schema":1,"session":options.session,"session_sha256":hash(&options.session)?,"program":session.program,"program_sha256":hash(&session.program)?,"probe_sha256":hash(&std::env::current_exe()?)?,"queries":[],"passed":false});
    let outcome = run(&session, &mut result);
    match &outcome {
        Ok(()) => result["passed"] = json!(true),
        Err(error) => result["error"] = json!(format!("{error:#}")),
    }
    fs::write(options.output, serde_json::to_vec_pretty(&result)?)?;
    outcome
}

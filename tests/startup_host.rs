#![cfg(windows)]

use anyhow::{Context, Result, ensure};
use blueberry::{probe::Harness, pty};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[test]
fn profile_hook_keeps_shell_skips_recursion_and_returns_to_parent() -> Result<()> {
    let evidence =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/startup-terminal-evidence");
    std::fs::create_dir_all(&evidence)?;
    let root = tempfile::Builder::new()
        .prefix("startup-")
        .tempdir_in(evidence)?
        .keep();
    eprintln!("startup evidence: {}", root.display());
    let profile = root.as_path().join("中文 profile.ps1");
    std::fs::write(
        &profile,
        b"Write-Output ('BB_PARENT_RESUMED_' + $env:BLUEBERRY_ACTIVE)\n",
    )?;
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_blueberry"));
    let shell = pty::default_shell();
    let local = root.as_path().join("local");
    let roaming = root.as_path().join("roaming");
    let status = std::process::Command::new(&executable)
        .args(["startup", "enable", "--profile"])
        .arg(&profile)
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", &roaming)
        .status()?;
    ensure!(status.success(), "Configure isolated profile");
    let hook = std::fs::read_to_string(&profile)?;
    let start = hook.find("$blueberryAutoScript =").unwrap();
    let end = start + hook[start..].find("if ($Host.Name").unwrap();
    // The test loads its private profile via -Command; real installations are
    // loaded by ConsoleHost. Keep every other guard, especially ACTIVE.
    let hook = format!(
        "{}$blueberryAutoScript = $false\n{}",
        &hook[..start],
        &hook[end..]
    );
    let trace = root.as_path().join("startup-trace.jsonl");
    let hook = hook.replace(
        " run --shell $blueberryAutoShell",
        &format!(
            " run --trace '{}' --data-dir '{}' --shell $blueberryAutoShell",
            trace.display().to_string().replace('\'', "''"),
            root.as_path()
                .join("data")
                .display()
                .to_string()
                .replace('\'', "''")
        ),
    );
    std::fs::write(&profile, hook)?;
    let quoted = profile.display().to_string().replace('\'', "''");
    let environment = BTreeMap::from([
        ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
        ("BLUEBERRY_ACTIVE".into(), "0".into()),
        ("TERM".into(), "xterm-256color".into()),
        ("LOCALAPPDATA".into(), local.to_string_lossy().into_owned()),
        ("APPDATA".into(), roaming.to_string_lossy().into_owned()),
        ("BLUEBERRY_PROBE_TOKEN".into(), "startup-test".into()),
    ]);
    let args = [
        "-NoLogo",
        "-NoProfile",
        "-NoExit",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
    ]
    .map(String::from);
    let mut args = args.to_vec();
    args.push(format!(". '{quoted}'"));
    let mut terminal = Harness::start(&shell, &args, root.as_path(), &environment, String::new())?;
    // A real profile may replace the prompt (Starship, Oh My Posh, etc.).
    // Observe editor entry rather than assuming the stock "PS " prompt.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !std::fs::read_to_string(&trace).is_ok_and(|text| text.contains("direct_readline_begin"))
    {
        ensure!(
            Instant::now() < deadline,
            "profile child did not enter the editor: {}",
            terminal.contents()
        );
        let _ = terminal.pump(Duration::from_millis(20));
    }
    send_editor_command(
        &mut terminal,
        "Write-Output ('BB_ACTIVE_' + $env:BLUEBERRY_ACTIVE); Write-Output ('BB_EDITION_' + $PSVersionTable.PSEdition)",
    )?;
    wait_line(&mut terminal, "BB_ACTIVE_1")?;
    let edition = if shell
        .file_name()
        .unwrap()
        .to_string_lossy()
        .eq_ignore_ascii_case("powershell.exe")
    {
        "Desktop"
    } else {
        "Core"
    };
    wait_line(&mut terminal, &format!("BB_EDITION_{edition}"))?;
    send_editor_command(
        &mut terminal,
        &format!(". '{quoted}'; Write-Output BB_NESTED_SKIPPED"),
    )?;
    wait_line(&mut terminal, "BB_NESTED_SKIPPED")?;
    terminal.send_text("exit\r")?;
    wait_line(&mut terminal, "BB_PARENT_RESUMED_0")?;
    terminal.send_text("Write-Output ('BB_OUTER_' + $env:BLUEBERRY_ACTIVE)\r")?;
    wait_line(&mut terminal, "BB_OUTER_0")?;
    // The parent is now plain PowerShell: the nested Blueberry protocol has
    // exited, so the probe's host-aware finish chord is no longer applicable.
    terminal.stop()?;
    Ok(())
}

#[test]
fn no_arguments_falls_back_to_inbox_shell_without_pwsh_on_path() -> Result<()> {
    let evidence =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/startup-terminal-evidence");
    std::fs::create_dir_all(&evidence)?;
    let root = tempfile::Builder::new()
        .prefix("startup-")
        .tempdir_in(evidence)?
        .keep();
    eprintln!("startup evidence: {}", root.display());
    let environment = BTreeMap::from([
        ("BLUEBERRY_TEST_SHELL".into(), String::new()),
        ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
        ("BLUEBERRY_ACTIVE".into(), "0".into()),
        ("PATH".into(), root.as_path().to_string_lossy().into_owned()),
        (
            "LOCALAPPDATA".into(),
            root.as_path().join("local").to_string_lossy().into_owned(),
        ),
        (
            "APPDATA".into(),
            root.as_path()
                .join("roaming")
                .to_string_lossy()
                .into_owned(),
        ),
        ("BLUEBERRY_PROBE_TOKEN".into(), "fallback-test".into()),
    ]);
    let mut terminal = Harness::start(
        &PathBuf::from(env!("CARGO_BIN_EXE_blueberry")),
        &[],
        root.as_path(),
        &environment,
        String::new(),
    )?;
    terminal.wait_text("PS ", Duration::from_secs(30))?;
    send_editor_command(
        &mut terminal,
        "Write-Output ('BB_FALLBACK_' + $PSVersionTable.PSEdition + '_' + $env:BLUEBERRY_ACTIVE)",
    )?;
    wait_line(&mut terminal, "BB_FALLBACK_Desktop_1").context("fallback Shell command output")?;
    // finish waits for the editor to settle before submitting exit.
    terminal
        .finish(Duration::from_secs(10))
        .context("fallback Shell normal exit")?;
    Ok(())
}

fn send_editor_command(terminal: &mut Harness, command: &str) -> Result<()> {
    terminal.send_text(command)?;
    terminal.send_text("\r")?;
    Ok(())
}

fn wait_line(terminal: &mut Harness, expected: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !terminal
        .contents()
        .lines()
        .any(|line| line.trim() == expected)
    {
        terminal
            .pump(deadline.saturating_duration_since(Instant::now()))
            .with_context(|| format!("wait for output line {expected}"))?;
    }
    Ok(())
}

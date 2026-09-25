#![cfg(windows)]

use anyhow::{Result, ensure};
use blueberry::{probe::Harness, pty};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[test]
fn profile_hook_keeps_shell_skips_recursion_and_returns_to_parent() -> Result<()> {
    let root = tempfile::tempdir()?;
    let profile = root.path().join("中文 profile.ps1");
    std::fs::write(
        &profile,
        b"Write-Output ('BB_PARENT_RESUMED_' + $env:BLUEBERRY_ACTIVE)\n",
    )?;
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_blueberry"));
    let shell = pty::default_shell();
    let local = root.path().join("local");
    let roaming = root.path().join("roaming");
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
    std::fs::write(&profile, hook)?;
    let quoted = profile.display().to_string().replace('\'', "''");
    let environment = BTreeMap::from([
        ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
        ("BLUEBERRY_ACTIVE".into(), "0".into()),
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
    let mut terminal = Harness::start(
        &shell,
        &args,
        root.path(),
        &environment,
        "startup-test".into(),
    )?;
    terminal.event("prompt_end", Duration::from_secs(30))?;
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
    terminal.event("prompt_end", Duration::from_secs(30))?;
    send_editor_command(
        &mut terminal,
        &format!(". '{quoted}'; Write-Output BB_NESTED_SKIPPED"),
    )?;
    wait_line(&mut terminal, "BB_NESTED_SKIPPED")?;
    terminal.event("prompt_end", Duration::from_secs(30))?;
    terminal.send(b"exit\r")?;
    wait_line(&mut terminal, "BB_PARENT_RESUMED_0")?;
    terminal.send(b"Write-Output ('BB_OUTER_' + $env:BLUEBERRY_ACTIVE)\r")?;
    wait_line(&mut terminal, "BB_OUTER_0")?;
    // The parent is now plain PowerShell: the nested Blueberry protocol has
    // exited, so the probe's host-aware finish chord is no longer applicable.
    terminal.stop()?;
    Ok(())
}

#[test]
fn no_arguments_falls_back_to_inbox_shell_without_pwsh_on_path() -> Result<()> {
    let root = tempfile::tempdir()?;
    let environment = BTreeMap::from([
        ("BLUEBERRY_TEST_SHELL".into(), String::new()),
        ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
        ("BLUEBERRY_ACTIVE".into(), "0".into()),
        ("PATH".into(), root.path().to_string_lossy().into_owned()),
        (
            "LOCALAPPDATA".into(),
            root.path().join("local").to_string_lossy().into_owned(),
        ),
        (
            "APPDATA".into(),
            root.path().join("roaming").to_string_lossy().into_owned(),
        ),
        ("BLUEBERRY_PROBE_TOKEN".into(), "fallback-test".into()),
    ]);
    let mut terminal = Harness::start(
        &PathBuf::from(env!("CARGO_BIN_EXE_blueberry")),
        &[],
        root.path(),
        &environment,
        "fallback-test".into(),
    )?;
    terminal.event("prompt_end", Duration::from_secs(30))?;
    send_editor_command(
        &mut terminal,
        "Write-Output ('BB_FALLBACK_' + $PSVersionTable.PSEdition + '_' + $env:BLUEBERRY_ACTIVE)",
    )?;
    wait_line(&mut terminal, "BB_FALLBACK_Desktop_1")?;
    terminal.finish(Duration::from_secs(10))?;
    Ok(())
}

fn send_editor_command(terminal: &mut Harness, command: &str) -> Result<()> {
    terminal.send(command.as_bytes())?;
    terminal.send(b"\x1b[32;5u")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let buffer =
            terminal.event("buffer", deadline.saturating_duration_since(Instant::now()))?;
        if buffer["line"].as_str() == Some(command) {
            break;
        }
    }
    terminal.send(b"\r")?;
    Ok(())
}

fn wait_line(terminal: &mut Harness, expected: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !terminal
        .contents()
        .lines()
        .any(|line| line.trim() == expected)
    {
        terminal.pump(deadline.saturating_duration_since(Instant::now()))?;
    }
    Ok(())
}

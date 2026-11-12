#![cfg(windows)]

use anyhow::{Context, Result, ensure};
use blueberry::{probe::Harness, pty};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[test]
fn repeated_direct_start_accepts_immediate_exit_and_releases_data_files() -> Result<()> {
    for _ in 0..3 {
        let root = tempfile::tempdir()?;
        let data = root.path().join("data");
        let shell = pty::default_shell();
        let args = vec![
            "run".into(),
            "--host-mode".into(),
            "direct".into(),
            "--no-profile".into(),
            "--shell".into(),
            shell.to_string_lossy().into_owned(),
            "--data-dir".into(),
            data.to_string_lossy().into_owned(),
        ];
        let mut terminal = Harness::start(
            std::path::Path::new(env!("CARGO_BIN_EXE_blueberry")),
            &args,
            root.path(),
            &BTreeMap::from([("BLUEBERRY_NO_HISTORY".into(), "1".into())]),
            String::new(),
        )?;
        terminal.send_text("exit\r")?;
        terminal.wait_exit(Duration::from_secs(20))?;
        // A stopped host must leave no handle preventing data-directory moves.
        if data.exists() {
            std::fs::rename(&data, root.path().join("retired-data"))?;
        }
    }
    Ok(())
}

#[test]
fn profile_hook_attaches_to_current_shell_preserves_first_key_and_exits_once() -> Result<()> {
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
        "Set-PSReadLineKeyHandler -Chord g -ScriptBlock { param($key,$arg); [Microsoft.PowerShell.PSConsoleReadLine]::SelfInsert($key,$arg); [IO.File]::WriteAllText((Join-Path $env:LOCALAPPDATA 'user-g.txt'),'g') }\r\n",
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
    let mut module_roots = vec![root.join("Modules")];
    module_roots.extend(pty::module_search_paths(&shell));
    let data = root.join("data");
    let quoted = profile.display().to_string().replace('\'', "''");
    let environment = BTreeMap::from([
        ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
        ("BLUEBERRY_ACTIVE".into(), "0".into()),
        (
            "PSModulePath".into(),
            std::env::join_paths(module_roots)?
                .to_string_lossy()
                .into_owned(),
        ),
        (
            "BLUEBERRY_PROFILE_PROBE_DATA".into(),
            data.to_string_lossy().into_owned(),
        ),
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
    ]
    .map(String::from);
    let args = args.to_vec();
    let mut terminal = Harness::start(&shell, &args, root.as_path(), &environment, String::new())?;
    // Execute the generated managed block in a real interactive parent with
    // its script/recursion guards intact. This is a hook regression, not a
    // measurement of ConsoleHost's automatic profile loading.
    terminal.wait_text(">", Duration::from_secs(20))?;
    terminal.send_text(&format!(". '{quoted}'\r"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !std::fs::read_dir(&data)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            std::fs::read(entry.path().join("adapter.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .is_some_and(|status| status["automatic_menu"] == true)
        })
    {
        ensure!(
            Instant::now() < deadline,
            "profile service did not attach: {}",
            terminal.contents()
        );
        let _ = terminal.pump(Duration::from_millis(20));
    }
    terminal.send_text("g")?;
    terminal.wait_text("› ", Duration::from_secs(20))?;
    ensure!(
        std::fs::read_to_string(local.join("user-g.txt"))? == "g",
        "user key handler did not run"
    );
    let pid = terminal.process_id().context("missing shell pid")?;
    send_editor_command(
        &mut terminal,
        "et-command Out-Null | Out-Null; Write-Output ('BB_ACTIVE_' + $env:BLUEBERRY_ACTIVE); Write-Output ('BB_PID_' + $PID); Write-Output ('BB_EDITION_' + $PSVersionTable.PSEdition)",
    )?;
    wait_line(&mut terminal, "BB_ACTIVE_1")?;
    wait_line(&mut terminal, &format!("BB_PID_{pid}"))?;
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
    ensure!(
        std::fs::read_dir(&data)?
            .flatten()
            .filter(|entry| entry.path().join("adapter.json").is_file())
            .count()
            == 1,
        "re-sourcing started a duplicate service"
    );
    terminal.send_text("exit\r")?;
    terminal.wait_exit(Duration::from_secs(20))?;
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

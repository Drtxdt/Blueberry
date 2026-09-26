#![cfg(all(windows, debug_assertions))]
//! Inherited-console tests run in an outer observing ConPTY, never a second
//! product ConPTY. Every session uses an isolated directory and no history.
use anyhow::{Context, Result, ensure};
use blueberry::probe::Harness;
use std::{collections::BTreeMap, time::Duration};
const TIMEOUT: Duration = Duration::from_secs(20);

#[test]
fn compiled_bridge_codec_runs_in_selected_shell() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bootstrap = blueberry::host::direct::ensure_bootstrap(dir.path())?;
    let script = dir.path().join("codec.ps1");
    std::fs::write(
        &script,
        format!("\u{feff}{}", include_str!("direct-bridge.tests.ps1")),
    )?;
    let result = std::process::Command::new(blueberry::pty::default_shell())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-AssemblyPath")
        .arg(bootstrap.with_file_name("direct-bridge.dll"))
        .output()?;
    ensure!(
        result.status.success(),
        "compiled codec failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

#[test]
fn plain_psreadline_preserves_the_same_unicode_input() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let module = std::env::var("BLUEBERRY_TEST_PSREADLINE_MODULE")
        .unwrap_or("PSReadLine".into())
        .replace('\'', "''");
    let mut harness = Harness::start(
        &blueberry::pty::default_shell(),
        &[
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NoExit".into(),
            "-Command".into(),
            format!("Import-Module '{module}'; Set-PSReadLineOption -HistorySaveStyle SaveNothing"),
        ],
        dir.path(),
        &BTreeMap::from([("BLUEBERRY_NO_HISTORY".into(), "1".into())]),
        String::new(),
    )?;
    harness.wait_text("PS ", TIMEOUT)?;
    harness.send("Write-Output '中文😀👩‍💻终端'\r".as_bytes())?;
    harness.wait_line("中文😀👩‍💻终端", TIMEOUT)?;
    harness.finish(TIMEOUT)
}

fn start(directory: &std::path::Path) -> Result<Harness> {
    let mut args = vec![
        "run".into(),
        "--host-mode".into(),
        "direct".into(),
        "--shell".into(),
        blueberry::pty::default_shell()
            .to_string_lossy()
            .into_owned(),
        "--no-profile".into(),
        "--data-dir".into(),
        directory.to_string_lossy().into_owned(),
        "--trace".into(),
        directory.join("trace.jsonl").to_string_lossy().into_owned(),
    ];
    if directory.join("config.toml").is_file() {
        args.extend([
            "--config".into(),
            directory.join("config.toml").to_string_lossy().into_owned(),
        ]);
    }
    Harness::start(
        std::path::Path::new(env!("CARGO_BIN_EXE_blueberry")),
        &args,
        directory,
        &BTreeMap::from([
            ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
            ("TERM".into(), "xterm-256color".into()),
        ]),
        String::new(),
    )
}

#[test]
fn direct_empty_line_cancels_menu_and_disabled_autotrigger_preserves_typing() -> Result<()> {
    for automatic in [true, false] {
        let dir = tempfile::tempdir()?;
        if !automatic {
            std::fs::write(
                dir.path().join("config.toml"),
                "[completion]\nauto_trigger = false\n",
            )?;
        }
        let mut h = start(dir.path())?;
        h.wait_text("PS ", TIMEOUT)?;
        h.send(b"git sw")?;
        if automatic {
            h.wait_text("switch", TIMEOUT)?;
        } else {
            h.wait_text("git sw", TIMEOUT)?;
        }
        h.send(b"\x01\x7f")?;
        let deadline = std::time::Instant::now() + Duration::from_millis(400);
        while std::time::Instant::now() < deadline {
            let _ = h.pump(Duration::from_millis(25));
        }
        ensure!(
            !h.viewport_contents().contains("› "),
            "empty/disabled auto query left command menu visible"
        );
        if !automatic {
            let status = std::fs::read_dir(dir.path())?
                .filter_map(Result::ok)
                .find_map(|entry| std::fs::read(entry.path().join("adapter.json")).ok())
                .context("missing disabled-menu session status")?;
            let status: serde_json::Value = serde_json::from_slice(&status)?;
            ensure!(
                status["automatic_menu"] == false
                    && status["disabled_reason"] == "completion.auto_trigger is disabled",
                "doctor status ignored automatic-menu configuration"
            );
        }
        h.send(b"Write-Output 'empty-edit-restored'\r")?;
        h.wait_line("empty-edit-restored", TIMEOUT)?;
        h.finish(TIMEOUT)?;
    }
    Ok(())
}

#[test]
fn direct_menu_accept_is_fill_only_and_shell_still_edits_unicode() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut harness = start(dir.path())?;
    harness.wait_text("PS ", TIMEOUT)?;
    harness.send("Write-Output '中文😀👩‍💻终端'\r".as_bytes())?;
    harness.wait_line("中文😀👩‍💻终端", TIMEOUT)?;
    harness.send(b"git sw")?;
    if let Err(error) = harness.wait_text("switch", TIMEOUT) {
        let statuses = std::fs::read_dir(dir.path())?
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::read_to_string(entry.path().join("adapter.json")).ok())
            .collect::<Vec<_>>();
        anyhow::bail!(
            "{error:#}; adapter status: {statuses:?}; numeric trace: {}",
            std::fs::read_to_string(dir.path().join("trace.jsonl")).unwrap_or_default()
        );
    }
    ensure!(
        !harness.contents().contains("已停用"),
        "bridge disabled: {}",
        harness.contents()
    );
    harness.send(b"\t")?;
    harness.wait_text("git switch", TIMEOUT).map_err(|error| {
        anyhow::anyhow!(
            "{error:#}; trace: {}",
            std::fs::read_to_string(dir.path().join("trace.jsonl")).unwrap_or_default()
        )
    })?;
    harness.send(b"\x01\x7f")?;
    harness.send("Write-Output '中文😀👩‍💻终端'".as_bytes())?;
    harness.send(b"\r")?;
    harness.wait_line("中文😀👩‍💻终端", TIMEOUT)?;
    harness.finish(TIMEOUT)
}

#[test]
fn direct_runtime_custom_binding_is_preserved() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut harness = start(dir.path())?;
    harness.wait_text("PS ", TIMEOUT)?;
    harness.send(b"Set-PSReadLineKeyHandler -Chord g -ScriptBlock { [Microsoft.PowerShell.PSConsoleReadLine]::Insert('CUSTOM') }; Write-Output 'binding-installed'\r")?;
    harness.wait_line("binding-installed", TIMEOUT)?;
    harness.send(b"g")?;
    harness.wait_text("CUSTOM", TIMEOUT)?;
    harness.send(b"\x01\x7f")?;
    harness.finish(TIMEOUT)
}

#[test]
fn direct_dismiss_external_program_and_ctrl_c_restore_shell() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut h = start(dir.path())?;
    h.wait_text("PS ", TIMEOUT)?;
    h.send(b"git sw")?;
    h.wait_text("switch", TIMEOUT)?;
    h.send(b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_")?;
    let until = std::time::Instant::now() + TIMEOUT;
    while h.viewport_contents().contains("› ") {
        ensure!(
            std::time::Instant::now() < until,
            "dismiss left menu visible"
        );
        let _ = h.pump(Duration::from_millis(25));
    }
    ensure!(
        h.viewport_contents().contains("git sw"),
        "dismiss changed line"
    );
    h.send(b"\x01")?;
    h.send(b"cmd.exe /d /c echo external-direct\r")?;
    h.wait_line("external-direct", TIMEOUT)?;
    h.send(b"git sw")?;
    h.wait_text("switch", TIMEOUT)?;
    h.send(b"\x03")?;
    h.wait_text("^C", TIMEOUT)?;
    h.send(b"Write-Output 'ctrl-c-restored'\r")?;
    h.wait_line("ctrl-c-restored", TIMEOUT)?;
    h.finish(TIMEOUT)
}

#[test]
fn direct_selected_frame_navigation_survives_resize_and_disconnect() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut harness = start(dir.path())?;
    harness.wait_text("PS ", TIMEOUT)?;
    harness.send(b"git sw")?;
    harness.wait_text("switch", TIMEOUT)?;
    harness.send(b"\x1b[B")?;
    harness.wait_text("›", TIMEOUT)?;
    // Wait until the selected candidate really changes; acceptance binds the
    // displayed frame rather than the newest asynchronously computed result.
    let deadline = std::time::Instant::now() + TIMEOUT;
    while !harness
        .viewport_contents()
        .lines()
        .any(|line| line.contains("›") && line.contains("show"))
    {
        ensure!(
            std::time::Instant::now() < deadline,
            "selection did not advance"
        );
        let _ = harness.pump(Duration::from_millis(50));
    }
    harness.send(b"\t")?;
    harness.wait_text("git show", TIMEOUT)?;
    harness.resize(10, 50)?;
    harness.send(b"\x01\x7f")?;
    harness.send(b"[Blueberry.Direct.Bridge]::Shutdown(); Write-Output 'disconnected'\r")?;
    harness.wait_text("disconnected", TIMEOUT)?;
    harness.send(b"Write-Output 'editing-restored'\r")?;
    harness.wait_text("editing-restored", TIMEOUT)?;
    harness.finish(TIMEOUT)
}

#[test]
fn direct_guided_form_cancel_preserves_right_text_and_confirm_does_not_execute() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut harness = start(dir.path())?;
    harness.wait_text("PS ", TIMEOUT)?;
    harness.send(b"git switch -c  --track")?;
    harness.send(&b"\x1b[D".repeat(" --track".len()))?;
    harness.send(b"\x1b[80;25;16;1;10;1_\x1b[80;25;16;0;10;1_")?;
    harness.wait_text("继续填写", TIMEOUT)?;
    harness.send(b"\r")?;
    harness.wait_text("填写参数", TIMEOUT)?;
    harness.send("分支😀".as_bytes())?;
    harness.wait_text("当前值", TIMEOUT)?;
    harness.send(b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_")?;
    // Escape is a terminal prefix. Confirm cancellation before sending a
    // new modified chord, so the observer does not synthesize an Alt chord.
    let deadline = std::time::Instant::now() + TIMEOUT;
    while harness.viewport_contents().contains("填写参数") {
        ensure!(
            std::time::Instant::now() < deadline,
            "cancel did not restore original line"
        );
        let _ = harness.pump(Duration::from_millis(50));
    }
    harness.send(b"\x1b[80;25;16;1;10;1_\x1b[80;25;16;0;10;1_")?;
    harness.wait_text("继续填写", TIMEOUT)?;
    harness.send(b"\r")?;
    harness.wait_text("填写参数", TIMEOUT)?;
    harness.send(b"new-branch")?;
    harness.send(b"\r")?;
    harness.wait_text("git switch -c new-branch", TIMEOUT)?;
    ensure!(
        harness.viewport_contents().contains("--track"),
        "right text lost"
    );
    ensure!(
        !harness.viewport_contents().contains("fatal:"),
        "form executed git"
    );
    harness.send(b"\x1a")?;
    harness.wait_text("git switch -c  --track", TIMEOUT)?;
    harness.finish(TIMEOUT)
}

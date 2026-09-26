#![cfg(all(windows, debug_assertions))]

//! End-to-end Beta checks against a real Blueberry host and a real
//! PSReadLine runspace. The tests deliberately use only temporary
//! directories and disable PSReadLine history.

use anyhow::{Context, Result, bail, ensure};
use blueberry::{config, probe::Harness};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tempfile::{TempDir, tempdir};

const PTY_TIMEOUT: Duration = Duration::from_secs(20);

struct RunningHost {
    harness: Harness,
    data_dir: TempDir,
    _cwd: TempDir,
    edit_path: PathBuf,
    native_marker: PathBuf,
}

fn protocol_chord(suffix: char) -> Vec<u8> {
    // F12 remains the protocol default in the temporary config. Keeping the
    // encoder here in sync with src/input.rs makes each test exercise the
    // actual PSReadLine reserved handler, rather than a shell-side shortcut.
    format!("\x1b[24~{suffix}").into_bytes()
}

fn ctrl_alt_space_records() -> &'static [u8] {
    // CSI-u retains both modifiers through Windows Server 2022 ConPTY.
    b"\x1b[32;7u"
}

fn ctrl_space_records() -> &'static [u8] {
    // Use CSI-u instead of ambiguous NUL or version-specific Win32 records.
    b"\x1b[32;5u"
}

#[test]
fn terminal_guided_package_form_fills_without_execution_and_cancel_preserves_line() -> Result<()> {
    let mut host = start_host()?;
    clear_line(&mut host.harness)?;
    host.harness.send(b"cargo test -p --release")?;
    host.harness.send(&b"\x1b[D".repeat(" --release".len()))?;
    read_real_buffer(
        &mut host.harness,
        "guided form original cursor",
        "cargo test -p --release",
        Some("cargo test -p".len() as u64),
    )?;
    host.harness.send(b"\x1b[112;7u")?;
    select_candidate(&mut host.harness, "继续填写 Cargo 包")?;
    // Drain late ordinary completion/history replies before accepting. They
    // must not replace the selected hub operation with a normal -p menu.
    let until = Instant::now() + Duration::from_millis(400);
    while Instant::now() < until {
        if let Err(error) = host.harness.pump(Duration::from_millis(50))
            && error.downcast_ref::<std::sync::mpsc::RecvTimeoutError>()
                != Some(&std::sync::mpsc::RecvTimeoutError::Timeout)
        {
            return Err(error);
        }
        ensure!(
            selected_candidate_line(&host.harness.viewport_contents())
                .is_some_and(|line| candidate_line_has(line, "继续填写 Cargo 包")),
            "late ordinary completion replaced the displayed hub operation"
        );
    }
    host.harness.send(b"\r")?;
    wait_until(
        &mut host.harness,
        "package parameter form",
        PTY_TIMEOUT,
        |screen| screen.contains("工作区包"),
    )?;
    host.harness.send(b"blue berry")?;
    wait_until(
        &mut host.harness,
        "typed package value",
        PTY_TIMEOUT,
        |screen| screen.contains("当前值：blue berry"),
    )?;
    host.harness.send(b"\r")?;
    let result = host.harness.event("edit_result", PTY_TIMEOUT)?;
    ensure!(result["applied"] == true, "guided edit rejected: {result}");
    read_real_buffer(
        &mut host.harness,
        "guided package",
        "cargo test -p 'blue berry' --release",
        None,
    )?;

    Ok(())
}

#[test]
fn terminal_guided_form_cancel_preserves_original_line() -> Result<()> {
    let mut host = start_host()?;
    clear_line(&mut host.harness)?;
    host.harness.send(b"cargo test -p --release")?;
    host.harness.send(&b"\x1b[D".repeat(" --release".len()))?;
    read_real_buffer(
        &mut host.harness,
        "cancelable form original cursor",
        "cargo test -p --release",
        Some("cargo test -p".len() as u64),
    )?;
    host.harness.send(b"\x1b[112;7u")?;
    select_candidate(&mut host.harness, "继续填写 Cargo 包")?;
    host.harness.send(b"\r")?;
    wait_until(
        &mut host.harness,
        "cancelable package form",
        PTY_TIMEOUT,
        |screen| screen.contains("工作区包"),
    )?;
    host.harness.send(b"\x1b")?;
    read_real_buffer(
        &mut host.harness,
        "canceled package",
        "cargo test -p --release",
        Some("cargo test -p".len() as u64),
    )?;
    Ok(())
}

#[test]
fn terminal_purpose_search_inserts_command_tokens_and_restores_normal_completion() -> Result<()> {
    let mut host = start_host()?;
    let config_path = host.data_dir.path().join("config.toml");
    let mut config: config::Config = toml::from_str(&fs::read_to_string(&config_path)?)?;
    config.ui.icon_style = "nerd".into();
    fs::write(config_path, toml::to_string(&config)?)?;
    clear_line(&mut host.harness)?;
    host.harness.send("查看分支".as_bytes())?;
    let _ = request_buffer(&mut host.harness, "查看分支")?;
    host.harness.send(b"\x1b[102;7u")?;
    host.harness.wait_text("git branch", PTY_TIMEOUT)?;
    let _ = accept_selected(&mut host.harness)?;
    let buffer = request_buffer(&mut host.harness, "git branch")?;
    ensure!(
        buffer["line"]
            .as_str()
            .is_some_and(|s| s.trim_end() == "git branch"),
        "invalid search insertion: {buffer}"
    );
    clear_line(&mut host.harness)?;
    host.harness.send(b"codex --model ")?;
    let buffer = request_buffer(&mut host.harness, "codex --model")?;
    host.harness.wait_text("<MODEL>", PTY_TIMEOUT)?;
    ensure!(
        buffer["line"]
            .as_str()
            .is_some_and(|s| !s.contains("<MODEL>")),
        "hint entered the buffer: {buffer}"
    );
    host.harness.finish(PTY_TIMEOUT)?;
    Ok(())
}

#[test]
fn terminal_static_function_metadata_refreshes_values_without_execution() -> Result<()> {
    let mut host = start_host()?;
    send_command(
        &mut host.harness,
        "function global:SS-Knowledge { param([ValidateSet('fast','slow')][string]$Mode) dynamicparam { throw 'must not execute' } process { throw 'must not execute' } }; Write-Output SS_METADATA_READY",
        "SS_METADATA_READY",
    )?;
    clear_line(&mut host.harness)?;
    host.harness.send(b"SS-Knowledge -Mode ")?;
    let _ = request_buffer(&mut host.harness, "SS-Knowledge")?;
    let metadata = host.harness.event("command_metadata", PTY_TIMEOUT)?;
    let _: blueberry::knowledge::HelpPage = serde_json::from_value(metadata["page"].clone())?;
    wait_until(
        &mut host.harness,
        "learned enum menu",
        PTY_TIMEOUT,
        |screen| {
            screen
                .lines()
                .any(|line| line.contains("│") && line.contains(" fast "))
        },
    )?;
    select_candidate(&mut host.harness, "fast")?;
    let _ = accept_selected(&mut host.harness)?;
    let buffer = request_buffer(&mut host.harness, "SS-Knowledge -Mode fast")
        .context("accept statically learned enum")?;
    ensure!(
        buffer["line"]
            .as_str()
            .is_some_and(|line| line.trim_end() == "SS-Knowledge -Mode fast"),
        "unexpected learned value: {buffer}"
    );
    host.harness.finish(PTY_TIMEOUT)?;
    Ok(())
}

fn ps_quote(value: &Path) -> String {
    format!("'{}'", value.to_string_lossy().replace('\'', "''"))
}

fn start_host() -> Result<RunningHost> {
    let cwd = tempdir().context("create terminal test cwd")?;
    fs::write(
        cwd.path().join("git.cmd"),
        b"@echo off\r\necho GIT_ARGS:%*\r\n",
    )
    .context("create deterministic git.cmd")?;
    fs::write(
        cwd.path().join("中文😀 文件.txt"),
        b"Blueberry Unicode fixture",
    )
    .context("create Unicode path fixture")?;

    let data_dir = tempdir().context("create terminal test data directory")?;
    let config_path = data_dir.path().join("config.toml");
    fs::write(&config_path, config::example()).context("write temporary config")?;

    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![cwd.path().to_owned()];
    paths.extend(std::env::split_paths(&inherited_path));
    let path = std::env::join_paths(paths).context("construct deterministic PATH")?;
    let token = format!("terminal-beta-{}", uuid::Uuid::new_v4());
    let native_marker = cwd.path().join("native-called.txt");
    let input_trace = cwd.path().join("input-trace.txt");
    let clipboard_fixture = cwd.path().join("clipboard-fixture.txt");
    let env = BTreeMap::from([
        (
            "APPDATA".to_owned(),
            data_dir.path().to_string_lossy().into_owned(),
        ),
        ("PATH".to_owned(), path.to_string_lossy().into_owned()),
        ("PATHEXT".to_owned(), ".COM;.EXE;.BAT;.CMD".to_owned()),
        ("TERM".to_owned(), "xterm-256color".to_owned()),
        ("NO_COLOR".to_owned(), "1".to_owned()),
        ("BLUEBERRY_NO_HISTORY".to_owned(), "1".to_owned()),
        // Debug hosts mirror protocol frames to this probe token without
        // changing the private child-shell token used by the live adapter.
        ("BLUEBERRY_PROBE_TOKEN".to_owned(), token.clone()),
        (
            "BLUEBERRY_NATIVE_MARKER".to_owned(),
            native_marker.to_string_lossy().into_owned(),
        ),
        (
            "BLUEBERRY_TEST_CLIPBOARD_FILE".to_owned(),
            clipboard_fixture.to_string_lossy().into_owned(),
        ),
        (
            "BLUEBERRY_TEST_INPUT_TRACE".to_owned(),
            input_trace.to_string_lossy().into_owned(),
        ),
    ]);
    let transport = std::env::var("BLUEBERRY_TEST_TRANSPORT").unwrap_or_else(|_| "osc".into());
    let args = vec![
        "--config".to_owned(),
        config_path.to_string_lossy().into_owned(),
        "run".to_owned(),
        "--transport".to_owned(),
        transport.clone(),
        "--no-profile".to_owned(),
        "--data-dir".to_owned(),
        data_dir.path().to_string_lossy().into_owned(),
    ];
    let program = PathBuf::from(env!("CARGO_BIN_EXE_blueberry"));
    let mut harness = Harness::start(&program, &args, cwd.path(), &env, token)
        .with_context(|| format!("start {}", program.display()))?;
    harness
        .wait_text("PS ", PTY_TIMEOUT)
        .context("wait for the initial PowerShell prompt")?;
    // Do not let bootstrap messages from the first prompt satisfy a later
    // lifecycle assertion.
    let capabilities = harness.event("capabilities", PTY_TIMEOUT)?;
    ensure!(
        capabilities["capabilities"]["command_metadata"] == true,
        "missing metadata capability: {capabilities}"
    );
    ensure!(
        capabilities["transport"] == transport,
        "requested transport was not active: {capabilities}"
    );
    harness.event("prompt_end", PTY_TIMEOUT)?;
    loop {
        if harness.event("commands", PTY_TIMEOUT)?["complete"] == true {
            break;
        }
    }

    let session_dir = fs::read_dir(data_dir.path())?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("session-"))
        })
        .context("locate host session directory")?;
    Ok(RunningHost {
        harness,
        edit_path: session_dir.join("edit.json"),
        data_dir,
        _cwd: cwd,
        native_marker,
    })
}

#[test]
fn terminal_workbench_reloads_external_store_and_keeps_last_good_snapshot() -> Result<()> {
    let mut host = start_host()?;
    let path = host.data_dir.path().join("Blueberry/commands.toml");
    fs::create_dir_all(path.parent().context("commands directory")?)?;
    fs::write(
        &path,
        "schema_version = 2\n[[favorites]]\nname = 'Reload Before'\ncommand = 'git status'\n",
    )?;
    host.harness.send(b"\x1b[112;7u")?;
    wait_for_live_store(&mut host.harness, "first live favorite", |screen| {
        screen.contains("Reload Before")
    })?;
    fs::write(
        &path,
        "schema_version = 2\n[[favorites]]\nname = 'Reload After'\ncommand = 'cargo test'\n",
    )?;
    wait_for_live_store(&mut host.harness, "external favorite update", |screen| {
        screen.contains("Reload After") && !screen.contains("Reload Before")
    })?;
    fs::write(&path, "schema_version = 2\n[[favorites]]\nname = '")?;
    wait_for_live_store(&mut host.harness, "store error", |screen| {
        screen.contains("收藏文件读取失败") && screen.contains("Reload After")
    })?;
    fs::write(
        &path,
        "schema_version = 2\n[[favorites]]\nname = 'Reload Fixed'\ncommand = 'git log'\n",
    )?;
    wait_for_live_store(&mut host.harness, "recovered favorite", |screen| {
        screen.contains("Reload Fixed") && !screen.contains("Reload After")
    })?;
    host.harness.send(b"\x1b")?;
    read_real_buffer(&mut host.harness, "closed workbench", "", Some(0))?;
    Ok(())
}

fn wait_for_live_store(
    harness: &mut Harness,
    description: &str,
    predicate: impl Fn(&str) -> bool,
) -> Result<()> {
    let deadline = Instant::now() + PTY_TIMEOUT;
    loop {
        let screen = harness.viewport_contents();
        if predicate(&screen) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for {description}; screen:\n{screen}");
        }
        // The cache monitor polls at 750 ms. An empty receive window is
        // normal while waiting for its background notification.
        let _ = harness.pump(Duration::from_millis(100));
    }
}

fn wait_until(
    harness: &mut Harness,
    description: &str,
    timeout: Duration,
    mut predicate: impl FnMut(&str) -> bool,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let contents = harness.viewport_contents();
        if predicate(&contents) {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("timed out waiting for {description}; screen:\n{contents}");
        }
        harness
            .pump(remaining)
            .with_context(|| format!("while waiting for {description}"))?;
    }
}

fn clear_line(harness: &mut Harness) -> Result<()> {
    // Do not concatenate Esc with Ctrl+A: VT input may decode that as an
    // Alt chord. Observe the actual cleared ReadLine buffer before continuing.
    harness.send(b"\x01\x7f")?;
    harness.send(ctrl_space_records())?;
    loop {
        if harness.event("buffer", PTY_TIMEOUT)?["line"] == "" {
            return wait_until(harness, "cleared menu", PTY_TIMEOUT, |screen| {
                !screen.lines().any(|line| line.contains("› "))
            });
        }
    }
}

fn send_command(harness: &mut Harness, command: &str, marker: &str) -> Result<()> {
    clear_line(harness)?;
    harness.send(command.as_bytes())?;
    let buffer = request_buffer(harness, command)?;
    ensure!(
        buffer["line"].as_str() == Some(command),
        "incomplete editor command: {buffer}"
    );
    harness.send(b"\r")?;
    harness.event("execute", PTY_TIMEOUT)?;
    harness.wait_text(marker, PTY_TIMEOUT)?;
    harness.event("prompt_end", PTY_TIMEOUT)?;
    Ok(())
}

fn cancel_to_prompt(harness: &mut Harness) -> Result<()> {
    harness.send(b"\x03")?;
    harness.event("prompt_end", PTY_TIMEOUT)?;
    Ok(())
}

fn request_buffer(harness: &mut Harness, expected_fragment: &str) -> Result<Value> {
    // Ctrl+Space is the public trigger and lets the host serialize this
    // request with any prompt/continuation input it has just forwarded. A
    // physical F12,s chord races the adapter's own continuation handler and
    // a bare NUL is rendered as `2` by the outer ConPTY input path.
    harness.send(ctrl_space_records())?;
    loop {
        let value = harness.event("buffer", PTY_TIMEOUT)?;
        if value["line"]
            .as_str()
            .is_some_and(|line| line.contains(expected_fragment))
        {
            return Ok(value);
        }
    }
}

fn read_real_buffer(
    harness: &mut Harness,
    description: &str,
    expected_line: &str,
    expected_cursor: Option<u64>,
) -> Result<Value> {
    // Move left and right through PSReadLine to request a fresh serialized
    // buffer without changing the final line or cursor. These are editor keys,
    // so an asynchronously refreshed completion menu cannot consume them as a
    // selection or acceptance action.
    harness.send(b"\x1b[D\x1b[C")?;
    let deadline = Instant::now() + PTY_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("timed out reading real PSReadLine buffer for {description}");
        }
        let value = harness.event("buffer", remaining)?;
        let line_matches = value["line"]
            .as_str()
            .is_some_and(|line| line.contains(expected_line));
        let cursor_matches = expected_cursor.is_none_or(|cursor| value["cursor"] == cursor);
        if line_matches && cursor_matches {
            return Ok(value);
        }
    }
}

fn wait_absent(harness: &mut Harness, path: &Path, description: &str) -> Result<()> {
    let deadline = Instant::now() + PTY_TIMEOUT;
    loop {
        if !path.exists() {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!(
                "timed out waiting for {description}; screen:\n{}",
                harness.viewport_contents()
            );
        }
        let _ = harness.pump(remaining.min(Duration::from_millis(50)));
    }
}

fn select_candidate(harness: &mut Harness, label: &str) -> Result<()> {
    wait_until(
        harness,
        &format!("real candidate before {label}"),
        PTY_TIMEOUT,
        |screen| selected_candidate_line(screen).is_some(),
    )?;
    let deadline = Instant::now() + PTY_TIMEOUT;
    for _ in 0..128 {
        let contents = harness.viewport_contents();
        if selected_candidate_line(&contents).is_some_and(|line| candidate_line_has(line, label)) {
            return Ok(());
        }
        let before = selected_candidate_line(&contents)
            .context("completion menu lost its selectable row")?
            .to_owned();
        harness.send(b"\x1b[B")?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        wait_until(harness, "menu navigation repaint", remaining, |screen| {
            selected_candidate_line(screen).is_some_and(|line| line != before)
        })?;
    }
    bail!(
        "candidate {label} was never selected (transport={}, selected={:?}); screen:\n{}",
        std::env::var("BLUEBERRY_TEST_TRANSPORT").unwrap_or_else(|_| "osc".into()),
        selected_candidate_line(&harness.viewport_contents()),
        harness.viewport_contents(),
    )
}

fn selected_candidate_line(contents: &str) -> Option<&str> {
    contents.lines().find(|line| {
        line.contains("› ")
            && !line.contains("正在加载候选")
            && !line.contains("环境刷新中")
            && !line.contains("无匹配命令")
            && !line.contains("提示 · 请继续输入")
            && !line.contains("Blueberry ")
            && !line.contains("请输入参数值")
    })
}

fn candidate_line_has(line: &str, label: &str) -> bool {
    line.contains(&format!(" {label} ")) || line.trim_end().ends_with(&format!(" {label}"))
}

#[test]
fn loading_notice_is_not_a_selectable_candidate() {
    assert!(selected_candidate_line("│ › 正在加载候选… │\n│ 提示 · 请继续输入 │").is_none());
    assert_eq!(
        selected_candidate_line("│ › 中文😀 文件.txt  文件 │"),
        Some("│ › 中文😀 文件.txt  文件 │")
    );
}

fn accept_selected(harness: &mut Harness) -> Result<Value> {
    harness.send(b"\t")?;
    let result = harness.event("edit_result", PTY_TIMEOUT)?;
    ensure!(
        result["applied"] == true,
        "selected edit was rejected: {result}"
    );
    Ok(result)
}

fn native_probe_command(path: &Path) -> String {
    format!(
        "function Test-BlueberryNative {{ param([string]$Value); Write-Output ('NATIVE:' + $Value) }}; Register-ArgumentCompleter -CommandName Test-BlueberryNative -ParameterName Value -ScriptBlock {{ param($commandName,$parameterName,$wordToComplete,$commandAst,$fakeBoundParameters); [IO.File]::AppendAllText({}, [string]::Concat('called',[char]10)); [System.Management.Automation.CompletionResult]::new('alpha','alpha',[System.Management.Automation.CompletionResultType]::ParameterValue,'native alpha') }}; Write-Output SS_NATIVE_READY",
        ps_quote(path)
    )
}

fn usage_snapshot(host: &mut RunningHost) -> Result<Value> {
    let path = host.data_dir.path().join("probe-usage.json");
    let deadline = Instant::now() + PTY_TIMEOUT;
    loop {
        if let Ok(bytes) = fs::read(&path)
            && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
            && value["entries"]
                .as_object()
                .is_some_and(|entries| !entries.is_empty())
        {
            return Ok(value);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!(
                "timed out waiting for applied learning statistic; screen:\n{}",
                host.harness.viewport_contents()
            );
        }
        let _ = host.harness.pump(remaining.min(Duration::from_millis(50)));
    }
}

#[test]
fn terminal_beta_multiline_continuation_keeps_safe_context_and_suppresses_unknown_text()
-> Result<()> {
    let mut host = start_host()?;
    for (prefix, tail) in [
        ("Write-Output |", "gi"),
        ("Write-Output (", "Get-Item"),
        ("Write-Output \"", "gi"),
        ("Write-Output '", "gi"),
        ("Write-Output \x60", "gi"),
    ] {
        clear_line(&mut host.harness)
            .with_context(|| format!("clear before continuation {prefix:?}"))?;
        host.harness.send(prefix.as_bytes())?;
        host.harness.send(b"\r")?;
        let editing = host
            .harness
            .event("editing", PTY_TIMEOUT)
            .with_context(|| format!("enter continuation {prefix:?}"))?;
        ensure!(
            editing["state"] == "continuation",
            "Enter did not report a confirmed continuation: {editing}"
        );
        host.harness.send(tail.as_bytes())?;
        let buffer = request_buffer(&mut host.harness, tail)
            .with_context(|| format!("confirm continuation {prefix:?} / {tail:?}"))?;
        let context = buffer["context"]
            .as_object()
            .with_context(|| format!("continuation omitted context: {buffer}"))?;
        ensure!(
            context["complex"] == true,
            "continuation context is not complex: {buffer}"
        );
        ensure!(
            context["suppressed"] == false,
            "safe continuation was suppressed: {buffer}"
        );
        ensure!(
            context["prefix"]
                .as_str()
                .is_some_and(|value| value.ends_with(tail)),
            "continuation prefix did not reach the live cursor: {buffer}"
        );
        cancel_to_prompt(&mut host.harness)
            .with_context(|| format!("cancel continuation {prefix:?}"))?;
    }

    // Here-string body text is intentionally opaque to the adapter.
    clear_line(&mut host.harness)?;
    host.harness.send(b"@\"")?;
    host.harness.send(b"\r")?;
    let editing = host.harness.event("editing", PTY_TIMEOUT)?;
    ensure!(
        editing["state"] == "continuation",
        "here-string Enter did not continue"
    );
    host.harness.send(b"BODY")?;
    let here = request_buffer(&mut host.harness, "BODY")?;
    ensure!(
        here["context"]["suppressed"] == true,
        "here-string body must suppress guessing: {here}"
    );
    cancel_to_prompt(&mut host.harness)?;

    clear_line(&mut host.harness)?;
    host.harness.send(b"# comment text")?;
    let comment = request_buffer(&mut host.harness, "# comment")?;
    ensure!(
        comment["context"]["suppressed"] == true,
        "comment text must suppress guessing: {comment}"
    );
    cancel_to_prompt(&mut host.harness)?;
    host.harness.finish(PTY_TIMEOUT)?;
    Ok(())
}

#[test]
fn terminal_beta_real_buffer_acceptance_preserves_suffix_quotes_and_unicode() -> Result<()> {
    let mut host = start_host()?;

    clear_line(&mut host.harness)?;
    host.harness.send(b"git")?;
    host.harness.send(b"\x1b[D\x1b[D")?;
    let _ = request_buffer(&mut host.harness, "git")?;
    select_candidate(&mut host.harness, "git")?;
    let _ = accept_selected(&mut host.harness)?;
    wait_until(
        &mut host.harness,
        "g|it acceptance",
        PTY_TIMEOUT,
        |screen| {
            screen
                .lines()
                .any(|line| line.trim_end().ends_with("> git"))
        },
    )?;

    clear_line(&mut host.harness)?;
    host.harness.send(b"giXYZ")?;
    host.harness.send(b"\x1b[D\x1b[D\x1b[D")?;
    let _ = request_buffer(&mut host.harness, "giXYZ")?;
    select_candidate(&mut host.harness, "git")?;
    let _ = accept_selected(&mut host.harness)?;
    wait_until(
        &mut host.harness,
        "right-hand suffix acceptance",
        PTY_TIMEOUT,
        |screen| screen.contains("> gitXYZ"),
    )?;

    clear_line(&mut host.harness)?;
    host.harness
        .send("Get-ChildItem -Name '中文😀".as_bytes())?;
    let _ = request_buffer(&mut host.harness, "Get-ChildItem -Name '中文😀")?;
    select_candidate(&mut host.harness, "中文😀 文件.txt")?;
    host.harness.send(b"\t")?;
    wait_until(
        &mut host.harness,
        "single quote and Unicode acceptance",
        PTY_TIMEOUT,
        |screen| {
            screen
                .lines()
                .any(|line| line.contains("Get-ChildItem -Name '") && line.contains("文件.txt'"))
        },
    )?;

    // A refreshed menu may legitimately own editor keys immediately after
    // acceptance. Start a fresh prompt for the independent double-quote case
    // instead of making this assertion depend on menu-dismissal timing.
    host.harness.stop()?;
    let mut host = start_host()?;
    host.harness
        .send("Get-ChildItem -Name \"中文😀".as_bytes())?;
    let _ = request_buffer(&mut host.harness, "中文😀")?;
    select_candidate(&mut host.harness, "中文😀 文件.txt")?;
    host.harness.send(b"\t")?;
    wait_until(
        &mut host.harness,
        "double quote and Unicode acceptance",
        PTY_TIMEOUT,
        |screen| {
            screen
                .lines()
                .any(|line| line.contains("Get-ChildItem -Name \"") && line.contains("文件.txt\""))
        },
    )?;

    host.harness.stop()?;
    Ok(())
}

#[test]
fn terminal_beta_native_completion_is_manual_and_uses_the_live_replacement_range() -> Result<()> {
    let mut host = start_host()?;
    send_command(
        &mut host.harness,
        &native_probe_command(&host.native_marker),
        "SS_NATIVE_READY",
    )?;

    clear_line(&mut host.harness)?;
    let line = "Test-BlueberryNative -Value a";
    host.harness.send(line.as_bytes())?;
    let buffer = request_buffer(&mut host.harness, "Test-BlueberryNative")?;
    ensure!(
        !host.native_marker.exists(),
        "automatic Rust completion invoked the user native completer: {buffer}"
    );

    // This is the physical Ctrl+Alt+Space event consumed by the configured
    // public native shortcut. The host then writes request.json and emits
    // the adapter's F12,n chord.
    host.harness.send(ctrl_alt_space_records())?;
    let native = host.harness.event("native_completion", PTY_TIMEOUT)?;
    ensure!(
        native["status"] == "ok",
        "manual native completion failed: {native}"
    );
    let native_line = native["line"].as_str().context("native line missing")?;
    let cursor = native["cursor"].as_u64().context("native cursor missing")?;
    let query_start = native_line
        .rfind('a')
        .context("native query suffix missing")?;
    let expected_start = native_line[..query_start].encode_utf16().count() as u64;
    ensure!(
        native["replace_start"] == expected_start && native["replace_end"] == cursor,
        "native completion did not retain its real replacement range: {native}"
    );
    let candidate = native["candidates"]
        .as_array()
        .and_then(|candidates| candidates.first())
        .context("native completion returned no candidates")?;
    ensure!(
        candidate["insert_text"] == "alpha",
        "unexpected native candidate: {native}"
    );
    ensure!(
        candidate["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("native alpha"))
            || candidate["description"]
                .as_str()
                .is_some_and(|description| description.contains("native alpha")),
        "native tooltip was not retained in the candidate details: {native}"
    );
    ensure!(
        fs::read_to_string(&host.native_marker)
            .unwrap_or_default()
            .contains("called"),
        "manual native request did not invoke the current-session completer"
    );
    select_candidate(&mut host.harness, "alpha")?;
    host.harness.send(b"\t")?;
    let applied = host.harness.event("edit_result", PTY_TIMEOUT)?;
    ensure!(
        applied["applied"] == true,
        "native edit was not applied: {applied}"
    );
    let inserted = request_buffer(&mut host.harness, "-Value alpha")?;
    ensure!(
        inserted["line"]
            .as_str()
            .is_some_and(|line| line.trim_end() == "Test-BlueberryNative -Value alpha"),
        "native replacement changed unrelated text: {inserted}"
    );
    host.harness.send(b"\r")?;
    host.harness.event("execute", PTY_TIMEOUT)?;
    host.harness.event("prompt_end", PTY_TIMEOUT)?;
    host.harness.wait_line("NATIVE:alpha", PTY_TIMEOUT)?;
    host.harness.finish(PTY_TIMEOUT)?;
    Ok(())
}

#[test]
fn terminal_beta_paste_and_history_mode_preserve_psreadline_editing() -> Result<()> {
    let mut host = start_host()?;
    send_command(
        &mut host.harness,
        "Write-Output SS_UP_HISTORY",
        "SS_UP_HISTORY",
    )?;
    clear_line(&mut host.harness)?;
    host.harness.send(b"gi")?;
    wait_until(&mut host.harness, "automatic menu", PTY_TIMEOUT, |screen| {
        screen.contains("› ")
    })?;
    host.harness.send(b"\x1b")?;
    wait_until(
        &mut host.harness,
        "completion menu dismissal before history navigation",
        PTY_TIMEOUT,
        |screen| !screen.lines().any(|line| line.contains("› ")),
    )?;
    host.harness.send(b"\x1b[A")?;
    wait_until(
        &mut host.harness,
        "complete PSReadLine history repaint",
        PTY_TIMEOUT,
        |screen| screen.contains("Write-Output SS_UP_HISTORY"),
    )?;
    let history = request_buffer(&mut host.harness, "Write-Output SS_UP_HISTORY")?;
    ensure!(
        history["line"] == "Write-Output SS_UP_HISTORY",
        "history mode did not reach PSReadLine history: {history}"
    );
    clear_line(&mut host.harness)?;
    let pasted = "Write-Output '粘贴😀 与空格'";
    host.harness
        .send(format!("\x1b[200~{pasted}\x1b[201~").as_bytes())?;
    let actual = read_real_buffer(&mut host.harness, "bracketed paste", pasted, None)?;
    ensure!(
        actual["line"] == pasted,
        "bracketed paste changed input: {actual}"
    );
    host.harness.send(b"\r")?;
    host.harness.event("execute", PTY_TIMEOUT)?;
    host.harness.event("prompt_end", PTY_TIMEOUT)?;
    host.harness.wait_line("粘贴😀 与空格", PTY_TIMEOUT)?;
    clear_line(&mut host.harness)?;
    let multiline = "Write-Output '第一行😀'\nWrite-Output '第二行😀'";
    host.harness
        .send(format!("\x1b[200~{multiline}\x1b[201~").as_bytes())?;
    let actual = read_real_buffer(
        &mut host.harness,
        "multiline bracketed paste",
        multiline,
        None,
    )?;
    ensure!(
        actual["line"] == multiline,
        "multiline paste executed or changed text before Enter: {actual}"
    );
    host.harness.send(b"\r")?;
    host.harness.event("execute", PTY_TIMEOUT)?;
    host.harness.event("prompt_end", PTY_TIMEOUT)?;
    host.harness.wait_line("第一行😀", PTY_TIMEOUT)?;
    host.harness.wait_line("第二行😀", PTY_TIMEOUT)?;
    clear_line(&mut host.harness)?;
    let clipboard_fixture = host._cwd.path().join("clipboard-fixture.txt");
    fs::write(
        &clipboard_fixture,
        "Get-PnpDevice -PresentOnly |\r\nWhere-Object {$_.InstanceId -like 'PCI\\VEN_15B7*'} |\r\nFormat-List *",
    )?;
    for chunk in [
        "Get-PnpDevice -PresentOnly |\r\n",
        "Where-Object {$_.InstanceId -like 'PCI\\VEN_15B7*'} |\r\n",
        "Format-List *",
    ] {
        host.harness.send(chunk.as_bytes())?;
        std::thread::sleep(Duration::from_millis(50));
    }
    let actual = read_real_buffer(
        &mut host.harness,
        "unmarked Windows Terminal multiline paste",
        "Get-PnpDevice -PresentOnly |\nWhere-Object {$_.InstanceId -like 'PCI\\VEN_15B7*'} |\nFormat-List *",
        None,
    )
    .with_context(|| {
        format!(
            "input metadata: {}",
            fs::read_to_string(host._cwd.path().join("input-trace.txt")).unwrap_or_default()
        )
    })?;
    ensure!(
        actual["line"]
            == "Get-PnpDevice -PresentOnly |\nWhere-Object {$_.InstanceId -like 'PCI\\VEN_15B7*'} |\nFormat-List *",
        "unmarked multiline paste lost or executed its first line: {actual}; input metadata: {}",
        fs::read_to_string(host._cwd.path().join("input-trace.txt")).unwrap_or_default()
    );
    fs::remove_file(clipboard_fixture)?;
    clear_line(&mut host.harness)?;
    host.harness.send(b"old selection")?;
    host.harness.send(b"\x01")?;
    host.harness.send(b"\x1b[200~one\r\ntwo\x1b[201~")?;
    let selected = read_real_buffer(
        &mut host.harness,
        "paste replaces selection",
        "one\ntwo",
        None,
    )?;
    ensure!(
        selected["line"] == "one\ntwo",
        "selection paste did not normalize and replace: {selected}"
    );
    host.harness.send(b"\x1a")?;
    wait_until(
        &mut host.harness,
        "paste undo repaint",
        PTY_TIMEOUT,
        |screen| screen.contains("old selection"),
    )?;
    let undone = request_buffer(&mut host.harness, "old selection")?;
    ensure!(
        undone["line"] == "old selection",
        "paste undo changed earlier text: {undone}"
    );
    clear_line(&mut host.harness)?;
    host.harness
        .send(b"\x1b[200~first \x1b[201~\x1b[200~second\x1b[201~")?;
    let queued = read_real_buffer(
        &mut host.harness,
        "consecutive paste order",
        "first second",
        None,
    )?;
    ensure!(
        queued["line"] == "first second",
        "consecutive paste payloads were overwritten: {queued}"
    );
    host.harness.finish(PTY_TIMEOUT)?;
    Ok(())
}

#[test]
fn terminal_beta_learning_records_only_an_applied_edit() -> Result<()> {
    let mut host = start_host()?;
    let usage_path = host.data_dir.path().join("probe-usage.json");

    // A stale payload is consumed and rejected before Replace is called. It
    // must not create a learning entry.
    if std::env::var("BLUEBERRY_TEST_TRANSPORT").as_deref() != Ok("pipe") {
        fs::write(
            &host.edit_path,
            serde_json::to_vec(&json!({
                "id": "stale-edit",
                "expectedLine": "different",
                "expectedCursor": 2,
                "start": 0,
                "length": 2,
                "text": "git"
            }))?,
        )?;
        host.harness.send(&protocol_chord('a'))?;
        let rejected = host.harness.event("error", PTY_TIMEOUT)?;
        ensure!(
            rejected["code"] == "edit_payload_rejected",
            "unexpected rejection: {rejected}"
        );
        wait_absent(&mut host.harness, &host.edit_path, "rejected edit payload")?;
    }
    if usage_path.exists() {
        let value: Value = serde_json::from_slice(&fs::read(&usage_path)?)?;
        ensure!(
            value["entries"]
                .as_object()
                .is_none_or(|entries| entries.is_empty()),
            "rejected edit changed local learning: {value}"
        );
    }

    clear_line(&mut host.harness)?;
    host.harness.send(b"gi")?;
    select_candidate(&mut host.harness, "git")?;
    host.harness.send(b"\t")?;
    let value = usage_snapshot(&mut host)?;
    let raw = fs::read_to_string(&usage_path)?;
    ensure!(
        !raw.contains("git") && !raw.contains(host._cwd.path().to_string_lossy().as_ref()),
        "learning file contains command or project text: {value}"
    );
    ensure!(
        value["entries"]
            .as_object()
            .is_some_and(|entries| !entries.is_empty()),
        "successful applied edit did not create a learning entry: {value}"
    );
    host.harness.finish(PTY_TIMEOUT)?;
    Ok(())
}

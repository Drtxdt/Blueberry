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
    buffer_marker: PathBuf,
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
fn terminal_purpose_search_inserts_command_tokens_and_restores_normal_completion() -> Result<()> {
    let mut host = start_host()?;
    let config_path = host.data_dir.path().join("config.toml");
    let mut config: config::Config = toml::from_str(&fs::read_to_string(&config_path)?)?;
    config.ui.icon_style = "nerd".into();
    fs::write(config_path, toml::to_string(&config)?)?;
    send_command(
        &mut host.harness,
        &buffer_probe_command(&host.buffer_marker),
        "SS_BUFFER_READY",
    )?;
    clear_line(&mut host.harness)?;
    host.harness.send("查看分支".as_bytes())?;
    let _ = request_buffer(&mut host.harness, "查看分支")?;
    host.harness.send(b"\x1b[102;7u")?;
    host.harness.wait_text("git branch", PTY_TIMEOUT)?;
    accept_selected(&mut host.harness)?;
    let buffer = read_real_buffer(
        &mut host.harness,
        &host.buffer_marker,
        "purpose search inserted command",
    )?;
    ensure!(
        buffer["line"]
            .as_str()
            .is_some_and(|s| s.trim_end() == "git branch"),
        "invalid search insertion: {buffer}"
    );
    clear_line(&mut host.harness)?;
    host.harness.send(b"codex --model ")?;
    let _ = request_buffer(&mut host.harness, "codex --model")?;
    host.harness.wait_text("<MODEL>", PTY_TIMEOUT)?;
    let buffer = read_real_buffer(
        &mut host.harness,
        &host.buffer_marker,
        "noninsertable argument hint",
    )?;
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
    accept_selected(&mut host.harness)?;
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
    let buffer_marker = cwd.path().join("buffer-state.json");
    let native_marker = cwd.path().join("native-called.txt");
    let input_trace = cwd.path().join("input-trace.txt");
    let clipboard_fixture = cwd.path().join("clipboard-fixture.txt");
    let env = BTreeMap::from([
        ("PATH".to_owned(), path.to_string_lossy().into_owned()),
        ("PATHEXT".to_owned(), ".COM;.EXE;.BAT;.CMD".to_owned()),
        ("TERM".to_owned(), "xterm-256color".to_owned()),
        ("NO_COLOR".to_owned(), "1".to_owned()),
        ("BLUEBERRY_NO_HISTORY".to_owned(), "1".to_owned()),
        // Debug hosts mirror protocol frames to this probe token without
        // changing the private child-shell token used by the live adapter.
        ("BLUEBERRY_PROBE_TOKEN".to_owned(), token.clone()),
        (
            "BLUEBERRY_TEST_BUFFER".to_owned(),
            buffer_marker.to_string_lossy().into_owned(),
        ),
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
        buffer_marker,
        native_marker,
    })
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
            .pump(remaining.min(Duration::from_millis(500)))
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

fn read_marker(harness: &mut Harness, path: &Path, description: &str) -> Result<Value> {
    let deadline = Instant::now() + PTY_TIMEOUT;
    loop {
        if let Ok(bytes) = fs::read(path)
            && !bytes.is_empty()
            && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
        {
            return Ok(value);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!(
                "timed out waiting for {description}; screen:\n{}",
                harness.viewport_contents()
            );
        }
        // A custom F10 handler writes the marker without necessarily
        // redrawing the terminal. Pump opportunistically while polling the
        // marker instead of guessing with a fixed sleep.
        let _ = harness.pump(remaining.min(Duration::from_millis(50)));
    }
}

fn read_real_buffer(harness: &mut Harness, path: &Path, description: &str) -> Result<Value> {
    let _ = fs::remove_file(path);
    // Keep the probe on one physical key. A multi-key PSReadLine chord can be
    // split when Blueberry opens a refreshed completion menu between the
    // prefix and suffix on slower Windows PowerShell 5.1 runners.
    harness.send(b"\x1b[21~")?;
    read_marker(harness, path, description)
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

fn accept_selected(harness: &mut Harness) -> Result<()> {
    harness.send(b"\t")?;
    let result = harness.event("edit_result", PTY_TIMEOUT)?;
    ensure!(
        result["applied"] == true,
        "selected edit was rejected: {result}"
    );
    // The adapter publishes the confirmed PSReadLine buffer immediately after
    // edit_result. Waiting for it serializes the next key chord with the
    // applied edit and prevents a fast completion refresh from racing the
    // probe on slower Windows PowerShell 5.1 runners.
    let _ = harness.event("buffer", PTY_TIMEOUT)?;
    Ok(())
}

fn buffer_probe_command(path: &Path) -> String {
    format!(
        "Set-PSReadLineKeyHandler -Chord 'F10' -ScriptBlock {{ $line=$null; $cursor=0; [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line,[ref]$cursor); $state=[object][ordered]@{{line=$line;cursor=$cursor}}; $json=($state | ConvertTo-Json -Compress -Depth 8); [IO.File]::WriteAllText({}, $json, [Text.UTF8Encoding]::new($false)) }}; Write-Output SS_BUFFER_READY",
        ps_quote(path)
    )
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
        clear_line(&mut host.harness)?;
        host.harness.send(prefix.as_bytes())?;
        host.harness.send(b"\r")?;
        let editing = host.harness.event("editing", PTY_TIMEOUT)?;
        ensure!(
            editing["state"] == "continuation",
            "Enter did not report a confirmed continuation: {editing}"
        );
        host.harness.send(tail.as_bytes())?;
        let buffer = request_buffer(&mut host.harness, tail)?;
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
        cancel_to_prompt(&mut host.harness)?;
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
fn terminal_beta_real_buffer_acceptance_preserves_suffix_quotes_unicode_and_undo() -> Result<()> {
    let mut host = start_host()?;
    send_command(
        &mut host.harness,
        &buffer_probe_command(&host.buffer_marker),
        "SS_BUFFER_READY",
    )?;

    clear_line(&mut host.harness)?;
    host.harness.send(b"git")?;
    host.harness.send(b"\x1b[D\x1b[D")?;
    let _ = request_buffer(&mut host.harness, "git")?;
    select_candidate(&mut host.harness, "git")?;
    accept_selected(&mut host.harness)?;
    let git = read_real_buffer(&mut host.harness, &host.buffer_marker, "g|it buffer")?;
    ensure!(
        git["line"] == "git",
        "g|it acceptance duplicated its suffix: {git}"
    );
    ensure!(
        git["cursor"] == 3,
        "g|it cursor is not after the command: {git}"
    );

    clear_line(&mut host.harness)?;
    host.harness.send(b"giXYZ")?;
    host.harness.send(b"\x1b[D\x1b[D\x1b[D")?;
    let _ = request_buffer(&mut host.harness, "giXYZ")?;
    select_candidate(&mut host.harness, "git")?;
    accept_selected(&mut host.harness)?;
    let suffix = read_real_buffer(&mut host.harness, &host.buffer_marker, "gi|XYZ buffer")?;
    ensure!(
        suffix["line"] == "gitXYZ",
        "acceptance did not preserve the right-hand suffix: {suffix}"
    );

    clear_line(&mut host.harness)?;
    host.harness
        .send("Get-ChildItem -Name '中文😀".as_bytes())?;
    let _ = request_buffer(&mut host.harness, "Get-ChildItem -Name '中文😀")?;
    select_candidate(&mut host.harness, "中文😀 文件.txt")?;
    accept_selected(&mut host.harness)?;
    let single = read_real_buffer(
        &mut host.harness,
        &host.buffer_marker,
        "single-quoted Unicode path",
    )?;
    ensure!(
        single["line"]
            .as_str()
            .is_some_and(|line| line.contains("'中文😀 文件.txt'")),
        "single quote or Unicode was not preserved: {single}"
    );
    host.harness.send(b"\r")?;
    host.harness.event("execute", PTY_TIMEOUT)?;
    host.harness.event("prompt_end", PTY_TIMEOUT)?;
    host.harness.wait_line("中文😀 文件.txt", PTY_TIMEOUT)?;

    clear_line(&mut host.harness)?;
    host.harness
        .send("Get-ChildItem -Name \"中文😀".as_bytes())?;
    let _ = request_buffer(&mut host.harness, "中文😀")?;
    let before_double = read_real_buffer(
        &mut host.harness,
        &host.buffer_marker,
        "pre-acceptance double-quoted buffer",
    )?;
    select_candidate(&mut host.harness, "中文😀 文件.txt")?;
    accept_selected(&mut host.harness)?;
    let double = read_real_buffer(
        &mut host.harness,
        &host.buffer_marker,
        "double-quoted Unicode path",
    )?;
    ensure!(
        double["line"]
            .as_str()
            .is_some_and(|line| line.contains("\"中文😀 文件.txt\"")),
        "double quote or Unicode was not preserved: {double}"
    );

    host.harness.send(b"\x1a")?;
    let undone = read_real_buffer(&mut host.harness, &host.buffer_marker, "PSReadLine undo")?;
    ensure!(
        undone["line"] == before_double["line"] && undone["cursor"] == before_double["cursor"],
        "Undo did not restore the real PSReadLine buffer: before={before_double}, after={undone}"
    );
    host.harness.finish(PTY_TIMEOUT)?;
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
        &buffer_probe_command(&host.buffer_marker),
        "SS_BUFFER_READY",
    )?;
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
    let history = read_real_buffer(&mut host.harness, &host.buffer_marker, "history mode Up")?;
    ensure!(
        history["line"] == "Write-Output SS_UP_HISTORY",
        "history mode did not reach PSReadLine history: {history}"
    );
    clear_line(&mut host.harness)?;
    let pasted = "Write-Output '粘贴😀 与空格'";
    host.harness
        .send(format!("\x1b[200~{pasted}\x1b[201~").as_bytes())?;
    let actual = read_real_buffer(&mut host.harness, &host.buffer_marker, "bracketed paste")?;
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
        &host.buffer_marker,
        "multiline bracketed paste",
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
        &host.buffer_marker,
        "unmarked Windows Terminal multiline paste",
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
        &host.buffer_marker,
        "paste replaces selection",
    )?;
    ensure!(
        selected["line"] == "one\ntwo",
        "selection paste did not normalize and replace: {selected}"
    );
    host.harness.send(b"\x1a")?;
    let undone = read_real_buffer(&mut host.harness, &host.buffer_marker, "paste undo")?;
    ensure!(
        undone["line"] == "old selection",
        "paste undo changed earlier text: {undone}"
    );
    clear_line(&mut host.harness)?;
    host.harness
        .send(b"\x1b[200~first \x1b[201~\x1b[200~second\x1b[201~")?;
    let queued = read_real_buffer(
        &mut host.harness,
        &host.buffer_marker,
        "consecutive paste order",
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

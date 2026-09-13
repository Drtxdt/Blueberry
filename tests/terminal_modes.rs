#![cfg(all(windows, debug_assertions))]

//! Real ConPTY regressions for terminal modes and long output.
//!
//! Each case starts an isolated Blueberry process with a temporary config,
//! data directory, working directory, and `BLUEBERRY_NO_HISTORY=1`. The
//! child PowerShell used by the host is the same executable selected for the
//! external `pwsh -NoProfile -File` fixture. The tests intentionally do not
//! modify a profile, global settings, or a real repository.
//!
//! The mouse case launches the ignored helper below inside the host's nested
//! PowerShell ConPTY.  The helper reads the resulting Windows console input
//! records directly, so the test covers the real outer-ConPTY-to-child path
//! rather than only the deterministic `input::mouse_bytes` encoder.

use anyhow::{Context, Result, bail, ensure};
use blueberry::{config, probe::Harness};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tempfile::{TempDir, tempdir};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    System::Console::{
        ENABLE_ECHO_INPUT, ENABLE_EXTENDED_FLAGS, ENABLE_LINE_INPUT, ENABLE_MOUSE_INPUT,
        ENABLE_PROCESSED_INPUT, ENABLE_QUICK_EDIT_MODE, ENABLE_VIRTUAL_TERMINAL_INPUT,
        ENABLE_WINDOW_INPUT, GetConsoleMode, INPUT_RECORD, KEY_EVENT, MOUSE_EVENT,
        ReadConsoleInputW, SetConsoleMode,
    },
};

const PTY_TIMEOUT: Duration = Duration::from_secs(20);
const MOUSE_HELPER_LOG_ENV: &str = "BLUEBERRY_MOUSE_HELPER_LOG";
const TEST_TRANSPORT_ENV: &str = "BLUEBERRY_TEST_TRANSPORT";
const MOUSE_COLUMN: i16 = 11;
const MOUSE_ROW: i16 = 6;
const VK_F12: u16 = 0x7b;
const VK_Q: u16 = 0x51;
const FROM_LEFT_1ST_BUTTON_PRESSED: u32 = 0x0001;

unsafe extern "system" {
    fn CreateFileW(
        lp_file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *const core::ffi::c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: HANDLE,
    ) -> HANDLE;
    fn CloseHandle(handle: HANDLE) -> i32;
}

fn open_console_input() -> Result<HANDLE> {
    let name: Vec<u16> = "CONIN$\0".encode_utf16().collect();
    let input = unsafe {
        CreateFileW(
            name.as_ptr(),
            0x8000_0000 | 0x4000_0000,
            0x0000_0001 | 0x0000_0002,
            std::ptr::null(),
            3,
            0,
            std::ptr::null_mut(),
        )
    };
    if input.is_null() || input == INVALID_HANDLE_VALUE {
        bail!("无法打开 CONIN$：{}", std::io::Error::last_os_error());
    }
    Ok(input)
}

struct ConsoleModeGuard {
    input: HANDLE,
    original: u32,
}

impl Drop for ConsoleModeGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = SetConsoleMode(self.input, self.original);
            let _ = CloseHandle(self.input);
        }
    }
}

/// This ignored test is intentionally run as a child of the real host test.
/// Keeping it in this integration binary means no helper executable or fake
/// product needs to be built or checked into the repository.
#[test]
#[ignore = "launched by conpty_mouse_passthrough_for_external_program"]
fn conpty_mouse_input_record_helper() -> Result<()> {
    let log_path = std::env::var_os(MOUSE_HELPER_LOG_ENV)
        .map(PathBuf::from)
        .context("missing helper log path")?;
    let mut log = fs::File::create(&log_path).context("create mouse helper log")?;
    let input = open_console_input()?;
    let mut original = 0u32;
    if unsafe { GetConsoleMode(input, &mut original) } == 0 {
        unsafe { CloseHandle(input) };
        bail!(
            "GetConsoleMode(CONIN$) 失败：{}",
            std::io::Error::last_os_error()
        );
    }
    let _mode_guard = ConsoleModeGuard { input, original };
    let mode = (original
        & !(ENABLE_LINE_INPUT
            | ENABLE_ECHO_INPUT
            | ENABLE_PROCESSED_INPUT
            | ENABLE_QUICK_EDIT_MODE
            | ENABLE_VIRTUAL_TERMINAL_INPUT))
        | ENABLE_EXTENDED_FLAGS
        | ENABLE_WINDOW_INPUT
        | ENABLE_MOUSE_INPUT;
    if unsafe { SetConsoleMode(input, mode) } == 0 {
        bail!(
            "SetConsoleMode(CONIN$) 失败：{}",
            std::io::Error::last_os_error()
        );
    }

    writeln!(log, "READY")?;
    log.flush()?;
    {
        let mut stdout = io::stdout().lock();
        // SGR (1006) is enabled after the legacy tracking mode (1000), just
        // as a real full-screen terminal application does.
        stdout.write_all(b"\x1b[?1000h\x1b[?1006hSS_MOUSE_READY\r\n")?;
        stdout.flush()?;
    }

    let mut records = vec![INPUT_RECORD::default(); 32];
    let mut saw_down = false;
    let mut saw_up = false;
    let mut saw_sentinel = false;
    let mut saw_f12 = false;
    'read: for batch in 0..256 {
        let mut count = 0u32;
        if unsafe {
            ReadConsoleInputW(
                input,
                records.as_mut_ptr(),
                records.len() as u32,
                &mut count,
            )
        } == 0
        {
            bail!(
                "ReadConsoleInputW 失败：{}",
                std::io::Error::last_os_error()
            );
        }
        for record in records.iter().take(count as usize) {
            match record.EventType as u32 {
                KEY_EVENT => {
                    let key = unsafe { record.Event.KeyEvent };
                    let unicode = unsafe { key.uChar.UnicodeChar };
                    writeln!(
                        log,
                        "KEY batch={batch} vk={} down={} repeat={} unicode={} ctrl=0x{:08x}",
                        key.wVirtualKeyCode,
                        key.bKeyDown,
                        key.wRepeatCount,
                        unicode,
                        key.dwControlKeyState,
                    )?;
                    if key.bKeyDown != 0 && key.wVirtualKeyCode == VK_F12 {
                        saw_f12 = true;
                    }
                    if key.bKeyDown != 0 && (key.wVirtualKeyCode == VK_Q || unicode == b'q' as u16)
                    {
                        saw_sentinel = true;
                    }
                }
                MOUSE_EVENT => {
                    let mouse = unsafe { record.Event.MouseEvent };
                    writeln!(
                        log,
                        "MOUSE batch={batch} x={} y={} buttons=0x{:08x} ctrl=0x{:08x} flags=0x{:08x}",
                        mouse.dwMousePosition.X,
                        mouse.dwMousePosition.Y,
                        mouse.dwButtonState,
                        mouse.dwControlKeyState,
                        mouse.dwEventFlags,
                    )?;
                    if mouse.dwMousePosition.X == MOUSE_COLUMN
                        && mouse.dwMousePosition.Y == MOUSE_ROW
                        && mouse.dwEventFlags == 0
                        && mouse.dwButtonState & FROM_LEFT_1ST_BUTTON_PRESSED != 0
                    {
                        saw_down = true;
                    }
                    if mouse.dwMousePosition.X == MOUSE_COLUMN
                        && mouse.dwMousePosition.Y == MOUSE_ROW
                        && mouse.dwEventFlags == 0
                        && mouse.dwButtonState == 0
                    {
                        saw_up = true;
                    }
                }
                event_type => {
                    writeln!(log, "OTHER batch={batch} event_type={event_type}")?;
                }
            }
        }
        log.flush()?;
        if saw_sentinel {
            break 'read;
        }
    }
    ensure!(saw_down, "helper 未收到期望的左键按下记录");
    ensure!(saw_up, "helper 未收到期望的左键释放记录");
    ensure!(saw_sentinel, "helper 未收到退出哨兵 q");
    writeln!(log, "DONE f12={}", u8::from(saw_f12))?;
    log.flush()?;
    {
        let mut stdout = io::stdout().lock();
        stdout.write_all(b"\x1b[?1006l\x1b[?1000lSS_MOUSE_DONE\r\n")?;
        stdout.flush()?;
    }
    ensure!(!saw_f12, "helper 收到了不应注入的 F12");
    Ok(())
}

struct RunningHost {
    harness: Harness,
    _data_dir: TempDir,
    _cwd: TempDir,
    shell: PathBuf,
}

fn selected_pwsh() -> PathBuf {
    std::env::var_os("BLUEBERRY_TEST_SHELL")
        .map(PathBuf::from)
        .unwrap_or_else(blueberry::pty::default_shell)
}

fn ps_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

fn startup_event(harness: &mut Harness, name: &str, phase: &str) -> Result<Value> {
    harness
        .event(name, PTY_TIMEOUT)
        .with_context(|| format!("startup phase {phase}: waiting for {name} event"))
}

fn selected_transport() -> Result<String> {
    let transport = std::env::var(TEST_TRANSPORT_ENV).unwrap_or_else(|_| "osc".to_owned());
    ensure!(
        matches!(transport.as_str(), "osc" | "pipe"),
        "{TEST_TRANSPORT_ENV} must be `osc` or `pipe`, got {transport:?}"
    );
    Ok(transport)
}

fn ctrl_space_records() -> &'static [u8] {
    // VK_SPACE=32, scan code=57, Ctrl only (control state 8). A bare NUL is
    // rendered as a literal `2` by the outer ConPTY instead of a shortcut.
    b"\x1b[32;57;0;1;8;1_\x1b[32;57;0;0;8;1_"
}

fn start_host() -> Result<RunningHost> {
    let transport = selected_transport()?;
    let cwd = tempdir().context("create terminal-modes working directory")?;
    fs::write(
        cwd.path().join("git.cmd"),
        b"@echo off\r\necho SS_RESIZE_GIT_OK\r\n",
    )
    .context("write deterministic git.cmd")?;

    let data_dir = tempdir().context("create terminal-modes data directory")?;
    let config_path = data_dir.path().join("config.toml");
    fs::write(
        &config_path,
        config::example().replace("icon_style = \"nerd\"", "icon_style = \"unicode\""),
    )
    .context("write temporary config")?;

    let shell = selected_pwsh();
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![cwd.path().to_owned()];
    paths.extend(std::env::split_paths(&inherited_path));
    let path = std::env::join_paths(paths).context("construct isolated PATH")?;
    let token = format!("terminal-modes-{}", uuid::Uuid::new_v4());
    let env = BTreeMap::from([
        ("PATH".to_owned(), path.to_string_lossy().into_owned()),
        ("PATHEXT".to_owned(), ".COM;.EXE;.BAT;.CMD".to_owned()),
        ("TERM".to_owned(), "xterm-256color".to_owned()),
        ("NO_COLOR".to_owned(), "1".to_owned()),
        // This keeps PSReadLine history and learning output inside data_dir.
        ("BLUEBERRY_NO_HISTORY".to_owned(), "1".to_owned()),
        ("BLUEBERRY_PROBE_TOKEN".to_owned(), token.clone()),
    ]);
    let program = PathBuf::from(env!("CARGO_BIN_EXE_blueberry"));
    let args = vec![
        "--config".to_owned(),
        config_path.to_string_lossy().into_owned(),
        "run".to_owned(),
        "--transport".to_owned(),
        transport.clone(),
        "--shell".to_owned(),
        shell.to_string_lossy().into_owned(),
        "--no-profile".to_owned(),
        "--data-dir".to_owned(),
        data_dir.path().to_string_lossy().into_owned(),
    ];
    let mut harness = Harness::start(&program, &args, cwd.path(), &env, token)
        .with_context(|| format!("start {}", program.display()))?;
    harness
        .wait_text("PS ", PTY_TIMEOUT)
        .context("startup phase initial prompt: waiting for PowerShell prompt")?;
    // The bootstrap path can publish an intentionally incomplete capability
    // frame before ConsoleHost loads PSReadLine. The first prompt may publish
    // a ready frame after registration. Explicitly consume capability frames
    // until the test observes ready=true, then use phase-specific context for
    // prompt and command waits. Harness::event consumes intervening messages;
    // this keeps startup readiness an explicit test assertion and makes a
    // missing frame report the phase that timed out. It does not infer the
    // cause of any host-side event ordering.
    let mut capabilities = startup_event(&mut harness, "capabilities", "capabilities")?;
    while capabilities["ready"].as_bool() != Some(true) {
        capabilities = startup_event(&mut harness, "capabilities", "ready capabilities")?;
    }
    ensure!(
        capabilities["transport"].as_str() == Some(transport.as_str()),
        "requested transport was not active: requested={transport}, capabilities={capabilities}"
    );
    startup_event(&mut harness, "prompt_end", "first prompt")?;
    // Terminal-mode checks are interaction tests, rather than startup
    // benchmarks. Let the lazy command snapshot finish before the first
    // query, so a later partial page cannot change the row being navigated.
    loop {
        if startup_event(&mut harness, "commands", "command snapshot")?["complete"] == true {
            break;
        }
    }
    Ok(RunningHost {
        harness,
        _data_dir: data_dir,
        _cwd: cwd,
        shell,
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
    // Do not concatenate Esc with Ctrl+A: VT input may decode that as an Alt
    // chord. Observe the actual PSReadLine buffer after clearing, so a queued
    // query cannot be paired with the previous line during a slow redraw.
    harness.send(b"\x01\x7f")?;
    harness.send(ctrl_space_records())?;
    loop {
        let value = harness.event("buffer", PTY_TIMEOUT)?;
        if value["line"] == "" && value["cursor"] == 0 {
            return Ok(());
        }
    }
}

fn clear_line_and_send(harness: &mut Harness, bytes: &[u8], description: &str) -> Result<()> {
    clear_line(harness).with_context(|| format!("clear line before {description}"))?;
    harness
        .send(bytes)
        .with_context(|| format!("send {description}"))
}

fn run_and_wait_for_marker(
    harness: &mut Harness,
    command: &str,
    marker: &str,
    description: &str,
) -> Result<()> {
    clear_line_and_send(harness, format!("{command}\r").as_bytes(), description)?;
    harness.event("execute", PTY_TIMEOUT)?;
    harness
        .wait_text(marker, PTY_TIMEOUT)
        .with_context(|| format!("wait for {description} output"))?;
    harness.event("prompt_end", PTY_TIMEOUT)?;
    Ok(())
}

fn request_real_buffer(harness: &mut Harness, expected_fragment: &str) -> Result<Value> {
    harness.send(ctrl_space_records())?;
    loop {
        let value = harness.event("buffer", PTY_TIMEOUT)?;
        let Some(line) = value["line"].as_str() else {
            continue;
        };
        let matches_expected = if expected_fragment.is_empty() {
            line.is_empty() && value["cursor"] == 0
        } else {
            line.contains(expected_fragment)
        };
        if matches_expected {
            return Ok(value);
        }
    }
}

fn selected_row(screen: &str) -> Option<String> {
    screen
        .lines()
        .find(|line| line.contains("› "))
        .map(str::to_owned)
}

fn selected_row_matches(screen: &str, label: &str) -> bool {
    screen.lines().any(|line| {
        let Some((_, row)) = line.split_once("› ") else {
            return false;
        };
        let row = ["⌘ ", "≈ ", "ƒ ", "◇ ", "↳ ", "− ", "□ ", "▰ ", "• "]
            .iter()
            .find_map(|icon| row.strip_prefix(*icon))
            .unwrap_or(row);
        let Some(rest) = row.strip_prefix(label) else {
            return false;
        };
        // Descriptions and padding begin with whitespace. This boundary
        // rejects a longer candidate such as `git-gui` when seeking `git`.
        rest.is_empty() || rest.starts_with(char::is_whitespace)
    })
}

fn select_candidate(harness: &mut Harness, label: &str) -> Result<()> {
    wait_until(
        harness,
        &format!("candidate {label}"),
        PTY_TIMEOUT,
        |screen| selected_row(screen).is_some(),
    )?;
    let deadline = Instant::now() + PTY_TIMEOUT;
    for _ in 0..1024 {
        let screen = harness.viewport_contents();
        if selected_row_matches(&screen, label) {
            return Ok(());
        }
        let before = selected_row(&screen).unwrap_or_default();
        harness.send(b"\x1b[B")?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        wait_until(harness, "menu navigation repaint", remaining, |screen| {
            selected_row(screen).is_some_and(|line| line != before)
        })?;
    }
    bail!(
        "candidate {label} was never selected; screen:\n{}",
        harness.viewport_contents()
    )
}

fn alternate_screen_fixture() -> &'static str {
    r#"param(
    [Parameter(Mandatory = $true)][string]$LogPath
)
$utf8 = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = $utf8
[Console]::InputEncoding = $utf8
$enter = [string]::Concat([char]27, '[?1049h', [char]27, '[2J', [char]27, '[H')
$leave = [string]::Concat([char]27, '[?1049l')
[Console]::Write($enter)
[Console]::WriteLine('SS_ALT_READY')
[Console]::Out.Flush()
while ($true) {
    $key = [Console]::ReadKey($true)
    $line = "key=$($key.Key);char=$([int][char]$key.KeyChar)"
    [System.IO.File]::AppendAllText($LogPath, $line + [Environment]::NewLine, $utf8)
    if ($key.Key -eq [ConsoleKey]::F12) {
        [Console]::WriteLine('SS_ALT_F12')
        [Console]::Out.Flush()
    }
    if ($key.Key -eq [ConsoleKey]::Q -and $key.KeyChar -eq 'q') {
        break
    }
}
[Console]::Write($leave)
[Console]::Out.Flush()
[Console]::WriteLine('SS_ALT_EXIT_MARKER')
[Console]::Out.Flush()
"#
}

#[test]
fn alternate_screen_for_external_pwsh_restores_main_screen_and_does_not_inject_f12() -> Result<()> {
    let mut host = start_host()?;
    let script_path = host._cwd.path().join("alternate-screen.ps1");
    let log_path = host._cwd.path().join("alternate-screen.keys");
    fs::write(&script_path, alternate_screen_fixture())
        .context("write alternate-screen fixture")?;

    run_and_wait_for_marker(
        &mut host.harness,
        "Write-Output SS_MAIN_BEFORE",
        "SS_MAIN_BEFORE",
        "main-screen marker",
    )?;
    let command = format!(
        "& {} -NoLogo -NoProfile -File {} {}",
        ps_quote(&host.shell),
        ps_quote(&script_path),
        ps_quote(&log_path)
    );
    clear_line_and_send(
        &mut host.harness,
        format!("{command}\r").as_bytes(),
        "external alternate-screen PowerShell",
    )?;
    host.harness.event("execute", PTY_TIMEOUT)?;
    host.harness
        .wait_text("SS_ALT_READY", PTY_TIMEOUT)
        .context("wait for alternate-screen ready marker")?;

    // A resize while the child owns the alternate screen must be forwarded to
    // the child without repainting a Blueberry menu over it.
    host.harness
        .resize(12, 60)
        .context("resize alternate screen small")?;
    host.harness
        .resize(34, 140)
        .context("resize alternate screen large")?;
    host.harness.send(b"q")?;
    host.harness
        .wait_text("SS_ALT_EXIT_MARKER", PTY_TIMEOUT)
        .context("wait for restored-screen marker")?;
    host.harness.event("prompt_end", PTY_TIMEOUT)?;

    let keys = fs::read_to_string(&log_path).context("read alternate-screen key log")?;
    ensure!(
        keys.contains("key=Q;"),
        "fixture did not receive the q key: {keys}"
    );
    ensure!(
        !keys.lines().any(|line| line.starts_with("key=F12;")),
        "Blueberry injected a protocol F12 while the external program was active: {keys}"
    );
    let screen = host.harness.viewport_contents();
    ensure!(
        screen.contains("SS_MAIN_BEFORE") && screen.contains("SS_ALT_EXIT_MARKER"),
        "main screen was not restored after alternate-screen exit:\n{screen}"
    );
    ensure!(
        !screen.contains("SS_ALT_READY") && !screen.contains("SS_ALT_F12"),
        "alternate-screen contents leaked into the restored main screen:\n{screen}"
    );

    // The same prompt must accept a real command candidate after the nested
    // full-screen process exits. This catches a stale `prompt=false` state as
    // well as an overlay left at the alternate screen's old coordinates.
    host.harness.send(b"gi")?;
    host.harness
        .wait_text("⌘ git ", PTY_TIMEOUT)
        .context("wait for Git menu after alternate-screen exit")?;
    select_candidate(&mut host.harness, "git")?;
    host.harness.send(b"\t")?;
    wait_until(
        &mut host.harness,
        "dismissed menu after alternate-screen exit",
        Duration::from_secs(5),
        |screen| !screen.contains("⌘ git "),
    )?;
    host.harness.send(b"\r")?;
    host.harness.event("execute", PTY_TIMEOUT)?;
    host.harness
        .wait_text("SS_RESIZE_GIT_OK", PTY_TIMEOUT)
        .context("execute Git candidate after alternate-screen exit")?;
    host.harness.event("prompt_end", PTY_TIMEOUT)?;
    host.harness.finish(PTY_TIMEOUT)?;
    Ok(())
}

#[test]
fn repeated_resize_and_large_output_preserve_menu_and_real_buffer() -> Result<()> {
    let mut host = start_host()?;

    run_and_wait_for_marker(
        &mut host.harness,
        "Write-Output SS_LARGE_OUTPUT_START",
        "SS_LARGE_OUTPUT_START",
        "large-output start marker",
    )?;
    run_and_wait_for_marker(
        &mut host.harness,
        "1..768 | ForEach-Object { 'SS_BULK_{0:D4}' -f $_ }; Write-Output SS_LARGE_OUTPUT_END",
        "SS_LARGE_OUTPUT_END",
        "fixed large output",
    )?;
    ensure!(
        host.harness.contents().contains("SS_BULK_0768"),
        "large output did not reach the ConPTY screen model"
    );

    // Request the real PSReadLine buffer at the fresh prompt before opening a
    // menu. The physical console record avoids the NUL shortcut ambiguity.
    let empty = request_real_buffer(&mut host.harness, "")?;
    ensure!(
        empty["line"] == "" && empty["cursor"] == 0,
        "buffer was not empty after large output: {empty}"
    );

    clear_line_and_send(&mut host.harness, b"gi", "post-output Git query")?;
    host.harness
        .wait_text("⌘ git ", PTY_TIMEOUT)
        .context("wait for menu after large output")?;

    for (rows, cols, label) in [
        (12, 60, "small resize"),
        (34, 140, "large resize"),
        (10, 50, "second small resize"),
        (30, 120, "final large resize"),
    ] {
        host.harness
            .resize(rows, cols)
            .with_context(|| format!("resize {label}"))?;
        wait_until(
            &mut host.harness,
            &format!("Git menu after {label}"),
            Duration::from_secs(5),
            |screen| screen.contains("⌘ git "),
        )?;
    }

    let before_accept = request_real_buffer(&mut host.harness, "gi")?;
    ensure!(
        before_accept["line"] == "gi" && before_accept["cursor"] == 2,
        "resize damaged the real PSReadLine buffer: {before_accept}"
    );
    select_candidate(&mut host.harness, "git")?;
    host.harness.send(b"\t")?;
    wait_until(
        &mut host.harness,
        "accepted post-output Git candidate",
        Duration::from_secs(5),
        |screen| !screen.contains("⌘ git "),
    )?;
    let after_accept = request_real_buffer(&mut host.harness, "git")?;
    let accepted_line = after_accept["line"].as_str().unwrap_or_default();
    ensure!(
        accepted_line == "git" || accepted_line == "git ",
        "Tab did not update the real buffer after resize/output: {after_accept}"
    );
    ensure!(
        after_accept["cursor"]
            .as_u64()
            .is_some_and(|cursor| cursor >= 3),
        "accepted Git cursor is invalid: {after_accept}"
    );
    // Enter must continue to execute the current line even while the buffer
    // probe has refreshed a menu. Keeping Esc out of this sequence avoids the
    // Windows terminal decoding an adjacent Esc+Enter as an Alt chord.
    host.harness.send(b"\r")?;
    host.harness
        .wait_text("SS_RESIZE_GIT_OK", PTY_TIMEOUT)
        .context("execute accepted Git after resize/output")?;
    host.harness.event("prompt_end", PTY_TIMEOUT)?;
    host.harness.finish(PTY_TIMEOUT)?;
    Ok(())
}

#[test]
fn conpty_mouse_passthrough_for_external_program() -> Result<()> {
    let mut host = start_host()?;
    let helper_executable = std::env::current_exe().context("locate terminal-modes test exe")?;
    let log_path = host._cwd.path().join("mouse-input-records.log");
    let command = format!(
        "$env:{MOUSE_HELPER_LOG_ENV} = {}; & {} --exact --ignored --nocapture conpty_mouse_input_record_helper",
        ps_quote(&log_path),
        ps_quote(&helper_executable),
    );

    clear_line_and_send(
        &mut host.harness,
        format!("{command}\r").as_bytes(),
        "external mouse helper",
    )?;
    host.harness.event("execute", PTY_TIMEOUT)?;
    host.harness
        .wait_text("SS_MOUSE_READY", PTY_TIMEOUT)
        .context("wait for external mouse helper readiness")?;

    // The bytes enter the same outer ConPTY input stream as a terminal
    // emulator's SGR mouse report.  Blueberry must decode them as a mouse
    // event, encode them for the nested child, and avoid its private F12
    // protocol chord while the child owns mouse mode.
    host.harness
        .send(b"\x1b[<0;12;7M\x1b[<0;12;7mq")
        .context("send SGR mouse down/up and helper sentinel")?;
    host.harness
        .wait_text("SS_MOUSE_DONE", PTY_TIMEOUT)
        .context("wait for external mouse helper completion")?;
    host.harness.event("prompt_end", PTY_TIMEOUT)?;

    let log = fs::read_to_string(&log_path).context("read mouse helper input log")?;
    ensure!(
        log.lines().any(|line| {
            line.contains("MOUSE")
                && line.contains("x=11 y=6")
                && line.contains("buttons=0x00000001")
                && line.contains("flags=0x00000000")
        }),
        "helper did not receive expected left-button down at SGR (12,7):\n{log}"
    );
    ensure!(
        log.lines().any(|line| {
            line.contains("MOUSE")
                && line.contains("x=11 y=6")
                && line.contains("buttons=0x00000000")
                && line.contains("flags=0x00000000")
        }),
        "helper did not receive expected left-button release at SGR (12,7):\n{log}"
    );
    ensure!(
        !log.lines()
            .any(|line| line.starts_with("KEY") && line.contains("vk=123")),
        "Blueberry injected F12 into the external helper:\n{log}"
    );
    ensure!(
        log.contains("DONE f12=0"),
        "external helper did not report a clean mouse-mode exit:\n{log}"
    );

    // The helper restored its console mode and disabled mouse tracking before
    // returning. A normal PowerShell command must still run at the same
    // prompt afterwards.
    run_and_wait_for_marker(
        &mut host.harness,
        "Write-Output SS_MOUSE_AFTER",
        "SS_MOUSE_AFTER",
        "PowerShell after external mouse helper",
    )?;
    host.harness.finish(PTY_TIMEOUT)?;
    Ok(())
}

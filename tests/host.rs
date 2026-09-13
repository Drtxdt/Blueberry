#![cfg(windows)]

use anyhow::{Context, Result, bail, ensure};
use blueberry::{config, probe::Harness};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::{TempDir, tempdir};

const PTY_TIMEOUT: Duration = Duration::from_secs(20);

struct RunningHost {
    harness: Harness,
    // Keep the data directory alive until the host process has exited. The
    // host writes its integration script, command cache, and edit file here.
    _data_dir: TempDir,
}

#[test]
fn host_reloads_context_descriptions_and_applies_the_same_real_buffer() -> Result<()> {
    let cwd = tempdir()?;
    std::fs::write(
        cwd.path().join("git.cmd"),
        b"@echo off\r\necho GIT_ARGS:%*\r\n",
    )?;
    let mut host = start_host(cwd.path())?;
    host.harness.send(b"git log --o")?;
    host.harness.wait_text("每条提交显示为一行", PTY_TIMEOUT)?;
    let config_path = host._data_dir.path().join("config.toml");
    let mut settings = config::Config::default();
    settings
        .descriptions
        .insert("git log --oneline".into(), "自定义精简历史".into());
    std::fs::write(&config_path, toml::to_string(&settings)?)?;
    // Ctrl+Alt+R in the Windows console input encoding used by the host.
    let reload = b"\x1b[82;19;18;1;10;1_\x1b[82;19;18;0;10;1_";
    host.harness.send(reload)?;
    host.harness.wait_text("自定义精简历史", PTY_TIMEOUT)?;
    settings
        .descriptions
        .insert("git log --oneline".into(), "更新后的中文说明".into());
    std::fs::write(&config_path, toml::to_string(&settings)?)?;
    host.harness.send(reload)?;
    host.harness.wait_text("更新后的中文说明", PTY_TIMEOUT)?;
    // Invalid reload preserves the last usable settings and current text.
    std::fs::write(&config_path, "[descriptions]\n\"git\" = \"\"\n")?;
    host.harness.send(reload)?;
    host.harness.send(b"\t")?;
    wait_until(
        &mut host.harness,
        "reloaded completion accepted",
        |screen| {
            screen
                .lines()
                .any(|line| line.trim_end().ends_with("> git log --oneline"))
        },
    )?;
    let previous_prompt_count = prompt_count(&host.harness.contents());
    host.harness.send(b"\r")?;
    wait_for_line(
        &mut host.harness,
        "GIT_ARGS:log --oneline",
        "actual accepted argument execution",
    )?;
    wait_for_next_prompt(
        &mut host.harness,
        previous_prompt_count,
        "prompt before inline choice",
    )?;
    host.harness.send(b"git status --ignored=mat")?;
    host.harness.wait_text("--ignored=matching", PTY_TIMEOUT)?;
    host.harness.send(b"\t")?;
    wait_until(&mut host.harness, "inline value accepted", |screen| {
        screen
            .lines()
            .any(|line| line.trim_end().ends_with("> git status --ignored=matching"))
    })?;
    host.harness.send(b"\r")?;
    wait_for_line(
        &mut host.harness,
        "GIT_ARGS:status --ignored=matching",
        "actual inline value execution",
    )?;
    host.harness.stop()?;
    Ok(())
}

fn start_host(cwd: &Path) -> Result<RunningHost> {
    let data_dir = tempdir().context("create host data directory")?;
    let config_path = data_dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        config::example().replace("icon_style = \"nerd\"", "icon_style = \"unicode\""),
    )
    .context("write test config")?;
    let program = PathBuf::from(env!("CARGO_BIN_EXE_blueberry"));
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let args = vec![
        "--config".to_owned(),
        config_path.to_string_lossy().into_owned(),
        "run".to_owned(),
        "--no-profile".to_owned(),
        "--data-dir".to_owned(),
        data_dir.path().to_string_lossy().into_owned(),
    ];

    // Put a deterministic git.cmd first in PATH. CommandIndex labels PATH
    // entries as Executable, which gives this test a stable completion row
    // without relying on a particular Git installation.
    let mut paths = vec![cwd.to_owned()];
    paths.extend(std::env::split_paths(&inherited_path));
    let path = std::env::join_paths(paths).context("construct test PATH")?;
    let env = BTreeMap::from([
        ("PATH".to_owned(), path.to_string_lossy().into_owned()),
        ("PATHEXT".to_owned(), ".COM;.EXE;.BAT;.CMD".to_owned()),
        ("TERM".to_owned(), "xterm-256color".to_owned()),
        ("NO_COLOR".to_owned(), "1".to_owned()),
        ("BLUEBERRY_NO_HISTORY".to_owned(), "1".to_owned()),
    ]);
    let token = format!("host-test-{}", uuid::Uuid::new_v4());
    let mut harness = Harness::start(&program, &args, cwd, &env, token)
        .with_context(|| format!("start {}", program.display()))?;
    harness
        .wait_text("PS ", PTY_TIMEOUT)
        .context("wait for the initial PowerShell prompt")?;
    Ok(RunningHost {
        harness,
        _data_dir: data_dir,
    })
}

fn wait_until(
    harness: &mut Harness,
    description: &str,
    predicate: impl FnMut(&str) -> bool,
) -> Result<()> {
    wait_until_for(harness, description, PTY_TIMEOUT, predicate)
}

fn wait_until_for(
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

fn prompt_count(contents: &str) -> usize {
    contents.match_indices("PS ").count()
}

fn wait_for_next_prompt(
    harness: &mut Harness,
    previous_count: usize,
    description: &str,
) -> Result<()> {
    wait_until(harness, description, |contents| {
        prompt_count(contents) > previous_count
            && contents.lines().last().is_some_and(|line| {
                line.trim_start().starts_with("PS ") && line.trim_end().ends_with('>')
            })
    })
}

fn line_is_present(contents: &str, expected: &str) -> bool {
    contents
        .lines()
        .any(|line| line.trim_end_matches([' ', '\r']) == expected)
}

fn wait_for_line(harness: &mut Harness, expected: &str, description: &str) -> Result<()> {
    wait_until(harness, description, |contents| {
        line_is_present(contents, expected)
    })
}

fn clear_line_and_send(harness: &mut Harness, command: &[u8], description: &str) -> Result<()> {
    let mut bytes = Vec::with_capacity(command.len() + 2);
    // Ctrl+A + Backspace is the Windows PSReadLine clear-line sequence used
    // by the host tests. Ctrl+U is not a clear-line binding in Windows mode.
    bytes.extend_from_slice(b"\x01\x7f");
    bytes.extend_from_slice(command);
    harness
        .send(&bytes)
        .with_context(|| format!("send {description}"))
}

fn run_and_wait_for_output(
    harness: &mut Harness,
    command: &[u8],
    marker: &str,
    description: &str,
) -> Result<()> {
    let previous_prompt_count = prompt_count(&harness.contents());
    clear_line_and_send(harness, command, description)?;
    wait_for_line(harness, marker, &format!("{description} output"))?;
    wait_for_next_prompt(
        harness,
        previous_prompt_count,
        "PowerShell prompt after command",
    )
}

#[test]
fn host_conpty_menu_accepts_options_restores_screen_and_keeps_control_keys_out() -> Result<()> {
    let cwd = tempdir().context("create completion cwd")?;
    std::fs::write(
        cwd.path().join("git.cmd"),
        b"@echo off\r\necho deterministic git placeholder\r\necho GIT_ARGS:%*\r\n",
    )
    .context("create deterministic git command")?;
    std::fs::write(cwd.path().join("中文文件.txt"), b"blueberry unicode test")
        .context("create Chinese completion candidate")?;
    std::fs::write(cwd.path().join("😀 file.txt"), b"blueberry emoji test")
        .context("create emoji completion candidate")?;

    let mut host = start_host(cwd.path())?;

    // Command completion: gi should show the deterministic executable and Tab
    // should replace the complete token with git. The overlay is gone once
    // the underlying screen contains the accepted line and its description is
    // no longer present.
    host.harness.send(b"gi")?;
    host.harness
        .wait_text("⌘ git ", PTY_TIMEOUT)
        .context("wait for git executable completion")?;
    host.harness
        .wait_text("≈ gi ", PTY_TIMEOUT)
        .context("wait for the current session's gi alias")?;
    ensure!(
        host.harness.contents().contains("git"),
        "git candidate did not appear; screen:\n{}",
        host.harness.contents()
    );
    // An alias arriving in a later snapshot must not steal the selected Git
    // candidate. If it was present in the initial frame, navigate past it.
    if !host.harness.viewport_contents().contains("› ⌘ git ") {
        host.harness.send(b"\x1b[B")?;
        wait_until(&mut host.harness, "selected Git candidate", |screen| {
            screen.contains("› ⌘ git ")
        })?;
    }
    host.harness.send(b"\t")?;
    wait_until(
        &mut host.harness,
        "accepted git row without menu",
        |contents| contents.contains("git") && !contents.contains("⌘ git "),
    )?;

    // Execute the accepted candidate itself. If Tab had left `gi` in the
    // buffer, this deterministic command would not produce its marker.
    let previous_prompt_count = prompt_count(&host.harness.contents());
    host.harness.send(b"\r")?;
    wait_for_line(
        &mut host.harness,
        "deterministic git placeholder",
        "deterministic git command output",
    )?;
    wait_for_next_prompt(
        &mut host.harness,
        previous_prompt_count,
        "prompt after accepted git command",
    )?;

    // Option completion uses the static pwsh specification. Esc must erase
    // the menu while preserving the underlying, still-editable command line.
    clear_line_and_send(&mut host.harness, b"pwsh -", "pwsh option query")?;
    host.harness
        .wait_text("-Command", PTY_TIMEOUT)
        .context("wait for pwsh option completion")?;
    host.harness.send(b"\x1b")?;
    wait_until(&mut host.harness, "option menu dismissal", |contents| {
        contents.contains("pwsh -") && !contents.contains("-Command")
    })?;
    run_and_wait_for_output(
        &mut host.harness,
        b"Write-Output BLUEBERRY_HOST_ESC_OK\r",
        "BLUEBERRY_HOST_ESC_OK",
        "post-Esc command",
    )?;

    // A UTF-8 path token should produce a real filesystem completion. Escape
    // then restores the line containing the Chinese query without leaving the
    // candidate row on the terminal.
    clear_line_and_send(
        &mut host.harness,
        "Get-ChildItem 中".as_bytes(),
        "Chinese filesystem query",
    )?;
    host.harness
        .wait_text("中文文件.txt", PTY_TIMEOUT)
        .context("wait for Chinese filesystem completion")?;
    host.harness.send(b"\x1b")?;
    wait_until(&mut host.harness, "Chinese menu dismissal", |contents| {
        contents.contains("Get-ChildItem 中") && !contents.contains("中文文件.txt")
    })?;

    // Complete an emoji filename in a filesystem cmdlet. The path candidate
    // is quoted because its name contains a space; executing the accepted
    // command prints the filename as its own output line. This exercises the
    // complete UTF-16 surrogate pair at the host boundary.
    let unicode_query = "Get-ChildItem -Name 😀";
    clear_line_and_send(
        &mut host.harness,
        unicode_query.as_bytes(),
        "emoji filesystem query",
    )?;
    host.harness
        .wait_text("😀 file.txt", PTY_TIMEOUT)
        .context("wait for emoji filesystem completion")?;
    wait_until(&mut host.harness, "emoji candidate row", |contents| {
        contents.contains("😀 file.txt") && contents.contains("□ 😀 file.txt")
    })?;
    host.harness.send(b"\t")?;
    wait_until(
        &mut host.harness,
        "quoted emoji candidate acceptance",
        |contents| {
            contents.lines().any(|line| {
                line.trim_end()
                    .ends_with("Get-ChildItem -Name \"😀 file.txt\"")
            })
        },
    )?;
    let previous_prompt_count = prompt_count(&host.harness.contents());
    host.harness.send(b"\r")?;
    wait_for_line(&mut host.harness, "😀 file.txt", "emoji completion output")?;
    wait_for_next_prompt(
        &mut host.harness,
        previous_prompt_count,
        "prompt after emoji completion",
    )?;

    // Leave a Unicode query in the editable buffer, dismiss its menu, then
    // clear that line with the Windows PSReadLine Ctrl+A/Backspace binding.
    // The following marker proves the stale emoji text was actually removed.
    let unicode_clear_query = "Write-Output .\\😀";
    clear_line_and_send(
        &mut host.harness,
        unicode_clear_query.as_bytes(),
        "emoji clear query",
    )?;
    wait_until(
        &mut host.harness,
        "emoji clear-query completion",
        |contents| {
            contents.contains(unicode_clear_query)
                && contents.contains("😀 file.txt")
                && contents.contains("□ 😀 file.txt")
        },
    )?;
    host.harness.send(b"\x1b")?;
    wait_until(
        &mut host.harness,
        "emoji clear-query menu dismissal",
        |contents| contents.contains(unicode_clear_query) && !contents.contains("□ 😀 file.txt"),
    )?;
    run_and_wait_for_output(
        &mut host.harness,
        b"Write-Output BLUEBERRY_UNICODE_CLEAR_OK\r",
        "BLUEBERRY_UNICODE_CLEAR_OK",
        "Unicode clear-line command",
    )?;

    // Git's --oneline belongs to log/show. Read the actual shell buffer after
    // Tab so a visually correct menu cannot hide a failed replacement.
    clear_line_and_send(&mut host.harness, b"git log --o", "git log option")?;
    host.harness.wait_text("--oneline", PTY_TIMEOUT)?;
    host.harness.send(b"\t")?;
    wait_until(&mut host.harness, "accepted git log option", |contents| {
        contents
            .lines()
            .any(|line| line.trim_end().ends_with("> git log --oneline"))
            && !contents.contains("− --oneline")
    })?;
    let previous_prompt_count = prompt_count(&host.harness.contents());
    host.harness.send(b"\r")?;
    wait_for_line(
        &mut host.harness,
        "GIT_ARGS:log --oneline",
        "actual accepted Git arguments",
    )?;
    wait_for_next_prompt(
        &mut host.harness,
        previous_prompt_count,
        "prompt after Git arguments",
    )?;
    run_and_wait_for_output(
        &mut host.harness,
        b"Write-Output BLUEBERRY_CONTEXT_OK\r",
        "BLUEBERRY_CONTEXT_OK",
        "post-option buffer clear",
    )?;

    // Run an actual child process after the Unicode interaction. Its marker
    // is intentionally unique so a leaked F12 sequence cannot be mistaken for
    // successful command output.
    std::fs::write(
        cwd.path().join("blueberry-readline.cmd"),
        b"@echo off\r\necho READ_READY\r\nset /p \"line=\"\r\necho RECEIVED:%line%\r\n",
    )
    .context("create interactive external command")?;
    let previous_prompt_count = prompt_count(&host.harness.contents());
    clear_line_and_send(
        &mut host.harness,
        b"cmd.exe /d /c blueberry-readline.cmd\r",
        "interactive external command",
    )?;
    wait_for_line(
        &mut host.harness,
        "READ_READY",
        "external command readiness",
    )?;
    host.harness.send(b"hello\r")?;
    wait_for_line(
        &mut host.harness,
        "RECEIVED:hello",
        "external command input",
    )?;
    wait_for_next_prompt(
        &mut host.harness,
        previous_prompt_count,
        "prompt after external command",
    )?;

    // Resize while the overlay is visible. A narrow frame hides descriptions
    // but keeps the candidate usable; restoring a wider frame redraws them.
    clear_line_and_send(&mut host.harness, b"gi", "resize menu query")?;
    host.harness
        .wait_text("⌘ git ", PTY_TIMEOUT)
        .context("wait for menu before resize")?;
    host.harness.resize(24, 80).context("resize PTY to 24x80")?;
    wait_until_for(
        &mut host.harness,
        "menu after 24x80 resize",
        Duration::from_secs(5),
        |contents| {
            contents.contains("git")
                && contents.lines().any(|line| {
                    let line = line.trim();
                    line.starts_with('╭') && line.ends_with('╮') && line.chars().count() == 79
                })
        },
    )?;
    let narrow_contents = host.harness.contents();
    ensure!(
        narrow_contents.lines().count() <= 24,
        "narrow resize produced too many screen rows; screen:\n{narrow_contents}"
    );
    // A ConPTY resize is asynchronous. A changed query acknowledges the
    // narrow child viewport before issuing the next resize, so an old frame
    // reflowed by the outer PTY cannot masquerade as that acknowledgement.
    host.harness.send(b"t")?;
    wait_until(
        &mut host.harness,
        "editable menu in narrow viewport",
        |contents| contents.contains("⌘ git ") && !contents.contains("≈ gi "),
    )?;
    host.harness
        .resize(30, 120)
        .context("resize PTY back to 30x120")?;
    wait_until_for(
        &mut host.harness,
        "menu after 30x120 resize",
        Duration::from_secs(5),
        |contents| {
            contents.contains("git")
                && contents.contains("⌘ git ")
                && contents.lines().any(|line| {
                    line.find('╭')
                        .zip(line.rfind('╮'))
                        .is_some_and(|(start, end)| {
                            start <= end && line[start..end + '╮'.len_utf8()].chars().count() == 100
                        })
                })
        },
    )?;
    host.harness.send(b"\x1b")?;
    wait_until_for(
        &mut host.harness,
        "menu dismissal after resize",
        Duration::from_secs(5),
        |contents| !contents.contains("⌘ git "),
    )?;

    host.harness.stop().context("stop host process")?;
    Ok(())
}

#[test]
fn host_finds_real_cargo_and_merges_more_than_512_shell_commands() -> Result<()> {
    let cwd = tempdir()?;
    let mut host = start_host(cwd.path())?;
    let cargo_version = std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .context("read the installed Cargo version")?;
    ensure!(cargo_version.status.success(), "Cargo fixture is available");
    let cargo_version = String::from_utf8(cargo_version.stdout)?.trim().to_owned();
    host.harness.send(b"car")?;
    host.harness.wait_text("⌘ cargo ", PTY_TIMEOUT)?;
    host.harness.send(b"\t --version\r")?;
    wait_for_line(
        &mut host.harness,
        &cargo_version,
        "execution of accepted Cargo command",
    )?;
    wait_until(&mut host.harness, "empty prompt after Cargo", |contents| {
        contents.lines().last().is_some_and(|line| {
            line.trim_start().starts_with("PS ") && line.trim_end().ends_with('>')
        })
    })?;

    run_and_wait_for_output(&mut host.harness,
        b"1..700 | ForEach-Object { Set-Alias ('ssfixture{0:D4}' -f $_) Write-Output }; Write-Output SNAPSHOT_CREATED\r",
        "SNAPSHOT_CREATED", "create session aliases")?;
    // Encode the actual Windows Ctrl+Alt+C key, including VK/scan code.
    // ESC + ETX can be interpreted as literal Ctrl+C with no physical C key.
    host.harness
        .send(b"\x1b[67;46;3;1;10;1_\x1b[67;46;3;0;10;1_")?;
    host.harness.send(b"ssfixture070")?;
    host.harness.wait_text("≈ ssfixture0700", PTY_TIMEOUT)?;
    host.harness.send(b"\t SNAPSHOT_ALIAS_OK\r")?;
    wait_for_line(
        &mut host.harness,
        "SNAPSHOT_ALIAS_OK",
        "alias from final snapshot batch",
    )?;
    wait_until(&mut host.harness, "empty prompt after alias", |contents| {
        contents.lines().last().is_some_and(|line| {
            line.trim_start().starts_with("PS ") && line.trim_end().ends_with('>')
        })
    })?;
    run_and_wait_for_output(&mut host.harness,
        b"Remove-Item Alias:ssfixture0700; Set-Alias ssfixture070x Write-Output; Write-Output SNAPSHOT_CHANGED\r",
        "SNAPSHOT_CHANGED", "replace one session alias")?;
    host.harness
        .send(b"\x1b[67;46;3;1;10;1_\x1b[67;46;3;0;10;1_ssfixture070")?;
    wait_until(
        &mut host.harness,
        "removed alias absent from completed snapshot",
        |contents| contents.contains("≈ ssfixture070x") && !contents.contains("≈ ssfixture0700"),
    )?;
    host.harness.finish(Duration::from_secs(5))?;
    Ok(())
}

#[test]
fn host_preserves_psreadline_history_search() -> Result<()> {
    let cwd = tempdir()?;
    let mut host = start_host(cwd.path())?;
    run_and_wait_for_output(
        &mut host.harness,
        b"Write-Output HISTORY_SEARCH_ONE\r",
        "HISTORY_SEARCH_ONE",
        "history fixture one",
    )?;
    run_and_wait_for_output(
        &mut host.harness,
        b"Write-Output HISTORY_SEARCH_TWO\r",
        "HISTORY_SEARCH_TWO",
        "history fixture two",
    )?;
    host.harness.send(b"\x12HISTORY_SEARCH_ONE")?;
    wait_until(
        &mut host.harness,
        "PSReadLine reverse history search",
        |contents| contents.contains("bck-i-search") && contents.contains("HISTORY_SEARCH_ONE"),
    )?;
    let previous_prompt_count = prompt_count(&host.harness.contents());
    host.harness.send(b"\x1b")?;
    wait_until(
        &mut host.harness,
        "history search ends with selected buffer",
        |contents| {
            !contents.contains("bck-i-search")
                && contents
                    .lines()
                    .last()
                    .is_some_and(|line| line.trim_end().ends_with("HISTORY_SEARCH_ONE"))
        },
    )?;
    host.harness.send(b"\r")?;
    wait_for_next_prompt(
        &mut host.harness,
        previous_prompt_count,
        "prompt after selected history command",
    )?;
    ensure!(
        host.harness
            .contents()
            .lines()
            .filter(|line| line.trim() == "HISTORY_SEARCH_ONE")
            .count()
            == 2,
        "history selected the requested command"
    );
    ensure!(
        !host.harness.contents().contains("[24~"),
        "private query keys leaked into history search"
    );
    host.harness.finish(Duration::from_secs(5))?;
    Ok(())
}

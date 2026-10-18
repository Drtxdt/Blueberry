#![cfg(all(windows, debug_assertions))]
use anyhow::{Result, ensure};
use blueberry::probe::Harness;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn wait(harness: &mut Harness, text: &str, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while !harness.viewport_contents().contains(text) {
        ensure!(
            Instant::now() < deadline,
            "missing {text}; screen: {}",
            harness.viewport_contents()
        );
        let _ = harness.pump(Duration::from_millis(20));
    }
    Ok(())
}

#[test]
fn ordinary_search_is_read_only_and_toggle_state_is_current() -> Result<()> {
    for word in ["docker", "helm", "uv"] {
        let directory = tempfile::tempdir()?;
        let config = directory.path().join("config.toml");
        let original = blueberry::config::example();
        fs::write(&config, original)?;
        let env = BTreeMap::from([
            ("PATH".into(), directory.path().display().to_string()),
            ("APPDATA".into(), directory.path().display().to_string()),
            (
                "LOCALAPPDATA".into(),
                directory.path().display().to_string(),
            ),
            ("TERM".into(), "xterm-256color".into()),
        ]);
        let mut harness = Harness::start(
            Path::new(env!("CARGO_BIN_EXE_blueberry")),
            &[
                "--config".into(),
                config.display().to_string(),
                "tools".into(),
            ],
            directory.path(),
            &env,
            String::new(),
        )?;
        wait(&mut harness, "Blueberry 工具管理", Duration::from_secs(10))?;
        harness.send_text(word)?;
        wait(
            &mut harness,
            &format!("搜索：{word}"),
            Duration::from_secs(2),
        )?;
        ensure!(
            fs::read(&config)? == original.as_bytes(),
            "search changed config: {word}"
        );
        ensure!(
            !harness.viewport_contents().contains("正在"),
            "search launched a task"
        );
        if word == "docker" {
            for (key, field, description) in [
                ("\x1bOS", "help", "帮助学习"),
                ("\x1b[15~", "dynamic", "动态候选"),
            ] {
                let settings = blueberry::config::load(Some(&config))?;
                let initial = if field == "help" {
                    settings.help.enabled
                } else {
                    settings.completion.dynamic
                };
                for expected in [!initial, initial] {
                    harness.send(key.as_bytes())?;
                    wait(
                        &mut harness,
                        &format!(
                            "docker：{description} {}",
                            if expected { "开启" } else { "关闭" }
                        ),
                        Duration::from_secs(2),
                    )?;
                    let value: toml::Value = toml::from_str(&fs::read_to_string(&config)?)?;
                    ensure!(
                        value["tools"]["docker"][field].as_bool() == Some(expected),
                        "toggle used stale settings"
                    );
                }
            }
        }
        harness.send_text("zzzzzzzz")?;
        wait(
            &mut harness,
            &format!("搜索：{word}zzzzzzzz"),
            Duration::from_secs(2),
        )?;
        ensure!(
            !harness.viewport_contents().contains("详情 ·"),
            "stale detail after empty filter"
        );
        harness.resize(12, 40)?;
        wait(
            &mut harness,
            &format!("搜索：{word}zzzzzzzz"),
            Duration::from_secs(2),
        )?;
        harness.send_text("\x1b")?;
        harness.wait_exit(Duration::from_secs(2))?;
    }
    Ok(())
}

#[test]
fn hanging_version_and_help_leave_search_resize_and_exit_responsive() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("fixture.rs");
    let tool = directory.path().join("git.exe");
    fs::write(
        &source,
        "fn main() { std::thread::sleep(std::time::Duration::from_secs(30)); }",
    )?;
    ensure!(
        Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&tool)
            .status()?
            .success(),
        "fixture compilation failed"
    );
    for key in ["\x1bOR", "\x1bOQ"] {
        let env = BTreeMap::from([
            ("PATH".into(), directory.path().display().to_string()),
            ("APPDATA".into(), directory.path().display().to_string()),
            (
                "LOCALAPPDATA".into(),
                directory.path().display().to_string(),
            ),
            ("TERM".into(), "xterm-256color".into()),
        ]);
        let mut harness = Harness::start(
            Path::new(env!("CARGO_BIN_EXE_blueberry")),
            &["tools".into()],
            directory.path(),
            &env,
            String::new(),
        )?;
        wait(&mut harness, "Blueberry 工具管理", Duration::from_secs(10))?;
        harness.send_text("git")?;
        wait(&mut harness, "搜索：git", Duration::from_secs(2))?;
        harness.send(key.as_bytes())?;
        wait(&mut harness, "正在", Duration::from_secs(2))?;
        harness.send_text("zz")?;
        harness.resize(12, 40)?;
        wait(&mut harness, "搜索：gitzz", Duration::from_secs(1))?;
        harness.send_text("\x1b")?;
        harness.wait_exit(Duration::from_secs(1))?;
    }
    Ok(())
}

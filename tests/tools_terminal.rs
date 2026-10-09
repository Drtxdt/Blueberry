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
    for key in ["v", "l"] {
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
        harness.send_text(key)?;
        wait(&mut harness, "正在", Duration::from_secs(2))?;
        harness.send_text("zz")?;
        harness.resize(12, 40)?;
        wait(&mut harness, "搜索：gitzz", Duration::from_secs(1))?;
        harness.send_text("\x1b")?;
        harness.wait_exit(Duration::from_secs(1))?;
    }
    Ok(())
}

#![cfg(all(windows, debug_assertions))]

//! Standalone workbench interaction through a real ConPTY. All user paths
//! and history are isolated; cancellation never invokes clip.exe.

use anyhow::{Context, Result, bail};
use blueberry::probe::Harness;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(15);

fn wait_screen(
    harness: &mut Harness,
    description: &str,
    predicate: impl Fn(&str) -> bool,
) -> Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let screen = harness.viewport_contents();
        if predicate(&screen) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for {description}; screen:\n{screen}");
        }
        let _ = harness.pump(Duration::from_millis(100));
    }
}

#[test]
fn standalone_template_form_cancel_and_query_shrink_leave_no_stale_rows() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = directory.path().join("Blueberry/commands.toml");
    fs::create_dir_all(store.parent().context("store parent")?)?;
    let original = "schema_version = 2\n[[templates]]\nid = 'branch'\nname = '模板'\ntokens = ['git', 'switch', '-c', '{branch}']\n[[templates.parameters]]\nname = 'branch'\nlabel = '分支名称'\nkind = 'text'\nrequired = true\n";
    fs::write(&store, original)?;
    let program = PathBuf::from(env!("CARGO_BIN_EXE_blueberry"));
    let args = vec!["hub".into(), "--query".into(), "模板😀".into()];
    let env = BTreeMap::from([
        (
            "APPDATA".into(),
            directory.path().to_string_lossy().into_owned(),
        ),
        ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
        ("TERM".into(), "xterm-256color".into()),
    ]);
    let mut harness = Harness::start(
        &program,
        &args,
        directory.path(),
        &env,
        uuid::Uuid::new_v4().to_string(),
    )?;
    wait_screen(&mut harness, "standalone workbench", |s| {
        s.contains("Blueberry 命令工作台")
    })?;
    harness.resize(14, 50)?;
    harness.send(b"\x7f")?;
    wait_screen(&mut harness, "shortened Unicode query", |s| {
        s.contains("模板") && !s.contains('😀')
    })?;
    harness.send(b"\r")?;
    wait_screen(&mut harness, "structured template form", |s| {
        s.contains("分支名称")
    })?;
    harness.send(b"new branch")?;
    wait_screen(&mut harness, "typed parameter", |s| {
        s.contains("new branch")
    })?;
    harness.send(b"\x1b")?;
    wait_screen(&mut harness, "canceled form", |s| {
        s.contains("Blueberry 命令工作台")
            && !s.contains("填写模板参数")
            && !s.contains("new branch")
    })?;
    assert_eq!(fs::read_to_string(&store)?, original);
    harness.send(b"\x1b")?;
    Ok(())
}

#![cfg(windows)]

use anyhow::{Result, ensure};
use blueberry::{config::Config, model::Candidate, overlay::Overlay, probe::Harness};
use std::{
    collections::BTreeMap,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

#[path = "support/console_snapshot.rs"]
mod console_snapshot;

#[test]
#[ignore = "native snapshot subprocess"]
fn console_snapshot_helper() -> Result<()> {
    console_snapshot::write_attached()
}

fn await_file(path: &Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !path.exists() {
        ensure!(Instant::now() < deadline, "missing {}", path.display());
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

#[test]
#[ignore = "isolated overlay subprocess"]
fn overlay_helper() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("OVERLAY_FIXTURE")?);
    let mut screen = vt100::Parser::new(30, 120, 0);
    screen.process(b"\x1b[2J\x1b[18;1HKEEP_UNOWNED_HISTORY\x1b[19;1HPS fixture> gi");
    let candidates: Vec<_> = (0..8)
        .map(|i| Candidate {
            label: format!("git-fixture-{i}"),
            insert_text: format!("git-fixture-{i}"),
            description: "本地程序".into(),
            kind: blueberry::model::CandidateKind::Command,
            ..Default::default()
        })
        .collect();
    let mut config = Config::default();
    config.ui.icon_style = "unicode".into();
    let mut output = std::io::stdout().lock();
    output.write_all(&screen.screen().state_formatted())?;
    let mut overlay = Overlay::default();
    overlay.draw(screen.screen(), &mut output, &candidates, 0, "gi", &config)?;
    output.flush()?;
    std::fs::write(dir.join("painted"), [])?;
    await_file(&dir.join("erase"))?;
    if std::env::var_os("OVERLAY_QUEUED_OUTPUT").is_some() {
        let mut pending = b"\x1b[20;1H\x1b[2KPS QUEUED_PROMPT> git".to_vec();
        screen.process(&pending);
        // Reproduce the host's queued output path: physical readback still
        // contains the old menu, while pending bytes replace it with a prompt.
        overlay.synchronize_pending_output(&mut pending, &mut output)?;
        overlay.erase(screen.screen(), &mut pending)?;
        output.write_all(&pending)?;
    } else {
        // Console reflow precedes the queued resize event; neither model nor
        // Overlay has received a resize notification.
        overlay.erase(screen.screen(), &mut output)?;
        if std::env::var_os("OVERLAY_ROUND_TRIP").is_some() {
            // Erasing characters does not erase ConPTY's soft-wrap metadata.
            // Paint a narrow menu into those rows, then let the parent widen.
            screen = vt100::Parser::new(24, 80, 0);
            let prompt = b"\x1b[12;1H\x1b[2KPS fixture> gi";
            screen.process(prompt);
            output.write_all(prompt)?;
            overlay.draw(screen.screen(), &mut output, &candidates, 0, "gi", &config)?;
            output.flush()?;
            std::fs::write(dir.join("narrow-ready"), [])?;
            await_file(&dir.join("wide"))?;
            overlay.erase(screen.screen(), &mut output)?;
        }
    }
    output.flush()?;
    std::fs::write(dir.join("erased"), [])?;
    await_file(&dir.join("finish"))?;
    Ok(())
}

#[test]
fn erase_after_console_resize_before_resize_event_removes_owned_rows() -> Result<()> {
    exercise_retirement(true, false)
}

#[test]
fn queued_shell_prompt_is_not_erased_using_stale_menu_readback() -> Result<()> {
    exercise_retirement(false, false)
}

#[test]
fn widening_after_narrow_repaint_removes_reflowed_menu_fragments() -> Result<()> {
    exercise_retirement(true, true)
}

fn exercise_retirement(resize: bool, round_trip: bool) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/overlay-resize-evidence");
    std::fs::create_dir_all(&root)?;
    let mut dir = tempfile::Builder::new().prefix("race-").tempdir_in(root)?;
    dir.disable_cleanup(true);
    eprintln!("overlay evidence: {}", dir.path().display());
    let mut env = BTreeMap::from([(
        "OVERLAY_FIXTURE".into(),
        dir.path().to_string_lossy().into_owned(),
    )]);
    if !resize {
        env.insert("OVERLAY_QUEUED_OUTPUT".into(), "1".into());
    }
    if round_trip {
        env.insert("OVERLAY_ROUND_TRIP".into(), "1".into());
    }
    let mut harness = Harness::start(
        &std::env::current_exe()?,
        &[
            "overlay_helper".into(),
            "--exact".into(),
            "--ignored".into(),
            "--nocapture".into(),
        ],
        dir.path(),
        &env,
        "overlay-resize".into(),
    )?;
    harness.wait_text("git-fixture-0", Duration::from_secs(20))?;
    if resize {
        harness.resize(24, 80)?;
    }
    console_snapshot::capture(&mut harness, &dir.path().join("before.json"))?;
    let before: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("before.json"))?)?;
    ensure!(
        before["screen"]
            .as_str()
            .unwrap_or_default()
            .contains("KEEP_UNOWNED_HISTORY"),
        "fixture must retain unrelated history before cleanup: {before}"
    );
    std::fs::write(dir.path().join("erase"), [])?;
    if round_trip {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !dir.path().join("narrow-ready").exists() {
            ensure!(Instant::now() < deadline, "narrow repaint did not finish");
            let _ = harness.pump(Duration::from_millis(5));
        }
        harness.resize(30, 120)?;
        console_snapshot::capture(&mut harness, &dir.path().join("wide-before.json"))?;
        std::fs::write(dir.path().join("wide"), [])?;
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while !dir.path().join("erased").exists() {
        ensure!(Instant::now() < deadline, "erase did not finish");
        let _ = harness.pump(Duration::from_millis(5));
    }
    console_snapshot::capture(&mut harness, &dir.path().join("after.json"))?;
    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("after.json"))?)?;
    std::fs::write(dir.path().join("finish"), [])?;
    harness.stop()?;
    ensure!(
        !after["screen"]
            .as_str()
            .unwrap_or_default()
            .contains("git-fixture-")
            && !after["screen"]
                .as_str()
                .unwrap_or_default()
                .contains(['╭', '╮', '╰', '╯', '│']),
        "owned menu survived an unobserved console resize: {after}"
    );
    ensure!(
        after["screen"]
            .as_str()
            .unwrap_or_default()
            .contains("PS fixture> gi"),
        "cleanup or reflow lost the prompt: {after}"
    );
    ensure!(
        after["screen"]
            .as_str()
            .unwrap_or_default()
            .contains("KEEP_UNOWNED_HISTORY"),
        "cleanup erased unrelated history: {after}"
    );
    if !resize {
        ensure!(
            after["screen"]
                .as_str()
                .unwrap_or_default()
                .contains("PS QUEUED_PROMPT> git"),
            "stale ownership readback erased queued shell prompt: {after}"
        );
    }
    Ok(())
}

#![cfg(all(windows, debug_assertions))]
//! Opt-in end-to-end checks against the frozen production TerminalCore model.
//! Text/viewport checks do not substitute for attribute or GUI acceptance.
use anyhow::{Context, Result, ensure};
use blueberry::probe::Harness;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::mpsc::RecvTimeoutError,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(25);

struct Fixture {
    h: Harness,
    directory: Option<tempfile::TempDir>,
    path: std::path::PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.h.stop();
        let _ = self.h.save_evidence(&self.path);
        if let Ok(screen) = self.h.observed_viewport() {
            let _ = std::fs::write(self.path.join("native-final-screen.txt"), screen);
        }
        if let Some(directory) = self.directory.take() {
            let _ = directory.keep();
        }
    }
}

fn start(mode: &str) -> Result<Fixture> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/native-terminal-regressions");
    std::fs::create_dir_all(&root)?;
    let directory = tempfile::Builder::new().prefix(mode).tempdir_in(root)?;
    std::fs::create_dir(directory.path().join("native-one"))?;
    std::fs::create_dir_all(directory.path().join("native-two/X"))?;
    let mut environment = BTreeMap::from([
        ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
        ("BLUEBERRY_ACTIVE".into(), "1".into()),
        ("BLUEBERRY_NATIVE_OVERLAY".into(), "4".into()),
        ("TERM".into(), "xterm-256color".into()),
    ]);
    blueberry::probe::inherit_native_observer_environment(&mut environment);
    let h = Harness::start(
        Path::new(env!("CARGO_BIN_EXE_blueberry")),
        &[
            "run".into(),
            "--host-mode".into(),
            mode.into(),
            "--transport".into(),
            "pipe".into(),
            "--shell".into(),
            blueberry::pty::default_shell()
                .to_string_lossy()
                .into_owned(),
            "--no-profile".into(),
            "--data-dir".into(),
            directory.path().join("data").to_string_lossy().into_owned(),
        ],
        directory.path(),
        &environment,
        String::new(),
    )?;
    ensure!(
        h.native_observer_identity().is_some(),
        "frozen native model not configured"
    );
    let path = directory.path().to_owned();
    let mut fixture = Fixture {
        h,
        directory: Some(directory),
        path,
    };
    wait(&mut fixture, |screen| {
        screen.lines().any(|line| line.trim_end().ends_with('>'))
    })?;
    Ok(fixture)
}

fn wait(f: &mut Fixture, predicate: impl Fn(&str) -> bool) -> Result<String> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let screen = f.h.observed_viewport()?;
        if predicate(&screen) {
            return Ok(screen);
        }
        if Instant::now() >= deadline {
            let path = f.path.clone();
            f.h.save_evidence(&path)?;
            std::fs::write(path.join("native-screen.txt"), &screen)?;
            anyhow::bail!("native terminal timeout: {}\n{screen}", path.display());
        }
        if let Err(error) = f.h.pump(Duration::from_millis(20))
            && error.downcast_ref::<RecvTimeoutError>() != Some(&RecvTimeoutError::Timeout)
        {
            return Err(error);
        }
    }
}

fn selected(screen: &str, name: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains('›') && line.contains(name))
}

fn complete_input(screen: &str) -> bool {
    screen
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .contains(">cdnative-")
}

#[test]
#[ignore = "requires the frozen CI native DLL and digest"]
fn native_navigation_and_tab_in_one_burst_accepts_the_new_frame() -> Result<()> {
    for mode in ["direct", "nested"] {
        let mut f = start(mode)?;
        f.h.send_text("cd native-")?;
        wait(&mut f, |screen| {
            selected(screen, "native-one") && complete_input(screen)
        })?;
        // The directory X exists only under the second candidate. Executing
        // this owned cd command verifies the actual edit, not a menu echo.
        f.h.send(b"\x1b[B\tX\r")?;
        let screen = wait(&mut f, |screen| {
            screen
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .contains("native-two\\X>")
        })?;
        ensure!(
            !screen.contains("Cannot find path"),
            "wrong candidate accepted: {screen}"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires the frozen CI native DLL and digest"]
fn native_bottom_menu_rapid_resize_and_escape_preserve_unicode_history() -> Result<()> {
    for mode in ["direct", "nested"] {
        let mut f = start(mode)?;
        // Generate Unicode from ASCII code points: this fixture tests output
        // restoration independently from IME/Unicode keyboard insertion.
        f.h.send_text("[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); 1..64 | ForEach-Object { [Console]::Write(([char]27).ToString()+'[38;2;17;123;239m'+([char]27).ToString()+']8;id=history;https://example.invalid/history'+[char]7+('BBH{0:D2}' -f $_)+[char]0x4e2d+[char]::ConvertFromUtf32(0x1f680)+'e'+[char]0x301+('w'*37)+([char]27).ToString()+']8;;'+[char]7+([char]27).ToString()+'[0m'+[Environment]::NewLine) }; [Console]::WriteLine('HISTORY_READY'); [Console]::SetCursorPosition(0,[Console]::WindowTop+[Console]::WindowHeight-1)")?;
        f.h.send(b"\r")?;
        wait(&mut f, |screen| {
            screen.lines().any(|line| line.trim() == "HISTORY_READY")
                && screen
                    .lines()
                    .last()
                    .is_some_and(|line| line.trim_end().ends_with('>'))
        })?;
        f.h.send_text("cd native-")?;
        let menu = wait(&mut f, |screen| {
            selected(screen, "native-one") && complete_input(screen)
        })?;
        let menu_row = menu
            .lines()
            .position(|line| selected(line, "native-one"))
            .context("selected row missing")?;
        let input_row = menu
            .lines()
            .position(|line| line.contains("cd native-"))
            .context("input row missing")?;
        ensure!(
            menu_row < input_row,
            "menu was not above bottom input: {menu}"
        );
        for (rows, columns) in [(12, 40), (30, 100), (12, 50), (30, 120)] {
            f.h.resize(rows, columns)?;
        }
        f.h.send(b"\x1b")?;
        let restored = wait(&mut f, |screen| {
            !screen.contains('│') && screen.contains("BBH64中🚀é")
        })?;
        ensure!(
            !restored.contains('�'),
            "history contains a replacement character: {restored}"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires the frozen CI native DLL and digest"]
fn native_history_navigation_stays_native_until_a_real_edit() -> Result<()> {
    for mode in ["direct", "nested"] {
        let mut f = start(mode)?;
        f.h.send_text("[Microsoft.PowerShell.PSConsoleReadLine]::AddToHistory('cd native-one'); [Microsoft.PowerShell.PSConsoleReadLine]::AddToHistory('cd native-two'); [Console]::WriteLine('HISTORY_SETUP')\r")?;
        wait(&mut f, |screen| {
            screen.lines().any(|line| line.trim() == "HISTORY_SETUP")
                && screen
                    .lines()
                    .last()
                    .is_some_and(|line| line.trim_end().ends_with('>'))
        })?;
        for (key, recalled) in [
            (b"\x1b[A".as_slice(), "cd native-two"),
            (b"\x1b[A".as_slice(), "cd native-one"),
            (b"\x1b[B".as_slice(), "cd native-two"),
        ] {
            f.h.send(key)?;
            wait(&mut f, |screen| {
                screen
                    .lines()
                    .last()
                    .is_some_and(|line| line.contains(recalled))
            })?;
            let settle = Instant::now() + Duration::from_millis(300);
            while Instant::now() < settle {
                let _ = f.h.pump(Duration::from_millis(20));
            }
            ensure!(
                !f.h.observed_viewport()?.contains('│'),
                "history opened a menu"
            );
        }
        // Moving to Home then deleting backwards changes nothing. Tab and
        // Escape remain native even when the editor itself changes the text.
        f.h.send(b"\x1b[H\x7f\t")?;
        f.h.send_text("\x1b")?;
        let settle = Instant::now() + Duration::from_millis(500);
        while Instant::now() < settle {
            let _ = f.h.pump(Duration::from_millis(20));
        }
        ensure!(
            !f.h.observed_viewport()?.contains('│'),
            "native edit reopened a menu"
        );
        f.h.send_text("cd native-")?;
        wait(&mut f, |screen| {
            selected(screen, "native-one") && complete_input(screen)
        })?;
    }
    Ok(())
}

#![cfg(all(windows, debug_assertions))]
//! Inspect the real editor buffer and execution arguments, never a real key.
use anyhow::{Result, ensure};
use blueberry::probe::Harness;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    time::{Duration, Instant},
};

const COMMAND: &str = "echo -n '24e25ad07ecdde7e5ed2ed2bb9a44578187ee59fec6b209dbd34f725350b3c55' | ssh-keygen -Y sign -n gitea -f /path_to_your_privkey";
const TIMEOUT: Duration = Duration::from_secs(20);

fn quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

fn read_json(h: &mut Harness, path: &Path) -> Result<Value> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Ok(text) = fs::read_to_string(path)
            && let Ok(value) = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        {
            return Ok(value);
        }
        ensure!(
            Instant::now() < deadline,
            "missing {}: {}",
            path.display(),
            h.contents()
        );
        let _ = h.pump(Duration::from_millis(10));
    }
}

fn envelope(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .map(|unit| format!("\x1b[0;0;{unit};1;0;1_\x1b[0;0;{unit};0;0;1_"))
        .collect::<String>()
        .into_bytes()
}

#[test]
fn signing_command_remains_exact_across_input_protocols() -> Result<()> {
    let mut reference = None;
    for mode in ["plain", "nested", "direct"] {
        let dir = tempfile::tempdir()?;
        let buffer = dir.path().join("buffer.json");
        let executions = dir.path().join("execution.json");
        let ready = dir.path().join("ready.json");
        let fixture = dir.path().join("fixture.ps1");
        fs::write(
            &fixture,
            format!(
                r#"
function global:ssh-keygen {{
    @{{ arguments=@($args); stdin=@($input) }} | ConvertTo-Json -Compress | Add-Content -LiteralPath {executions} -Encoding UTF8
}}
Set-PSReadLineKeyHandler -Key F4 -ScriptBlock {{
    $line=''; $cursor=0; [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line,[ref]$cursor)
    @{{line=$line;cursor=$cursor}} | ConvertTo-Json -Compress | Set-Content -LiteralPath {buffer} -Encoding UTF8
}}
'true' | Set-Content -LiteralPath {ready}
"#,
                executions = quote(&executions),
                buffer = quote(&buffer),
                ready = quote(&ready)
            ),
        )?;
        let shell = blueberry::pty::default_shell();
        let args = if mode == "plain" {
            let module = std::env::var("BLUEBERRY_TEST_PSREADLINE_MODULE")
                .ok()
                .map(|p| format!("Import-Module {} -ErrorAction Stop; ", quote(Path::new(&p))))
                .unwrap_or_default();
            vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NoExit".into(),
                "-Command".into(),
                format!(
                    "{module}Import-Module PSReadLine; Set-PSReadLineOption -HistorySaveStyle SaveNothing"
                ),
            ]
        } else {
            vec![
                "run".into(),
                "--host-mode".into(),
                mode.into(),
                "--transport".into(),
                "pipe".into(),
                "--no-profile".into(),
                "--shell".into(),
                shell.to_string_lossy().into_owned(),
                "--data-dir".into(),
                dir.path().join("data").to_string_lossy().into_owned(),
            ]
        };
        let program = if mode == "plain" {
            shell.as_path()
        } else {
            Path::new(env!("CARGO_BIN_EXE_blueberry"))
        };
        let mut h = Harness::start(
            program,
            &args,
            dir.path(),
            &BTreeMap::from([
                ("BLUEBERRY_NO_HISTORY".into(), "1".into()),
                ("TERM".into(), "xterm-256color".into()),
            ]),
            String::new(),
        )?;
        h.wait_text("PS ", TIMEOUT)?;
        h.send(format!(". {}\r", quote(&fixture)).as_bytes())?;
        read_json(&mut h, &ready)?;
        h.send(b"\x1bOS")?;
        ensure!(read_json(&mut h, &buffer)?["line"] == "");
        fs::remove_file(&buffer)?;
        for pasted in [false, true] {
            // First establish ordinary input, then switch to Win32 envelopes.
            h.send(b"x\x01\x7f")?;
            // Only nested negotiates bracketed paste. Stock Windows
            // PSReadLine (and direct) receives an unmarked console batch.
            if pasted && mode != "nested" {
                h.send(COMMAND.as_bytes())?;
            } else {
                let input = if pasted {
                    format!("\x1b[200~{COMMAND}\x1b[201~")
                } else {
                    COMMAND.into()
                };
                for part in envelope(&input).chunks(7) {
                    h.send(part)?;
                }
            }
            h.send(b"\x1bOS")?;
            let actual = read_json(&mut h, &buffer)?;
            ensure!(
                actual["line"] == COMMAND && actual["cursor"] == COMMAND.len(),
                "{mode}, pasted={pasted}: {actual}; screen={}",
                h.contents()
            );
            fs::remove_file(&buffer)?;
            h.send(b"\x1b[13;28;13;1;0;1_\x1b[13;28;13;0;0;1_")?;
            let actual = read_json(&mut h, &executions)?;
            if let Some(expected) = &reference {
                ensure!(&actual == expected, "{mode}: {actual}, expected {expected}");
            } else {
                reference = Some(actual);
            }
            fs::remove_file(&executions)?;
            // A second release must not execute the command a second time.
            h.send(b"\x1b[13;28;13;0;0;1_\x1bOS")?;
            let actual = read_json(&mut h, &buffer)?;
            ensure!(
                actual["line"] == "",
                "protocol text leaked after Enter in {mode}: {actual}"
            );
            ensure!(!executions.exists(), "release executed twice in {mode}");
            fs::remove_file(&buffer)?;
        }
        h.finish(TIMEOUT)?;
    }
    Ok(())
}

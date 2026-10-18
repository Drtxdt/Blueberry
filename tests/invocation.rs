#![cfg(windows)]
use anyhow::{Context, Result, ensure};
use blueberry::invocation::{ArgumentMode, Invocation, Word, literal};

#[test]
fn real_native_argv_roundtrips_legacy_and_standard_modes() -> Result<()> {
    let python = std::env::var("BLUEBERRY_ARGV_PYTHON").unwrap_or_else(|_| "python".into());
    let discovery = std::process::Command::new(&python)
        .args(["-c", "import sys; print(sys.executable)"])
        .output()?;
    ensure!(
        discovery.status.success(),
        "Python argv fixture unavailable"
    );
    let python = String::from_utf8(discovery.stdout)?.trim().to_owned();
    let dir = tempfile::tempdir()?;
    let probe = dir.path().join("argv.py");
    std::fs::write(
        &probe,
        "import json,sys\nwith open(sys.argv[1], 'w', encoding='utf-8') as f: json.dump(sys.argv[2:], f, ensure_ascii=False)\n",
    )?;
    let expected = [
        "",
        "two words",
        "a\"b",
        "a\\\"b",
        "trailing space\\",
        "'single'",
        ";echo injected",
        "@value",
        "a,b",
        "中文",
        "$env:HOME",
    ];
    for (shell, setting, mode) in [
        ("powershell.exe", "", ArgumentMode::NativeLegacy),
        (
            "pwsh.exe",
            "$PSNativeCommandArgumentPassing='Legacy';",
            ArgumentMode::NativeLegacy,
        ),
        (
            "pwsh.exe",
            "$PSNativeCommandArgumentPassing='Standard';",
            ArgumentMode::NativeStandard,
        ),
        (
            "pwsh.exe",
            "$PSNativeCommandArgumentPassing='Windows';",
            ArgumentMode::NativeStandard,
        ),
    ] {
        let output = dir.path().join("argv.json");
        let mut words = vec![
            Word::Syntax(format!("& {}", literal(&python, ArgumentMode::PowerShell))),
            Word::Literal(probe.display().to_string()),
            Word::Literal(output.display().to_string()),
        ];
        let context = blueberry::engine::CommandIndex::default().input_context("fixture ", 8);
        words.extend(expected.iter().map(|value| {
            Word::Expression(blueberry::engine::format_value_with_mode(
                value, "fixture ", &context, 8, mode,
            ))
        }));
        let command = format!("{setting} {}", Invocation { words }.render(mode));
        let result = std::process::Command::new(shell)
            .args(["-NoProfile", "-NonInteractive", "-Command", &command])
            .output()
            .with_context(|| format!("start {shell}"))?;
        ensure!(
            result.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let actual: Vec<String> = serde_json::from_slice(&std::fs::read(&output)?)?;
        assert_eq!(actual, expected, "{shell} {setting}");
        std::fs::remove_file(output)?;
    }
    Ok(())
}

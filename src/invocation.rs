//! Command syntax and literal data must not share an escaping path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Word {
    Syntax(String),
    Literal(String),
    /// An existing PowerShell expression, such as `$env:HOME` in a path.
    Expression(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArgumentMode {
    #[default]
    PowerShell,
    NativeStandard,
    NativeLegacy,
}

pub fn shell_mode(
    command: &str,
    environment: &std::collections::BTreeMap<String, String>,
    commands: &[crate::model::ShellCommand],
    cwd: &std::path::Path,
) -> ArgumentMode {
    let get = |key: &str| {
        environment
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
            .unwrap_or("")
    };
    let mut command = command.to_owned();
    for _ in 0..16 {
        let Some(entry) = commands
            .iter()
            .rev()
            .find(|entry| entry.name.eq_ignore_ascii_case(&command))
        else {
            break;
        };
        if entry.kind.eq_ignore_ascii_case("alias") {
            command = entry.definition.clone();
        } else if !entry.kind.eq_ignore_ascii_case("application") {
            return ArgumentMode::PowerShell;
        } else {
            break;
        }
    }
    let path = std::path::Path::new(&command);
    let executable = if path.components().count() > 1 {
        Some(cwd.join(path))
    } else {
        let extensions = get("PATHEXT");
        let extensions = if extensions.is_empty() {
            ".EXE;.COM;.CMD;.BAT;.PS1"
        } else {
            extensions
        };
        std::env::split_paths(get("PATH")).find_map(|directory| {
            let directory = cwd.join(directory);
            let direct = directory.join(&command);
            if path.extension().is_some() && direct.is_file() {
                return Some(direct);
            }
            extensions
                .split(';')
                .map(|extension| directory.join(format!("{command}{extension}")))
                .find(|candidate| candidate.is_file())
        })
    };
    let Some(executable) = executable else {
        return ArgumentMode::PowerShell;
    };
    let extension = executable
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension == "ps1" {
        return ArgumentMode::PowerShell;
    }
    match get("BLUEBERRY_NATIVE_ARGUMENT_MODE")
        .to_ascii_lowercase()
        .as_str()
    {
        "legacy" => ArgumentMode::NativeLegacy,
        "standard" => ArgumentMode::NativeStandard,
        "windows" => {
            let stem = executable
                .file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(extension.as_str(), "bat" | "cmd" | "js" | "vbs" | "wsf")
                || matches!(stem.as_str(), "cmd" | "cscript" | "wscript")
            {
                ArgumentMode::NativeLegacy
            } else {
                ArgumentMode::NativeStandard
            }
        }
        _ => ArgumentMode::PowerShell,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Invocation {
    pub words: Vec<Word>,
}

impl Invocation {
    pub fn render(&self, mode: ArgumentMode) -> String {
        self.words
            .iter()
            .map(|word| match word {
                Word::Syntax(value) | Word::Expression(value) => value.clone(),
                Word::Literal(value) => literal(value, mode),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub fn literal(value: &str, mode: ArgumentMode) -> String {
    let encoded;
    let value = if mode == ArgumentMode::NativeLegacy {
        encoded = windows_argument(value);
        &encoded
    } else {
        value
    };
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./:+-\\".contains(c))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

/// Encode one Windows argv element before the legacy PowerShell native binder
/// serializes it. Backslashes before quotes and a closing quote need doubling.
fn windows_argument(value: &str) -> String {
    let quote = value.is_empty() || value.chars().any(char::is_whitespace);
    let mut output = String::new();
    if quote {
        output.push('"');
    }
    let mut slashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        if ch == '"' {
            output.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
        } else {
            output.extend(std::iter::repeat_n('\\', slashes));
        }
        slashes = 0;
        output.push(ch);
    }
    output.extend(std::iter::repeat_n(
        '\\',
        slashes * if quote { 2 } else { 1 },
    ));
    if quote {
        output.push('"');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(windows)]
    fn session_functions_aliases_and_program_type_select_the_binder() {
        use std::collections::BTreeMap;
        let dir = tempfile::tempdir().unwrap();
        for name in ["tool.exe", "tool.cmd", "script.ps1"] {
            std::fs::write(dir.path().join(name), "fixture").unwrap();
        }
        let mut environment = BTreeMap::from([
            ("PATH".into(), dir.path().display().to_string()),
            ("PATHEXT".into(), ".EXE;.CMD;.PS1".into()),
            ("BLUEBERRY_NATIVE_ARGUMENT_MODE".into(), "Windows".into()),
        ]);
        assert_eq!(
            shell_mode("tool", &environment, &[], dir.path()),
            ArgumentMode::NativeStandard
        );
        assert_eq!(
            shell_mode("tool.cmd", &environment, &[], dir.path()),
            ArgumentMode::NativeLegacy
        );
        assert_eq!(
            shell_mode("script.ps1", &environment, &[], dir.path()),
            ArgumentMode::PowerShell
        );
        environment.insert("BLUEBERRY_NATIVE_ARGUMENT_MODE".into(), "Legacy".into());
        let alias = crate::model::ShellCommand {
            name: "alias".into(),
            kind: "Alias".into(),
            definition: "tool".into(),
        };
        assert_eq!(
            shell_mode(
                "alias",
                &environment,
                std::slice::from_ref(&alias),
                dir.path()
            ),
            ArgumentMode::NativeLegacy
        );
        let function = crate::model::ShellCommand {
            name: "tool".into(),
            kind: "Function".into(),
            definition: "param($x) $x".into(),
        };
        assert_eq!(
            shell_mode("alias", &environment, &[alias, function], dir.path()),
            ArgumentMode::PowerShell
        );
    }
    #[test]
    fn syntax_literals_and_expressions_have_distinct_semantics() {
        let command = Invocation {
            words: vec![
                Word::Syntax("Write-Output".into()),
                Word::Literal("$env:HOME; '中文'".into()),
                Word::Expression("$env:HOME".into()),
            ],
        };
        assert_eq!(
            command.render(ArgumentMode::PowerShell),
            "Write-Output '$env:HOME; ''中文''' $env:HOME"
        );
        assert_eq!(windows_argument(""), "\"\"");
        assert_eq!(windows_argument("a\\\"b"), "a\\\\\\\"b");
        assert_eq!(windows_argument("a b\\"), "\"a b\\\\\"");
    }
}

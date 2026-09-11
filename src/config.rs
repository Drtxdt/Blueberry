//! Configuration loading and validation for the completion UI.
//!
//! The configuration surface is deliberately small. Keeping validation here
//! means the renderer can assume values read from disk are bounded while still
//! remaining defensive for callers that construct a [`Config`] directly.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

const MAX_ROWS: usize = 100;
const MAX_WIDTH: usize = 512;
const MAX_RESULTS: usize = 1_000;

/// Top-level ShellSense configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub ui: UiConfig,
    pub completion: CompletionConfig,
}

/// Presentation settings for the completion menu.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub max_rows: usize,
    pub width: usize,
    pub descriptions: bool,
    /// One of `rounded`, `square`, or `none`.
    pub border: String,
    pub icons: bool,
    pub foreground: String,
    pub background: String,
    pub selected_foreground: String,
    pub selected_background: String,
    pub border_color: String,
    pub description_color: String,
    pub match_color: String,
}

/// Completion collection settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CompletionConfig {
    pub max_results: usize,
    pub auto_trigger: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            max_rows: 8,
            width: 80,
            descriptions: true,
            border: "rounded".to_owned(),
            icons: true,
            foreground: "default".to_owned(),
            background: "default".to_owned(),
            selected_foreground: "#ffffff".to_owned(),
            selected_background: "#264f78".to_owned(),
            border_color: "#6688aa".to_owned(),
            description_color: "#8f9baa".to_owned(),
            match_color: "#ffcc66".to_owned(),
        }
    }
}

impl Default for CompletionConfig {
    fn default() -> Self {
        Self {
            max_results: 100,
            auto_trigger: true,
        }
    }
}

impl Config {
    /// Validate all bounded and enumerated configuration values.
    pub fn validate(&self) -> Result<()> {
        validate_ui(&self.ui)?;
        validate_completion(&self.completion)?;
        Ok(())
    }
}

/// Load a configuration file.
///
/// When `path` is omitted, the platform default path is used. A missing
/// default file means "use defaults"; an explicitly supplied missing path is
/// reported as an error so a typo cannot silently disable a user's settings.
pub fn load(path: Option<&Path>) -> Result<Config> {
    let explicit_path = path.is_some();
    let path = path.map(Path::to_path_buf).unwrap_or_else(default_path);

    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !explicit_path => {
            return Ok(Config::default());
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("unable to read configuration file '{}'", path.display())
            });
        }
    };

    let config: Config = toml::from_str(&contents).map_err(|error| {
        anyhow!(
            "unable to parse configuration file '{}': {error}",
            path.display()
        )
    })?;

    config
        .validate()
        .map_err(|error| anyhow!("invalid configuration in '{}': {error}", path.display()))?;

    Ok(config)
}

/// Return the platform-specific path used when no configuration path is
/// supplied to [`load`].
pub fn default_path() -> PathBuf {
    config_root().join("config.toml")
}

/// Return the platform-specific cache directory used by ShellSense.
pub fn cache_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(root) = env::var_os("LOCALAPPDATA") {
            return PathBuf::from(root).join("shellsense");
        }
        if let Some(root) = env::var_os("APPDATA") {
            return PathBuf::from(root).join("shellsense").join("cache");
        }
        PathBuf::from("shellsense").join("cache")
    }

    #[cfg(not(windows))]
    {
        if let Some(root) = env::var_os("XDG_CACHE_HOME") {
            return PathBuf::from(root).join("shellsense");
        }
        if let Some(root) = env::var_os("HOME") {
            return PathBuf::from(root).join(".cache").join("shellsense");
        }
        PathBuf::from(".cache").join("shellsense")
    }
}

/// Return a TOML example suitable for writing as a starting point for a
/// user's configuration.
pub fn example() -> &'static str {
    r##"# ShellSense configuration

[ui]
max_rows = 8
width = 80
descriptions = true
border = "rounded" # "rounded", "square", or "none"
icons = true
foreground = "default"
background = "default"
selected_foreground = "#ffffff"
selected_background = "#264f78"
border_color = "#6688aa"
description_color = "#8f9baa"
match_color = "#ffcc66"

[completion]
max_results = 100
auto_trigger = true
"##
}

fn config_root() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(root) = env::var_os("APPDATA") {
            return PathBuf::from(root).join("shellsense");
        }
        if let Some(root) = env::var_os("USERPROFILE") {
            return PathBuf::from(root)
                .join("AppData")
                .join("Roaming")
                .join("shellsense");
        }
        PathBuf::from("shellsense")
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(root) = env::var_os("HOME") {
            return PathBuf::from(root)
                .join("Library")
                .join("Application Support")
                .join("shellsense");
        }
        return PathBuf::from("shellsense");
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if let Some(root) = env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(root).join("shellsense");
        }
        if let Some(root) = env::var_os("HOME") {
            return PathBuf::from(root).join(".config").join("shellsense");
        }
        PathBuf::from(".config").join("shellsense")
    }
}

fn validate_ui(ui: &UiConfig) -> Result<()> {
    if !(1..=MAX_ROWS).contains(&ui.max_rows) {
        bail!(
            "ui.max_rows must be between 1 and {MAX_ROWS} (got {})",
            ui.max_rows
        );
    }
    if !(1..=MAX_WIDTH).contains(&ui.width) {
        bail!(
            "ui.width must be between 1 and {MAX_WIDTH} columns (got {})",
            ui.width
        );
    }

    match ui.border.as_str() {
        "rounded" | "square" | "none" => {}
        _ => bail!(
            "ui.border must be one of 'rounded', 'square', or 'none' (got '{}')",
            ui.border
        ),
    }

    for (field, value) in [
        ("ui.foreground", &ui.foreground),
        ("ui.background", &ui.background),
        ("ui.selected_foreground", &ui.selected_foreground),
        ("ui.selected_background", &ui.selected_background),
        ("ui.border_color", &ui.border_color),
        ("ui.description_color", &ui.description_color),
        ("ui.match_color", &ui.match_color),
    ] {
        if !is_valid_color(value) {
            bail!(
                "{field} must be 'default' or a six-digit #RRGGBB color (got '{}')",
                value
            );
        }
    }

    Ok(())
}

fn validate_completion(completion: &CompletionConfig) -> Result<()> {
    if !(1..=MAX_RESULTS).contains(&completion.max_results) {
        bail!(
            "completion.max_results must be between 1 and {MAX_RESULTS} (got {})",
            completion.max_results
        );
    }
    Ok(())
}

pub(crate) fn is_valid_color(value: &str) -> bool {
    value == "default"
        || (value.len() == 7
            && value.as_bytes()[0] == b'#'
            && value.as_bytes()[1..]
                .iter()
                .all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn defaults_are_valid_and_example_round_trips() {
        let config = Config::default();
        config.validate().expect("defaults should validate");

        let parsed: Config = toml::from_str(example()).expect("example should parse");
        assert_eq!(parsed.ui.max_rows, config.ui.max_rows);
        assert_eq!(parsed.completion.max_results, config.completion.max_results);
    }

    #[test]
    fn load_validates_bounds_and_colors() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("config.toml");
        fs::write(&path, "[ui]\nmax_rows = 0\nmatch_color = \"#12345\"\n").expect("write config");

        let error = load(Some(&path)).expect_err("invalid config should fail");
        let message = error.to_string();
        assert!(message.contains("ui.max_rows"), "{message}");
        assert!(message.contains("between 1"), "{message}");
    }

    #[test]
    fn unknown_fields_are_rejected_with_their_name() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("config.toml");
        fs::write(&path, "[ui]\nwat = true\n").expect("write config");

        let error = load(Some(&path)).expect_err("unknown field should fail");
        assert!(error.to_string().contains("wat"));
    }

    #[test]
    fn explicit_missing_path_is_actionable() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("missing.toml");

        let error = load(Some(&path)).expect_err("missing explicit path should fail");
        assert!(
            error
                .to_string()
                .contains("unable to read configuration file")
        );
        assert!(error.to_string().contains("missing.toml"));
    }

    #[test]
    fn colors_accept_default_and_hex_only() {
        assert!(is_valid_color("default"));
        assert!(is_valid_color("#abcdef"));
        assert!(is_valid_color("#ABCDEF"));
        assert!(!is_valid_color("#12345"));
        assert!(!is_valid_color("123456"));
        assert!(!is_valid_color("#1234567"));
    }
}

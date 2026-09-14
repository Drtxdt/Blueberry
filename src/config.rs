//! Configuration loading and validation for the completion UI.
//!
//! The configuration surface is deliberately small. Keeping validation here
//! means the renderer can assume values read from disk are bounded while still
//! remaining defensive for callers that construct a [`Config`] directly.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

const MAX_ROWS: usize = 100;
const MAX_WIDTH: usize = 512;
const MAX_RESULTS: usize = 1_000;

/// Top-level Blueberry configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub ui: UiConfig,
    pub completion: CompletionConfig,
    /// Offline overrides keyed by the full canonical command context.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub descriptions: BTreeMap<String, String>,
    pub keys: KeyBindings,
    pub learning: LearningConfig,
    pub specs: SpecsConfig,
    pub help: HelpConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeyBindings {
    pub trigger: String,
    pub native: String,
    pub search: String,
    pub details: String,
    pub refresh: String,
    pub reload: String,
    pub protocol_prefix: String,
}
impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            trigger: "Ctrl+Space".into(),
            native: "Ctrl+Alt+Space".into(),
            search: "Ctrl+Alt+F".into(),
            details: "F1".into(),
            refresh: "Ctrl+Alt+C".into(),
            reload: "Ctrl+Alt+R".into(),
            protocol_prefix: "F12".into(),
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LearningConfig {
    pub enabled: bool,
}
impl Default for LearningConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpecsConfig {
    pub directory: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct HelpConfig {
    pub enabled: bool,
}
impl Default for HelpConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
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
    pub icon_style: String,
    pub foreground: String,
    pub background: String,
    pub selected_foreground: String,
    pub selected_background: String,
    pub border_color: String,
    pub description_color: String,
    pub match_color: String,
    pub theme: String,
    pub status_bar: bool,
}

/// Completion collection settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CompletionConfig {
    pub max_results: usize,
    pub auto_trigger: bool,
    pub fuzzy: bool,
    pub dynamic: bool,
    pub append_space: bool,
    pub up_arrow_history: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            max_rows: 8,
            width: 0,
            descriptions: true,
            border: "rounded".to_owned(),
            icons: true,
            icon_style: "unicode".into(),
            foreground: "default".to_owned(),
            background: "default".to_owned(),
            selected_foreground: "#ffffff".to_owned(),
            selected_background: "#264f78".to_owned(),
            border_color: "#6688aa".to_owned(),
            description_color: "#8f9baa".to_owned(),
            match_color: "#ffcc66".to_owned(),
            theme: "dark".into(),
            status_bar: true,
        }
    }
}

impl Default for CompletionConfig {
    fn default() -> Self {
        Self {
            max_results: 100,
            auto_trigger: true,
            fuzzy: true,
            dynamic: true,
            append_space: true,
            up_arrow_history: true,
        }
    }
}

impl Config {
    /// Validate all bounded and enumerated configuration values.
    pub fn validate(&self) -> Result<()> {
        validate_ui(&self.ui)?;
        validate_completion(&self.completion)?;
        let mut seen = std::collections::HashSet::new();
        for (name, chord) in [
            ("trigger", &self.keys.trigger),
            ("native", &self.keys.native),
            ("search", &self.keys.search),
            ("details", &self.keys.details),
            ("refresh", &self.keys.refresh),
            ("reload", &self.keys.reload),
        ] {
            let parsed = crate::input::parse_chord(chord)
                .map_err(|error| anyhow!("keys.{name}: {error}"))?;
            if !seen.insert(parsed) {
                bail!("keys.{name}: duplicate shortcut '{chord}'");
            }
        }
        if !matches!(
            self.keys.protocol_prefix.as_str(),
            "F5" | "F6" | "F7" | "F8" | "F9" | "F10" | "F11" | "F12"
        ) {
            bail!("keys.protocol_prefix must be F5 through F12");
        }
        if self.descriptions.len() > 10_000 {
            bail!("descriptions must contain at most 10000 entries");
        }
        for (key, value) in &self.descriptions {
            if key.trim().is_empty() || key.chars().count() > 512 {
                bail!("description keys must contain 1 to 512 characters");
            }
            if value.trim().is_empty() || value.chars().count() > 2_048 {
                bail!("descriptions.{key} must contain 1 to 2048 characters");
            }
        }
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

    let mut config: Config = toml::from_str(&contents).map_err(|error| {
        anyhow!(
            "unable to parse configuration file '{}': {error}",
            path.display()
        )
    })?;

    let raw: toml::Value = toml::from_str(&contents)?;
    config.apply_theme(raw.get("ui").and_then(toml::Value::as_table));
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

pub fn specs_dir(config: &Config, config_path: Option<&Path>) -> PathBuf {
    config.specs.directory.clone().unwrap_or_else(|| {
        config_path
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(config_root)
            .join("specs")
    })
}
pub fn statistics_path() -> PathBuf {
    config_root().join("usage.json")
}

impl Config {
    pub(crate) fn apply_theme(&mut self, explicit: Option<&toml::Table>) {
        let palette = match self.ui.theme.as_str() {
            "light" => [
                "#202020", "#ffffff", "#ffffff", "#005fb8", "#657585", "#505050", "#9c3600",
            ],
            "high_contrast" => [
                "#ffffff", "#000000", "#000000", "#ffff00", "#ffffff", "#ffffff", "#00ffff",
            ],
            _ => return,
        };
        for ((key, field), value) in [
            ("foreground", &mut self.ui.foreground),
            ("background", &mut self.ui.background),
            ("selected_foreground", &mut self.ui.selected_foreground),
            ("selected_background", &mut self.ui.selected_background),
            ("border_color", &mut self.ui.border_color),
            ("description_color", &mut self.ui.description_color),
            ("match_color", &mut self.ui.match_color),
        ]
        .into_iter()
        .zip(palette)
        {
            if !explicit.is_some_and(|table| table.contains_key(key)) {
                *field = value.into();
            }
        }
    }
}

/// Return the platform-specific cache directory used by Blueberry.
pub fn cache_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(root) = env::var_os("LOCALAPPDATA") {
            return PathBuf::from(root).join("Blueberry").join("cache");
        }
        if let Some(root) = env::var_os("APPDATA") {
            return PathBuf::from(root).join("Blueberry").join("cache");
        }
        PathBuf::from("blueberry").join("cache")
    }

    #[cfg(not(windows))]
    {
        if let Some(root) = env::var_os("XDG_CACHE_HOME") {
            return PathBuf::from(root).join("Blueberry");
        }
        if let Some(root) = env::var_os("HOME") {
            return PathBuf::from(root).join(".cache").join("Blueberry");
        }
        PathBuf::from(".cache").join("Blueberry")
    }
}

/// Return a TOML example suitable for writing as a starting point for a
/// user's configuration.
pub fn example() -> &'static str {
    r##"# Blueberry configuration

[ui]
max_rows = 8
width = 0 # 0: adapt to the terminal width
theme = "dark" # dark, light, high_contrast
status_bar = true
descriptions = true
border = "rounded" # "rounded", "square", or "none"
icons = true
icon_style = "nerd"
# Uncomment individual colors to override the selected theme.
# foreground = "default"
# background = "default"
# selected_foreground = "#ffffff"
# selected_background = "#264f78"
# border_color = "#6688aa"
# description_color = "#8f9baa"
# match_color = "#ffcc66"

[completion]
max_results = 100
auto_trigger = true
fuzzy = true
dynamic = true
append_space = true
up_arrow_history = true

[keys]
trigger = "Ctrl+Space"
native = "Ctrl+Alt+Space"
search = "Ctrl+Alt+F"
details = "F1"
refresh = "Ctrl+Alt+C"
reload = "Ctrl+Alt+R"
protocol_prefix = "F12" # F5..F12; takes effect in a new session

[learning]
enabled = true # local selection counts only; learning clear removes them

[specs]
# directory = 'C:\Users\you\AppData\Roaming\blueberry\specs'

[descriptions]
# Optional overrides; keys distinguish command scopes and option case.
# "git log --oneline" = "每条提交显示为一行"
# "cargo" = "构建项目并管理 Rust 依赖"
# "mytool" = "运行我的本地工具"
"##
}

fn config_root() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(root) = env::var_os("APPDATA") {
            return PathBuf::from(root).join("Blueberry");
        }
        if let Some(root) = env::var_os("USERPROFILE") {
            return PathBuf::from(root)
                .join("AppData")
                .join("Roaming")
                .join("Blueberry");
        }
        PathBuf::from("blueberry")
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(root) = env::var_os("HOME") {
            return PathBuf::from(root)
                .join("Library")
                .join("Application Support")
                .join("Blueberry");
        }
        PathBuf::from("blueberry")
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if let Some(root) = env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(root).join("Blueberry");
        }
        if let Some(root) = env::var_os("HOME") {
            return PathBuf::from(root).join(".config").join("Blueberry");
        }
        PathBuf::from(".config").join("Blueberry")
    }
}

fn validate_ui(ui: &UiConfig) -> Result<()> {
    if !matches!(ui.icon_style.as_str(), "nerd" | "unicode") {
        bail!("ui.icon_style must be nerd or unicode");
    }
    if !(1..=MAX_ROWS).contains(&ui.max_rows) {
        bail!(
            "ui.max_rows must be between 1 and {MAX_ROWS} (got {})",
            ui.max_rows
        );
    }
    if ui.width > MAX_WIDTH {
        bail!(
            "ui.width must be between 0 (automatic) and {MAX_WIDTH} columns (got {})",
            ui.width
        );
    }

    if !matches!(ui.theme.as_str(), "dark" | "light" | "high_contrast") {
        bail!("ui.theme must be dark, light, or high_contrast");
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

    #[test]
    fn description_overrides_are_optional_validated_and_reloaded() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "[ui]\nwidth = 60\n").unwrap();
        assert!(load(Some(&path)).unwrap().descriptions.is_empty());
        fs::write(
            &path,
            "[descriptions]\n\"git log --oneline\" = \"逐行查看提交\"\n",
        )
        .unwrap();
        assert_eq!(
            load(Some(&path)).unwrap().descriptions["git log --oneline"],
            "逐行查看提交"
        );
        fs::write(
            &path,
            "[descriptions]\n\"git log --oneline\" = \"查看精简历史\"\n",
        )
        .unwrap();
        assert_eq!(
            load(Some(&path)).unwrap().descriptions["git log --oneline"],
            "查看精简历史"
        );
        fs::write(&path, "[descriptions]\n\"git\" = \" \"\n").unwrap();
        assert!(load(Some(&path)).is_err());
    }
}

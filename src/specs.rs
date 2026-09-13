//! Context-aware completion over the compiled ShellSense specification catalog.
//!
//! The public functions in this module intentionally keep the 0.2 API. The
//! catalog implementation is separate so the same parser can serve the
//! compiled built-ins and validated user TOML overlays.

use crate::{
    model::CandidateKind,
    spec_catalog::{self, Catalog, ValueKind},
};

/// A completion item produced by a command specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecCandidate {
    pub name: String,
    pub description: String,
    pub key: String,
    pub kind: CandidateKind,
    /// Expanded Chinese help and offline example text for F1/details panes.
    pub detail: String,
    /// `builtin` or the user specification file that supplied this item.
    pub source: String,
    /// A fixed, read-only dynamic data source identifier, when applicable.
    pub provider: Option<String>,
    /// Whether the host should add the next separator after insertion.
    pub append_space: bool,
}

/// The result of applying a command specification to an invocation prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecResult {
    pub candidates: Vec<SpecCandidate>,
    pub context: String,
    pub path_values: bool,
    /// Whether the active path value accepts directories only.
    pub directories_only: bool,
    pub options_ended: bool,
    /// Provider attached to the active context or pending value option.
    pub provider: Option<String>,
    /// Delimiter used by an option that accepts an inline value.
    pub value_delimiter: Option<char>,
    pub argument_hint: String,
}

/// Return the canonical root command name for a command or one of its
/// executable/PowerShell aliases.
pub fn canonical_command(command: &str) -> Option<&'static str> {
    let trimmed = command
        .trim()
        .trim_matches(|character| character == '\'' || character == '"');
    let basename = trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed);
    let lower = basename.to_ascii_lowercase();
    let stem = [".exe", ".cmd", ".bat", ".com"]
        .iter()
        .find_map(|suffix| lower.strip_suffix(suffix))
        .unwrap_or(&lower);
    match stem {
        "codex" => Some("codex"),
        "python" | "python3" => Some("python"),
        "uv" => Some("uv"),
        "rustc" => Some("rustc"),
        "winget" => Some("winget"),
        "dotnet" => Some("dotnet"),
        "git" => Some("git"),
        "cargo" => Some("cargo"),
        "npm" => Some("npm"),
        "pnpm" => Some("pnpm"),
        "docker" => Some("docker"),
        "pwsh" | "powershell" => Some("pwsh"),
        "gh" => Some("gh"),
        "set-location" | "cd" | "chdir" | "sl" => Some("Set-Location"),
        "get-childitem" | "gci" | "dir" | "ls" => Some("Get-ChildItem"),
        _ => None,
    }
}

/// Return a short, human-readable description for a recognized root command.
pub fn describe_command(command: &str) -> Option<&'static str> {
    let canonical = canonical_command(command)?;
    spec_catalog::builtin_root_description(canonical)
}

/// Return every built-in description, including entries in values, commands,
/// details, and options that are not currently reachable from one parser
/// context. This is kept as an inspection API for localization coverage.
#[doc(hidden)]
pub fn all_builtin_descriptions() -> Vec<&'static str> {
    spec_catalog::builtin_descriptions()
}

/// Complete a decoded argument prefix using the built-in catalog.
pub fn complete(command: &str, preceding_args: &[String], prefix: &str) -> Option<SpecResult> {
    Catalog::builtin().complete(command, preceding_args, prefix)
}

/// Complete a decoded argument prefix without applying a string-prefix
/// filter. The parser still uses `prefix` to identify inline values such as
/// `--name=value` and short attached values; the caller can then apply exact,
/// prefix, fuzzy, or usage ranking without losing scope information.
pub fn complete_unfiltered(
    command: &str,
    preceding_args: &[String],
    prefix: &str,
) -> Option<SpecResult> {
    Catalog::builtin().complete_unfiltered(command, preceding_args, prefix)
}

/// Look up a completion from a caller-owned catalog with the legacy prefix
/// filtering behavior.
pub fn complete_with_catalog(
    catalog: &Catalog,
    command: &str,
    preceding_args: &[String],
    prefix: &str,
) -> Option<SpecResult> {
    catalog.complete(command, preceding_args, prefix)
}

/// Look up all candidates from a caller-owned catalog while preserving the
/// active value/option context for host-side ranking.
pub fn complete_unfiltered_with_catalog(
    catalog: &Catalog,
    command: &str,
    preceding_args: &[String],
    prefix: &str,
) -> Option<SpecResult> {
    catalog.complete_unfiltered(command, preceding_args, prefix)
}

// Keep this import exercised in rustdoc and make the value-kind relationship
// explicit to downstream maintainers. The actual parser lives in catalog.
#[allow(dead_code)]
fn value_kind_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::None => "none",
        ValueKind::Text => "text",
        ValueKind::Path => "path",
        ValueKind::Directory => "directory",
    }
}

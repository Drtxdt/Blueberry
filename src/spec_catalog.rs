//! Versioned declarative command specifications.
//!
//! Built-in specifications are compiled from `specs/builtin.toml` by
//! `build.rs`; no built-in TOML is parsed while the application starts. User
//! specifications are deliberately limited to this data-only format. They
//! can describe command trees and fixed provider identifiers, but cannot run
//! scripts or load arbitrary executable plugins.

use crate::model::CandidateKind;
use crate::specs::{SpecCandidate, SpecResult};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

const SCHEMA_VERSION: u32 = 1;

/// Provider identifiers accepted by the declarative format. The provider
/// implementation lives in the engine; a spec can only name one of these
/// fixed, read-only data sources.
pub const ALLOWED_PROVIDERS: &[&str] = &[
    "git",
    "git.refs",
    "git.branches",
    "git.remotes",
    "git.tags",
    "git.worktrees",
    "git.status",
    "git.paths",
    "cargo.packages",
    "cargo.features",
    "cargo.bins",
    "cargo.examples",
    "cargo.tests",
    "cargo.benches",
    "npm.scripts",
    "npm.workspaces",
    "npm.dependencies",
    "pnpm.scripts",
    "pnpm.workspaces",
    "pnpm.dependencies",
    "powershell.env",
    "powershell.paths",
    "powershell.redirects",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueKind {
    None,
    Text,
    Path,
    Directory,
}

impl ValueKind {
    pub(crate) fn is_path(self) -> bool {
        matches!(self, Self::Path | Self::Directory)
    }

    pub(crate) fn is_directory(self) -> bool {
        self == Self::Directory
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValueDef {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OptionDef {
    pub(crate) names: Vec<String>,
    pub(crate) description: String,
    pub(crate) detail: String,
    pub(crate) value: ValueKind,
    pub(crate) values: Vec<ValueDef>,
    pub(crate) optional_value: bool,
    pub(crate) repeatable: bool,
    pub(crate) conflicts: Vec<String>,
    pub(crate) requires: Vec<String>,
    pub(crate) positional: bool,
    pub(crate) provider: Option<String>,
    pub(crate) value_delimiter: Option<char>,
    pub(crate) append_space: bool,
    pub(crate) short_cluster: bool,
    pub(crate) value_name: String,
    pub(crate) source: String,
}

impl OptionDef {
    pub(crate) fn name(&self) -> &str {
        self.names.first().map(String::as_str).unwrap_or("")
    }

    fn matches_name(&self, token: &str, insensitive: bool) -> bool {
        self.names
            .iter()
            .any(|name| token_eq(name, token, insensitive))
    }

    fn matches_inline<'a>(&self, token: &'a str, insensitive: bool) -> Option<&'a str> {
        for name in &self.names {
            if self.value == ValueKind::None {
                continue;
            }
            if name.starts_with("--") {
                // A long option always uses `=` for the option/value
                // boundary. `value_delimiter` describes separators inside
                // the value (for example `--features=a,b`); it must never
                // make `--features,a` look like an attached option value.
                let marker = format!("{name}=");
                if token.len() >= marker.len()
                    && token
                        .get(..marker.len())
                        .is_some_and(|head| token_eq(head, &marker, insensitive))
                {
                    return token.get(marker.len()..);
                }
            } else if token.len() > name.len()
                && token
                    .get(..name.len())
                    .is_some_and(|head| token_eq(head, name, insensitive))
            {
                return token.get(name.len()..);
            }
        }
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Node {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) detail: String,
    pub(crate) positional: ValueKind,
    pub(crate) children: Vec<String>,
    pub(crate) options: Vec<OptionDef>,
    pub(crate) examples: Vec<String>,
    pub(crate) keywords: Vec<String>,
    pub(crate) provider: Option<String>,
    pub(crate) value_delimiter: Option<char>,
    pub(crate) source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecDiagnostic {
    pub source: String,
    pub path: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Warning,
    Error,
}

impl fmt::Display for DiagnosticSeverity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Warning => formatter.write_str("warning"),
            Self::Error => formatter.write_str("error"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecSummary {
    pub path: String,
    pub description: String,
    pub source: String,
    pub provider: Option<String>,
    pub child_count: usize,
    pub option_count: usize,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub files: Vec<SpecFileSummary>,
    pub diagnostics: Vec<SpecDiagnostic>,
}

impl CheckReport {
    pub fn is_valid(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecFileSummary {
    pub path: String,
    pub valid: bool,
    pub node_count: usize,
    pub option_set_count: usize,
    /// Optional catalog metadata from the file header, exposed so callers
    /// can show which version was checked.
    pub name: String,
    pub version: String,
}

#[derive(Debug)]
pub enum CatalogLoadError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for CatalogLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "无法读取规格目录 {}: {source}", path.display())
            }
        }
    }
}

impl Error for CatalogLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub(crate) nodes: BTreeMap<String, Node>,
    aliases: BTreeMap<String, String>,
    diagnostics: Vec<SpecDiagnostic>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    schema_version: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    aliases: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    option_sets: Vec<OptionSetSpec>,
    #[serde(default)]
    nodes: Vec<NodeSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionSetSpec {
    id: String,
    #[serde(default)]
    options: Vec<RawOptionSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOptionSpec {
    name: String,
    #[serde(default)]
    names: Vec<String>,
    description: Option<String>,
    detail: Option<String>,
    value_kind: Option<String>,
    #[serde(default)]
    values: Vec<RawValueSpec>,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    repeatable: bool,
    #[serde(default)]
    conflicts: Vec<String>,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    positional: bool,
    provider: Option<String>,
    value_delimiter: Option<String>,
    append_space: Option<bool>,
    #[serde(default)]
    short_cluster: bool,
    #[serde(default)]
    value_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawValueSpec {
    name: String,
    description: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeSpec {
    path: String,
    name: Option<String>,
    description: Option<String>,
    detail: Option<String>,
    positional: Option<String>,
    children: Option<Vec<String>>,
    option_sets: Option<Vec<String>>,
    #[serde(default)]
    options: Vec<RawOptionSpec>,
    examples: Option<Vec<String>>,
    keywords: Option<Vec<String>>,
    provider: Option<String>,
    value_delimiter: Option<String>,
}

#[derive(Debug)]
pub(crate) struct StaticValue {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) detail: &'static str,
}

#[derive(Debug)]
pub(crate) struct StaticOption {
    pub(crate) names: &'static [&'static str],
    pub(crate) description: &'static str,
    pub(crate) detail: &'static str,
    pub(crate) value_kind: &'static str,
    pub(crate) values: &'static [StaticValue],
    pub(crate) optional_value: bool,
    pub(crate) repeatable: bool,
    pub(crate) conflicts: &'static [&'static str],
    pub(crate) requires: &'static [&'static str],
    pub(crate) positional: bool,
    pub(crate) provider: Option<&'static str>,
    pub(crate) value_delimiter: Option<char>,
    pub(crate) append_space: bool,
    pub(crate) short_cluster: bool,
    pub(crate) value_name: &'static str,
}

#[derive(Debug)]
pub(crate) struct StaticOptionSet {
    pub(crate) id: &'static str,
    pub(crate) options: &'static [StaticOption],
}

#[derive(Debug)]
pub(crate) struct StaticNode {
    pub(crate) path: &'static str,
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) detail: &'static str,
    pub(crate) positional: &'static str,
    pub(crate) children: &'static [&'static str],
    pub(crate) option_sets: &'static [&'static str],
    pub(crate) options: &'static [StaticOption],
    pub(crate) examples: &'static [&'static str],
    pub(crate) keywords: &'static [&'static str],
    pub(crate) provider: Option<&'static str>,
    pub(crate) value_delimiter: Option<char>,
}

#[derive(Debug)]
pub(crate) struct StaticAlias {
    pub(crate) canonical: &'static str,
    pub(crate) aliases: &'static [&'static str],
}

include!(concat!(env!("OUT_DIR"), "/builtin_specs.rs"));

pub(crate) fn builtin_root_description(command: &str) -> Option<&'static str> {
    BUILTIN_ROOT_DESCRIPTIONS
        .iter()
        .find_map(|(name, description)| (*name == command).then_some(*description))
}

pub(crate) fn builtin_descriptions() -> Vec<&'static str> {
    BUILTIN_DESCRIPTION_INVENTORY.to_vec()
}

impl Catalog {
    /// Merge one entry-specific help page without replacing user-authored semantics.
    pub fn apply_help(&mut self, record: &crate::knowledge::Record) {
        if record.error.is_some() {
            return;
        }
        let root = self
            .canonical_command(&record.entry.command)
            .unwrap_or_else(|| record.entry.command.clone());
        let path = std::iter::once(root.as_str())
            .chain(record.context.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        let source = format!("help:{}", record.entry.path.display());
        self.aliases
            .insert(normalize_alias(&record.entry.command), root.clone());
        let empty = |path: String, description: String| Node {
            name: path.rsplit(' ').next().unwrap_or("").into(),
            path,
            description: description.clone(),
            detail: description,
            positional: ValueKind::Path,
            children: Vec::new(),
            options: Vec::new(),
            examples: Vec::new(),
            keywords: Vec::new(),
            provider: None,
            value_delimiter: None,
            source: source.clone(),
        };
        let user_children: std::collections::HashSet<String> = self
            .nodes
            .values()
            .filter(|n| n.source != "builtin" && !n.source.starts_with("help:"))
            .map(|n| n.path.clone())
            .collect();
        let node = self
            .nodes
            .entry(path.clone())
            .or_insert_with(|| empty(path.clone(), record.page.description.clone()));
        let user_node = node.source != "builtin" && !node.source.starts_with("help:");
        if record.page.complete && !user_node {
            node.children.retain(|child| {
                user_children.contains(child)
                    || record
                        .page
                        .commands
                        .iter()
                        .any(|(name, _)| child == &format!("{path} {name}"))
            });
            node.options.retain(|option| {
                option.source != "builtin" && !option.source.starts_with("help:")
                    || record
                        .page
                        .options
                        .iter()
                        .any(|raw| raw.names.iter().any(|name| option.names.contains(name)))
            });
        }
        for raw in &record.page.options {
            if let Some(old) = node
                .options
                .iter_mut()
                .find(|o| raw.names.iter().any(|n| o.names.contains(n)))
            {
                if old.source == "builtin" || old.source.starts_with("help:") {
                    if record.page.complete {
                        old.names = raw.names.clone();
                        old.names.sort_by_key(|name| std::cmp::Reverse(name.len()));
                    }
                    if !raw.value_name.is_empty() {
                        old.value_name = raw.value_name.clone();
                        if old.value == ValueKind::None {
                            old.value = ValueKind::Text;
                        }
                    } else if record.page.complete {
                        old.value_name.clear();
                        old.value = ValueKind::None;
                    }
                    if !raw.values.is_empty() {
                        old.values = raw
                            .values
                            .iter()
                            .map(|name| {
                                old.values
                                    .iter()
                                    .find(|value| &value.name == name)
                                    .cloned()
                                    .unwrap_or_else(|| ValueDef {
                                        name: name.clone(),
                                        description: format!("选择 {name}"),
                                        detail: String::new(),
                                    })
                            })
                            .collect();
                    }
                    old.detail = format!("{}\n{}", old.description, raw.detail);
                }
                continue;
            }
            if user_node {
                continue;
            }
            let mut names = raw.names.clone();
            names.sort_by_key(|n| std::cmp::Reverse(n.len()));
            node.options.push(OptionDef {
                names,
                description: raw.description.clone(),
                detail: raw.detail.clone(),
                value: if raw.value_name.is_empty() {
                    ValueKind::None
                } else {
                    ValueKind::Text
                },
                values: raw
                    .values
                    .iter()
                    .map(|v| ValueDef {
                        name: v.clone(),
                        description: format!("选择 {v}"),
                        detail: String::new(),
                    })
                    .collect(),
                optional_value: false,
                repeatable: raw.repeatable,
                conflicts: Vec::new(),
                requires: Vec::new(),
                positional: false,
                provider: None,
                value_delimiter: None,
                append_space: true,
                short_cluster: false,
                value_name: raw.value_name.clone(),
                source: source.clone(),
            });
        }
        for (name, description) in &record.page.commands {
            let child = format!("{path} {name}");
            if !user_node && !node.children.contains(&child) {
                node.children.push(child);
            }
            let _ = description;
        }
        for (name, description) in &record.page.commands {
            let child = format!("{path} {name}");
            self.nodes
                .entry(child.clone())
                .or_insert_with(|| empty(child, description.clone()));
        }
    }
    /// Name and schema version of the generated built-in catalog.
    pub fn builtin_metadata() -> (&'static str, &'static str) {
        (BUILTIN_CATALOG_NAME, BUILTIN_CATALOG_VERSION)
    }

    /// Return the process-wide immutable built-in catalog. It is constructed
    /// from generated Rust tables on first use and never reparses TOML.
    pub fn builtin() -> &'static Self {
        static BUILTIN: OnceLock<Catalog> = OnceLock::new();
        BUILTIN.get_or_init(Self::from_builtin)
    }

    fn from_builtin() -> Self {
        let mut nodes = BTreeMap::new();
        let sets: BTreeMap<&str, &'static [StaticOption]> = BUILTIN_OPTION_SETS
            .iter()
            .map(|set| (set.id, set.options))
            .collect();
        for node in BUILTIN_NODES {
            let mut options = Vec::new();
            for set_name in node.option_sets {
                if let Some(set) = sets.get(set_name) {
                    options.extend(set.iter().map(option_from_static));
                }
            }
            options.extend(node.options.iter().map(option_from_static));
            nodes.insert(
                node.path.to_owned(),
                Node {
                    path: node.path.to_owned(),
                    name: node.name.to_owned(),
                    description: node.description.to_owned(),
                    detail: node.detail.to_owned(),
                    positional: parse_value_kind(node.positional).unwrap_or(ValueKind::Text),
                    children: node
                        .children
                        .iter()
                        .map(|child| (*child).to_owned())
                        .collect(),
                    options,
                    keywords: node.keywords.iter().map(|s| (*s).to_owned()).collect(),
                    examples: node
                        .examples
                        .iter()
                        .map(|example| (*example).to_owned())
                        .collect(),
                    provider: node.provider.map(str::to_owned),
                    value_delimiter: node.value_delimiter,
                    source: "builtin".to_owned(),
                },
            );
        }
        let mut aliases = BTreeMap::new();
        for alias in BUILTIN_ALIASES {
            aliases.insert(normalize_alias(alias.canonical), alias.canonical.to_owned());
            for spelling in alias.aliases {
                aliases.insert(normalize_alias(spelling), alias.canonical.to_owned());
            }
        }
        Self {
            nodes,
            aliases,
            diagnostics: Vec::new(),
        }
    }

    /// Load all user `*.toml` files from a directory over a clone of the
    /// built-in catalog. Files are sorted by filename. A malformed file is
    /// reported and skipped, so a previously valid snapshot remains usable;
    /// only directory-level I/O returns an error.
    pub fn load_user_dir(path: impl AsRef<Path>) -> Result<Self, CatalogLoadError> {
        let path = path.as_ref();
        let mut catalog = Self::builtin().clone();
        // A first run has no user-spec directory yet. Treat that state as an
        // empty directory so the host can start with generated built-ins;
        // callers that explicitly validate a path can still use check_dir to
        // report a missing directory.
        let report = match read_user_files(path) {
            Ok(report) => report,
            Err(CatalogLoadError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(catalog);
            }
            Err(error) => return Err(error),
        };
        for parsed in report.parsed {
            match parsed.file {
                Ok(file) => {
                    if let Err(message) = catalog.apply_file(&file, &parsed.path) {
                        catalog.diagnostics.push(SpecDiagnostic {
                            source: "user".to_owned(),
                            path: parsed.path.display().to_string(),
                            severity: DiagnosticSeverity::Error,
                            message,
                            context: None,
                        });
                    }
                }
                Err(message) => catalog.diagnostics.push(SpecDiagnostic {
                    source: "user".to_owned(),
                    path: parsed.path.display().to_string(),
                    severity: DiagnosticSeverity::Error,
                    message,
                    context: None,
                }),
            }
        }
        Ok(catalog)
    }

    /// Validate every user file without mutating the active catalog.
    pub fn check_dir(path: impl AsRef<Path>) -> Result<CheckReport, CatalogLoadError> {
        let report = read_user_files(path.as_ref())?;
        let mut diagnostics = Vec::new();
        let mut files = Vec::new();
        for parsed in report.parsed {
            let mut valid = false;
            let mut node_count = 0;
            let mut option_set_count = 0;
            let mut name = String::new();
            let mut version = String::new();
            match parsed.file {
                Ok(file) => {
                    node_count = file.nodes.len();
                    option_set_count = file.option_sets.len();
                    name = file.name.clone();
                    version = file.version.clone();
                    match validate_file(&file) {
                        Ok(()) => valid = true,
                        Err(message) => diagnostics.push(SpecDiagnostic {
                            source: "user".to_owned(),
                            path: parsed.path.display().to_string(),
                            severity: DiagnosticSeverity::Error,
                            message,
                            context: None,
                        }),
                    }
                }
                Err(message) => diagnostics.push(SpecDiagnostic {
                    source: "user".to_owned(),
                    path: parsed.path.display().to_string(),
                    severity: DiagnosticSeverity::Error,
                    message,
                    context: None,
                }),
            }
            files.push(SpecFileSummary {
                path: parsed.path.display().to_string(),
                valid,
                node_count,
                option_set_count,
                name,
                version,
            });
        }
        Ok(CheckReport { files, diagnostics })
    }

    /// Return a compact view suitable for `specs list` or the F1 diagnostics
    /// panel. User nodes retain the path of the exact context they override.
    pub fn list(&self) -> Vec<SpecSummary> {
        self.nodes
            .values()
            .map(|node| SpecSummary {
                path: node.path.clone(),
                description: node.description.clone(),
                source: node.source.clone(),
                provider: node.provider.clone(),
                child_count: node.children.len(),
                option_count: node.options.len(),
                examples: node.examples.clone(),
            })
            .collect()
    }

    pub fn diagnostics(&self) -> &[SpecDiagnostic] {
        &self.diagnostics
    }

    pub fn canonical_command(&self, command: &str) -> Option<String> {
        let normalized = normalize_command(command);
        if let Some(canonical) = self.aliases.get(&normalized) {
            return Some(canonical.clone());
        }
        self.nodes
            .keys()
            .filter(|path| !path.contains(' '))
            .find(|path| normalize_alias(path) == normalized)
            .cloned()
    }

    /// Return the localized description for a root command or one of its
    /// declared aliases.  The value is borrowed from this catalog so user
    /// catalogs can be reloaded without leaking strings into a process-wide
    /// static table.
    pub fn describe_command(&self, command: &str) -> Option<&str> {
        let normalized = normalize_command(command);
        let canonical = self
            .aliases
            .get(&normalized)
            .map(String::as_str)
            .or_else(|| {
                self.nodes
                    .keys()
                    .filter(|path| !path.contains(' '))
                    .find(|path| normalize_alias(path) == normalized)
                    .map(String::as_str)
            })?;
        self.nodes
            .get(canonical)
            .map(|node| node.description.as_str())
    }

    pub fn lookup(
        &self,
        command: &str,
        preceding_args: &[String],
        prefix: &str,
    ) -> Option<SpecResult> {
        self.complete(command, preceding_args, prefix)
    }

    pub fn lookup_unfiltered(
        &self,
        command: &str,
        preceding_args: &[String],
        prefix: &str,
    ) -> Option<SpecResult> {
        self.complete_unfiltered(command, preceding_args, prefix)
    }

    pub fn complete(
        &self,
        command: &str,
        preceding_args: &[String],
        prefix: &str,
    ) -> Option<SpecResult> {
        let mut result = self.complete_unfiltered(command, preceding_args, prefix)?;
        let insensitive = result.context.starts_with("pwsh")
            || result.context.starts_with("Set-Location")
            || result.context.starts_with("Get-ChildItem");
        result
            .candidates
            .retain(|candidate| starts_with(&candidate.name, prefix, insensitive));
        Some(result)
    }

    pub fn complete_unfiltered(
        &self,
        command: &str,
        preceding_args: &[String],
        prefix: &str,
    ) -> Option<SpecResult> {
        let root = self.canonical_command(command)?;
        let parsed = self.parse(&root, preceding_args);
        let context = parsed.path.join(" ");
        let insensitive = root == "pwsh" || root == "Set-Location" || root == "Get-ChildItem";
        let mut result = SpecResult {
            candidates: Vec::new(),
            context,
            path_values: false,
            directories_only: false,
            options_ended: parsed.options_ended,
            provider: effective_provider(parsed.node.provider.clone(), &root),
            value_delimiter: parsed.node.value_delimiter,
            argument_hint: String::new(),
        };

        if let Some(option) = parsed.pending {
            result.argument_hint = format!(
                "{} {} {}",
                result.context,
                option.name(),
                if option.value_name.is_empty() {
                    "<VALUE>"
                } else {
                    &option.value_name
                }
            );
            result.path_values = option.value.is_path();
            result.directories_only = option.value.is_directory();
            result.provider = effective_provider(option.provider.clone(), &root);
            result.value_delimiter = option.value_delimiter;
            add_values(&mut result.candidates, &parsed.path, &option, prefix, None);
            return Some(result);
        }
        if !parsed.options_ended
            && let Some((option, inline_prefix)) =
                self.find_inline_option(&parsed.node, prefix, insensitive)
        {
            result.argument_hint = format!(
                "{} {} {}",
                result.context,
                option.name(),
                if option.value_name.is_empty() {
                    "<VALUE>"
                } else {
                    &option.value_name
                }
            );
            result.path_values = option.value.is_path();
            result.directories_only = option.value.is_directory();
            result.provider = effective_provider(option.provider.clone(), &root);
            result.value_delimiter = option.value_delimiter;
            add_values(
                &mut result.candidates,
                &parsed.path,
                &option,
                prefix,
                Some(inline_prefix),
            );
            return Some(result);
        }

        let options_requested = prefix.starts_with('-')
            || (prefix.is_empty() && root == "pwsh" && parsed.path.len() == 1);
        if !parsed.options_ended && options_requested {
            // A node provider describes positional values, not flag names.
            result.provider = None;
            for option in &parsed.node.options {
                if !option_allowed(option, &parsed.seen_options) {
                    continue;
                }
                let name = option.name().to_owned();
                result.candidates.push(SpecCandidate {
                    name,
                    description: option.description.clone(),
                    detail: format!(
                        "{}\n格式：{} {}",
                        option.detail,
                        option.name(),
                        option.value_name
                    ),
                    key: format!("{} {}", result.context, option.name()),
                    kind: CandidateKind::Option,
                    source: option.source.clone(),
                    provider: effective_provider(
                        option.provider.clone().or(parsed.node.provider.clone()),
                        &root,
                    ),
                    append_space: option.append_space,
                });
            }
        } else if !parsed.options_ended && parsed.allow_subcommands && !prefix.starts_with('-') {
            for child_path in &parsed.node.children {
                let Some(child) = self.nodes.get(child_path) else {
                    continue;
                };
                result.candidates.push(SpecCandidate {
                    name: child.name.clone(),
                    description: child.description.clone(),
                    detail: format!(
                        "{}\n{}",
                        child.detail,
                        child
                            .examples
                            .iter()
                            .map(|e| format!("示例：{e}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    key: format!("{} {}", result.context, child.name),
                    kind: CandidateKind::Subcommand,
                    source: child.source.clone(),
                    provider: effective_provider(child.provider.clone(), &root),
                    append_space: true,
                });
            }
        }
        result.path_values = parsed.node.positional.is_path();
        result.directories_only = parsed.node.positional.is_directory();
        Some(result)
    }

    fn parse(&self, root: &str, args: &[String]) -> Parsed {
        let mut path = vec![root.to_owned()];
        let mut node = self.nodes.get(root).cloned().unwrap_or_else(|| Node {
            path: root.to_owned(),
            name: root.to_owned(),
            description: String::new(),
            detail: String::new(),
            positional: ValueKind::Text,
            children: Vec::new(),
            options: Vec::new(),
            keywords: Vec::new(),
            examples: Vec::new(),
            provider: None,
            value_delimiter: None,
            source: "user".to_owned(),
        });
        let mut pending: Option<OptionDef> = None;
        let mut options_ended = false;
        let mut allow_subcommands = !node.children.is_empty();
        let mut seen_options: BTreeSet<String> = BTreeSet::new();
        let mut index = 0;
        while index < args.len() {
            let token = args[index].as_str();
            if pending.take().is_some() {
                index += 1;
                continue;
            }
            if !options_ended && token == "--" {
                options_ended = true;
                allow_subcommands = false;
                index += 1;
                continue;
            }
            if root == "cargo"
                && path.len() == 1
                && allow_subcommands
                && token.starts_with('+')
                && token.len() > 1
            {
                index += 1;
                continue;
            }
            if !options_ended && token.starts_with('-') && token != "-" {
                if let Some(option) = find_option(&node.options, token, root_is_insensitive(root)) {
                    seen_options.insert(option.name().to_owned());
                    let attached = option
                        .matches_inline(token, root_is_insensitive(root))
                        .is_some();
                    if option.value != ValueKind::None && !option.optional_value && !attached {
                        pending = Some(option.clone());
                    }
                } else if let Some((options, has_value)) = find_short_cluster(&node.options, token)
                {
                    for option in options {
                        seen_options.insert(option.name().to_owned());
                        if option.value != ValueKind::None && !option.optional_value {
                            if !has_value {
                                pending = Some(option.clone());
                            }
                            break;
                        }
                    }
                }
                index += 1;
                continue;
            }
            if allow_subcommands {
                if let Some(child_path) = node.children.iter().find(|child_path| {
                    self.nodes.get(*child_path).is_some_and(|child| {
                        token_eq(&child.name, token, root_is_insensitive(root))
                    })
                }) && let Some(child) = self.nodes.get(child_path).cloned()
                {
                    path.push(child.name.clone());
                    node = child;
                    allow_subcommands = !node.children.is_empty();
                    index += 1;
                    continue;
                }
                allow_subcommands = false;
            }
            index += 1;
        }
        Parsed {
            path,
            node,
            pending,
            options_ended,
            allow_subcommands,
            seen_options,
        }
    }

    fn find_inline_option<'a>(
        &self,
        node: &Node,
        prefix: &'a str,
        insensitive: bool,
    ) -> Option<(OptionDef, &'a str)> {
        node.options.iter().find_map(|option| {
            option
                .matches_inline(prefix, insensitive)
                .map(|suffix| (option.clone(), suffix))
        })
    }

    fn apply_file(&mut self, file: &CatalogFile, path: &Path) -> Result<(), String> {
        validate_file(file)?;
        // Resolve a complete file against a clone first. A later node, alias,
        // or option error must never leave half of an invalid snapshot in the
        // live catalog used by completion and `specs list`.
        let mut next = self.clone();
        next.apply_file_inner(file, path)?;
        *self = next;
        Ok(())
    }

    fn apply_file_inner(&mut self, file: &CatalogFile, path: &Path) -> Result<(), String> {
        let mut resolved_sets = BTreeMap::new();
        for set in &file.option_sets {
            let mut options = Vec::new();
            for raw in &set.options {
                options.push(option_from_raw(raw, path)?);
            }
            resolved_sets.insert(set.id.clone(), options);
        }
        for raw_node in &file.nodes {
            let old = self.nodes.get(&raw_node.path).cloned();
            let mut node = old.clone().unwrap_or_else(|| Node {
                path: raw_node.path.clone(),
                name: raw_node.name.clone().unwrap_or_else(|| {
                    raw_node
                        .path
                        .rsplit_once(' ')
                        .map(|(_, n)| n.to_owned())
                        .unwrap_or_else(|| raw_node.path.clone())
                }),
                description: String::new(),
                detail: String::new(),
                positional: ValueKind::Text,
                children: Vec::new(),
                options: Vec::new(),
                keywords: Vec::new(),
                examples: Vec::new(),
                provider: None,
                value_delimiter: None,
                source: path.display().to_string(),
            });
            if let Some(keywords) = &raw_node.keywords {
                node.keywords = keywords.clone();
            }
            if let Some(name) = &raw_node.name {
                node.name = name.clone();
            }
            if let Some(description) = &raw_node.description {
                node.description = description.clone();
            } else if node.description.is_empty() {
                return Err(format!(
                    "nodes[{}] requires description for a new context",
                    raw_node.path
                ));
            }
            if let Some(detail) = &raw_node.detail {
                node.detail = detail.clone();
            } else if node.detail.is_empty() {
                node.detail = node.description.clone();
            }
            if let Some(positional) = &raw_node.positional {
                node.positional = parse_value_kind(positional).ok_or_else(|| {
                    format!("nodes[{}]: invalid positional {positional}", raw_node.path)
                })?;
            }
            if let Some(children) = &raw_node.children {
                node.children = normalize_children(&raw_node.path, children);
            }
            if raw_node.option_sets.is_some() && !raw_node.options.is_empty() {
                return Err(format!(
                    "nodes[{}]: option_sets and inline options are mutually exclusive",
                    raw_node.path
                ));
            }
            if let Some(option_sets) = &raw_node.option_sets {
                let mut options = Vec::new();
                for option_set in option_sets {
                    let Some(set) = resolved_sets.get(option_set) else {
                        return Err(format!(
                            "nodes[{}]: unknown option set {option_set}",
                            raw_node.path
                        ));
                    };
                    options.extend(set.iter().cloned());
                }
                node.options = options;
            } else if !raw_node.options.is_empty() {
                node.options = raw_node
                    .options
                    .iter()
                    .map(|option| option_from_raw(option, path))
                    .collect::<Result<Vec<_>, _>>()?;
            }
            if let Some(examples) = &raw_node.examples {
                node.examples = examples.clone();
            }
            if let Some(provider) = &raw_node.provider {
                ensure_provider(provider)?;
                node.provider = Some(provider.clone());
            }
            if let Some(delimiter) = &raw_node.value_delimiter {
                node.value_delimiter = parse_delimiter(delimiter)?;
            }
            node.source = path.display().to_string();
            self.nodes.insert(raw_node.path.clone(), node);
            // A user-defined root context is a valid command even when the
            // file does not repeat an [aliases] entry. Register it here so
            // canonical_command and lookup use the same exact context.
            if !raw_node.path.contains(' ') {
                self.aliases
                    .entry(normalize_alias(&raw_node.path))
                    .or_insert_with(|| raw_node.path.clone());
            }
        }
        for node in file.nodes.iter().filter(|node| node.children.is_none()) {
            if let Some((parent, _)) = node.path.rsplit_once(' ')
                && let Some(parent_node) = self.nodes.get_mut(parent)
                && !parent_node.children.contains(&node.path)
            {
                parent_node.children.push(node.path.clone());
            }
        }
        for (canonical, aliases) in &file.aliases {
            if !self
                .nodes
                .keys()
                .any(|path| path == canonical || path.starts_with(&format!("{canonical} ")))
            {
                return Err(format!("alias canonical root has no node: {canonical}"));
            }
            self.aliases
                .insert(normalize_alias(canonical), canonical.clone());
            for alias in aliases {
                self.aliases
                    .insert(normalize_alias(alias), canonical.clone());
            }
        }
        Ok(())
    }
}

struct ParsedFile {
    path: PathBuf,
    file: Result<CatalogFile, String>,
}

struct UserFiles {
    parsed: Vec<ParsedFile>,
}

fn read_user_files(path: &Path) -> Result<UserFiles, CatalogLoadError> {
    let mut paths = Vec::new();
    let entries = fs::read_dir(path).map_err(|source| CatalogLoadError::Io {
        path: path.to_owned(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| CatalogLoadError::Io {
            path: path.to_owned(),
            source,
        })?;
        let entry_path = entry.path();
        if entry_path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
        {
            paths.push(entry_path);
        }
    }
    paths.sort();
    let parsed = paths
        .into_iter()
        .map(|path| {
            let file = fs::read_to_string(&path)
                .map_err(|error| format!("无法读取文件: {error}"))
                .and_then(|text| {
                    toml::from_str::<CatalogFile>(&text)
                        .map_err(|error| format!("TOML 解析失败: {error}"))
                });
            ParsedFile { path, file }
        })
        .collect();
    Ok(UserFiles { parsed })
}

fn validate_file(file: &CatalogFile) -> Result<(), String> {
    if file.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "schema_version {} 不受支持，当前支持 {}",
            file.schema_version, SCHEMA_VERSION
        ));
    }
    let mut set_ids = BTreeSet::new();
    for set in &file.option_sets {
        if set.id.trim().is_empty() || !set_ids.insert(&set.id) {
            return Err(format!("option set 无效或重复: {}", set.id));
        }
        let mut option_names = BTreeSet::new();
        for option in &set.options {
            validate_raw_option(option)?;
            if !option_names.insert(&option.name) {
                return Err(format!(
                    "option set {} 中存在重复选项 {}",
                    set.id, option.name
                ));
            }
        }
    }
    let mut paths = BTreeSet::new();
    for node in &file.nodes {
        if node.path.trim().is_empty() || !paths.insert(&node.path) {
            return Err(format!("节点路径为空或重复: {}", node.path));
        }
        if let Some(description) = &node.description
            && description.trim().is_empty()
        {
            return Err(format!("节点 {} 的 description 不能为空", node.path));
        }
        if let Some(detail) = &node.detail
            && detail.trim().is_empty()
        {
            return Err(format!("节点 {} 的 detail 不能为空", node.path));
        }
        if let Some(positional) = &node.positional
            && parse_value_kind(positional).is_none()
        {
            return Err(format!(
                "节点 {} 的 positional 无效: {positional}",
                node.path
            ));
        }
        if let Some(provider) = &node.provider {
            ensure_provider(provider)?;
        }
        if let Some(delimiter) = &node.value_delimiter {
            parse_delimiter(delimiter)?;
        }
        if node.option_sets.is_some() && !node.options.is_empty() {
            return Err(format!(
                "节点 {} 同时使用 option_sets 和 inline options",
                node.path
            ));
        }
        for option in &node.options {
            validate_raw_option(option)?;
        }
        for set in node.option_sets.as_deref().unwrap_or(&[]) {
            if !set_ids.contains(set) {
                return Err(format!("节点 {} 引用了未知 option set {set}", node.path));
            }
        }
    }
    for (canonical, aliases) in &file.aliases {
        if canonical.trim().is_empty() || aliases.iter().any(|alias| alias.trim().is_empty()) {
            return Err("aliases 不能包含空名称".to_owned());
        }
    }
    Ok(())
}

fn validate_raw_option(option: &RawOptionSpec) -> Result<(), String> {
    if !option.name.starts_with('-') || option.name == "undefined" {
        return Err(format!("无效选项：{}", option.name));
    }
    if option.description.as_deref().is_some_and(|s| {
        matches!(s.trim(), "参数选项" | "待定" | "undefined") || s.contains("用途暂未收录")
    }) {
        return Err(format!("占位说明：{}", option.name));
    }
    if option.name.trim().is_empty() || option.name.chars().any(char::is_whitespace) {
        return Err(format!("option 名称无效: {}", option.name));
    }
    if option
        .description
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(format!("option {} 的 description 不能为空", option.name));
    }
    if let Some(value_kind) = &option.value_kind
        && parse_value_kind(value_kind).is_none()
    {
        return Err(format!(
            "option {} 的 value_kind 无效: {value_kind}",
            option.name
        ));
    }
    if option.value_kind.as_deref() == Some("none") && !option.values.is_empty() {
        return Err(format!("flag option {} 不能定义 values", option.name));
    }
    for value in &option.values {
        if value.name.trim().is_empty() {
            return Err(format!("option {} 的 value 名称不能为空", option.name));
        }
        if value
            .description
            .as_deref()
            .is_some_and(|description| description.trim().is_empty())
        {
            return Err(format!(
                "option {} 的 value description 不能为空",
                option.name
            ));
        }
    }
    if let Some(provider) = &option.provider {
        ensure_provider(provider)?;
    }
    if let Some(delimiter) = &option.value_delimiter {
        parse_delimiter(delimiter)?;
    }
    Ok(())
}

fn option_from_static(option: &StaticOption) -> OptionDef {
    OptionDef {
        names: option.names.iter().map(|name| (*name).to_owned()).collect(),
        description: option.description.to_owned(),
        detail: option.detail.to_owned(),
        value: parse_value_kind(option.value_kind).unwrap_or(ValueKind::None),
        values: option
            .values
            .iter()
            .map(|value| ValueDef {
                name: value.name.to_owned(),
                description: value.description.to_owned(),
                detail: value.detail.to_owned(),
            })
            .collect(),
        optional_value: option.optional_value,
        repeatable: option.repeatable,
        conflicts: option
            .conflicts
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        requires: option
            .requires
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        positional: option.positional,
        provider: option.provider.map(str::to_owned),
        value_delimiter: option.value_delimiter,
        append_space: option.append_space,
        value_name: option.value_name.to_owned(),
        short_cluster: option.short_cluster,
        source: "builtin".to_owned(),
    }
}

fn option_from_raw(option: &RawOptionSpec, path: &Path) -> Result<OptionDef, String> {
    let mut names = Vec::with_capacity(option.names.len() + 1);
    names.push(option.name.clone());
    names.extend(
        option
            .names
            .iter()
            .filter(|name| *name != &option.name)
            .cloned(),
    );
    let inferred_value = if option.values.is_empty() {
        ValueKind::None
    } else {
        ValueKind::Text
    };
    let value = option
        .value_kind
        .as_deref()
        .and_then(parse_value_kind)
        .unwrap_or(inferred_value);
    let description = option
        .description
        .clone()
        .unwrap_or_else(|| "规格选项".to_owned());
    let detail = option.detail.clone().unwrap_or_else(|| description.clone());
    Ok(OptionDef {
        names,
        description,
        detail,
        value,
        values: option
            .values
            .iter()
            .map(|value| {
                let description = value
                    .description
                    .clone()
                    .unwrap_or_else(|| "可用值".to_owned());
                ValueDef {
                    name: value.name.clone(),
                    detail: value.detail.clone().unwrap_or_else(|| description.clone()),
                    description,
                }
            })
            .collect(),
        optional_value: option.optional,
        repeatable: option.repeatable,
        conflicts: option.conflicts.clone(),
        requires: option.requires.clone(),
        positional: option.positional,
        provider: option.provider.clone(),
        value_delimiter: option
            .value_delimiter
            .as_deref()
            .map(parse_delimiter)
            .transpose()?
            .flatten(),
        append_space: option.append_space.unwrap_or(true),
        value_name: option.value_name.to_owned(),
        short_cluster: option.short_cluster,
        source: path.display().to_string(),
    })
}

fn add_values(
    candidates: &mut Vec<SpecCandidate>,
    path: &[String],
    option: &OptionDef,
    prefix: &str,
    inline_prefix: Option<&str>,
) {
    let context = path.join(" ");
    for value in &option.values {
        let name = if inline_prefix.is_some() {
            inline_value_spelling(option.name(), &value.name, option.value_delimiter, prefix)
        } else {
            value.name.clone()
        };
        candidates.push(SpecCandidate {
            name,
            description: value.description.clone(),
            detail: value.detail.clone(),
            key: format!("{context} {} {}", option.name(), value.name),
            kind: CandidateKind::Value,
            source: option.source.clone(),
            provider: option.provider.clone(),
            append_space: false,
        });
    }
    // The unfiltered API intentionally keeps every valid value. `complete`
    // applies the old prefix filter after the parser has identified context;
    // this keeps --name=value and short attached values lossless.
    let _ = prefix;
}

fn inline_value_spelling(
    option: &str,
    value: &str,
    delimiter: Option<char>,
    prefix: &str,
) -> String {
    if option.starts_with("--") {
        let assignment = if prefix.starts_with(&format!("{option}=")) {
            '='
        } else {
            delimiter.unwrap_or('=')
        };
        format!("{option}{assignment}{value}")
    } else {
        format!("{option}{value}")
    }
}

fn option_allowed(option: &OptionDef, seen: &BTreeSet<String>) -> bool {
    if !option.repeatable && seen.contains(option.name()) {
        return false;
    }
    if option
        .conflicts
        .iter()
        .any(|conflict| seen.contains(conflict))
    {
        return false;
    }
    option
        .requires
        .iter()
        .all(|required| seen.contains(required))
}

fn find_option<'a>(
    options: &'a [OptionDef],
    token: &str,
    insensitive: bool,
) -> Option<&'a OptionDef> {
    options.iter().find(|option| {
        option.matches_name(token, insensitive)
            || option.matches_inline(token, insensitive).is_some()
    })
}

fn find_short_cluster<'a>(
    options: &'a [OptionDef],
    token: &str,
) -> Option<(Vec<&'a OptionDef>, bool)> {
    if token.len() <= 2 || !token.starts_with('-') || token.starts_with("--") {
        return None;
    }
    let mut found = Vec::new();
    let chars = token[1..].char_indices();
    for (offset, character) in chars {
        let name = format!("-{character}");
        let option = options.iter().find(|option| {
            (option.short_cluster || (option.name().len() == 2 && option.name().starts_with('-')))
                && option.names.iter().any(|candidate| candidate == &name)
        })?;
        found.push(option);
        if option.value != ValueKind::None {
            let end = 1 + offset + character.len_utf8();
            return Some((found, token.len() > end));
        }
    }
    Some((found, false))
}

fn parse_value_kind(value: &str) -> Option<ValueKind> {
    match value {
        "none" => Some(ValueKind::None),
        // `value` was used by the first user-catalog examples; retain it as
        // a documented spelling while normalizing internally to Text.
        "text" | "value" => Some(ValueKind::Text),
        "path" => Some(ValueKind::Path),
        "directory" => Some(ValueKind::Directory),
        _ => None,
    }
}

fn parse_delimiter(value: &str) -> Result<Option<char>, String> {
    let mut chars = value.chars();
    let Some(character) = chars.next() else {
        return Err("value_delimiter 不能为空".to_owned());
    };
    if chars.next().is_some() {
        return Err("value_delimiter 必须只有一个字符".to_owned());
    }
    Ok(Some(character))
}

fn ensure_provider(provider: &str) -> Result<(), String> {
    if ALLOWED_PROVIDERS.contains(&provider) {
        Ok(())
    } else {
        Err(format!("未知 provider {provider}；只能使用固定只读数据源"))
    }
}

fn effective_provider(provider: Option<String>, root: &str) -> Option<String> {
    // npm option sets are intentionally shared by pnpm because their common
    // flags have the same grammar. Keep the provider family aligned with the
    // executable so the runtime dispatches to the pnpm reader for a pnpm
    // command without duplicating the entire option catalog.
    if root.eq_ignore_ascii_case("pnpm")
        && let Some(provider) = provider.as_deref()
        && let Some(scope) = provider.strip_prefix("npm.")
    {
        return Some(format!("pnpm.{scope}"));
    }
    provider
}

fn normalize_children(parent: &str, children: &[String]) -> Vec<String> {
    children
        .iter()
        .map(|child| {
            if child.contains(' ') {
                child.clone()
            } else {
                format!("{parent} {child}")
            }
        })
        .collect()
}

fn normalize_alias(value: &str) -> String {
    value.trim().trim_matches(['\'', '"']).to_ascii_lowercase()
}

fn normalize_command(value: &str) -> String {
    let trimmed = value.trim().trim_matches(['\'', '"']);
    let basename = trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed);
    let lower = basename.to_ascii_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".com"] {
        if let Some(stem) = lower.strip_suffix(suffix) {
            return stem.to_owned();
        }
    }
    lower
}

fn token_eq(left: &str, right: &str, insensitive: bool) -> bool {
    if insensitive {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn starts_with(candidate: &str, prefix: &str, insensitive: bool) -> bool {
    candidate
        .get(..prefix.len())
        .is_some_and(|head| token_eq(head, prefix, insensitive))
}

fn root_is_insensitive(root: &str) -> bool {
    matches!(root, "pwsh" | "Set-Location" | "Get-ChildItem")
}

struct Parsed {
    path: Vec<String>,
    node: Node,
    pending: Option<OptionDef>,
    options_ended: bool,
    allow_subcommands: bool,
    seen_options: BTreeSet<String>,
}

//! Offline, bounded project-aware completion providers.
//!
//! The providers in this module deliberately do not call a package manager,
//! Cargo, or a Git command which can update a repository.  They inspect local
//! manifests and use a small fixed set of read-only Git queries.  The host is
//! responsible for scheduling at most two [`collect`] calls at a time; each
//! call accepts a cancellation flag so a stale request can stop at a safe
//! boundary.

use crate::model::CandidateKind;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(not(windows))]
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::{io::AsRawHandle, process::CommandExt};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{ERROR_BROKEN_PIPE, GetLastError};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::PeekNamedPipe;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

const GIT_DEADLINE: Duration = Duration::from_secs(1);
const GIT_OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const MANIFEST_LIMIT: u64 = 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 256;

/// A project completion request captured by the host.
///
/// `revision` is an opaque request identifier and intentionally does not
/// participate in [`snapshot_key`].  It lets the host discard stale results
/// without making otherwise identical project snapshots miss the cache.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectQuery {
    pub revision: u64,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub prefix: String,
    pub cwd: PathBuf,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Optional fixed provider identifier from user configuration.  Accepted
    /// values are `git`, `cargo`, `npm`, and `pnpm`.
    #[serde(default)]
    pub provider: Option<String>,
}

impl ProjectQuery {
    pub fn new(
        revision: u64,
        command: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
        prefix: impl Into<String>,
        cwd: impl Into<PathBuf>,
        environment: BTreeMap<String, String>,
    ) -> Self {
        Self {
            revision,
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
            prefix: prefix.into(),
            cwd: cwd.into(),
            environment,
            provider: None,
        }
    }
}

/// The dynamic provider selected from a command name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Git,
    Cargo,
    Npm,
    Pnpm,
    Python,
    Conda,
    Poetry,
    Yarn,
    Bun,
    Rustup,
    Go,
    Dotnet,
    Cmake,
    Docker,
    Kubectl,
    Helm,
    Ssh,
    DevTools,
    WindowsTools,
    CloudData,
}

/// Select a project provider for an executable name or path.
pub fn provider_for_command(command: &str) -> Option<ProviderKind> {
    let name = executable_stem(command);
    match name.as_str() {
        "git" => Some(ProviderKind::Git),
        "cargo" => Some(ProviderKind::Cargo),
        "npm" => Some(ProviderKind::Npm),
        "pnpm" => Some(ProviderKind::Pnpm),
        "python" | "python3" | "pip" | "pip3" | "uv" => Some(ProviderKind::Python),
        "conda" | "mamba" | "micromamba" => Some(ProviderKind::Conda),
        "poetry" => Some(ProviderKind::Poetry),
        "yarn" => Some(ProviderKind::Yarn),
        "bun" => Some(ProviderKind::Bun),
        "rustup" => Some(ProviderKind::Rustup),
        "go" => Some(ProviderKind::Go),
        "dotnet" => Some(ProviderKind::Dotnet),
        "cmake" | "ctest" => Some(ProviderKind::Cmake),
        "docker" | "docker-compose" => Some(ProviderKind::Docker),
        "kubectl" => Some(ProviderKind::Kubectl),
        "helm" => Some(ProviderKind::Helm),
        "ssh" | "scp" | "sftp" => Some(ProviderKind::Ssh),
        "deno" | "mvn" | "gradle" | "make" | "gmake" | "ninja" | "just" | "task"
        | "terraform" | "tofu" | "ansible" | "ansible-playbook" => {
            Some(ProviderKind::DevTools)
        }
        "scoop" | "choco" | "wsl" | "taskkill" | "sc" | "get-module"
        | "import-module" | "remove-module" => Some(ProviderKind::WindowsTools),
        "aws" | "az" | "gcloud" | "psql" | "mysql" | "sqlite3" | "redis-cli"
        | "mongosh" => Some(ProviderKind::CloudData),
        _ => None,
    }
}

/// A candidate returned by a project provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCandidate {
    pub value: String,
    #[serde(default)]
    pub description: String,
    pub kind: CandidateKind,
    pub source: String,
}

/// The result of one project-provider request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderResult {
    pub candidates: Vec<ProviderCandidate>,
    /// `true` means the provider did not have a complete snapshot.  Such a
    /// result must not be persisted in [`ProviderCache`].
    #[serde(default)]
    pub incomplete: bool,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    /// Best-effort project root used by local ranking and cache invalidation.
    #[serde(default)]
    pub project_root: Option<PathBuf>,
    /// Directories read while collecting this snapshot.  The host may watch
    /// these non-recursively so a manifest/target edit invalidates precisely
    /// the project cache which supplied the candidates.
    #[serde(default)]
    pub watch_paths: Vec<PathBuf>,
    /// Work-tree or other directories which must be watched recursively for
    /// this snapshot to stay current.  This is deliberately separate from
    /// `watch_paths`: only providers which need nested filesystem changes
    /// (currently Git status paths) should request recursive watching.
    #[serde(default)]
    pub recursive_watch_paths: Vec<PathBuf>,
}

impl ProviderResult {
    fn push(&mut self, candidate: ProviderCandidate) {
        // Dynamic snapshots are complete only after the provider has walked
        // all applicable local data.  Keep the display limit at the host
        // boundary; truncating here would make the result impossible to
        // cache as a complete project snapshot.
        self.candidates.push(candidate);
    }

    fn finish(mut self) -> Self {
        self.candidates.sort_by(|left, right| {
            left.value
                .cmp(&right.value)
                .then_with(|| left.source.cmp(&right.source))
        });
        self.candidates
            .dedup_by(|left, right| left.value == right.value && left.kind == right.kind);
        self.watch_paths.sort();
        self.watch_paths.dedup();
        self.recursive_watch_paths.sort();
        self.recursive_watch_paths.dedup();
        self
    }
}

/// A stable cache key for a query.  It contains the effective request context
/// but omits `revision`, which is only a stale-result guard.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SnapshotKey {
    pub command: String,
    pub args: Vec<String>,
    pub prefix: String,
    pub cwd: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub provider: Option<String>,
}

/// A project-context key which intentionally excludes the current prefix.
/// Use this key for expensive project snapshot reuse while the user edits a
/// token; use [`SnapshotKey`] when the filtered result itself is cached.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProjectKey {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub provider: Option<String>,
    /// The already-typed portion of a delimited value (for example `fast,`).
    /// The active fragment is intentionally omitted so a snapshot survives
    /// ordinary edits to the current feature/package name.
    pub selector_prefix: String,
}

pub fn snapshot_key(query: &ProjectQuery) -> SnapshotKey {
    SnapshotKey {
        command: query.command.clone(),
        args: normalized_args(query),
        prefix: query.prefix.clone(),
        cwd: query.cwd.clone(),
        environment: query.environment.clone(),
        provider: query.provider.clone(),
    }
}

pub fn project_key(query: &ProjectQuery) -> ProjectKey {
    ProjectKey {
        command: query.command.clone(),
        args: normalized_args(query),
        cwd: query.cwd.clone(),
        environment: query.environment.clone(),
        provider: query.provider.clone(),
        selector_prefix: query_value_prefix(query),
    }
}

/// Return the stable value prefix preceding the fragment currently being
/// typed.  For Cargo features this is the comma-delimited portion, without an
/// attached `--features=` spelling.  The host can prepend this string to a
/// raw provider candidate when it builds the replacement text.
pub fn query_value_prefix(query: &ProjectQuery) -> String {
    let provider_scope = query.provider.as_deref().and_then(|provider| {
        provider.split_once('.').and_then(|(family, scope)| {
            (family.eq_ignore_ascii_case("cargo") && scope.eq_ignore_ascii_case("features"))
                .then_some(())
        })
    });
    let (_, active) = cargo_context(&query.args);
    let feature_context = provider_scope.is_some()
        || active == Some(CargoValueKind::Features)
        || query.prefix.starts_with("--features=")
        || query.prefix.starts_with("--feature=");
    if !feature_context {
        return String::new();
    }
    let value = query
        .prefix
        .strip_prefix("--features=")
        .or_else(|| query.prefix.strip_prefix("--feature="))
        .unwrap_or(&query.prefix);
    value
        .rfind(',')
        .map(|index| value[..index + 1].to_owned())
        .unwrap_or_default()
}

/// Normalize a query for a reusable project snapshot.  The active typed
/// fragment is cleared; the value slot and its preceding CSV prefix remain
/// represented by [`query_value_prefix`] and [`project_key`].
pub fn normalize_query(query: &ProjectQuery) -> ProjectQuery {
    let mut normalized = query.clone();
    normalized.args = normalized_args(query);
    normalized.prefix.clear();
    normalized
}

/// Return arguments with the active inline value option represented as a
/// separate trailing option.  The PowerShell adapter supplies the current
/// token in `prefix`, so an inline `--features=...`/`--package=...` token is
/// absent from `args`.  Adding its selector here makes cache keys and
/// snapshots distinguish the value slot even when the provider override is
/// missing.  The function is idempotent and never copies the active value.
pub fn normalized_args(query: &ProjectQuery) -> Vec<String> {
    let mut args = query.args.clone();
    let selector = cargo_selector_option(query);
    if let Some(selector) = selector {
        let already_trailing = args
            .last()
            .map(|argument| argument.trim_matches(['"', '\'']))
            .is_some_and(|argument| argument == selector);
        if !already_trailing {
            args.push(selector.to_owned());
        }
    }
    args
}

fn cargo_selector_option(query: &ProjectQuery) -> Option<&'static str> {
    let provider_selector = query.provider.as_deref().and_then(|provider| {
        let (family, scope) = provider.split_once('.')?;
        if !family.eq_ignore_ascii_case("cargo") {
            return None;
        }
        match scope.to_ascii_lowercase().as_str() {
            "packages" => Some("--package"),
            "features" => Some("--features"),
            "bins" => Some("--bin"),
            "examples" => Some("--example"),
            "tests" => Some("--test"),
            "benches" => Some("--bench"),
            _ => None,
        }
    });
    if provider_selector.is_some() {
        return provider_selector;
    }
    if query.prefix.starts_with("--features=") || query.prefix.starts_with("--feature=") {
        return Some("--features");
    }
    if query.prefix.starts_with("--package=") || query.prefix.starts_with("-p=") {
        return Some("--package");
    }
    if query.prefix.starts_with("--bin=") {
        return Some("--bin");
    }
    if query.prefix.starts_with("--example=") {
        return Some("--example");
    }
    if query.prefix.starts_with("--test=") {
        return Some("--test");
    }
    if query.prefix.starts_with("--bench=") {
        return Some("--bench");
    }
    cargo_context(&query.args).1.map(|kind| match kind {
        CargoValueKind::Package => "--package",
        CargoValueKind::Features => "--features",
        CargoValueKind::Bin => "--bin",
        CargoValueKind::Example => "--example",
        CargoValueKind::Test => "--test",
        CargoValueKind::Bench => "--bench",
    })
}

/// A deterministic, process-independent fingerprint for a project snapshot.
///
/// This is useful for tracing and for lightweight host maps.  Cache lookups
/// should use [`SnapshotKey`] or [`ProviderCache`] so a theoretical hash
/// collision cannot return another project's candidates.
pub fn snapshot_fingerprint(query: &ProjectQuery) -> u64 {
    let key = snapshot_key(query);
    let mut hash = 0xcbf29ce484222325u64;
    fn add(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(0x100000001b3);
        }
        *hash ^= 0xff;
        *hash = hash.wrapping_mul(0x100000001b3);
    }
    add(&mut hash, key.command.as_bytes());
    for argument in &key.args {
        add(&mut hash, argument.as_bytes());
    }
    add(&mut hash, key.prefix.as_bytes());
    add(&mut hash, key.cwd.to_string_lossy().as_bytes());
    for (name, value) in &key.environment {
        add(&mut hash, name.as_bytes());
        add(&mut hash, value.as_bytes());
    }
    if let Some(provider) = key.provider {
        add(&mut hash, provider.as_bytes());
    }
    hash
}

/// Fingerprint the expensive project context while omitting the active typed
/// fragment.  `query_value_prefix` remains part of the key so a different
/// already-selected CSV value cannot be confused with this one.
pub fn project_fingerprint(query: &ProjectQuery) -> u64 {
    let key = project_key(query);
    let mut hash = 0xcbf29ce484222325u64;
    fn add(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(0x100000001b3);
        }
        *hash ^= 0xff;
        *hash = hash.wrapping_mul(0x100000001b3);
    }
    add(&mut hash, key.command.as_bytes());
    for argument in &key.args {
        add(&mut hash, argument.as_bytes());
    }
    add(&mut hash, key.cwd.to_string_lossy().as_bytes());
    for (name, value) in &key.environment {
        add(&mut hash, name.as_bytes());
        add(&mut hash, value.as_bytes());
    }
    if let Some(provider) = key.provider {
        add(&mut hash, provider.as_bytes());
    }
    add(&mut hash, key.selector_prefix.as_bytes());
    hash
}

/// A bounded in-memory cache of complete provider snapshots.
#[derive(Debug)]
pub struct ProviderCache {
    entries: HashMap<SnapshotKey, ProviderResult>,
    project_entries: HashMap<ProjectKey, ProviderResult>,
    capacity: usize,
}

impl Default for ProviderCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            project_entries: HashMap::new(),
            capacity: MAX_CACHE_ENTRIES,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            project_entries: HashMap::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len() + self.project_entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.project_entries.is_empty()
    }

    /// Return a complete cached result, if present.
    pub fn get(&self, query: &ProjectQuery) -> Option<ProviderResult> {
        self.entries.get(&snapshot_key(query)).cloned()
    }

    pub fn lookup(&self, query: &ProjectQuery) -> Option<ProviderResult> {
        self.get(query)
    }

    /// Return an unfiltered snapshot keyed without the current token prefix.
    pub fn get_project(&self, query: &ProjectQuery) -> Option<ProviderResult> {
        self.project_entries.get(&project_key(query)).cloned()
    }

    pub fn lookup_project(&self, query: &ProjectQuery) -> Option<ProviderResult> {
        self.get_project(query)
    }

    /// Insert a result only if it is complete.  Returns whether it was stored.
    pub fn insert(&mut self, query: &ProjectQuery, result: ProviderResult) -> bool {
        if result.incomplete {
            return false;
        }
        if self.entries.len() >= self.capacity
            && !self.entries.contains_key(&snapshot_key(query))
            && let Some(oldest) = self.entries.keys().next().cloned()
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(snapshot_key(query), result);
        true
    }

    /// Insert an unfiltered project snapshot.  Incomplete results are never
    /// retained because they could otherwise hide newly available branches,
    /// package members, or scripts on a later request.
    pub fn insert_project(&mut self, query: &ProjectQuery, result: ProviderResult) -> bool {
        if result.incomplete {
            return false;
        }
        let key = project_key(query);
        if self.project_entries.len() >= self.capacity
            && !self.project_entries.contains_key(&key)
            && let Some(oldest) = self.project_entries.keys().next().cloned()
        {
            self.project_entries.remove(&oldest);
        }
        self.project_entries.insert(key, result);
        true
    }

    pub fn invalidate_all(&mut self) {
        self.entries.clear();
        self.project_entries.clear();
    }

    pub fn clear(&mut self) {
        self.invalidate_all();
    }

    /// Invalidate snapshots whose working directory is the path or a child of
    /// it.  The reverse relation is included so a project-root update also
    /// invalidates a cache populated from a parent directory.
    pub fn invalidate_path(&mut self, path: &Path) {
        let path = lexical_normalize(path);
        let affected = |cwd: &Path, result: &ProviderResult| {
            let cwd = lexical_normalize(cwd);
            cwd.starts_with(&path)
                || path.starts_with(&cwd)
                || result.project_root.as_ref().is_some_and(|root| {
                    let root = lexical_normalize(root);
                    root.starts_with(&path) || path.starts_with(&root)
                })
                || result.watch_paths.iter().any(|watch| {
                    let watch = lexical_normalize(watch);
                    watch.starts_with(&path) || path.starts_with(&watch)
                })
                || result.recursive_watch_paths.iter().any(|watch| {
                    let watch = lexical_normalize(watch);
                    watch.starts_with(&path) || path.starts_with(&watch)
                })
        };
        self.entries
            .retain(|key, result| !affected(&key.cwd, result));
        self.project_entries
            .retain(|key, result| !affected(&key.cwd, result));
    }

    pub fn invalidate_cwd(&mut self, cwd: &Path) {
        self.invalidate_path(cwd);
    }

    pub fn invalidate_command(&mut self, command: &str) {
        let command = executable_stem(command);
        self.entries
            .retain(|key, _| executable_stem(&key.command) != command);
        self.project_entries
            .retain(|key, _| executable_stem(&key.command) != command);
    }
}

/// An optional synchronized cache for callers which share a cache between
/// provider worker threads.  The host can also keep [`ProviderCache`] local to
/// its scheduler when it already owns synchronization.
pub type SharedProviderCache = Arc<Mutex<ProviderCache>>;

/// Collect project-aware candidates synchronously and cooperatively.
///
/// Returned values are filtered with a case-insensitive prefix match.  The
/// host remains responsible for calculating replacement ranges and quoting;
/// no candidate contains shell syntax beyond the value read from the project.
pub fn collect(query: &ProjectQuery, cancelled: &AtomicBool) -> ProviderResult {
    if let Some(provider) = query.provider.as_deref()
        && !valid_provider_id(provider)
    {
        return ProviderResult {
            incomplete: true,
            diagnostics: vec![format!("未知项目补全数据源：{provider}")],
            ..ProviderResult::default()
        };
    }
    let Some(provider) = selected_provider(query) else {
        return ProviderResult::default();
    };
    match provider {
        ProviderKind::Git => collect_git(query, cancelled),
        ProviderKind::Cargo => collect_cargo(query, cancelled),
        ProviderKind::Npm => collect_node(query, cancelled, false),
        ProviderKind::Pnpm => collect_node(query, cancelled, true),
        ProviderKind::Yarn | ProviderKind::Bun => collect_node(query, cancelled, false),
        ProviderKind::Python => collect_python(query, cancelled),
        ProviderKind::Conda => collect_conda(query, cancelled),
        ProviderKind::Poetry => collect_python(query, cancelled),
        ProviderKind::Rustup => collect_rustup(query, cancelled),
        ProviderKind::Go => collect_go(query, cancelled),
        ProviderKind::Dotnet => collect_dotnet(query, cancelled),
        ProviderKind::Cmake => collect_cmake(query, cancelled),
        ProviderKind::Docker => collect_docker(query, cancelled),
        ProviderKind::Kubectl => collect_kubeconfig(query, cancelled),
        ProviderKind::Helm => collect_helm(query, cancelled),
        ProviderKind::Ssh => collect_ssh(query, cancelled),
        ProviderKind::DevTools => collect_dev_tools(query, cancelled),
        ProviderKind::WindowsTools => collect_windows_tools(query, cancelled),
        ProviderKind::CloudData => collect_cloud_data(query, cancelled),
    }
}

/// Collect an unfiltered project snapshot.  This is the preferred input for
/// project-level caching: callers can reuse it for many changing prefixes and
/// perform shell-specific filtering at the host boundary.
pub fn collect_snapshot(query: &ProjectQuery, cancelled: &AtomicBool) -> ProviderResult {
    let snapshot_query = normalize_query(query);
    collect(&snapshot_query, cancelled)
}

/// Alias kept for hosts which name the operation after its project scope.
pub fn collect_project(query: &ProjectQuery, cancelled: &AtomicBool) -> ProviderResult {
    collect(query, cancelled)
}

fn selected_provider(query: &ProjectQuery) -> Option<ProviderKind> {
    if let Some(provider) = query.provider.as_deref() {
        let lower = provider.to_ascii_lowercase();
        if lower == "powershell.modules" || lower.starts_with("windows.") {
            return Some(ProviderKind::WindowsTools);
        }
        if lower.starts_with("cloud.") || lower.starts_with("db.") {
            return Some(ProviderKind::CloudData);
        }
        let family = lower
            .split_once('.')
            .map(|(family, _)| family)
            .unwrap_or(lower.as_str());
        return match family {
            "git" => Some(ProviderKind::Git),
            "cargo" => Some(ProviderKind::Cargo),
            "npm" => Some(ProviderKind::Npm),
            "pnpm" => Some(ProviderKind::Pnpm),
            "python" => Some(ProviderKind::Python),
            "conda" => Some(ProviderKind::Conda),
            "poetry" => Some(ProviderKind::Poetry),
            "yarn" => Some(ProviderKind::Yarn),
            "bun" => Some(ProviderKind::Bun),
            "rustup" => Some(ProviderKind::Rustup),
            "go" => Some(ProviderKind::Go),
            "dotnet" => Some(ProviderKind::Dotnet),
            "cmake" => Some(ProviderKind::Cmake),
            "docker" => Some(ProviderKind::Docker),
            "kubectl" => Some(ProviderKind::Kubectl),
            "helm" => Some(ProviderKind::Helm),
            "ssh" => Some(ProviderKind::Ssh),
            "deno" | "maven" | "gradle" | "make" | "ninja" | "just" | "task"
            | "terraform" | "tofu" | "ansible" => Some(ProviderKind::DevTools),
            _ => None,
        };
    }
    provider_for_command(&query.command)
}

fn valid_provider_id(provider: &str) -> bool {
    crate::tool_registry::provider_known(provider)
}

fn provider_scope<'a>(query: &'a ProjectQuery, family: &str) -> Option<&'a str> {
    let provider = query.provider.as_deref()?;
    let (scope_family, scope) = provider.split_once('.')?;
    scope_family.eq_ignore_ascii_case(family).then_some(scope)
}

fn cancelled(cancelled: &AtomicBool) -> bool {
    cancelled.load(Ordering::Acquire)
}

fn executable_stem(command: &str) -> String {
    let trimmed = command.trim().trim_matches('"').trim_matches('\'');
    let file = trimmed
        .rsplit(['\\', '/'])
        .next()
        .filter(|file| !file.is_empty())
        .unwrap_or(trimmed);
    let lower = file.to_ascii_lowercase();
    for extension in [".exe", ".cmd", ".bat", ".com"] {
        if let Some(stem) = lower.strip_suffix(extension) {
            return stem.to_owned();
        }
    }
    lower
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                output.pop();
            }
            other => output.push(other.as_os_str()),
        }
    }
    output
}

fn lower(value: &str) -> String {
    value.to_lowercase()
}

fn prefix_matches(value: &str, prefix: &str) -> bool {
    lower(value).starts_with(&lower(prefix))
}

fn add_filtered(
    result: &mut ProviderResult,
    value: impl Into<String>,
    description: impl Into<String>,
    kind: CandidateKind,
    source: impl Into<String>,
    prefix: &str,
) {
    let value = value.into();
    if prefix_matches(&value, prefix) {
        result.push(ProviderCandidate {
            value,
            description: description.into(),
            kind,
            source: source.into(),
        });
    }
}

fn add_watch_path(result: &mut ProviderResult, path: &Path) {
    if path.as_os_str().is_empty() {
        return;
    }
    let path = lexical_normalize(path);
    if !result.watch_paths.contains(&path) {
        result.watch_paths.push(path);
    }
}

fn add_recursive_watch_path(result: &mut ProviderResult, path: &Path) {
    if path.as_os_str().is_empty() {
        return;
    }
    let path = lexical_normalize(path);
    if !result.recursive_watch_paths.contains(&path) {
        result.recursive_watch_paths.push(path);
    }
}

fn record_watch_path(paths: &mut Vec<PathBuf>, path: &Path) {
    if path.as_os_str().is_empty() {
        return;
    }
    let path = lexical_normalize(path);
    if !paths.contains(&path) {
        paths.push(path);
    }
}

/// Record the nearest existing directory for a path which may not exist yet.
/// Workspace globs and optional target directories need this parent watched
/// so that creating the missing child invalidates a cached project snapshot.
fn record_existing_watch_ancestor(paths: &mut Vec<PathBuf>, path: &Path) {
    if path.as_os_str().is_empty() {
        return;
    }
    let mut current = path.to_path_buf();
    loop {
        if current.is_dir() {
            record_watch_path(paths, &current);
            return;
        }
        if !current.pop() {
            return;
        }
    }
}

fn read_limited(path: &Path, limit: u64) -> io::Result<String> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("文件超过 {} 字节", limit),
        ));
    }
    let file = File::open(path)?;
    let mut bytes = Vec::with_capacity(metadata.len().min(limit) as usize);
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("文件超过 {} 字节", limit),
        ));
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn find_upwards(start: &Path, file_name: &str) -> Option<PathBuf> {
    let mut current = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        let candidate = current.join(file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn resolve_path(base: &Path, value: &str) -> PathBuf {
    let value = value.trim_matches('"').trim_matches('\'');
    let path = PathBuf::from(value);
    if path.is_absolute() {
        lexical_normalize(&path)
    } else {
        lexical_normalize(&base.join(path))
    }
}

fn split_command_args(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.trim_matches('"').trim_matches('\'').to_owned())
        .collect()
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.replace('\\', "/");
    let value = value.replace('\\', "/");
    fn matches(pattern: &[u8], value: &[u8]) -> bool {
        if pattern.is_empty() {
            return value.is_empty();
        }
        match pattern[0] {
            b'*' => {
                matches(&pattern[1..], value)
                    || (!value.is_empty() && matches(pattern, &value[1..]))
            }
            b'?' => !value.is_empty() && matches(&pattern[1..], &value[1..]),
            byte => value.first().copied() == Some(byte) && matches(&pattern[1..], &value[1..]),
        }
    }
    matches(pattern.as_bytes(), value.as_bytes())
}

fn has_wildcard(value: &str) -> bool {
    value.contains('*') || value.contains('?')
}

const MAX_MISSING_PROJECT_WATCH_ANCESTORS: usize = 16;

/// Keep a no-manifest snapshot connected to the directories in which a
/// project manifest may subsequently appear. The live watcher subscribes to
/// these paths non-recursively, so a manifest created in a parent directory
/// can invalidate an otherwise empty cached result without scanning that
/// parent tree.
fn record_missing_project_watches(paths: &mut Vec<PathBuf>, start: &Path) {
    let mut current = if start.is_dir() {
        start.to_path_buf()
    } else {
        start
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| start.to_path_buf())
    };
    for _ in 0..MAX_MISSING_PROJECT_WATCH_ANCESTORS {
        if current.as_os_str().is_empty() {
            break;
        }
        record_watch_path(paths, &current);
        if !current.pop() {
            break;
        }
    }
}

fn ignored_project_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                ".git" | ".blueberry" | "node_modules" | "target"
            )
        })
}

fn expand_project_pattern(
    base: &Path,
    pattern: &str,
    manifest_name: &str,
    cancelled_flag: Option<&AtomicBool>,
    watch_paths: &mut Vec<PathBuf>,
) -> (Vec<PathBuf>, bool) {
    let raw_pattern = pattern.trim().trim_matches('"').trim_matches('\'');
    let path = Path::new(raw_pattern);
    record_existing_watch_ancestor(watch_paths, base);
    let (mut paths, parts) = if path.is_absolute() {
        let mut anchor = PathBuf::new();
        let mut wildcard_seen = false;
        let mut parts = Vec::new();
        for component in path.components() {
            let value = component.as_os_str().to_string_lossy().into_owned();
            if !wildcard_seen && !has_wildcard(&value) {
                anchor.push(component.as_os_str());
            } else {
                wildcard_seen = true;
                parts.push(value);
            }
        }
        (vec![anchor], parts)
    } else {
        (
            vec![base.to_path_buf()],
            raw_pattern
                .replace('\\', "/")
                .split('/')
                .filter(|part| !part.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
        )
    };
    let mut incomplete = false;
    for part_index in 0..parts.len() {
        let part = &parts[part_index];
        if cancelled_flag.is_some_and(cancelled) {
            return (Vec::new(), true);
        }
        let mut next = Vec::new();
        if part == "**" {
            let mut queue = paths.clone();
            let mut visited = HashSet::new();
            let mut scanned_directories = 0usize;
            while let Some(directory) = queue.pop() {
                if cancelled_flag.is_some_and(cancelled) {
                    return (Vec::new(), true);
                }
                if !visited.insert(directory.clone()) {
                    continue;
                }
                scanned_directories = scanned_directories.saturating_add(1);
                if scanned_directories.is_multiple_of(64) {
                    // Keep cancellation responsive for large monorepos while
                    // leaving the filesystem call itself synchronous. A
                    // timeout around read_dir would report fake completeness.
                    if cancelled_flag.is_some_and(cancelled) {
                        return (Vec::new(), true);
                    }
                    std::thread::yield_now();
                }
                record_existing_watch_ancestor(watch_paths, &directory);
                next.push(directory.clone());
                let entries = match fs::read_dir(&directory) {
                    Ok(entries) => entries,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(_) => {
                        incomplete = true;
                        continue;
                    }
                };
                for entry in entries {
                    let Ok(entry) = entry else {
                        incomplete = true;
                        continue;
                    };
                    if cancelled_flag.is_some_and(cancelled) {
                        return (Vec::new(), true);
                    }
                    let path = entry.path();
                    let explicitly_requested = parts[part_index + 1..].iter().any(|next| {
                        !has_wildcard(next)
                            && path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .is_some_and(|name| name.eq_ignore_ascii_case(next))
                    });
                    if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
                        && (!ignored_project_directory(&path) || explicitly_requested)
                    {
                        queue.push(path);
                    }
                }
            }
        } else if has_wildcard(part) {
            for directory in &paths {
                record_existing_watch_ancestor(watch_paths, directory);
                let entries = match fs::read_dir(directory) {
                    Ok(entries) => entries,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(_) => {
                        incomplete = true;
                        continue;
                    }
                };
                for entry in entries {
                    let Ok(entry) = entry else {
                        incomplete = true;
                        continue;
                    };
                    if cancelled_flag.is_some_and(cancelled) {
                        return (Vec::new(), true);
                    }
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if wildcard_match(part, &name) {
                        next.push(entry.path());
                    }
                }
            }
        } else {
            for directory in paths {
                let candidate = directory.join(part);
                record_existing_watch_ancestor(watch_paths, &candidate);
                next.push(candidate);
            }
        }
        paths = next;
    }
    paths = paths
        .into_iter()
        .filter_map(|path| {
            if path.is_file() {
                (path.file_name().and_then(|name| name.to_str()) == Some(manifest_name))
                    .then(|| path.parent().unwrap_or(path.as_path()).to_path_buf())
            } else if path.join(manifest_name).is_file() {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    (paths, incomplete)
}

// ---- Git -----------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct GitContext {
    cwd: PathBuf,
    git_dir: Option<PathBuf>,
    work_tree: Option<PathBuf>,
    subcommand: String,
    rest: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct GitRefs {
    local_branches: Vec<String>,
    remote_branches: Vec<String>,
    tags: Vec<String>,
    remotes: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct GitNeeds {
    refs: bool,
    branches: bool,
    remote_branches: bool,
    tags: bool,
    remotes: bool,
    worktrees: bool,
    status_paths: bool,
}

struct FixedOutput {
    status: Option<ExitStatus>,
    stdout: Vec<u8>,
    truncated: bool,
    read_error: bool,
    cancelled: bool,
    timed_out: bool,
    spawn_error: Option<String>,
}

impl FixedOutput {
    fn failed(&self) -> bool {
        self.status.map(|status| !status.success()).unwrap_or(true)
    }

    fn interrupted(&self) -> bool {
        self.cancelled
            || self.timed_out
            || self.truncated
            || self.read_error
            || self.spawn_error.is_some()
    }
}

fn parse_git_context(query: &ProjectQuery) -> GitContext {
    let args = split_command_args(&query.args);
    let mut cwd = if query.cwd.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        query.cwd.clone()
    };
    let mut git_dir = None;
    let mut work_tree = None;
    let mut subcommand_index = None;
    let mut index = 0usize;
    while index < args.len() {
        let token = &args[index];
        if token == "--" {
            break;
        }
        if token == "-C" {
            if let Some(value) = args.get(index + 1) {
                cwd = resolve_path(&cwd, value);
                index += 2;
                continue;
            }
            break;
        }
        if let Some(value) = token.strip_prefix("-C").filter(|value| !value.is_empty()) {
            cwd = resolve_path(&cwd, value);
            index += 1;
            continue;
        }
        if let Some(value) = git_directory_value(&args, &mut index, "--git-dir") {
            git_dir = Some(resolve_path(&cwd, &value));
            continue;
        }
        if let Some(value) = git_directory_value(&args, &mut index, "--work-tree") {
            work_tree = Some(resolve_path(&cwd, &value));
            continue;
        }
        if token == "-c" || token == "--config-env" {
            index = (index + 2).min(args.len());
            continue;
        }
        if token == "--exec-path" || token == "--namespace" || token == "--super-prefix" {
            index = (index + 2).min(args.len());
            continue;
        }
        if token.starts_with('-') {
            index += 1;
            continue;
        }
        subcommand_index = Some(index);
        break;
    }
    let subcommand_index = subcommand_index.unwrap_or(args.len());
    GitContext {
        cwd,
        git_dir,
        work_tree,
        subcommand: args.get(subcommand_index).cloned().unwrap_or_default(),
        rest: args
            .get(subcommand_index.saturating_add(1)..)
            .unwrap_or_default()
            .to_vec(),
    }
}

fn git_directory_value(args: &[String], index: &mut usize, option: &str) -> Option<String> {
    let token = args.get(*index)?;
    if token == option {
        let value = args.get(*index + 1)?.clone();
        *index += 2;
        Some(value)
    } else if let Some(value) = token.strip_prefix(&format!("{option}=")) {
        *index += 1;
        Some(value.to_owned())
    } else {
        None
    }
}

fn git_command_requires_value(token: &str) -> bool {
    let option = token.split_once('=').map(|(name, _)| name).unwrap_or(token);
    matches!(
        option,
        "-C" | "--git-dir"
            | "--work-tree"
            | "-c"
            | "--config-env"
            | "--source"
            | "--pathspec-from-file"
            | "--output"
            | "-o"
            | "--format"
            | "--pretty"
            | "--date"
            | "--author"
            | "--message"
            | "-m"
            | "--file"
            | "-F"
            | "--strategy"
            | "--strategy-option"
            | "-X"
            | "--upload-pack"
            | "--receive-pack"
            | "--exec"
            | "--namespace"
            | "--super-prefix"
            | "--jobs"
            | "-j"
            | "--unified"
            | "-U"
            | "--inter-hunk-context"
            | "--diff-algorithm"
            | "--diff-merges"
            | "--cleanup"
            | "--push-option"
            | "--filter"
            // Command-specific options which consume a following token.  In
            // particular, worktree add's `-b new-branch path` must leave only
            // `path` as a positional so the first dynamic value remains a
            // filesystem path and the ref is offered only afterwards.
            | "-b"
            | "-B"
            | "--orphan"
            | "--reason"
    )
}

fn git_option_requires_value(subcommand: &str, token: &str) -> bool {
    let option = token.split_once('=').map(|(name, _)| name).unwrap_or(token);
    // These short names are global Git options before the subcommand, but
    // branch uses them as flag-only rename/copy switches after `git branch`.
    if subcommand.eq_ignore_ascii_case("branch")
        && matches!(option, "-m" | "-M" | "-c" | "-C" | "--move" | "--copy")
    {
        return false;
    }
    if subcommand.eq_ignore_ascii_case("checkout") && option == "-m" {
        return false;
    }
    git_command_requires_value(token)
}

fn git_positionals(subcommand: &str, rest: &[String]) -> (Vec<String>, bool) {
    let mut positionals = Vec::new();
    let mut after_double_dash = false;
    let mut index = 0usize;
    while index < rest.len() {
        let token = &rest[index];
        if after_double_dash {
            positionals.push(token.clone());
            index += 1;
            continue;
        }
        if token == "--" {
            after_double_dash = true;
            index += 1;
            continue;
        }
        if token.starts_with('-') {
            if git_option_requires_value(subcommand, token) && !token.contains('=') {
                index = (index + 2).min(rest.len());
            } else {
                index += 1;
            }
            continue;
        }
        positionals.push(token.clone());
        index += 1;
    }
    (positionals, after_double_dash)
}

fn git_active_value_option(subcommand: &str, rest: &[String]) -> Option<String> {
    let mut index = 0usize;
    let mut active = None;
    while index < rest.len() {
        let token = &rest[index];
        if token == "--" {
            break;
        }
        if token.starts_with('-') {
            if git_option_requires_value(subcommand, token) {
                if token.contains('=') {
                    active = Some(
                        token
                            .split_once('=')
                            .map(|(name, _)| name)
                            .unwrap_or(token)
                            .to_owned(),
                    );
                    index += 1;
                } else if index + 1 >= rest.len() {
                    active = Some(token.clone());
                    break;
                } else {
                    active = None;
                    index += 2;
                }
            } else {
                index += 1;
            }
        } else {
            active = None;
            index += 1;
        }
    }
    active
}

fn git_needs(ctx: &GitContext, forced_scope: Option<&str>) -> GitNeeds {
    let subcommand = lower(&ctx.subcommand);
    let (positionals, after_double_dash) = git_positionals(&subcommand, &ctx.rest);
    let active = git_active_value_option(&subcommand, &ctx.rest);
    let mut needs = GitNeeds::default();
    if let Some(scope) = forced_scope {
        match scope.to_ascii_lowercase().as_str() {
            "refs" => {
                if matches!(subcommand.as_str(), "checkout" | "reset") && after_double_dash {
                    needs.status_paths = true;
                } else {
                    needs.refs = true;
                    needs.branches = true;
                    needs.remote_branches = true;
                    needs.tags = true;
                }
            }
            "branches" => {
                needs.refs = true;
                needs.branches = true;
                needs.remote_branches = true;
            }
            "remotes" => needs.remotes = true,
            "paths" | "status" => needs.status_paths = true,
            "tags" => {
                needs.refs = true;
                needs.tags = true;
            }
            "worktrees" => {
                if subcommand == "worktree" {
                    let action = positionals
                        .first()
                        .map(|value| lower(value))
                        .unwrap_or_default();
                    if action == "add"
                        && positionals.len().saturating_sub(1) >= 1
                        && !matches!(active.as_deref(), Some("-b" | "-B" | "--orphan"))
                    {
                        // `git worktree add <path> <commit-ish>`: after the
                        // destination path, the dynamic value is a branch,
                        // tag, or other local ref rather than another path.
                        needs.refs = true;
                        needs.branches = true;
                        needs.remote_branches = true;
                        needs.tags = true;
                    } else if action != "add" {
                        needs.worktrees = true;
                    }
                } else {
                    needs.worktrees = true;
                }
            }
            _ => {}
        }
        return needs;
    }
    match subcommand.as_str() {
        "switch" | "checkout" => {
            // Everything after `--` is a pathspec for checkout.  Returning
            // here is important: adding refs as well makes a branch with the
            // same prefix look like a path completion and can insert the
            // wrong value.
            if after_double_dash && subcommand == "checkout" {
                needs.status_paths = true;
                return needs;
            }
            let creating = ctx
                .rest
                .iter()
                .any(|arg| matches!(arg.as_str(), "-b" | "-B" | "-c" | "--orphan"));
            if !creating && active.as_deref() != Some("--pathspec-from-file") {
                needs.refs = true;
                needs.branches = true;
                needs.remote_branches = true;
            }
            if subcommand == "checkout"
                && (after_double_dash || (!positionals.is_empty() && !creating))
            {
                needs.status_paths = true;
            }
        }
        "merge" | "rebase" | "cherry-pick" | "cherry_pick" | "revert" | "show" | "log" => {
            if after_double_dash && matches!(subcommand.as_str(), "show" | "log") {
                needs.status_paths = true;
                return needs;
            }
            if active.as_deref() == Some("--format") || active.as_deref() == Some("--pretty") {
                return needs;
            }
            needs.refs = true;
            needs.branches = true;
            needs.remote_branches = true;
            needs.tags = true;
            if matches!(subcommand.as_str(), "show" | "log")
                && (after_double_dash || !positionals.is_empty())
            {
                needs.status_paths = true;
            }
        }
        "tag" => {
            let deleting = ctx
                .rest
                .iter()
                .any(|arg| matches!(arg.as_str(), "-d" | "-D" | "--delete"));
            let listing = ctx.rest.iter().any(|arg| {
                matches!(arg.as_str(), "-l" | "--list" | "-v" | "--verify")
                    || (arg.starts_with("-n") && !arg.starts_with("--"))
            });
            if deleting || listing {
                needs.refs = true;
                needs.tags = true;
            }
        }
        "branch" => {
            let deleting = ctx
                .rest
                .iter()
                .any(|arg| matches!(arg.as_str(), "-d" | "-D" | "--delete"));
            let moving = ctx
                .rest
                .iter()
                .any(|arg| matches!(arg.as_str(), "-m" | "-M" | "--move"));
            let copying = ctx
                .rest
                .iter()
                .any(|arg| matches!(arg.as_str(), "-c" | "-C" | "--copy"));
            let list_local = ctx
                .rest
                .iter()
                .any(|arg| matches!(arg.as_str(), "-l" | "--list"));
            let list_remote = ctx
                .rest
                .iter()
                .any(|arg| matches!(arg.as_str(), "-r" | "--remotes"));
            let list_all = ctx
                .rest
                .iter()
                .any(|arg| matches!(arg.as_str(), "-a" | "--all"));
            if deleting
                || list_local
                || list_remote
                || list_all
                || (moving && positionals.is_empty())
                || (copying && positionals.is_empty())
            {
                needs.refs = true;
                needs.branches = !list_remote || list_all;
                needs.remote_branches = list_remote || list_all;
            }
        }
        "push" => {
            if positionals.is_empty() {
                needs.remotes = true;
            } else if positionals.len() == 1 {
                needs.refs = true;
                needs.branches = true;
                needs.tags = true;
            }
        }
        "pull" => {
            if positionals.is_empty() {
                needs.remotes = true;
            } else {
                needs.refs = true;
                needs.branches = true;
                needs.remote_branches = true;
                needs.tags = true;
            }
        }
        "fetch" => {
            if positionals.is_empty() {
                needs.remotes = true;
            }
        }
        "remote" => {
            if positionals.iter().any(|arg| {
                matches!(
                    lower(arg).as_str(),
                    "remove" | "rm" | "rename" | "set-head" | "set-url" | "get-url" | "show"
                )
            }) {
                needs.remotes = true;
            }
        }
        "worktree" => {
            let action = positionals
                .first()
                .map(|value| lower(value))
                .unwrap_or_default();
            match action.as_str() {
                "remove" | "rm" | "lock" | "unlock" | "move" | "repair" | "list" => {
                    needs.worktrees = true;
                }
                "add"
                    if positionals.len() >= 2
                        && !matches!(active.as_deref(), Some("-b" | "-B" | "--orphan")) =>
                {
                    needs.refs = true;
                    needs.branches = true;
                    needs.remote_branches = true;
                    needs.tags = true;
                }
                _ => {}
            }
        }
        "restore" => {
            if active.as_deref() == Some("--source") {
                needs.refs = true;
                needs.branches = true;
                needs.remote_branches = true;
                needs.tags = true;
            } else {
                needs.status_paths = true;
            }
        }
        "reset" => {
            if !after_double_dash && positionals.is_empty() {
                needs.refs = true;
                needs.branches = true;
                needs.remote_branches = true;
                needs.tags = true;
            }
            if after_double_dash || !positionals.is_empty() {
                needs.status_paths = true;
            }
        }
        "diff" => {
            needs.status_paths = true;
            if !after_double_dash && positionals.len() < 2 {
                needs.refs = true;
                needs.branches = true;
                needs.remote_branches = true;
                needs.tags = true;
            }
        }
        "status" | "add" | "rm" | "mv" | "clean" | "ls-files" => {
            needs.status_paths = true;
        }
        _ => {}
    }
    needs
}

fn make_git_command(query: &ProjectQuery, ctx: &GitContext, args: &[&str]) -> Command {
    // `CreateProcessW` resolves an executable using the launching process's
    // environment, before the child environment supplied by `.envs()` is in
    // effect.  A query can deliberately carry a different PATH (tests and a
    // shell which changed PATH are both legitimate), so resolve the fixed
    // local Git executable from that snapshot on Windows.  Batch wrappers are
    // supported through cmd.exe; this also makes a controlled fake Git
    // observable without allowing a shell to reinterpret any user input.
    #[cfg(windows)]
    let mut command = {
        let program = git_program_from_environment(query);
        if matches!(
            program.extension().and_then(|extension| extension.to_str()),
            Some(extension) if extension.eq_ignore_ascii_case("cmd")
                || extension.eq_ignore_ascii_case("bat")
        ) {
            let shell = query
                .environment
                .get("COMSPEC")
                .cloned()
                .or_else(|| query.environment.get("ComSpec").cloned())
                .unwrap_or_else(|| "cmd.exe".to_owned());
            let mut command = Command::new(shell);
            command.arg("/D").arg("/S").arg("/C").arg(program);
            command
        } else {
            Command::new(program)
        }
    };
    #[cfg(not(windows))]
    let mut command = Command::new("git");
    command.current_dir(&ctx.cwd);
    command
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("--no-optional-locks");
    if let Some(git_dir) = &ctx.git_dir {
        command.arg(format!("--git-dir={}", git_dir.display()));
    }
    if let Some(work_tree) = &ctx.work_tree {
        command.arg(format!("--work-tree={}", work_tree.display()));
    }
    command.args(args);
    command.envs(&query.environment);
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never");
    if ctx.git_dir.is_some() {
        command.env_remove("GIT_DIR");
    }
    if ctx.work_tree.is_some() {
        command.env_remove("GIT_WORK_TREE");
    }
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(windows)]
fn unsafe_batch_git_path(query: &ProjectQuery, ctx: &GitContext) -> Option<String> {
    let program = git_program_from_environment(query);
    let is_batch = program
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        });
    if !is_batch {
        return None;
    }
    let unsafe_character = |path: &Path| {
        path.to_string_lossy().chars().find(|character| {
            matches!(
                character,
                '&' | '|' | '<' | '>' | '(' | ')' | '^' | '%' | '!' | '"'
            )
        })
    };
    for (label, path) in [
        ("Git 批处理程序", program.as_path()),
        (
            "--git-dir",
            ctx.git_dir.as_deref().unwrap_or_else(|| Path::new("")),
        ),
        (
            "--work-tree",
            ctx.work_tree.as_deref().unwrap_or_else(|| Path::new("")),
        ),
    ] {
        if path.as_os_str().is_empty() {
            continue;
        }
        if let Some(character) = unsafe_character(path) {
            return Some(format!(
                "{label}路径包含 cmd.exe shell 元字符 {character:?}"
            ));
        }
    }
    None
}

#[cfg(windows)]
fn git_program_from_environment(query: &ProjectQuery) -> PathBuf {
    let path = query
        .environment
        .get("PATH")
        .or_else(|| query.environment.get("Path"));
    if let Some(path) = path {
        for directory in std::env::split_paths(std::ffi::OsStr::new(path)) {
            for name in ["git.exe", "git.cmd", "git.bat"] {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from("git")
}

#[cfg(not(windows))]
fn read_child_stdout(
    mut stdout: impl Read,
    limit: usize,
    truncated: Arc<AtomicBool>,
) -> (Vec<u8>, bool, bool) {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8 * 1024];
    loop {
        match stdout.read(&mut buffer) {
            Ok(0) => return (bytes, false, false),
            Ok(count) => {
                if bytes.len().saturating_add(count) > limit {
                    truncated.store(true, Ordering::Release);
                    bytes.truncate(limit);
                    return (bytes, true, false);
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            Err(_) => return (bytes, false, true),
        }
    }
}

#[cfg(windows)]
enum AvailableGitOutput {
    Empty,
    Progress,
    Closed,
    Truncated,
    Error,
}

#[cfg(windows)]
fn read_available_git_stdout(
    stdout: &mut std::process::ChildStdout,
    bytes: &mut Vec<u8>,
    limit: usize,
) -> AvailableGitOutput {
    const READ_CHUNK: usize = 64 * 1024;
    let mut available = 0u32;
    let peeked = unsafe {
        PeekNamedPipe(
            stdout.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    };
    if peeked == 0 {
        return if unsafe { GetLastError() } == ERROR_BROKEN_PIPE {
            AvailableGitOutput::Closed
        } else {
            AvailableGitOutput::Error
        };
    }
    if available == 0 {
        return AvailableGitOutput::Empty;
    }
    let remaining = limit.saturating_sub(bytes.len());
    if remaining == 0 {
        return AvailableGitOutput::Truncated;
    }
    let read_len = (available as usize).min(remaining).min(READ_CHUNK);
    let start = bytes.len();
    bytes.resize(start + read_len, 0);
    match stdout.read(&mut bytes[start..]) {
        Ok(0) => {
            bytes.truncate(start);
            AvailableGitOutput::Closed
        }
        Ok(count) => {
            bytes.truncate(start + count);
            if (available as usize) > remaining {
                AvailableGitOutput::Truncated
            } else {
                AvailableGitOutput::Progress
            }
        }
        Err(_) => {
            bytes.truncate(start);
            AvailableGitOutput::Error
        }
    }
}

#[cfg(windows)]
fn run_fixed_git(
    query: &ProjectQuery,
    ctx: &GitContext,
    args: &[&str],
    deadline: Instant,
    cancelled_flag: &AtomicBool,
) -> FixedOutput {
    if cancelled(cancelled_flag) {
        return FixedOutput {
            status: None,
            stdout: Vec::new(),
            truncated: false,
            read_error: false,
            cancelled: true,
            timed_out: false,
            spawn_error: None,
        };
    }
    #[cfg(windows)]
    if let Some(reason) = unsafe_batch_git_path(query, ctx) {
        return FixedOutput {
            status: None,
            stdout: Vec::new(),
            truncated: false,
            read_error: false,
            cancelled: false,
            timed_out: false,
            spawn_error: Some(reason),
        };
    }
    let mut command = make_git_command(query, ctx, args);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return FixedOutput {
                status: None,
                stdout: Vec::new(),
                truncated: false,
                read_error: false,
                cancelled: false,
                timed_out: false,
                spawn_error: Some(error.to_string()),
            };
        }
    };
    let mut stdout = child.stdout.take();
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut read_error = false;
    let mut was_cancelled = false;
    let mut timed_out = false;

    let status = loop {
        if cancelled(cancelled_flag) {
            was_cancelled = true;
            let _ = child.kill();
            break child.wait().ok();
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            break child.wait().ok();
        }

        let read_state = match stdout.as_mut() {
            Some(stdout) => read_available_git_stdout(stdout, &mut bytes, GIT_OUTPUT_LIMIT),
            None => AvailableGitOutput::Closed,
        };
        match read_state {
            AvailableGitOutput::Progress => continue,
            AvailableGitOutput::Truncated => {
                truncated = true;
                let _ = child.kill();
                break child.wait().ok();
            }
            AvailableGitOutput::Error => {
                read_error = true;
                let _ = child.kill();
                break child.wait().ok();
            }
            AvailableGitOutput::Empty | AvailableGitOutput::Closed => {}
        }

        match child.try_wait() {
            Ok(Some(exit_status)) => {
                // A process can exit while bytes are still buffered.  Drain
                // only what is available, checking the same deadline and
                // cancellation flag between every read.  This never waits on
                // a descendant which inherited the pipe handle.
                loop {
                    if cancelled(cancelled_flag) {
                        was_cancelled = true;
                        break;
                    }
                    if Instant::now() >= deadline {
                        timed_out = true;
                        break;
                    }
                    let read_state = match stdout.as_mut() {
                        Some(stdout) => {
                            read_available_git_stdout(stdout, &mut bytes, GIT_OUTPUT_LIMIT)
                        }
                        None => AvailableGitOutput::Closed,
                    };
                    match read_state {
                        AvailableGitOutput::Progress => continue,
                        AvailableGitOutput::Truncated => {
                            truncated = true;
                            break;
                        }
                        AvailableGitOutput::Error => {
                            read_error = true;
                            break;
                        }
                        AvailableGitOutput::Empty | AvailableGitOutput::Closed => break,
                    }
                }
                break Some(exit_status);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(2)),
            Err(_) => {
                read_error = true;
                let _ = child.kill();
                break child.wait().ok();
            }
        }
    };

    FixedOutput {
        status,
        stdout: bytes,
        truncated,
        read_error,
        cancelled: was_cancelled,
        timed_out,
        spawn_error: None,
    }
}

#[cfg(not(windows))]
fn run_fixed_git(
    query: &ProjectQuery,
    ctx: &GitContext,
    args: &[&str],
    deadline: Instant,
    cancelled_flag: &AtomicBool,
) -> FixedOutput {
    if cancelled(cancelled_flag) {
        return FixedOutput {
            status: None,
            stdout: Vec::new(),
            truncated: false,
            read_error: false,
            cancelled: true,
            timed_out: false,
            spawn_error: None,
        };
    }
    let mut command = make_git_command(query, ctx, args);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return FixedOutput {
                status: None,
                stdout: Vec::new(),
                truncated: false,
                read_error: false,
                cancelled: false,
                timed_out: false,
                spawn_error: Some(error.to_string()),
            };
        }
    };
    let stdout = child.stdout.take();
    let truncated_flag = Arc::new(AtomicBool::new(false));
    let reader_flag = Arc::clone(&truncated_flag);
    let reader = thread::spawn(move || match stdout {
        Some(stdout) => read_child_stdout(stdout, GIT_OUTPUT_LIMIT, reader_flag),
        None => (Vec::new(), false, true),
    });
    let mut was_cancelled = false;
    let mut timed_out = false;
    let status = loop {
        if cancelled(cancelled_flag) {
            was_cancelled = true;
            let _ = child.kill();
            break child.wait().ok();
        }
        if truncated_flag.load(Ordering::Acquire) {
            let _ = child.kill();
            break child.wait().ok();
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            break child.wait().ok();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(_) => {
                let _ = child.kill();
                break child.wait().ok();
            }
        }
    };
    let (stdout, truncated, read_error) = reader.join().unwrap_or((Vec::new(), false, true));
    FixedOutput {
        status,
        stdout,
        truncated,
        read_error,
        cancelled: was_cancelled,
        timed_out,
        spawn_error: None,
    }
}

fn append_git_process_diagnostic(result: &mut ProviderResult, label: &str, output: &FixedOutput) {
    if output.cancelled {
        result.diagnostics.push(format!("Git {label} 查询已取消"));
        result.incomplete = true;
    } else if output.timed_out {
        result
            .diagnostics
            .push(format!("Git {label} 查询超过 1 秒截止时间"));
        result.incomplete = true;
    } else if output.truncated {
        result
            .diagnostics
            .push(format!("Git {label} 输出超过预算，结果不完整"));
        result.incomplete = true;
    } else if output.read_error {
        result
            .diagnostics
            .push(format!("读取 Git {label} 输出失败"));
        result.incomplete = true;
    } else if let Some(error) = &output.spawn_error {
        result.diagnostics.push(format!("无法启动 Git：{error}"));
        result.incomplete = true;
    } else if output.failed() {
        let status = output
            .status
            .and_then(|status| status.code().map(|code| code.to_string()))
            .unwrap_or_else(|| "无退出码".to_owned());
        result.diagnostics.push(format!(
            "Git {label} 查询返回非零状态（{status}），结果不完整"
        ));
        result.incomplete = true;
    }
}

fn collect_git(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let ctx = parse_git_context(query);
    let needs = git_needs(&ctx, provider_scope(query, "git"));
    let mut result = ProviderResult {
        project_root: git_project_root(&ctx),
        ..ProviderResult::default()
    };
    git_watch_paths(&ctx, &mut result, needs.status_paths);
    if ctx.subcommand.is_empty()
        || (!needs.refs && !needs.remotes && !needs.worktrees && !needs.status_paths)
    {
        return result;
    }
    let deadline = Instant::now() + GIT_DEADLINE;
    let mut refs = GitRefs::default();
    if needs.refs {
        let output = run_fixed_git(
            query,
            &ctx,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "--sort=refname",
                "refs/heads",
                "refs/remotes",
                "refs/tags",
            ],
            deadline,
            cancelled_flag,
        );
        append_git_process_diagnostic(&mut result, "引用", &output);
        if !output.interrupted() && !output.failed() {
            refs = parse_git_refs(&output.stdout);
        }
    }
    if needs.remotes && !cancelled(cancelled_flag) && Instant::now() < deadline {
        let output = run_fixed_git(query, &ctx, &["remote"], deadline, cancelled_flag);
        append_git_process_diagnostic(&mut result, "远端", &output);
        if !output.interrupted() && !output.failed() {
            refs.remotes.extend(
                output
                    .stdout
                    .split(|byte| *byte == b'\n' || *byte == b'\r')
                    .filter_map(|line| String::from_utf8(line.to_vec()).ok())
                    .map(|line| line.trim().to_owned())
                    .filter(|line| !line.is_empty()),
            );
        }
    }
    if needs.worktrees && !cancelled(cancelled_flag) && Instant::now() < deadline {
        let output = run_fixed_git(
            query,
            &ctx,
            &["worktree", "list", "--porcelain"],
            deadline,
            cancelled_flag,
        );
        append_git_process_diagnostic(&mut result, "工作树", &output);
        if !output.interrupted() && !output.failed() {
            for path in parse_worktrees(&output.stdout) {
                add_filtered(
                    &mut result,
                    path,
                    "Git 工作树目录",
                    CandidateKind::Directory,
                    "git.worktree",
                    &query.prefix,
                );
            }
        }
    }
    if needs.status_paths && !cancelled(cancelled_flag) && Instant::now() < deadline {
        let output = run_fixed_git(
            query,
            &ctx,
            &["status", "--short", "--untracked-files=all", "-z"],
            deadline,
            cancelled_flag,
        );
        append_git_process_diagnostic(&mut result, "状态路径", &output);
        if !output.interrupted() && !output.failed() {
            for path in parse_status_paths(&output.stdout) {
                add_filtered(
                    &mut result,
                    path,
                    "Git 状态中的路径",
                    CandidateKind::File,
                    "git.status_path",
                    &query.prefix,
                );
            }
        }
    }
    if needs.refs {
        if needs.branches {
            for branch in &refs.local_branches {
                add_filtered(
                    &mut result,
                    branch.clone(),
                    "本地 Git 分支",
                    CandidateKind::Value,
                    "git.branch",
                    &query.prefix,
                );
            }
        }
        if needs.remote_branches {
            for branch in &refs.remote_branches {
                add_filtered(
                    &mut result,
                    branch.clone(),
                    "远端 Git 分支",
                    CandidateKind::Value,
                    "git.remote_branch",
                    &query.prefix,
                );
            }
        }
        if needs.tags {
            for tag in &refs.tags {
                add_filtered(
                    &mut result,
                    tag.clone(),
                    "Git 标签",
                    CandidateKind::Value,
                    "git.tag",
                    &query.prefix,
                );
            }
        }
    }
    if needs.remotes {
        for remote in refs.remotes {
            add_filtered(
                &mut result,
                remote,
                "Git 远端名称",
                CandidateKind::Value,
                "git.remote",
                &query.prefix,
            );
        }
    }
    if cancelled(cancelled_flag) {
        result.incomplete = true;
        result.diagnostics.push("Git 项目查询已取消".to_owned());
    }
    result.finish()
}

fn git_watch_paths(ctx: &GitContext, result: &mut ProviderResult, watch_work_tree: bool) {
    if let Some(work_tree) = &ctx.work_tree {
        if watch_work_tree {
            add_recursive_watch_path(result, work_tree);
        } else {
            add_watch_path(result, work_tree);
        }
    } else if watch_work_tree && let Some(root) = result.project_root.clone() {
        add_recursive_watch_path(result, &root);
    }
    let git_dir = ctx.git_dir.clone().or_else(|| {
        let root = git_project_root(ctx)?;
        let marker = root.join(".git");
        if marker.is_dir() {
            Some(marker)
        } else {
            let contents = fs::read_to_string(marker).ok()?;
            let value = contents.trim().strip_prefix("gitdir:")?.trim();
            (!value.is_empty()).then(|| resolve_path(&root, value))
        }
    });
    let Some(git_dir) = git_dir else {
        return;
    };
    add_watch_path(result, &git_dir);
    if let Ok(contents) = fs::read_to_string(git_dir.join("commondir")) {
        let common = contents.trim();
        if !common.is_empty() {
            add_watch_path(result, &resolve_path(&git_dir, common));
        }
    }
}

fn git_project_root(ctx: &GitContext) -> Option<PathBuf> {
    if let Some(work_tree) = &ctx.work_tree
        && work_tree.is_dir()
    {
        return Some(lexical_normalize(work_tree));
    }
    if let Some(git_dir) = &ctx.git_dir
        && git_dir.is_dir()
        && git_dir.file_name().and_then(|name| name.to_str()) == Some(".git")
        && let Some(parent) = git_dir.parent()
    {
        return Some(lexical_normalize(parent));
    }
    let mut current = if ctx.cwd.is_dir() {
        ctx.cwd.clone()
    } else {
        ctx.cwd.parent()?.to_path_buf()
    };
    loop {
        let marker = current.join(".git");
        if marker.is_dir() || marker.is_file() {
            return Some(lexical_normalize(&current));
        }
        if !current.pop() {
            return None;
        }
    }
}

fn parse_git_refs(bytes: &[u8]) -> GitRefs {
    let mut refs = GitRefs::default();
    let mut seen_remotes = BTreeSet::new();
    for line in String::from_utf8_lossy(bytes).lines() {
        let reference = line.trim();
        if let Some(branch) = reference.strip_prefix("refs/heads/") {
            if !branch.is_empty() {
                refs.local_branches.push(branch.to_owned());
            }
        } else if let Some(branch) = reference.strip_prefix("refs/remotes/") {
            let Some((remote, name)) = branch.split_once('/') else {
                continue;
            };
            if remote.is_empty() || name.is_empty() || name.eq_ignore_ascii_case("HEAD") {
                continue;
            }
            seen_remotes.insert(remote.to_owned());
            refs.remote_branches.push(branch.to_owned());
        } else if let Some(tag) = reference.strip_prefix("refs/tags/")
            && !tag.is_empty()
        {
            refs.tags.push(tag.to_owned());
        }
    }
    refs.remotes.extend(seen_remotes);
    refs
}

fn parse_worktrees(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_status_paths(bytes: &[u8]) -> Vec<String> {
    let mut paths = Vec::new();
    let mut fields = bytes.split(|byte| *byte == 0);
    while let Some(field) = fields.next() {
        if field.len() < 3 {
            continue;
        }
        // `status --short -z` uses two status bytes followed by a space.
        let path = &field[3..];
        if !path.is_empty() {
            paths.push(String::from_utf8_lossy(path).into_owned());
        }
        // Rename/copy records carry the old path as a second NUL-delimited
        // field. Offering it is useful for checkout/restore path completion.
        let status = &field[..2];
        if (status.contains(&b'R') || status.contains(&b'C'))
            && let Some(previous) = fields.next()
            && !previous.is_empty()
        {
            paths.push(String::from_utf8_lossy(previous).into_owned());
        }
    }
    paths
}

// ---- Cargo ---------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct CargoPackage {
    name: String,
    root: PathBuf,
    features: BTreeSet<String>,
    bins: BTreeSet<String>,
    examples: BTreeSet<String>,
    tests: BTreeSet<String>,
    benches: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
struct CargoProject {
    packages: Vec<CargoPackage>,
    /// Manifest directories touched even when the manifest is invalid.
    watch_paths: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CargoValueKind {
    Package,
    Features,
    Bin,
    Example,
    Test,
    Bench,
}

fn cargo_option_value(token: &str) -> Option<CargoValueKind> {
    let option = token.split_once('=').map(|(name, _)| name).unwrap_or(token);
    match option {
        "-p" | "--package" => Some(CargoValueKind::Package),
        "--features" => Some(CargoValueKind::Features),
        "--bin" => Some(CargoValueKind::Bin),
        "--example" => Some(CargoValueKind::Example),
        "--test" => Some(CargoValueKind::Test),
        "--bench" => Some(CargoValueKind::Bench),
        _ => None,
    }
}

fn cargo_manifest_option(token: &str) -> bool {
    token == "--manifest-path" || token.starts_with("--manifest-path=")
}

fn cargo_global_option_requires_value(token: &str) -> bool {
    let option = token.split_once('=').map(|(name, _)| name).unwrap_or(token);
    matches!(
        option,
        "--color"
            | "--config"
            | "--target"
            | "--target-dir"
            | "--jobs"
            | "-j"
            | "--profile"
            | "--message-format"
    )
}

fn cargo_context(args: &[String]) -> (String, Option<CargoValueKind>) {
    let args = split_command_args(args);
    let mut command = String::new();
    let mut command_index = args.len();
    let mut index = 0usize;
    while index < args.len() {
        let token = &args[index];
        if token.starts_with('+') && command.is_empty() {
            index += 1;
            continue;
        }
        if cargo_manifest_option(token)
            || (cargo_option_value(token).is_some() && !token.contains('='))
            || (cargo_global_option_requires_value(token) && !token.contains('='))
        {
            index = (index + 2).min(args.len());
            continue;
        }
        if token.starts_with('-') {
            index += 1;
            continue;
        }
        if !token.starts_with('-') {
            command = token.clone();
            command_index = index;
            break;
        }
        index += 1;
    }
    if command.is_empty() {
        return (command, None);
    }
    let mut active = None;
    let mut index = command_index + 1;
    while index < args.len() {
        let token = &args[index];
        if let Some(kind) = cargo_option_value(token) {
            if token.contains('=') {
                // An attached option already carries its value.  Since
                // `args` contains only tokens preceding the current token,
                // it cannot be the value slot currently being edited.
                active = None;
                index += 1;
            } else if index + 1 >= args.len() {
                active = Some(kind);
                break;
            } else {
                active = None;
                index += 2;
            }
        } else if token == "--" {
            break;
        } else {
            active = None;
            index += 1;
        }
    }
    (command, active)
}

fn cargo_manifest_path(query: &ProjectQuery) -> Option<PathBuf> {
    let args = split_command_args(&query.args);
    let mut index = 0usize;
    while index < args.len() {
        let token = &args[index];
        if token == "--manifest-path" {
            if let Some(value) = args.get(index + 1) {
                return Some(resolve_path(&query.cwd, value));
            }
            return None;
        }
        if let Some(value) = token.strip_prefix("--manifest-path=") {
            return Some(resolve_path(&query.cwd, value));
        }
        index += 1;
    }
    find_upwards(&query.cwd, "Cargo.toml")
}

fn cargo_project_root(manifest: &Path) -> Option<PathBuf> {
    let manifest_root = manifest.parent().unwrap_or(manifest);
    find_cargo_workspace_manifest(manifest_root)
        .and_then(|workspace| workspace.parent().map(lexical_normalize))
        .or_else(|| Some(lexical_normalize(manifest_root)))
}

fn find_cargo_workspace_manifest(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        let candidate = current.join("Cargo.toml");
        if let Ok(text) = read_limited(&candidate, MANIFEST_LIMIT)
            && let Ok(value) = toml::from_str::<toml::Value>(&text)
            && value.get("workspace").is_some()
        {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn toml_string(value: Option<&toml::Value>) -> Option<String> {
    value.and_then(toml::Value::as_str).map(ToOwned::to_owned)
}

fn toml_strings(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn target_names_from_array(value: Option<&toml::Value>) -> BTreeSet<String> {
    value
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|target| target.get("name").and_then(toml::Value::as_str))
        .map(ToOwned::to_owned)
        .collect()
}

fn add_standard_target_files(
    root: &Path,
    directory: &str,
    targets: &mut BTreeSet<String>,
    cancelled_flag: Option<&AtomicBool>,
) -> bool {
    let directory = root.join(directory);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    for entry in entries {
        if cancelled_flag.is_some_and(cancelled) {
            return true;
        }
        let Ok(entry) = entry else {
            return true;
        };
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            return true;
        };
        if kind.is_file() {
            if path.extension().and_then(|extension| extension.to_str()) == Some("rs")
                && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
            {
                targets.insert(stem.to_owned());
            }
        } else if kind.is_dir()
            && path.join("main.rs").is_file()
            && let Some(name) = path.file_name().and_then(|name| name.to_str())
        {
            targets.insert(name.to_owned());
        }
    }
    false
}

fn parse_cargo_package(
    path: &Path,
    value: &toml::Value,
    cancelled_flag: Option<&AtomicBool>,
) -> (Option<CargoPackage>, bool) {
    let Some(package) = value.get("package").and_then(toml::Value::as_table) else {
        return (None, false);
    };
    let Some(name) = toml_string(package.get("name")) else {
        return (None, false);
    };
    let mut parsed = CargoPackage {
        name,
        root: path.parent().unwrap_or(path).to_path_buf(),
        ..CargoPackage::default()
    };
    if let Some(features) = value.get("features").and_then(toml::Value::as_table) {
        parsed.features.extend(features.keys().cloned());
    }
    parsed.bins = target_names_from_array(value.get("bin"));
    parsed.examples = target_names_from_array(value.get("example"));
    parsed.tests = target_names_from_array(value.get("test"));
    parsed.benches = target_names_from_array(value.get("bench"));
    if parsed.root.join("src/main.rs").is_file() {
        parsed.bins.insert(parsed.name.clone());
    }
    let bins_overflow = add_standard_target_files(
        &parsed.root.join("src"),
        "bin",
        &mut parsed.bins,
        cancelled_flag,
    );
    let examples_overflow = add_standard_target_files(
        &parsed.root,
        "examples",
        &mut parsed.examples,
        cancelled_flag,
    );
    let tests_overflow =
        add_standard_target_files(&parsed.root, "tests", &mut parsed.tests, cancelled_flag);
    let benches_overflow =
        add_standard_target_files(&parsed.root, "benches", &mut parsed.benches, cancelled_flag);
    (
        Some(parsed),
        bins_overflow || examples_overflow || tests_overflow || benches_overflow,
    )
}

fn cargo_excluded(root: &Path, package_root: &Path, patterns: &[String]) -> bool {
    let relative = package_root
        .strip_prefix(root)
        .unwrap_or(package_root)
        .to_string_lossy()
        .replace('\\', "/");
    patterns.iter().any(|pattern| {
        let pattern = pattern.replace('\\', "/");
        wildcard_match(&pattern, &relative) || pattern.trim_end_matches('/').eq(&relative)
    })
}

fn load_cargo_project(
    manifest: &Path,
    cancelled_flag: &AtomicBool,
) -> (CargoProject, Vec<String>, bool) {
    let mut diagnostics = Vec::new();
    let mut incomplete = false;
    let mut project = CargoProject::default();
    if let Some(parent) = manifest.parent() {
        record_watch_path(&mut project.watch_paths, parent);
    }
    let text = match read_limited(manifest, MANIFEST_LIMIT) {
        Ok(text) => text,
        Err(error) => {
            diagnostics.push(format!(
                "读取 Cargo 清单 {} 失败：{error}",
                manifest.display()
            ));
            return (CargoProject::default(), diagnostics, true);
        }
    };
    let value: toml::Value = match toml::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            diagnostics.push(format!(
                "解析 Cargo 清单 {} 失败：{error}",
                manifest.display()
            ));
            return (CargoProject::default(), diagnostics, true);
        }
    };
    // Cargo resolves a command launched from a workspace member against the
    // nearest ancestor virtual/root workspace.  Reuse the root loader so a
    // prompt inside `crates/app` still sees sibling package names and target
    // values.  The current member is added when it is not listed explicitly.
    if value.get("workspace").is_none()
        && let Some(workspace_manifest) =
            find_cargo_workspace_manifest(manifest.parent().unwrap_or(manifest))
        && lexical_normalize(&workspace_manifest) != lexical_normalize(manifest)
    {
        let (mut project, mut parent_diagnostics, mut parent_incomplete) =
            load_cargo_project(&workspace_manifest, cancelled_flag);
        diagnostics.append(&mut parent_diagnostics);
        if let Some(parent) = manifest.parent() {
            record_watch_path(&mut project.watch_paths, parent);
        }
        let member_root = manifest.parent().unwrap_or(manifest).to_path_buf();
        if !project
            .packages
            .iter()
            .any(|package| lexical_normalize(&package.root) == lexical_normalize(&member_root))
        {
            let (package, targets_incomplete) =
                parse_cargo_package(manifest, &value, Some(cancelled_flag));
            parent_incomplete |= targets_incomplete;
            if let Some(package) = package {
                project.packages.push(package);
            }
        }
        return (project, diagnostics, parent_incomplete);
    }
    let root = manifest.parent().unwrap_or(manifest);
    let (root_package, root_targets_incomplete) =
        parse_cargo_package(manifest, &value, Some(cancelled_flag));
    incomplete |= root_targets_incomplete;
    if root_targets_incomplete {
        diagnostics.push("Cargo 标准目标扫描未完成，结果不完整".to_owned());
    }
    if let Some(package) = root_package {
        project.packages.push(package);
    }
    let Some(workspace) = value.get("workspace").and_then(toml::Value::as_table) else {
        return (project, diagnostics, incomplete);
    };
    let members = toml_strings(workspace.get("members"));
    let excludes = toml_strings(workspace.get("exclude"));
    if members.is_empty() {
        return (project, diagnostics, incomplete);
    }
    let mut seen_roots = project
        .packages
        .iter()
        .map(|package| lexical_normalize(&package.root))
        .collect::<HashSet<_>>();
    for member in members {
        if cancelled(cancelled_flag) {
            incomplete = true;
            diagnostics.push("Cargo 工作区扫描已取消".to_owned());
            break;
        }
        let (paths, scan_incomplete) = expand_project_pattern(
            root,
            &member,
            "Cargo.toml",
            Some(cancelled_flag),
            &mut project.watch_paths,
        );
        incomplete |= scan_incomplete;
        if scan_incomplete {
            diagnostics.push("Cargo 工作区路径扫描未完成，结果不完整".to_owned());
        }
        if paths.is_empty() && !has_wildcard(&member) {
            diagnostics.push(format!("Cargo 工作区成员不存在：{member}"));
            incomplete = true;
        }
        for package_root in paths {
            if cancelled(cancelled_flag) {
                incomplete = true;
                diagnostics.push("Cargo 工作区扫描已取消".to_owned());
                break;
            }
            if cargo_excluded(root, &package_root, &excludes) {
                continue;
            }
            let normalized = lexical_normalize(&package_root);
            if !seen_roots.insert(normalized) {
                continue;
            }
            let child_manifest = package_root.join("Cargo.toml");
            record_watch_path(&mut project.watch_paths, &package_root);
            let text = match read_limited(&child_manifest, MANIFEST_LIMIT) {
                Ok(text) => text,
                Err(error) => {
                    diagnostics.push(format!(
                        "读取 Cargo 清单 {} 失败：{error}",
                        child_manifest.display()
                    ));
                    incomplete = true;
                    continue;
                }
            };
            let value: toml::Value = match toml::from_str(&text) {
                Ok(value) => value,
                Err(error) => {
                    diagnostics.push(format!(
                        "解析 Cargo 清单 {} 失败：{error}",
                        child_manifest.display()
                    ));
                    incomplete = true;
                    continue;
                }
            };
            let (package, targets_incomplete) =
                parse_cargo_package(&child_manifest, &value, Some(cancelled_flag));
            incomplete |= targets_incomplete;
            if targets_incomplete {
                diagnostics.push(format!(
                    "Cargo 标准目标扫描未完成：{}",
                    child_manifest.display()
                ));
            }
            if let Some(package) = package {
                project.packages.push(package);
            } else {
                diagnostics.push(format!(
                    "Cargo 工作区成员缺少 package.name：{}",
                    child_manifest.display()
                ));
                incomplete = true;
            }
        }
    }
    (project, diagnostics, incomplete)
}

fn cargo_selected_package<'a>(
    args: &[String],
    project: &'a CargoProject,
) -> Option<&'a CargoPackage> {
    let args = split_command_args(args);
    let mut index = 0usize;
    while index < args.len() {
        let token = &args[index];
        if token == "-p" || token == "--package" {
            if let Some(name) = args.get(index + 1) {
                return project
                    .packages
                    .iter()
                    .find(|package| package.name == *name);
            }
        } else if let Some(name) = token.strip_prefix("--package=") {
            return project.packages.iter().find(|package| package.name == name);
        }
        index += 1;
    }
    None
}

fn cargo_feature_parts(prefix: &str) -> (&str, &str, BTreeSet<String>) {
    let prefix = prefix
        .strip_prefix("--features=")
        .or_else(|| prefix.strip_prefix("--feature="))
        .unwrap_or(prefix);
    let split_at = prefix.rfind(',').map(|index| index + 1).unwrap_or(0);
    let before = &prefix[..split_at];
    let tail = &prefix[split_at..];
    let selected = before
        .trim_end_matches(',')
        .split(',')
        .filter_map(|feature| {
            let feature = feature.trim();
            (!feature.is_empty()).then(|| feature.to_owned())
        })
        .collect();
    (before, tail, selected)
}

fn collect_cargo(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let Some(manifest) = cargo_manifest_path(query) else {
        let mut result = ProviderResult::default();
        record_missing_project_watches(&mut result.watch_paths, &query.cwd);
        return result.finish();
    };
    let (project, diagnostics, incomplete) = load_cargo_project(&manifest, cancelled_flag);
    let (command, active) = cargo_context(&query.args);
    let mut result = ProviderResult {
        diagnostics,
        incomplete,
        project_root: cargo_project_root(&manifest),
        ..ProviderResult::default()
    };
    cargo_watch_paths(&manifest, &project, &mut result);
    if cancelled(cancelled_flag) {
        result.incomplete = true;
        result.diagnostics.push("Cargo 项目查询已取消".to_owned());
        return result.finish();
    }
    let selected = cargo_selected_package(&query.args, &project);
    let packages = selected.into_iter().collect::<Vec<_>>();
    let packages = if packages.is_empty() {
        project.packages.iter().collect::<Vec<_>>()
    } else {
        packages
    };
    let forced_kind = provider_scope(query, "cargo").and_then(|scope| {
        match scope.to_ascii_lowercase().as_str() {
            "features" => Some(CargoValueKind::Features),
            "packages" => Some(CargoValueKind::Package),
            "bins" => Some(CargoValueKind::Bin),
            "examples" => Some(CargoValueKind::Example),
            "tests" => Some(CargoValueKind::Test),
            "benches" => Some(CargoValueKind::Bench),
            _ => None,
        }
    });
    let kind = forced_kind.or(active).or_else(|| {
        // A prefix containing an attached option is also a value context when
        // the adapter supplied the current token as part of the prefix.
        if query.prefix.starts_with("--features=") {
            Some(CargoValueKind::Features)
        } else {
            None
        }
    });
    match kind {
        Some(CargoValueKind::Package) => {
            for package in &project.packages {
                add_filtered(
                    &mut result,
                    package.name.clone(),
                    "Cargo 工作区包",
                    CandidateKind::Value,
                    "cargo.package",
                    &query.prefix,
                );
            }
        }
        Some(CargoValueKind::Features) => {
            let (before, tail, selected) = cargo_feature_parts(&query.prefix);
            let mut features = BTreeSet::new();
            for package in packages {
                features.extend(package.features.iter().cloned());
            }
            for feature in features {
                if selected.contains(&feature) {
                    continue;
                }
                let value = format!("{before}{feature}");
                if prefix_matches(&feature, tail) {
                    result.push(ProviderCandidate {
                        value,
                        description: "Cargo 功能 feature".to_owned(),
                        kind: CandidateKind::Value,
                        source: "cargo.feature".to_owned(),
                    });
                }
            }
        }
        Some(CargoValueKind::Bin)
        | Some(CargoValueKind::Example)
        | Some(CargoValueKind::Test)
        | Some(CargoValueKind::Bench) => {
            for package in packages {
                let (targets, source, description) = match kind {
                    Some(CargoValueKind::Bin) => (&package.bins, "cargo.bin", "Cargo 二进制目标"),
                    Some(CargoValueKind::Example) => {
                        (&package.examples, "cargo.example", "Cargo 示例目标")
                    }
                    Some(CargoValueKind::Test) => (&package.tests, "cargo.test", "Cargo 测试目标"),
                    Some(CargoValueKind::Bench) => {
                        (&package.benches, "cargo.bench", "Cargo 基准目标")
                    }
                    _ => unreachable!(),
                };
                for target in targets {
                    add_filtered(
                        &mut result,
                        target.clone(),
                        description,
                        CandidateKind::Value,
                        source,
                        &query.prefix,
                    );
                }
            }
        }
        None => {
            let command = lower(&command);
            if command == "run" || command == "test" || command == "build" || command == "check" {
                for package in &project.packages {
                    add_filtered(
                        &mut result,
                        package.name.clone(),
                        "Cargo 工作区包",
                        CandidateKind::Value,
                        "cargo.package",
                        &query.prefix,
                    );
                }
            }
        }
    }
    if incomplete {
        result.incomplete = true;
    }
    result.finish()
}

fn cargo_watch_paths(manifest: &Path, project: &CargoProject, result: &mut ProviderResult) {
    if let Some(parent) = manifest.parent() {
        add_watch_path(result, parent);
    }
    for path in &project.watch_paths {
        add_watch_path(result, path);
    }
    if let Some(root) = result.project_root.clone() {
        add_watch_path(result, &root);
    }
    for package in &project.packages {
        add_watch_path(result, &package.root);
        for directory in ["src/bin", "examples", "tests", "benches"] {
            add_watch_path(result, &package.root.join(directory));
        }
    }
}

// ---- npm / pnpm ----------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct NodePackage {
    name: String,
    root: PathBuf,
    scripts: BTreeSet<String>,
    dependencies: BTreeSet<String>,
    local_dependencies: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
struct NodeProject {
    packages: Vec<NodePackage>,
    /// Manifest directories touched even when a member cannot be parsed.
    watch_paths: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeValueKind {
    Script,
    Dependency,
    Workspace,
}

fn json_object_keys(value: Option<&serde_json::Value>) -> BTreeSet<String> {
    value
        .and_then(serde_json::Value::as_object)
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

fn json_workspace_patterns(value: Option<&serde_json::Value>) -> Vec<String> {
    if let Some(patterns) = value.and_then(serde_json::Value::as_array) {
        return patterns
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .collect();
    }
    value
        .and_then(serde_json::Value::as_object)
        .and_then(|object| object.get("packages"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn json_dependency_names(
    value: Option<&serde_json::Value>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut all = BTreeSet::new();
    let mut local = BTreeSet::new();
    let Some(object) = value.and_then(serde_json::Value::as_object) else {
        return (all, local);
    };
    for (name, version) in object {
        all.insert(name.clone());
        let local_value = version.as_str().is_some_and(|version| {
            version.starts_with("workspace:")
                || version.starts_with("file:")
                || version.starts_with("link:")
                || version.starts_with('.')
                || version.starts_with('/')
                || version.starts_with('\\')
        });
        if local_value {
            local.insert(name.clone());
        }
    }
    (all, local)
}

fn parse_node_package(path: &Path, value: &serde_json::Value) -> NodePackage {
    let package = value.as_object();
    let name = package
        .and_then(|object| object.get("name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let scripts = json_object_keys(package.and_then(|object| object.get("scripts")));
    let mut dependencies = BTreeSet::new();
    let mut local_dependencies = BTreeSet::new();
    for field in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        let (all, local) = json_dependency_names(package.and_then(|object| object.get(field)));
        dependencies.extend(all);
        local_dependencies.extend(local);
    }
    NodePackage {
        name,
        root: path.parent().unwrap_or(path).to_path_buf(),
        scripts,
        dependencies,
        local_dependencies,
    }
}

fn parse_pnpm_workspace_patterns(text: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut in_packages = false;
    for line in text.lines() {
        let without_comment = line.split_once('#').map(|(value, _)| value).unwrap_or(line);
        let trimmed = without_comment.trim();
        if trimmed == "packages:" {
            in_packages = true;
            continue;
        }
        if let Some(inline) = trimmed.strip_prefix("packages:") {
            let inline = inline.trim();
            if inline.starts_with('[') && inline.ends_with(']') {
                patterns.extend(
                    inline[1..inline.len() - 1]
                        .split(',')
                        .map(str::trim)
                        .map(|value| value.trim_matches('"').trim_matches('\''))
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned),
                );
            }
            in_packages = false;
            continue;
        }
        if in_packages {
            let Some(value) = trimmed.strip_prefix('-') else {
                if !trimmed.is_empty() && !trimmed.starts_with(' ') && !trimmed.starts_with('\t') {
                    in_packages = false;
                }
                continue;
            };
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                patterns.push(value.to_owned());
            }
        }
    }
    patterns
}

fn find_upwards_named(start: &Path, file_name: &str) -> Option<PathBuf> {
    find_upwards(start, file_name)
}

fn node_workspace_manifest(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        let candidate = current.join("package.json");
        if let Ok(text) = read_limited(&candidate, MANIFEST_LIMIT)
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
        {
            let object = value.as_object();
            if object.is_some_and(|object| object.contains_key("workspaces")) {
                return Some(candidate);
            }
        }
        if !current.pop() {
            return None;
        }
    }
}

fn load_node_project(
    query: &ProjectQuery,
    pnpm: bool,
    cancelled_flag: &AtomicBool,
) -> (NodeProject, Vec<String>, bool) {
    let effective_cwd = node_effective_cwd(query);
    let nearest_package = find_upwards_named(&effective_cwd, "package.json");
    let pnpm_workspace = find_upwards_named(&effective_cwd, "pnpm-workspace.yaml");
    let workspace_package = node_workspace_manifest(&effective_cwd);
    let pnpm_root_manifest = pnpm_workspace.as_ref().and_then(|workspace| {
        let candidate = workspace.parent()?.join("package.json");
        candidate.is_file().then_some(candidate)
    });
    let root_dir_hint = pnpm_workspace
        .as_ref()
        .and_then(|workspace| workspace.parent().map(Path::to_path_buf));
    // A pnpm workspace may intentionally omit a root package.json. In that
    // case the nearest package belongs to a member, while patterns in
    // pnpm-workspace.yaml are relative to the workspace root.
    let root_manifest = if pnpm_workspace.is_some() {
        pnpm_root_manifest
    } else {
        workspace_package.or(nearest_package.clone())
    };
    let seed_manifest = root_manifest.clone().or(nearest_package.clone());
    let Some(root_dir) = root_dir_hint
        .or_else(|| {
            seed_manifest
                .as_ref()
                .and_then(|manifest| manifest.parent().map(Path::to_path_buf))
        })
        .or_else(|| {
            root_manifest
                .as_ref()
                .and_then(|manifest| manifest.parent().map(Path::to_path_buf))
        })
    else {
        let mut project = NodeProject::default();
        record_missing_project_watches(&mut project.watch_paths, &effective_cwd);
        return (project, Vec::new(), false);
    };
    let mut diagnostics = Vec::new();
    let mut incomplete = false;
    let mut project = NodeProject::default();
    record_watch_path(&mut project.watch_paths, &root_dir);
    // A nearer package.json may be created after we used an ancestor's
    // manifest (including a package.json in the user's home directory).
    // Watch each search directory up to that root, without scanning its tree.
    for directory in effective_cwd.ancestors() {
        record_watch_path(&mut project.watch_paths, directory);
        if lexical_normalize(directory) == lexical_normalize(&root_dir) {
            break;
        }
    }
    let mut patterns = Vec::new();
    if let Some(seed_manifest) = &seed_manifest {
        if let Some(parent) = seed_manifest.parent() {
            record_watch_path(&mut project.watch_paths, parent);
        }
        let root_text = match read_limited(seed_manifest, MANIFEST_LIMIT) {
            Ok(text) => text,
            Err(error) => {
                diagnostics.push(format!("读取 package.json 失败：{error}"));
                return (NodeProject::default(), diagnostics, true);
            }
        };
        let root_value: serde_json::Value = match serde_json::from_str(&root_text) {
            Ok(value) => value,
            Err(error) => {
                diagnostics.push(format!(
                    "解析 package.json {} 失败：{error}",
                    seed_manifest.display()
                ));
                return (NodeProject::default(), diagnostics, true);
            }
        };
        if !root_value.is_object() {
            diagnostics.push(format!(
                "package.json {} 必须是 JSON 对象",
                seed_manifest.display()
            ));
            return (NodeProject::default(), diagnostics, true);
        }
        project
            .packages
            .push(parse_node_package(seed_manifest, &root_value));
        // A member used as a seed in a rootless pnpm workspace must not make
        // its own unrelated `workspaces` field redefine the enclosing root.
        if root_manifest
            .as_ref()
            .is_some_and(|manifest| manifest == seed_manifest)
        {
            patterns.extend(json_workspace_patterns(root_value.get("workspaces")));
        }
    }
    let workspace_path =
        pnpm_workspace.or_else(|| find_upwards_named(&root_dir, "pnpm-workspace.yaml"));
    if let Some(workspace_path) = workspace_path {
        match read_limited(&workspace_path, MANIFEST_LIMIT) {
            Ok(text) => patterns.extend(parse_pnpm_workspace_patterns(&text)),
            Err(error) => {
                diagnostics.push(format!("读取 pnpm-workspace.yaml 失败：{error}"));
                incomplete = true;
            }
        }
    }
    let mut seen_roots = project
        .packages
        .iter()
        .map(|package| lexical_normalize(&package.root))
        .collect::<HashSet<_>>();
    seen_roots.insert(lexical_normalize(&root_dir));
    let mut excluded_patterns = Vec::new();
    patterns.retain(|pattern| {
        if let Some(excluded) = pattern.strip_prefix('!') {
            excluded_patterns.push(excluded.to_owned());
            false
        } else {
            true
        }
    });
    for pattern in patterns {
        if cancelled(cancelled_flag) {
            diagnostics.push("JavaScript 工作区扫描已取消".to_owned());
            incomplete = true;
            break;
        }
        let (paths, scan_incomplete) = expand_project_pattern(
            &root_dir,
            &pattern,
            "package.json",
            Some(cancelled_flag),
            &mut project.watch_paths,
        );
        incomplete |= scan_incomplete;
        if scan_incomplete {
            diagnostics.push("JavaScript 工作区路径扫描未完成，结果不完整".to_owned());
        }
        for package_root in paths {
            let normalized = lexical_normalize(&package_root);
            if !seen_roots.insert(normalized) {
                continue;
            }
            let relative = package_root
                .strip_prefix(&root_dir)
                .unwrap_or(&package_root)
                .to_string_lossy()
                .replace('\\', "/");
            if excluded_patterns
                .iter()
                .any(|pattern| wildcard_match(pattern, &relative))
            {
                continue;
            }
            if cancelled(cancelled_flag) {
                diagnostics.push("JavaScript 工作区扫描已取消".to_owned());
                incomplete = true;
                break;
            }
            let package_manifest = package_root.join("package.json");
            record_watch_path(&mut project.watch_paths, &package_root);
            let text = match read_limited(&package_manifest, MANIFEST_LIMIT) {
                Ok(text) => text,
                Err(error) => {
                    diagnostics.push(format!(
                        "读取 package.json {} 失败：{error}",
                        package_manifest.display()
                    ));
                    incomplete = true;
                    continue;
                }
            };
            let value: serde_json::Value = match serde_json::from_str(&text) {
                Ok(value) => value,
                Err(error) => {
                    diagnostics.push(format!(
                        "解析 package.json {} 失败：{error}",
                        package_manifest.display()
                    ));
                    incomplete = true;
                    continue;
                }
            };
            if !value.is_object() {
                diagnostics.push(format!(
                    "package.json {} 必须是 JSON 对象",
                    package_manifest.display()
                ));
                incomplete = true;
                continue;
            }
            project
                .packages
                .push(parse_node_package(&package_manifest, &value));
        }
    }
    let _ = pnpm;
    (project, diagnostics, incomplete)
}

fn node_context(args: &[String], pnpm: bool) -> (String, Option<NodeValueKind>) {
    let args = split_command_args(args);
    let mut command = String::new();
    let mut command_index = args.len();
    let mut index = 0usize;
    while index < args.len() {
        let token = &args[index];
        if token == "--filter" || token == "--workspace" {
            if index + 1 >= args.len() {
                return (command, Some(NodeValueKind::Workspace));
            }
            index += 2;
            continue;
        }
        if token == "-w" || token == "--workspace-root" {
            index += 1;
            continue;
        }
        if token == "--dir" || token == "-C" || token == "--prefix" {
            index = (index + 2).min(args.len());
            continue;
        }
        if token.starts_with('-') {
            index += 1;
            continue;
        }
        command = token.clone();
        command_index = index;
        break;
    }
    let mut active = None;
    if command.is_empty() {
        return (command, active);
    }
    index = command_index + 1;
    while index < args.len() {
        let token = &args[index];
        if token == "--filter" || token == "--workspace" {
            if index + 1 >= args.len() {
                active = Some(NodeValueKind::Workspace);
                break;
            }
            active = None;
            index += 2;
            continue;
        }
        if token == "-w" || token == "--workspace-root" {
            index += 1;
            continue;
        }
        if token == "--" {
            break;
        }
        index += 1;
    }
    if command.eq_ignore_ascii_case("run") || command.eq_ignore_ascii_case("run-script") {
        active = Some(NodeValueKind::Script);
    } else if matches!(
        command.to_ascii_lowercase().as_str(),
        "install" | "i" | "add" | "update" | "uninstall" | "remove" | "rm"
    ) {
        active = Some(NodeValueKind::Dependency);
    }
    let _ = pnpm;
    (command, active)
}

fn node_workspace_selectors(args: &[String], pnpm: bool) -> Vec<String> {
    let args = split_command_args(args);
    let mut selectors = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        let token = &args[index];
        let takes_selector = token == "--filter"
            || token.starts_with("--filter=")
            || (!pnpm && (token == "--workspace" || token == "-w"))
            || (!pnpm && token.starts_with("--workspace="));
        if token.starts_with("--filter=") || (!pnpm && token.starts_with("--workspace=")) {
            if let Some((_, value)) = token.split_once('=')
                && !value.is_empty()
            {
                selectors.push(value.to_owned());
            }
            index += 1;
        } else if takes_selector {
            if let Some(value) = args.get(index + 1)
                && !value.is_empty()
            {
                selectors.push(value.clone());
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    selectors
}

fn node_selector_core(selector: &str) -> String {
    selector
        .trim()
        .trim_matches(['"', '\''])
        .trim_start_matches("...")
        .trim_end_matches("...")
        .to_owned()
}

fn node_selector_matches(
    package: &NodePackage,
    selector: &str,
    project_root: Option<&Path>,
) -> bool {
    let selector = node_selector_core(selector);
    if selector.is_empty() {
        return false;
    }
    if package.name.eq_ignore_ascii_case(&selector) || wildcard_match(&selector, &package.name) {
        return true;
    }
    let Some(project_root) = project_root else {
        return false;
    };
    let relative = package
        .root
        .strip_prefix(project_root)
        .unwrap_or(&package.root)
        .to_string_lossy()
        .replace('\\', "/");
    selector == relative
        || selector == format!("./{relative}")
        || wildcard_match(&selector, &relative)
}

fn selected_node_packages<'a>(
    project: &'a NodeProject,
    selectors: &[String],
    project_root: Option<&Path>,
) -> Vec<&'a NodePackage> {
    if selectors.is_empty() {
        return project.packages.iter().collect();
    }
    project
        .packages
        .iter()
        .filter(|package| {
            selectors
                .iter()
                .any(|selector| node_selector_matches(package, selector, project_root))
        })
        .collect()
}

fn node_effective_cwd(query: &ProjectQuery) -> PathBuf {
    let mut cwd = query.cwd.clone();
    let args = split_command_args(&query.args);
    let mut index = 0usize;
    while index < args.len() {
        let token = &args[index];
        if matches!(token.as_str(), "--dir" | "-C" | "--prefix")
            && let Some(value) = args.get(index + 1)
        {
            cwd = resolve_path(&cwd, value);
            index += 2;
            continue;
        }
        let value = token
            .strip_prefix("--dir=")
            .or_else(|| token.strip_prefix("--prefix="));
        if let Some(value) = value {
            cwd = resolve_path(&cwd, value);
        }
        index += 1;
    }
    cwd
}

fn collect_node(query: &ProjectQuery, cancelled_flag: &AtomicBool, pnpm: bool) -> ProviderResult {
    let (project, diagnostics, incomplete) = load_node_project(query, pnpm, cancelled_flag);
    if project.packages.is_empty() && diagnostics.is_empty() {
        let result = ProviderResult {
            watch_paths: project.watch_paths,
            ..ProviderResult::default()
        };
        return result.finish();
    }
    let (_, active) = node_context(&query.args, pnpm);
    let active = provider_scope(query, if pnpm { "pnpm" } else { "npm" })
        .and_then(|scope| match scope.to_ascii_lowercase().as_str() {
            "scripts" => Some(NodeValueKind::Script),
            "workspaces" => Some(NodeValueKind::Workspace),
            "dependencies" => Some(NodeValueKind::Dependency),
            _ => None,
        })
        .or(active);
    let mut result = ProviderResult {
        diagnostics,
        incomplete,
        project_root: node_project_root(query),
        ..ProviderResult::default()
    };
    node_watch_paths(&project, &mut result);
    let selectors = node_workspace_selectors(&query.args, pnpm);
    let selected_packages =
        selected_node_packages(&project, &selectors, result.project_root.as_deref());
    if cancelled(cancelled_flag) {
        result.incomplete = true;
        result
            .diagnostics
            .push("JavaScript 项目查询已取消".to_owned());
        return result.finish();
    }
    match active {
        Some(NodeValueKind::Workspace) => {
            for package in &selected_packages {
                if !package.name.is_empty() {
                    add_filtered(
                        &mut result,
                        package.name.clone(),
                        "工作区包",
                        CandidateKind::Value,
                        "node.workspace",
                        &query.prefix,
                    );
                }
            }
        }
        Some(NodeValueKind::Script) => {
            let source = if pnpm { "pnpm.script" } else { "npm.script" };
            for package in &selected_packages {
                for script in &package.scripts {
                    add_filtered(
                        &mut result,
                        script.clone(),
                        "package.json 脚本",
                        CandidateKind::Value,
                        source,
                        &query.prefix,
                    );
                }
            }
        }
        Some(NodeValueKind::Dependency) => {
            let source = if pnpm {
                "pnpm.dependency"
            } else {
                "npm.dependency"
            };
            for package in &selected_packages {
                for dependency in &package.dependencies {
                    let description = if package.local_dependencies.contains(dependency) {
                        "项目本地依赖"
                    } else {
                        "package.json 依赖"
                    };
                    add_filtered(
                        &mut result,
                        dependency.clone(),
                        description,
                        CandidateKind::Value,
                        source,
                        &query.prefix,
                    );
                }
            }
        }
        None => {}
    }
    result.finish()
}

fn node_watch_paths(project: &NodeProject, result: &mut ProviderResult) {
    if let Some(root) = result.project_root.clone() {
        add_watch_path(result, &root);
    }
    for path in &project.watch_paths {
        add_watch_path(result, path);
    }
    for package in &project.packages {
        add_watch_path(result, &package.root);
    }
}

fn node_project_root(query: &ProjectQuery) -> Option<PathBuf> {
    let cwd = node_effective_cwd(query);
    find_upwards_named(&cwd, "pnpm-workspace.yaml")
        .or_else(|| node_workspace_manifest(&cwd))
        .or_else(|| find_upwards_named(&cwd, "package.json"))
        .and_then(|manifest| manifest.parent().map(lexical_normalize))
}

pub fn project_actions(cwd:&Path, query:&str)->Vec<ProviderCandidate>{
    let query=query.trim().to_lowercase();let mut actions=Vec::new();
    let matches=|words:&[&str]|query.is_empty()||words.iter().any(|word|word.to_lowercase().contains(&query)||query.contains(&word.to_lowercase()));
    if let Some(path)=find_upwards(cwd,"package.json") && let Ok(text)=read_limited(&path,MANIFEST_LIMIT) && let Ok(value)=serde_json::from_str::<serde_json::Value>(&text) && let Some(scripts)=value.get("scripts").and_then(serde_json::Value::as_object) {
        for name in scripts.keys(){let words=if name.contains("test"){vec!["测试","运行测试"]}else if name.contains("dev")||name.contains("start"){vec!["开发服务","启动服务"]}else{vec!["运行脚本"]};if matches(&words){actions.push(ProviderCandidate{value:format!("npm run {name}"),description:format!("package.json 脚本 · {}",words[0]),kind:CandidateKind::Command,source:"project.action.node".into()});}}
    }
    if find_upwards(cwd,"Cargo.toml").is_some() && matches(&["测试","运行测试"]){actions.push(ProviderCandidate{value:"cargo test".into(),description:"当前 Rust 项目测试".into(),kind:CandidateKind::Command,source:"project.action.cargo".into()});}
    if find_upwards(cwd,"go.mod").is_some() && matches(&["测试","运行测试"]){actions.push(ProviderCandidate{value:"go test ./...".into(),description:"当前 Go 模块测试".into(),kind:CandidateKind::Command,source:"project.action.go".into()});}
    if find_upwards(cwd,"pyproject.toml").is_some() && matches(&["测试","运行测试"]){actions.push(ProviderCandidate{value:"uv run pytest".into(),description:"当前 Python 项目测试".into(),kind:CandidateKind::Command,source:"project.action.python".into()});}
    if let Some(path)=["compose.yaml","compose.yml","docker-compose.yaml","docker-compose.yml"].iter().find_map(|name|find_upwards(cwd,name)) && let Ok(text)=read_limited(&path,MANIFEST_LIMIT) {
        let mut in_services=false;for line in text.lines(){if line.trim()=="services:"{in_services=true;continue}if in_services&&!line.starts_with(' ')&&!line.trim().is_empty(){in_services=false}if in_services&&line.starts_with("  ")&&!line.starts_with("    ")&&line.trim().ends_with(':'){let service=line.trim().trim_end_matches(':');if matches(&["日志","服务日志","查看日志"]){actions.push(ProviderCandidate{value:format!("docker compose logs -f {service}"),description:"当前 Compose 服务日志".into(),kind:CandidateKind::Command,source:"project.action.compose".into()});}}}
    }
    actions
}

fn active_scope<'a>(query: &'a ProjectQuery, family: &str) -> &'a str {
    provider_scope(query, family).unwrap_or("")
}

fn run_bounded_query(query:&ProjectQuery, program:&str, args:&[&str], cancelled_flag:&AtomicBool)->Result<Vec<String>,String>{
    if cancelled(cancelled_flag){return Err("查询已取消".into())}
    let output_path=env::temp_dir().join(format!("blueberry-query-{}.txt",uuid::Uuid::new_v4()));
    let mut output=File::create(&output_path).map_err(|e|e.to_string())?;
    let stdout=output.try_clone().map_err(|e|e.to_string())?;
    let mut command=Command::new(program);command.args(args).current_dir(&query.cwd).env_clear().envs(&query.environment).stdin(Stdio::null()).stdout(Stdio::from(stdout)).stderr(Stdio::null());
    #[cfg(windows)] command.creation_flags(CREATE_NO_WINDOW);
    let mut child=match command.spawn(){Ok(child)=>child,Err(error)=>{let _=fs::remove_file(&output_path);return Err(error.to_string())}};
    let deadline=Instant::now()+Duration::from_secs(2);let status=loop{if cancelled(cancelled_flag)||Instant::now()>=deadline{let _=child.kill();let _=child.wait();let _=fs::remove_file(&output_path);return Err(if cancelled(cancelled_flag){"查询已取消"}else{"查询超时"}.into())}match child.try_wait(){Ok(Some(status))=>break status,Ok(None)=>std::thread::sleep(Duration::from_millis(5)),Err(error)=>{let _=child.kill();let _=fs::remove_file(&output_path);return Err(error.to_string())}}};
    if !status.success(){let _=fs::remove_file(&output_path);return Err(format!("命令退出码 {}",status.code().unwrap_or(-1)))}
    output.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;let mut bytes=Vec::new();output.take(2*1024*1024+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;let _=fs::remove_file(&output_path);if bytes.len()>2*1024*1024{return Err("查询输出超过 2 MiB".into())}
    Ok(String::from_utf8_lossy(&bytes).lines().map(str::trim).filter(|line|!line.is_empty()).map(str::to_owned).collect())
}

fn add_values<I>(result: &mut ProviderResult, values: I, description: &str, source: &str, prefix: &str)
where
    I: IntoIterator,
    I::Item: Into<String>,
{
    for value in values {
        add_filtered(result, value.into(), description, CandidateKind::Value, source, prefix);
    }
}

fn collect_python(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    if active_scope(query,"python")=="dependencies" || matches!(executable_stem(&query.command).as_str(),"pip"|"pip3") {
        if let Some(root)=query.environment.get("VIRTUAL_ENV").or_else(||query.environment.get("CONDA_PREFIX")) {
            let root=PathBuf::from(root);
            let candidates=[root.join("Lib/site-packages"),root.join("lib")];
            for dir in candidates { add_watch_path(&mut result,&dir); if let Ok(entries)=fs::read_dir(dir){let packages=entries.flatten().filter_map(|e|{let name=e.file_name().to_string_lossy().into_owned();name.strip_suffix(".dist-info").or_else(||name.strip_suffix(".egg-info")).map(|n|n.split('-').next().unwrap_or(n).replace('_',"-"))});add_values(&mut result,packages,"当前 Python 环境已安装包","python.installed",&query.prefix);} }
        }
    }
    let Some(manifest) = find_upwards(&query.cwd, "pyproject.toml") else { return result };
    result.project_root = manifest.parent().map(Path::to_path_buf);
    add_watch_path(&mut result, &manifest);
    if cancelled(cancelled_flag) { result.incomplete = true; return result; }
    if let Ok(text) = read_limited(&manifest, MANIFEST_LIMIT) {
        let scope = query.provider.as_deref().unwrap_or("");
        if scope.ends_with("scripts") || query.args.iter().any(|a| a == "run") {
            let Ok(value) = text.parse::<toml::Value>() else { return result.finish() };
            let scripts = value.get("project").and_then(|v| v.get("scripts")).and_then(toml::Value::as_table)
                .into_iter().flat_map(|t| t.keys().cloned())
                .chain(value.get("tool").and_then(|v| v.get("poetry")).and_then(|v| v.get("scripts")).and_then(toml::Value::as_table).into_iter().flat_map(|t| t.keys().cloned()));
            add_values(&mut result, scripts, "Python 项目脚本", "python.script", &query.prefix);
        } else {
            let Ok(value) = text.parse::<toml::Value>() else { return result.finish() };
            let dependencies = value.get("project").and_then(|v| v.get("dependencies")).and_then(toml::Value::as_array)
                .into_iter().flatten().filter_map(toml::Value::as_str).filter_map(|s| s.split([' ', '<', '>', '=', '[', ';']).next()).map(str::to_owned);
            add_values(&mut result, dependencies, "pyproject.toml 依赖", "python.dependency", &query.prefix);
        }
    }
    result.finish()
}

fn collect_conda(query: &ProjectQuery, _: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let mut roots = BTreeSet::new();
    for key in ["CONDA_PREFIX", "CONDA_ROOT", "MAMBA_ROOT_PREFIX"] {
        if let Some(value) = query.environment.get(key).filter(|v| !v.is_empty()) { roots.insert(PathBuf::from(value)); }
    }
    let mut envs = BTreeSet::new();
    for root in roots {
        if let Some(name) = root.file_name().and_then(|v| v.to_str()) { envs.insert(name.to_owned()); }
        let env_dir = root.join("envs");
        add_watch_path(&mut result, &env_dir);
        if let Ok(entries) = fs::read_dir(env_dir) {
            envs.extend(entries.flatten().filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().into_string().ok()));
        }
    }
    if active_scope(query,"conda")=="packages" {
        if let Some(prefix)=query.environment.get("CONDA_PREFIX") {
            let dir=PathBuf::from(prefix).join("conda-meta");add_watch_path(&mut result,&dir);
            if let Ok(entries)=fs::read_dir(dir){let packages=entries.flatten().filter_map(|e|e.file_name().into_string().ok()).filter_map(|n|n.strip_suffix(".json").map(str::to_owned)).map(|n|n.rsplitn(3,'-').last().unwrap_or(&n).to_owned());add_values(&mut result,packages,"当前 Conda 环境已安装包","conda.package",&query.prefix);}
        }
    } else {
        add_values(&mut result, envs, "本机 Conda 环境", "conda.environment", &query.prefix);
    }
    result.finish()
}

fn collect_rustup(query: &ProjectQuery, _: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let root = query.environment.get("RUSTUP_HOME").map(PathBuf::from).or_else(|| query.environment.get("USERPROFILE").map(|p| PathBuf::from(p).join(".rustup")));
    let Some(root) = root else { return result };
    let folder = match active_scope(query, "rustup") { "targets" => "toolchains", "components" => "toolchains", _ => "toolchains" };
    let dir = root.join(folder); add_watch_path(&mut result, &dir);
    if let Ok(entries) = fs::read_dir(dir) { add_values(&mut result, entries.flatten().filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().into_string().ok()), "已安装 Rust 工具链", "rustup.toolchain", &query.prefix); }
    result.finish()
}

fn collect_go(query: &ProjectQuery, _: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let manifest = find_upwards(&query.cwd, "go.mod").or_else(|| find_upwards(&query.cwd, "go.work"));
    result.project_root = manifest.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf);
    if let Some(path) = manifest { add_watch_path(&mut result, &path); }
    let root = result.project_root.clone().unwrap_or_else(|| query.cwd.clone());
    if let Ok(entries) = fs::read_dir(&root) {
        let values = entries.flatten().filter_map(|e| {
            let p=e.path();
            if p.is_dir() { Some(format!("./{}", e.file_name().to_string_lossy())) }
            else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("go")) { Some(e.file_name().to_string_lossy().into_owned()) } else { None }
        });
        add_values(&mut result, values, "当前 Go 项目", "go.project", &query.prefix);
    }
    result.finish()
}

fn collect_dotnet(query: &ProjectQuery, _: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let mut current = Some(query.cwd.as_path());
    let mut root = query.cwd.clone();
    while let Some(dir) = current {
        let has_solution = fs::read_dir(dir).ok().is_some_and(|mut entries| {
            entries.any(|entry| {
                entry.ok().is_some_and(|entry| {
                    entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("sln"))
                })
            })
        });
        if has_solution {
            root = dir.to_path_buf();
            break;
        }
        current = dir.parent();
    }
    result.project_root=Some(root.clone()); add_watch_path(&mut result, &root);
    if let Ok(entries)=fs::read_dir(root) { add_values(&mut result, entries.flatten().filter_map(|e| { let p=e.path(); let ext=p.extension()?.to_str()?; matches!(ext.to_ascii_lowercase().as_str(), "sln"|"csproj"|"fsproj"|"vbproj").then(|| e.file_name().to_string_lossy().into_owned()) }), "解决方案或项目", "dotnet.project", &query.prefix); }
    result.finish()
}

fn collect_cmake(query: &ProjectQuery, _: &AtomicBool) -> ProviderResult {
    let mut result=ProviderResult::default();
    let Some(path)=find_upwards(&query.cwd,"CMakePresets.json").or_else(|| find_upwards(&query.cwd,"CMakeUserPresets.json")) else { return result };
    result.project_root=path.parent().map(Path::to_path_buf); add_watch_path(&mut result,&path);
    if let Ok(text)=read_limited(&path,MANIFEST_LIMIT) && let Ok(json)=serde_json::from_str::<serde_json::Value>(&text) {
        let key=match active_scope(query,"cmake") { "build_presets"=>"buildPresets", "test_presets"=>"testPresets", _=>"configurePresets" };
        let values=json.get(key).and_then(|v|v.as_array()).into_iter().flatten().filter_map(|v|v.get("name")?.as_str()).map(str::to_owned);
        add_values(&mut result,values,"CMake 预设","cmake.preset",&query.prefix);
    }
    result.finish()
}

fn quoted_name(line: &str, keyword: &str) -> Option<String> {
    let rest = line.trim().strip_prefix(keyword)?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value = rest[quote.len_utf8()..].split(quote).next()?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn simple_yaml_keys(text: &str, section: &str) -> Vec<String> {
    let mut in_section = false;
    let mut values = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == format!("{section}:") {
            in_section = true;
            continue;
        }
        if in_section && !line.starts_with([' ', '\t']) && !trimmed.is_empty() {
            in_section = false;
        }
        if in_section
            && line.starts_with("  ")
            && !line.starts_with("    ")
            && trimmed.ends_with(':')
        {
            let value = trimmed.trim_end_matches(':').trim_matches(['"', '\'']);
            if !value.is_empty() {
                values.push(value.to_owned());
            }
        }
    }
    values
}

fn collect_dev_tools(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    if cancelled(cancelled_flag) {
        result.incomplete = true;
        return result;
    }
    let family = query
        .provider
        .as_deref()
        .and_then(|provider| provider.split('.').next())
        .map(str::to_owned)
        .unwrap_or_else(|| executable_stem(&query.command));
    let mut values = BTreeSet::new();
    let mut source = "dev.project";
    let mut description = "当前项目候选";
    match family.as_str() {
        "deno" => {
            if let Some(path) = find_upwards(&query.cwd, "deno.json")
                .or_else(|| find_upwards(&query.cwd, "deno.jsonc"))
            {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        values.extend(
                            json.get("tasks")
                                .and_then(serde_json::Value::as_object)
                                .into_iter()
                                .flat_map(|tasks| tasks.keys().cloned()),
                        );
                    }
                }
            }
            source = "deno.task";
            description = "deno.json 项目任务";
        }
        "maven" => {
            if let Some(path) = find_upwards(&query.cwd, "pom.xml") {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    for tag in ["module", "id"] {
                        let open = format!("<{tag}>");
                        let close = format!("</{tag}>");
                        for tail in text.split(&open).skip(1) {
                            if let Some((value, _)) = tail.split_once(&close) {
                                let value = value.trim();
                                if !value.is_empty() && !value.contains('<') {
                                    values.insert(value.to_owned());
                                }
                            }
                        }
                    }
                }
            }
            source = "maven.module";
            description = "Maven 模块或 profile";
        }
        "gradle" => {
            let path = ["build.gradle.kts", "build.gradle"]
                .iter()
                .find_map(|name| find_upwards(&query.cwd, name));
            if let Some(path) = path {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    for line in text.lines() {
                        let trimmed = line.trim();
                        if let Some(value) = trimmed.strip_prefix("task ") {
                            let value = value.split([' ', '(', '{']).next().unwrap_or("");
                            if !value.is_empty() { values.insert(value.into()); }
                        }
                        for prefix in ["tasks.register(\"", "tasks.named(\""] {
                            if let Some(value) = trimmed.strip_prefix(prefix).and_then(|v| v.split('"').next()) {
                                values.insert(value.into());
                            }
                        }
                    }
                }
            }
            source = "gradle.task";
            description = "Gradle 文件中声明的任务";
        }
        "make" => {
            if let Some(path) = ["Makefile", "makefile", "GNUmakefile"]
                .iter().find_map(|name| find_upwards(&query.cwd, name))
            {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    for line in text.lines().filter(|line| !line.chars().next().is_some_and(|c| [' ', '\t', '.', '#'].contains(&c))) {
                        if let Some((target, _)) = line.split_once(':') {
                            let target = target.trim();
                            if !target.is_empty() && !target.contains(['%', '=', '$']) {
                                values.insert(target.into());
                            }
                        }
                    }
                }
            }
            source = "make.target";
            description = "Makefile 构建目标";
        }
        "ninja" => {
            if let Some(path) = find_upwards(&query.cwd, "build.ninja") {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    for line in text.lines() {
                        if let Some(target) = line.trim().strip_prefix("build ").and_then(|v| v.split(':').next()) {
                            values.extend(target.split_whitespace().map(str::to_owned));
                        }
                    }
                }
            }
            source = "ninja.target";
            description = "Ninja 构建目标";
        }
        "just" => {
            if let Some(path) = ["Justfile", "justfile"]
                .iter().find_map(|name| find_upwards(&query.cwd, name))
            {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    for line in text.lines().filter(|line| !line.chars().next().is_some_and(|c| [' ', '\t', '#', '@'].contains(&c))) {
                        if let Some((recipe, _)) = line.split_once(':') {
                            let recipe = recipe.split_whitespace().next().unwrap_or("");
                            if !recipe.is_empty() && !recipe.contains('=') { values.insert(recipe.into()); }
                        }
                    }
                }
            }
            source = "just.recipe";
            description = "Justfile 配方";
        }
        "task" => {
            if let Some(path) = ["Taskfile.yml", "Taskfile.yaml", "taskfile.yml", "taskfile.yaml"]
                .iter().find_map(|name| find_upwards(&query.cwd, name))
            {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    values.extend(simple_yaml_keys(&text, "tasks"));
                }
            }
            source = "task.task";
            description = "Taskfile 任务";
        }
        "terraform" | "tofu" => {
            let root = query.cwd.clone();
            result.project_root = Some(root.clone());
            add_watch_path(&mut result, &root);
            if let Ok(entries) = fs::read_dir(&root) {
                for path in entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext == "tf")) {
                    add_watch_path(&mut result, &path);
                    if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                        for line in text.lines() {
                            if let Some(value) = quoted_name(line, "variable").or_else(|| quoted_name(line, "module")) {
                                values.insert(value);
                            }
                        }
                    }
                }
            }
            source = if family == "tofu" { "tofu.symbol" } else { "terraform.symbol" };
            description = "当前基础设施项目的模块或变量";
        }
        "ansible" => {
            let explicit = query.environment.get("ANSIBLE_INVENTORY").map(PathBuf::from);
            let path = explicit.filter(|path| path.is_file()).or_else(|| {
                ["inventory", "hosts", "inventory.yml", "inventory.yaml"]
                    .iter().find_map(|name| find_upwards(&query.cwd, name))
            });
            if let Some(path) = path {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    for line in text.lines() {
                        let trimmed = line.trim();
                        if trimmed.is_empty() || trimmed.chars().next().is_some_and(|c| ['#', '[', '-', '{'].contains(&c)) || trimmed.ends_with(':') { continue; }
                        let host = trimmed.split_whitespace().next().unwrap_or("");
                        if !host.is_empty() && !host.contains('=') { values.insert(host.into()); }
                    }
                }
            }
            source = "ansible.inventory";
            description = "静态 Ansible inventory 主机";
        }
        _ => {}
    }
    add_values(&mut result, values, description, source, &query.prefix);
    result.finish()
}

fn collect_windows_tools(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let provider = query.provider.as_deref().unwrap_or("");
    match provider {
        "windows.scoop_packages" => {
            let root = query
                .environment
                .get("SCOOP")
                .map(PathBuf::from)
                .or_else(|| query.environment.get("USERPROFILE").map(|p| PathBuf::from(p).join("scoop")));
            if let Some(dir) = root.map(|root| root.join("apps")) {
                add_watch_path(&mut result, &dir);
                if let Ok(entries) = fs::read_dir(dir) {
                    add_values(&mut result, entries.flatten().filter(|entry| entry.path().is_dir()).filter_map(|entry| entry.file_name().into_string().ok()), "已安装 Scoop 软件包", "windows.scoop.package", &query.prefix);
                }
            }
        }
        "windows.choco_packages" => {
            let root = query.environment.get("ChocolateyInstall").map(PathBuf::from)
                .or_else(|| query.environment.get("ProgramData").map(|p| PathBuf::from(p).join("chocolatey")));
            if let Some(dir) = root.map(|root| root.join("lib")) {
                add_watch_path(&mut result, &dir);
                if let Ok(entries) = fs::read_dir(dir) {
                    add_values(&mut result, entries.flatten().filter(|entry| entry.path().is_dir()).filter_map(|entry| entry.file_name().into_string().ok()).map(|name| name.trim_end_matches(".install").to_owned()), "已安装 Chocolatey 软件包", "windows.choco.package", &query.prefix);
                }
            }
        }
        "powershell.modules" => {
            if let Some(paths) = query.environment.get("PSModulePath") {
                for dir in env::split_paths(paths) {
                    add_watch_path(&mut result, &dir);
                    if let Ok(entries) = fs::read_dir(dir) {
                        add_values(&mut result, entries.flatten().filter(|entry| entry.path().is_dir()).filter_map(|entry| entry.file_name().into_string().ok()), "本机 PowerShell 模块", "powershell.module", &query.prefix);
                    }
                }
            }
        }
        "windows.wsl_distros" => match run_bounded_query(query, "wsl", &["-l", "-q"], cancelled_flag) {
            Ok(values) => add_values(&mut result, values.into_iter().map(|value| value.replace('\0', "").trim().to_owned()).filter(|value| !value.is_empty()), "已安装 WSL 发行版", "windows.wsl.distro", &query.prefix),
            Err(error) => result.diagnostics.push(format!("读取 WSL 发行版失败：{error}")),
        },
        "windows.processes" => match run_bounded_query(query, "tasklist", &["/fo", "csv", "/nh"], cancelled_flag) {
            Ok(values) => add_values(&mut result, values.into_iter().filter_map(|line| line.trim_matches('"').split("\",\"").next().map(str::to_owned)), "本机进程映像名称", "windows.process", &query.prefix),
            Err(error) => result.diagnostics.push(format!("读取进程列表失败：{error}")),
        },
        "windows.services" => match run_bounded_query(query, "sc", &["query", "state=", "all"], cancelled_flag) {
            Ok(values) => add_values(&mut result, values.into_iter().filter_map(|line| line.strip_prefix("SERVICE_NAME:").map(str::trim).map(str::to_owned)), "本机 Windows 服务", "windows.service", &query.prefix),
            Err(error) => result.diagnostics.push(format!("读取服务列表失败：{error}")),
        },
        _ => {}
    }
    result.finish()
}

fn ini_sections(path: &Path, result: &mut ProviderResult) -> Vec<String> {
    add_watch_path(result, path);
    read_limited(path, MANIFEST_LIMIT)
        .map(|text| {
            text.lines()
                .filter_map(|line| {
                    let section = line.trim().strip_prefix('[')?.strip_suffix(']')?.trim();
                    let section = section.strip_prefix("profile ").unwrap_or(section).trim();
                    (!section.is_empty()).then(|| section.to_owned())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn cloud_home(query: &ProjectQuery) -> Option<PathBuf> {
    query
        .environment
        .get("USERPROFILE")
        .or_else(|| query.environment.get("HOME"))
        .map(PathBuf::from)
}

fn remote_values(
    result: &mut ProviderResult,
    query: &ProjectQuery,
    program: &str,
    args: &[&str],
    description: &str,
    source: &str,
    cancelled_flag: &AtomicBool,
) {
    match run_bounded_query(query, program, args, cancelled_flag) {
        Ok(values) => add_values(result, values, description, source, &query.prefix),
        Err(error) => {
            result.incomplete = true;
            result
                .diagnostics
                .push(format!("远程资源读取失败（{program}）：{error}"));
        }
    }
}

fn collect_cloud_data(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let provider = query.provider.as_deref().unwrap_or("");
    let home = cloud_home(query);
    let remote = query
        .environment
        .get("BLUEBERRY_REMOTE_REQUESTED")
        .is_some_and(|value| value == "1");

    match provider {
        "cloud.aws_profiles" => {
            let mut values = Vec::new();
            if let Some(home) = &home {
                values.extend(ini_sections(&home.join(".aws/config"), &mut result));
                values.extend(ini_sections(&home.join(".aws/credentials"), &mut result));
            }
            add_values(&mut result, values, "本机 AWS profile", "cloud.aws.profile", &query.prefix);
        }
        "cloud.azure_profiles" => {
            if let Some(home) = &home {
                let path = home.join(".azure/azureProfile.json");
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, 2 * MANIFEST_LIMIT)
                    && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
                {
                    let values = json.get("subscriptions").and_then(|v| v.as_array()).into_iter()
                        .flatten().filter_map(|item| item.get("name").and_then(|v| v.as_str()));
                    add_values(&mut result, values, "本机 Azure 订阅", "cloud.azure.profile", &query.prefix);
                }
            }
        }
        "cloud.gcloud_profiles" => {
            let config_root = query.environment.get("APPDATA")
                .map(|root| PathBuf::from(root).join("gcloud/configurations"))
                .or_else(|| home.as_ref().map(|root| root.join(".config/gcloud/configurations")));
            if let Some(root) = config_root {
                add_watch_path(&mut result, &root);
                if let Ok(entries) = fs::read_dir(root) {
                    let values = entries.flatten().filter_map(|entry| {
                        entry.file_name().to_str()?.strip_prefix("config_").map(str::to_owned)
                    });
                    add_values(&mut result, values, "本机 gcloud 配置", "cloud.gcloud.profile", &query.prefix);
                }
            }
        }
        "db.postgres" if !remote => {
            if let Some(home) = &home {
                let path = query.environment.get("APPDATA")
                    .map(|root| PathBuf::from(root).join("postgresql/.pg_service.conf"))
                    .filter(|path| path.is_file())
                    .unwrap_or_else(|| home.join(".pg_service.conf"));
                let values = ini_sections(&path, &mut result);
                add_values(&mut result, values, "本机 PostgreSQL 服务配置", "db.postgres.service", &query.prefix);
            }
        }
        "db.mysql" if !remote => {
            if let Some(home) = &home {
                let values = ini_sections(&home.join(".my.cnf"), &mut result);
                add_values(&mut result, values, "本机 MySQL 配置组", "db.mysql.profile", &query.prefix);
            }
        }
        "cloud.aws_resources" if remote => remote_values(&mut result, query, "aws", &["s3api", "list-buckets", "--query", "Buckets[].Name", "--output", "text"], "AWS S3 bucket", "cloud.aws.remote", cancelled_flag),
        "cloud.azure_resources" if remote => remote_values(&mut result, query, "az", &["group", "list", "--query", "[].name", "-o", "tsv"], "Azure 资源组", "cloud.azure.remote", cancelled_flag),
        "cloud.gcloud_resources" if remote => remote_values(&mut result, query, "gcloud", &["projects", "list", "--format=value(projectId)"], "Google Cloud 项目", "cloud.gcloud.remote", cancelled_flag),
        "db.postgres" if remote => remote_values(&mut result, query, "psql", &["-Atc", "SELECT datname FROM pg_database WHERE datallowconn"], "PostgreSQL 数据库", "db.postgres.remote", cancelled_flag),
        "db.mysql" if remote => remote_values(&mut result, query, "mysql", &["-NBe", "SHOW DATABASES"], "MySQL 数据库", "db.mysql.remote", cancelled_flag),
        "db.redis" if remote => remote_values(&mut result, query, "redis-cli", &["--scan"], "Redis key", "db.redis.remote", cancelled_flag),
        "db.mongo" if remote => remote_values(&mut result, query, "mongosh", &["--quiet", "--eval", "db.getMongo().getDBNames().join('\\n')"], "MongoDB 数据库", "db.mongo.remote", cancelled_flag),
        _ => {}
    }
    result.finish()
}

fn collect_docker(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let mut result=ProviderResult::default();
    let remote=query.environment.get("DOCKER_HOST").is_some_and(|host|!host.starts_with("npipe://")&&!host.starts_with("unix://")&&!host.contains("localhost")&&!host.contains("127.0.0.1"));
    if remote && query.environment.get("BLUEBERRY_REMOTE_REQUESTED").is_some_and(|v|v=="1") {
        match run_bounded_query(query,"docker",&["ps","--format","{{.Names}}"],cancelled_flag){Ok(values)=>add_values(&mut result,values,"远程 Docker 容器","docker.remote.container",&query.prefix),Err(error)=>{result.incomplete=true;result.diagnostics.push(format!("Docker 资源读取失败：{error}"));}}
    }
    let names=["compose.yaml","compose.yml","docker-compose.yaml","docker-compose.yml"];
    let Some(path)=names.iter().find_map(|n|find_upwards(&query.cwd,n)) else { return result };
    result.project_root=path.parent().map(Path::to_path_buf); add_watch_path(&mut result,&path);
    if let Ok(text)=read_limited(&path,MANIFEST_LIMIT) {
        let mut in_services=false;
        let values=text.lines().filter_map(|line| { if line.trim()=="services:" {in_services=true;return None} if in_services && !line.starts_with(' ')&&!line.trim().is_empty(){in_services=false} if in_services { let trimmed=line.trim(); (line.starts_with("  ")&&!line.starts_with("    ")&&trimmed.ends_with(':')).then(||trimmed.trim_end_matches(':').to_owned()) } else {None} });
        add_values(&mut result,values,"Compose 服务","docker.compose.service",&query.prefix);
    }
    result.finish()
}

fn collect_kubeconfig(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let mut result=ProviderResult::default();
    let path=query.environment.get("KUBECONFIG").and_then(|v|env::split_paths(v).next()).or_else(||query.environment.get("USERPROFILE").map(|p|PathBuf::from(p).join(".kube/config")));
    let Some(path)=path else{return result}; add_watch_path(&mut result,&path);
    if let Ok(text)=read_limited(&path,2*MANIFEST_LIMIT) {
        let mut wanted=false; let scope=active_scope(query,"kubectl");
        let values=text.lines().filter_map(|line| {let t=line.trim(); if t=="contexts:" {wanted=true;return None} if wanted&&t.ends_with(':')&&!line.starts_with(' '){wanted=false} if wanted&&t.starts_with("- name:"){Some(t.trim_start_matches("- name:").trim().to_owned())}else if scope=="contexts"&&t.starts_with("current-context:"){Some(t.trim_start_matches("current-context:").trim().to_owned())}else{None}});
        add_values(&mut result,values,"Kubernetes 上下文","kubectl.context",&query.prefix);
    }
    if query.environment.get("BLUEBERRY_REMOTE_REQUESTED").is_some_and(|v|v=="1") {
        let scope=active_scope(query,"kubectl");
        let args=if scope=="namespaces"{vec!["get","namespaces","-o","name"]}else{let kind=query.args.iter().find(|arg|matches!(arg.as_str(),"pods"|"pod"|"deployments"|"deployment"|"services"|"service"|"statefulsets"|"daemonsets"|"jobs")).map(String::as_str).unwrap_or("pods");vec!["get",kind,"-o","name"]};
        match run_bounded_query(query,"kubectl",&args,cancelled_flag){Ok(values)=>add_values(&mut result,values,"当前 Kubernetes 上下文资源","kubectl.remote",&query.prefix),Err(error)=>{result.incomplete=true;result.diagnostics.push(format!("Kubernetes 资源读取失败：{error}"));}}
    }
    result.finish()
}

fn collect_helm(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let mut result=ProviderResult::default();
    let mut current=Some(query.cwd.as_path());
    while let Some(dir)=current { let chart=dir.join("Chart.yaml"); if chart.is_file(){result.project_root=Some(dir.to_path_buf());add_watch_path(&mut result,&chart);add_filtered(&mut result,dir.to_string_lossy().into_owned(),"本地 Helm Chart",CandidateKind::Directory,"helm.chart",&query.prefix);break} current=dir.parent(); }
    if active_scope(query,"helm")=="releases" && query.environment.get("BLUEBERRY_REMOTE_REQUESTED").is_some_and(|v|v=="1") {match run_bounded_query(query,"helm",&["list","-q"],cancelled_flag){Ok(values)=>add_values(&mut result,values,"当前上下文 Helm release","helm.remote.release",&query.prefix),Err(error)=>{result.incomplete=true;result.diagnostics.push(format!("Helm release 读取失败：{error}"));}}}
    result.finish()
}

fn collect_ssh(query: &ProjectQuery, _: &AtomicBool) -> ProviderResult {
    let mut result=ProviderResult::default();
    let Some(profile)=query.environment.get("USERPROFILE") else{return result};
    let path=PathBuf::from(profile).join(".ssh/config"); add_watch_path(&mut result,&path);
    if let Ok(text)=read_limited(&path,MANIFEST_LIMIT) {
        let hosts=text.lines().filter_map(|line| {let t=line.trim(); let rest=t.strip_prefix("Host ").or_else(||t.strip_prefix("host "))?; Some(rest.split_whitespace().filter(|h|!h.contains(['*','?','!'])).map(str::to_owned).collect::<Vec<_>>())}).flatten();
        add_values(&mut result,hosts,"SSH 配置主机","ssh.host",&query.prefix);
    }
    result.finish()
}

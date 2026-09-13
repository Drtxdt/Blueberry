//! Command discovery and deterministic completion for the native host.
//!
//! Discovery is deliberately separate from completion. A [`Discovery`] owns
//! the directory iterators and advances them in small, bounded batches, while
//! [`CommandIndex`] is a cheap immutable snapshot that can be queried at any
//! point. This means a large PATH never blocks the first completion query and
//! a partially built index is still useful to the caller.

use crate::model::{Candidate, CandidateKind, Completion, InputContext, ShellCommand};
use crate::specs;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions, ReadDir};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CACHE_VERSION: u32 = 2;
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";
const MAX_RESULTS: usize = 1_000;
const DISCOVERY_BATCH_ENTRIES: usize = 256;
const DISCOVERY_BATCH_BUDGET: Duration = Duration::from_millis(4);
const MAX_ALIAS_DEPTH: usize = 8;
const MAX_SYMLINK_DEPTH: usize = 64;

/// An ordered command index. Entries are kept in discovery order, so the
/// first matching PATH entry wins both indexing and display order.
#[derive(Clone)]
pub struct CommandIndex {
    entries: Vec<IndexedCommand>,
    by_name: HashMap<String, usize>,
    context: CacheContext,
    context_valid: bool,
    complete: bool,
    /// Commands discovered from PATH. Keeping this baseline lets a session
    /// alias shadow an application temporarily and restores the application
    /// when the next complete session snapshot no longer contains that alias.
    base_entries: Vec<IndexedCommand>,
    session_commands: HashMap<String, IndexedCommand>,
    session_order: Vec<String>,
}

impl Default for CommandIndex {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            by_name: HashMap::new(),
            context: CacheContext::default(),
            context_valid: false,
            complete: true,
            base_entries: Vec::new(),
            session_commands: HashMap::new(),
            session_order: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct IndexedCommand {
    name: String,
    kind: CandidateKind,
    #[serde(default)]
    description: String,
    #[serde(default)]
    definition: String,
    #[serde(default)]
    executable_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct CacheContext {
    path: String,
    pathext: String,
    #[serde(default)]
    directories: Vec<DirectoryStamp>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct DirectoryStamp {
    path: String,
    /// Directory modification time in nanoseconds since the Unix epoch.
    /// `None` is also used for a missing directory and for a deliberately
    /// skipped remote PATH entry.
    modified: Option<u128>,
    /// A failed metadata access is distinct from an empty directory. It is
    /// retained in the context so a future cache validation cannot silently
    /// turn an access failure into a successful scan.
    #[serde(default = "default_accessible")]
    accessible: bool,
}

fn default_accessible() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    context: CacheContext,
    /// Only complete snapshots are persisted. The field is intentionally
    /// required for v2 files so a hand-written or truncated cache is rejected.
    complete: bool,
    commands: Vec<IndexedCommand>,
}

/// A resumable PATH discovery operation.
///
/// [`step`](Self::step) processes at most roughly 256 directory entries or
/// four milliseconds of work, whichever comes first. It never sleeps: the
/// host worker can wait on its condition variable between calls and can
/// cancel the scan at a batch boundary.
pub struct Discovery {
    index: CommandIndex,
    directories: Vec<PathBuf>,
    extensions: Vec<String>,
    directory_index: usize,
    current: Option<DirectoryScan>,
    access_error: bool,
    done: bool,
}

struct DirectoryScan {
    read_dir: ReadDir,
    found: Vec<FoundCommand>,
}

struct FoundCommand {
    extension_rank: usize,
    lower_name: String,
    stem: String,
    executable_path: PathBuf,
}

enum EntryInspection {
    Candidate(FoundCommand),
    Ignore,
    AccessError,
}

enum DirectoryStart {
    Opened,
    Skipped,
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LinkStatus {
    Local,
    Remote,
    Dangling,
    Inaccessible,
}

impl Discovery {
    /// Start discovery from an explicit PATH/PATHEXT snapshot.
    pub fn new(path: &OsStr, pathext: &OsStr) -> Self {
        // The cache context is deliberately captured before opening any
        // directory. A directory can change while a long scan is running;
        // using the pre-scan stamps makes that race invalidate the cache on
        // the next launch instead of recording a misleading post-scan state.
        let context = context_for_env(path, pathext);
        let directories = split_path(path);
        let extensions = parse_pathext(pathext);
        let index = CommandIndex {
            entries: Vec::new(),
            by_name: HashMap::new(),
            context,
            context_valid: true,
            complete: false,
            base_entries: Vec::new(),
            session_commands: HashMap::new(),
            session_order: Vec::new(),
        };
        Self {
            index,
            directories,
            extensions,
            directory_index: 0,
            current: None,
            access_error: false,
            done: false,
        }
    }

    /// Advance discovery by one bounded batch. Returns `true` after every
    /// PATH directory has been examined. An access error still ends the scan,
    /// but the resulting snapshot remains incomplete and cannot be cached.
    pub fn step(&mut self) -> bool {
        if self.done {
            return true;
        }

        let started = Instant::now();
        let mut processed = 0usize;
        loop {
            if processed >= DISCOVERY_BATCH_ENTRIES
                || (processed > 0 && started.elapsed() >= DISCOVERY_BATCH_BUDGET)
            {
                break;
            }

            if self.current.is_none() {
                match self.begin_next_directory() {
                    DirectoryStart::Opened => {}
                    DirectoryStart::Skipped => {
                        // A missing, remote, or inaccessible PATH entry is
                        // still a directory boundary. Counting it prevents a
                        // long run of skipped entries from escaping the
                        // batch budget.
                        processed += 1;
                        continue;
                    }
                    DirectoryStart::Finished => break,
                }
            }

            let Some(scan) = self.current.as_mut() else {
                break;
            };
            match scan.read_dir.next() {
                Some(Ok(entry)) => {
                    processed += 1;
                    match inspect_executable(&entry, &self.extensions) {
                        EntryInspection::Candidate(candidate) => scan.found.push(candidate),
                        EntryInspection::Ignore => {}
                        EntryInspection::AccessError => self.access_error = true,
                    }
                }
                Some(Err(error)) => {
                    processed += 1;
                    // A broken target is handled by inspect_executable. An
                    // iterator error means the directory could not be fully
                    // enumerated and therefore must make the snapshot
                    // incomplete.
                    let _ = error;
                    self.access_error = true;
                }
                None => {
                    // EOF is the boundary at which the directory's sorted
                    // candidates become part of the snapshot. Count that
                    // boundary as work as well as ordinary entries.
                    processed += 1;
                    self.finish_current_directory();
                }
            }
        }

        if self.current.is_none() && self.directory_index >= self.directories.len() {
            self.done = true;
            self.index.complete = !self.access_error;
        }
        self.done
    }

    /// Return a queryable snapshot of all entries found so far.
    pub fn snapshot(&self) -> CommandIndex {
        let mut snapshot = self.index.clone();
        // Discovery has no session overlay. Keeping the baseline synchronized
        // is what lets the host call replace_shell_commands on every batch.
        snapshot.base_entries = snapshot.entries.clone();
        snapshot.session_commands.clear();
        snapshot.session_order.clear();
        snapshot.complete = self.done && !self.access_error;
        snapshot
    }

    /// Whether the scan has ended and every directory was enumerated without
    /// an access error.
    pub fn is_complete(&self) -> bool {
        self.done && !self.access_error
    }

    fn begin_next_directory(&mut self) -> DirectoryStart {
        let Some(directory) = self.directories.get(self.directory_index).cloned() else {
            self.done = true;
            self.index.complete = !self.access_error;
            return DirectoryStart::Finished;
        };
        self.directory_index += 1;

        // A local PATH symlink is fine, but a symlink chain ending at a
        // UNC/remote target is intentionally ignored before read_dir can
        // block on the remote server.
        match symlink_target_status(&directory) {
            LinkStatus::Remote => return DirectoryStart::Skipped,
            LinkStatus::Inaccessible => {
                self.access_error = true;
                return DirectoryStart::Skipped;
            }
            LinkStatus::Local | LinkStatus::Dangling => {}
        }

        match fs::read_dir(&directory) {
            Ok(read_dir) => {
                self.current = Some(DirectoryScan {
                    read_dir,
                    found: Vec::new(),
                });
                DirectoryStart::Opened
            }
            Err(error) => {
                // A missing PATH component is ordinary and is treated as
                // examined. Permission and other enumeration failures are
                // recorded so the result cannot be mistaken for a full
                // scan and cached.
                if is_scan_access_error(&error) {
                    self.access_error = true;
                }
                DirectoryStart::Skipped
            }
        }
    }

    fn finish_current_directory(&mut self) {
        let Some(mut scan) = self.current.take() else {
            return;
        };
        scan.found.sort_by(|left, right| {
            left.extension_rank
                .cmp(&right.extension_rank)
                .then_with(|| left.lower_name.cmp(&right.lower_name))
                .then_with(|| {
                    left.stem
                        .to_ascii_lowercase()
                        .cmp(&right.stem.to_ascii_lowercase())
                })
        });
        for found in scan.found {
            self.index.insert_command(
                IndexedCommand {
                    name: found.stem,
                    kind: CandidateKind::Command,
                    description: String::new(),
                    definition: String::new(),
                    executable_path: Some(found.executable_path),
                },
                false,
            );
        }
    }
}

impl CommandIndex {
    /// Discover executable commands using the process PATH and PATHEXT.
    pub fn discover() -> Self {
        let path = env::var_os("PATH").unwrap_or_default();
        let pathext =
            env::var_os("PATHEXT").unwrap_or_else(|| OsStr::new(DEFAULT_PATHEXT).to_os_string());
        Self::discover_with_env(&path, &pathext)
    }

    /// Discover commands with an explicit environment snapshot. The scan is
    /// advanced to completion for this synchronous convenience API; callers
    /// that need incremental work should use [`Discovery`] directly.
    pub fn discover_with_env(path: &OsStr, pathext: &OsStr) -> Self {
        let mut discovery = Discovery::new(path, pathext);
        while !discovery.step() {}
        discovery.snapshot()
    }

    /// Load a cache only when it was written for the current PATH/PATHEXT.
    pub fn load(path: &Path) -> Result<Self> {
        let context = current_context();
        Self::load_with_context(path, context)
    }

    /// Load a cache against an explicit PATH/PATHEXT snapshot.
    pub fn load_with_env(path: &Path, path_env: &OsStr, pathext: &OsStr) -> Result<Self> {
        Self::load_with_context(path, context_for_env(path_env, pathext))
    }

    fn load_with_context(path: &Path, context: CacheContext) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("read command cache {}", path.display()))?;
        let cache: CacheFile = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse command cache {}", path.display()))?;
        if cache.version != CACHE_VERSION {
            bail!("unsupported command cache version {}", cache.version);
        }
        if !cache.complete {
            bail!("command cache contains an incomplete discovery");
        }
        if cache.context != context {
            bail!("command cache PATH/PATHEXT context is stale");
        }

        let mut index = Self {
            entries: Vec::new(),
            by_name: HashMap::new(),
            context: cache.context,
            context_valid: true,
            complete: true,
            base_entries: Vec::new(),
            session_commands: HashMap::new(),
            session_order: Vec::new(),
        };
        for command in cache.commands {
            index.insert_command(command, false);
        }
        index.base_entries = index.entries.clone();
        Ok(index)
    }

    /// Save an index as a complete file and atomically publish it.
    pub fn save(&self, path: &Path) -> Result<()> {
        if !self.is_complete() {
            bail!("cannot persist an incomplete command discovery");
        }
        let cache = CacheFile {
            version: CACHE_VERSION,
            context: if self.context_valid {
                self.context.clone()
            } else {
                current_context()
            },
            complete: true,
            // Session aliases/functions/cmdlets are supplied by the current
            // PowerShell process and must not become persistent cache data.
            commands: self.base_entries.clone(),
        };
        let bytes = serde_json::to_vec(&cache).context("serialize command cache")?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create command cache directory {}", parent.display()))?;
        }

        static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("commands");
        let temporary = parent.join(format!(
            ".{file_name}.tmp-{}-{stamp}-{counter}",
            std::process::id()
        ));

        let write_result = (|| -> io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.flush()?;
            file.sync_all()?;
            drop(file);
            replace_file(&temporary, path)
        })();

        if let Err(error) = write_result {
            let _ = fs::remove_file(&temporary);
            return Err(error).with_context(|| format!("write command cache {}", path.display()));
        }
        Ok(())
    }

    /// Merge commands reported by the actual PowerShell session.
    pub fn merge_shell_commands(&mut self, commands: Vec<ShellCommand>) {
        for command in commands {
            self.merge_session_command(command);
        }
        self.rebuild_entries();
    }

    /// Replace the session command snapshot and restore PATH commands that
    /// were previously shadowed by aliases/functions/cmdlets.
    pub fn replace_shell_commands(&mut self, commands: Vec<ShellCommand>) {
        self.session_commands.clear();
        self.session_order.clear();
        for command in commands {
            self.merge_session_command(command);
        }
        self.rebuild_entries();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether this snapshot represents a complete PATH scan. A complete
    /// empty index is valid; it is distinct from a scan still in progress.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Complete a line at a UTF-8 byte cursor. The returned replacement range
    /// always spans the complete current token, including its suffix after the
    /// cursor, when a cursor is placed in the middle of a token.
    pub fn complete(&self, line: &str, cursor: usize, cwd: &Path, limit: usize) -> Completion {
        self.complete_with_descriptions(line, cursor, cwd, limit, &BTreeMap::new())
    }

    /// Complete with optional localized descriptions. The map is keyed by the
    /// declarative spec candidate key; command descriptions use a normalized
    /// canonical command key.
    pub fn complete_with_descriptions(
        &self,
        line: &str,
        cursor: usize,
        cwd: &Path,
        limit: usize,
        overrides: &BTreeMap<String, String>,
    ) -> Completion {
        self.complete_configured(line, cursor, cwd, limit, overrides, None, false, true, None)
    }

    pub fn input_context(&self, line: &str, cursor: usize) -> InputContext {
        let cursor = clamp_cursor(line, cursor);
        let tokens = lex_line(line);
        let current = current_token(line, &tokens, cursor);
        let start = segment_start(&tokens, current.start);
        let command = tokens
            .iter()
            .find(|token| !token.separator && token.start >= start && token.start <= current.start);
        let command_position = command.is_none_or(|command| command.start == current.start);
        let redirect = tokens
            .iter()
            .rev()
            .find(|t| t.separator && t.end <= current.start)
            .is_some_and(|t| t.raw == ">");
        InputContext {
            command: if redirect {
                "__shell_redirection".into()
            } else {
                command
                    .map(|c| self.resolve_command(&decode_power_shell(&c.raw)))
                    .unwrap_or_default()
            },
            arguments: preceding_arguments(&tokens, start, current.start),
            prefix: TokenContext::from(line, &current, cursor).value_before,
            replace_start: current.start,
            replace_end: current.end,
            command_position: command_position && !redirect,
            suppressed: comment_at_cursor(line, cursor),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_configured(
        &self,
        line: &str,
        cursor: usize,
        cwd: &Path,
        limit: usize,
        overrides: &BTreeMap<String, String>,
        context: Option<&InputContext>,
        fuzzy: bool,
        paths: bool,
        catalog: Option<&crate::spec_catalog::Catalog>,
    ) -> Completion {
        let cursor = clamp_cursor(line, cursor);
        let derived = self.input_context(line, cursor);
        let context = context.unwrap_or(&derived);
        if context.replace_start > cursor
            || context.replace_end < cursor
            || line
                .get(context.replace_start..context.replace_end)
                .is_none()
        {
            return Completion::default();
        }
        let current = Token {
            start: context.replace_start,
            end: context.replace_end,
            raw: line[context.replace_start..context.replace_end].into(),
            separator: false,
        };
        let mut replacement = Completion {
            replace_start: current.start,
            replace_end: current.end,
            candidates: Vec::new(),
            incomplete: !self.complete,
            argument_hint: String::new(),
        };
        let effective_limit = limit.min(MAX_RESULTS);
        if effective_limit == 0 || context.suppressed {
            return replacement;
        }

        let token_context = TokenContext::from(line, &current, cursor);
        let prefix = context.prefix.clone();

        if context.command_position {
            let mut candidates =
                self.command_candidates(&prefix, &token_context, effective_limit, overrides, fuzzy);
            if let Some(catalog) = catalog {
                for candidate in &mut candidates {
                    let (resolved, targets) =
                        self.resolve_command_with_alias_targets(&candidate.label);
                    let canonical = catalog
                        .canonical_command(&resolved)
                        .unwrap_or_else(|| resolved.clone());
                    if override_value(overrides, &candidate.label).is_none()
                        && override_value(overrides, &resolved).is_none()
                        && override_value(overrides, &canonical).is_none()
                        && targets
                            .iter()
                            .all(|target| override_value(overrides, target).is_none())
                        && let Some(description) = catalog.describe_command(&resolved)
                    {
                        candidate.description = description.into();
                        if let Some(node) = catalog.nodes.get(&canonical) {
                            candidate.description_source = node.source.clone();
                            candidate.detail = format!(
                                "{}\n{}\n示例：\n{}",
                                candidate.detail,
                                node.detail,
                                node.examples.join("\n")
                            );
                        }
                    }
                }
            }
            return Completion {
                candidates,
                ..replacement
            };
        }

        let preceding_args = &context.arguments;
        let resolved_command = self.resolve_command(&context.command);
        let catalog = catalog.unwrap_or_else(|| crate::spec_catalog::Catalog::builtin());
        let spec_result = if fuzzy {
            catalog.lookup_unfiltered(&resolved_command, preceding_args, &prefix)
        } else {
            catalog.lookup(&resolved_command, preceding_args, &prefix)
        };
        let mut candidates = Vec::new();
        let mut path_values = false;
        let mut no_spec = true;

        if let Some(result) = spec_result {
            replacement.argument_hint = result.argument_hint.clone();
            no_spec = false;
            path_values = result.path_values || result.options_ended;
            let prefix_lower = prefix.to_ascii_lowercase();
            let mut seen = HashSet::new();
            for candidate in result.candidates {
                if crate::ranking::match_class(&candidate.name, &prefix_lower, fuzzy).is_none()
                    // Spec keys intentionally preserve option spelling. In
                    // particular, git -C and git -c are distinct candidates.
                    || !seen.insert(candidate.key.clone())
                {
                    continue;
                }
                let description = override_value(overrides, &candidate.key)
                    .unwrap_or_else(|| candidate.description.to_owned());
                let insert_text = format_insert(&candidate.name, &token_context, false);
                candidates.push(Candidate {
                    label: candidate.name,
                    insert_text,
                    description,
                    kind: candidate.kind,
                    id: candidate.key,
                    source: candidate.source,
                    detail: candidate.detail,
                    append_space: candidate.append_space,
                    ..Default::default()
                });
                if candidates.len() >= effective_limit {
                    break;
                }
            }
        }

        // Specs explicitly opt into path values. Unknown commands retain the
        // useful local filesystem fallback used by the original engine, while
        // options are never treated as paths.
        if paths && candidates.is_empty() && !prefix.starts_with('-') && (path_values || no_spec) {
            candidates = filesystem_candidates(cwd, &prefix, &token_context, effective_limit);
            if matches!(
                resolved_command.to_ascii_lowercase().as_str(),
                "cd" | "set-location" | "push-location" | "pushd"
            ) {
                candidates.retain(|c| c.kind == CandidateKind::Directory);
            }
        }
        if !candidates
            .iter()
            .any(|c| matches!(c.kind, CandidateKind::File | CandidateKind::Directory))
        {
            crate::ranking::sort(&mut candidates, &prefix, fuzzy, None, cwd, effective_limit);
        }

        Completion {
            candidates,
            ..replacement
        }
    }

    fn insert_command(&mut self, command: IndexedCommand, replace: bool) {
        let mut command = command;
        command.name = command.name.trim().to_owned();
        let key = command_key(&command.name);
        if key.is_empty() {
            return;
        }
        if let Some(&index) = self.by_name.get(&key) {
            if replace
                && command_priority(&command.kind) >= command_priority(&self.entries[index].kind)
            {
                self.entries[index] = command;
            }
            return;
        }
        let index = self.entries.len();
        self.entries.push(command);
        self.by_name.insert(key, index);
    }

    fn merge_session_command(&mut self, command: ShellCommand) {
        let name = command.name.trim();
        if name.is_empty() {
            return;
        }
        let command_kind = shell_kind(&command.kind);
        let executable_path = if command_kind == CandidateKind::Command {
            definition_path(&command.definition)
        } else {
            None
        };
        let indexed = IndexedCommand {
            name: name.to_owned(),
            kind: command_kind,
            // Descriptions are resolved at query time from the canonical
            // command/spec. The definition remains solely for alias target
            // resolution and is never presented as a guessed description.
            description: String::new(),
            definition: command.definition,
            executable_path,
        };
        let key = command_key(name);
        if let Some(existing) = self.session_commands.get(&key) {
            if command_priority(&indexed.kind) < command_priority(&existing.kind) {
                return;
            }
        } else {
            self.session_order.push(key.clone());
        }
        self.session_commands.insert(key, indexed);
    }

    fn rebuild_entries(&mut self) {
        self.entries = self.base_entries.clone();
        self.by_name.clear();
        for index in 0..self.entries.len() {
            self.by_name
                .insert(command_key(&self.entries[index].name), index);
        }
        for key in &self.session_order {
            let Some(command) = self.session_commands.get(key).cloned() else {
                continue;
            };
            if let Some(&index) = self.by_name.get(key) {
                if command_priority(&command.kind) >= command_priority(&self.entries[index].kind) {
                    self.entries[index] = command;
                }
            } else {
                let index = self.entries.len();
                self.entries.push(command);
                self.by_name.insert(key.clone(), index);
            }
        }
    }

    fn command_candidates(
        &self,
        prefix: &str,
        context: &TokenContext,
        limit: usize,
        overrides: &BTreeMap<String, String>,
        fuzzy: bool,
    ) -> Vec<Candidate> {
        let prefix_lower = prefix.to_ascii_lowercase();
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();

        for command in &self.entries {
            if crate::ranking::match_class(&command.name, &prefix_lower, fuzzy).is_none() {
                continue;
            }
            if seen.insert(command_key(&command.name)) {
                candidates.push(Candidate {
                    label: command.name.clone(),
                    insert_text: format_insert(&command.name, context, false),
                    description: self.command_description(command, overrides),
                    kind: command.kind,
                    id: format!("command:{}", command.name),
                    detail: if command.kind == CandidateKind::Alias {
                        format!("别名目标：{}", self.resolve_command(&command.name))
                    } else {
                        String::new()
                    },
                    source: match &command.executable_path {
                        Some(path) => path.display().to_string(),
                        None => "PowerShell 会话".into(),
                    },
                    append_space: true,
                    ..Default::default()
                });
            }
        }
        // A short exact command (git) should precede git-gui for the prefix
        // gi. PATH order still decides which duplicate is indexed.
        crate::ranking::sort(&mut candidates, prefix, fuzzy, None, Path::new(""), limit);
        candidates
    }

    fn command_description(
        &self,
        command: &IndexedCommand,
        overrides: &BTreeMap<String, String>,
    ) -> String {
        // Let a configured description for the actual candidate win first.
        // This is important for aliases and locally-installed tools: the
        // visible name is the key a user can recognize and customize.
        if let Some(description) = override_value(overrides, &command.name) {
            return description;
        }

        let (resolved, alias_targets) = self.resolve_command_with_alias_targets(&command.name);
        // An alias can point at a locally-defined command that is not in the
        // static catalog. Give that target a chance to provide its own
        // configured description before falling back to the alias source.
        // Preserve each raw target spelling here: `cd` is itself an alias for
        // `Set-Location`, and a user may intentionally configure `cd` and
        // `Set-Location` differently.
        for target in &alias_targets {
            if let Some(description) = override_value(overrides, target) {
                return description;
            }
        }
        if let Some(description) = override_value(overrides, &resolved) {
            return description;
        }

        if let Some(canonical) = specs::canonical_command(&resolved) {
            if let Some(description) = override_value(overrides, canonical) {
                return description;
            }
            if let Some(description) = specs::describe_command(canonical) {
                return description.to_owned();
            }
        }

        // Keep unknown descriptions factual. In particular, do not display a
        // function body or an arbitrary alias definition as if it were a
        // human-authored command description. Alias targets are limited to
        // their first decoded token so arguments and bodies never leak into
        // the menu.
        match command.kind {
            CandidateKind::Alias => "命令别名",
            CandidateKind::Function => "会话函数",
            CandidateKind::Cmdlet => "PowerShell 命令",
            _ => "本地程序",
        }
        .to_owned()
    }

    pub fn executable(&self, command: &str) -> Option<PathBuf> {
        let mut name = decode_power_shell(command);
        for _ in 0..MAX_ALIAS_DEPTH {
            if name.contains(['/', '\\']) {
                return Some(PathBuf::from(name));
            }
            if let Some(i) = self.by_name.get(&command_key(&name)) {
                let entry = &self.entries[*i];
                if entry.kind == CandidateKind::Alias {
                    name = first_definition_token(&entry.definition)?;
                    continue;
                }
                if entry.kind != CandidateKind::Command {
                    return None;
                }
                return entry.executable_path.clone();
            }
            if without_program_extension(&name.to_ascii_lowercase()).is_some() {
                return split_path(OsStr::new(&self.context.path))
                    .into_iter()
                    .map(|p| p.join(&name))
                    .find(|p| p.is_file());
            }
            return None;
        }
        None
    }
    pub fn has_command(&self, command: &str) -> bool {
        self.by_name.contains_key(&command_key(command))
    }
    pub fn purpose_search(
        &self,
        catalog: &crate::spec_catalog::Catalog,
        line: &str,
        cursor: usize,
        supplied: Option<&InputContext>,
        limit: usize,
        overrides: &BTreeMap<String, String>,
    ) -> Completion {
        let derived = self.input_context(line, cursor);
        let context = supplied.unwrap_or(&derived);
        let mut result = Completion {
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            ..Default::default()
        };
        if context.suppressed
            || line
                .get(context.replace_start..context.replace_end)
                .is_none()
        {
            return result;
        }
        let query = context.prefix.trim_matches(['\'', '"']).to_lowercase();
        if query.is_empty() {
            return result;
        }
        let mut choices = Vec::new();
        if context.command_position {
            for node in catalog.nodes.values() {
                let root = node.path.split_whitespace().next().unwrap_or("");
                let mut path = node.path.as_str();
                let mut reachable = true;
                while let Some((parent, _)) = path.rsplit_once(' ') {
                    if !catalog
                        .nodes
                        .get(parent)
                        .is_some_and(|n| n.children.iter().any(|c| c == path))
                    {
                        reachable = false;
                        break;
                    }
                    path = parent;
                }
                if !self.has_command(root) || !reachable {
                    continue;
                }
                choices.push(Candidate {
                    label: node.path.clone(),
                    insert_text: node.path.clone(),
                    description: node.description.clone(),
                    detail: node.detail.clone(),
                    id: node.path.clone(),
                    source: node.source.clone(),
                    kind: CandidateKind::Command,
                    append_space: true,
                    ..Default::default()
                });
            }
            // Session-only functions and aliases are useful even without a catalog entry.
            let token = TokenContext::from(
                line,
                &Token {
                    start: context.replace_start,
                    end: context.replace_end,
                    raw: context.prefix.clone(),
                    separator: false,
                },
                cursor,
            );
            choices.extend(self.command_candidates("", &token, MAX_RESULTS, overrides, false));
        } else {
            for prefix in ["", "-"] {
                if let Some(found) = catalog.lookup_unfiltered(
                    &self.resolve_command(&context.command),
                    &context.arguments,
                    prefix,
                ) {
                    result.argument_hint = found.argument_hint;
                    choices.extend(found.candidates.into_iter().map(|c| Candidate {
                        label: c.name.clone(),
                        insert_text: c.name,
                        description: c.description,
                        detail: c.detail,
                        id: c.key,
                        source: c.source,
                        kind: c.kind,
                        append_space: c.append_space,
                        ..Default::default()
                    }));
                }
            }
        }
        let mut scored = Vec::new();
        let mut seen = HashSet::new();
        for mut candidate in choices {
            if !seen.insert(candidate.id.clone()) {
                continue;
            }
            if let Some(text) = overrides.get(&candidate.id) {
                candidate.description = text.clone();
                candidate.description_source = "user".into();
            }
            let keywords = catalog
                .nodes
                .get(&candidate.id)
                .map(|n| n.keywords.join(" "))
                .unwrap_or_default()
                .to_lowercase();
            let (score, reason) = if candidate.label.to_lowercase().contains(&query) {
                (0, "命令名")
            } else if keywords.contains(&query) {
                (1, "用途关键词")
            } else if candidate.description.to_lowercase().contains(&query) {
                (2, "用途说明")
            } else {
                continue;
            };
            candidate.match_reason = reason.into();
            crate::knowledge::annotate(&mut candidate);
            crate::completion::preserve_suffix(
                &mut candidate,
                line,
                cursor,
                result.replace_start,
                result.replace_end,
            );
            scored.push((score, candidate));
        }
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
        result.candidates = scored.into_iter().take(limit).map(|(_, c)| c).collect();
        result
    }

    fn resolve_command(&self, command: &str) -> String {
        self.resolve_command_with_alias_targets(command).0
    }

    fn resolve_command_with_alias_targets(&self, command: &str) -> (String, Vec<String>) {
        let mut current = decode_power_shell(command).trim().to_owned();
        let mut visited = HashSet::new();
        let mut alias_targets = Vec::new();
        for _ in 0..MAX_ALIAS_DEPTH {
            let key = command_key(&current);
            if key.is_empty() || !visited.insert(key.clone()) {
                break;
            }

            // Resolve a session alias before asking specs to canonicalize the
            // name. An alias is allowed to shadow a known external command.
            let lookup_key = command_lookup_key(&key);
            if let Some(index) = self
                .by_name
                .get(&lookup_key)
                .and_then(|position| self.entries.get(*position))
                && index.kind == CandidateKind::Alias
            {
                let Some(target) = first_definition_token(&index.definition) else {
                    break;
                };
                alias_targets.push(target.clone());
                current = target;
                continue;
            }

            if let Some(canonical) = specs::canonical_command(&current) {
                return (canonical.to_owned(), alias_targets);
            }
            if let Some(stem) = without_program_extension(&key)
                && let Some(canonical) = specs::canonical_command(stem)
            {
                return (canonical.to_owned(), alias_targets);
            }
            return (current, alias_targets);
        }
        (current, alias_targets)
    }
}

fn current_context() -> CacheContext {
    let path = env::var_os("PATH").unwrap_or_default();
    let pathext =
        env::var_os("PATHEXT").unwrap_or_else(|| OsStr::new(DEFAULT_PATHEXT).to_os_string());
    context_for_env(&path, &pathext)
}

fn context_for_env(path: &OsStr, pathext: &OsStr) -> CacheContext {
    CacheContext {
        path: path.to_string_lossy().into_owned(),
        pathext: pathext.to_string_lossy().into_owned(),
        directories: split_path(path)
            .into_iter()
            .map(|directory| directory_stamp(&directory))
            .collect(),
    }
}

fn directory_stamp(path: &Path) -> DirectoryStamp {
    if is_remote_path(path) || path_resolves_remote(path) {
        return DirectoryStamp {
            path: path.to_string_lossy().into_owned(),
            modified: None,
            accessible: true,
        };
    }
    match fs::metadata(path) {
        Ok(metadata) => DirectoryStamp {
            path: path.to_string_lossy().into_owned(),
            modified: metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos()),
            accessible: true,
        },
        Err(error) => DirectoryStamp {
            path: path.to_string_lossy().into_owned(),
            modified: None,
            accessible: !is_scan_access_error(&error),
        },
    }
}

fn is_remote_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    if text.starts_with("//") {
        return true;
    }
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("\\\\?\\unc\\") || lower.starts_with("\\\\.\\unc\\") {
        return true;
    }
    // Verbatim disk paths (\\?\\C:\\...) and device/volume paths are
    // local. A normal double-backslash path without a verbatim disk prefix is
    // a UNC path and therefore remote.
    lower.starts_with("\\\\") && !lower.starts_with("\\\\?\\") && !lower.starts_with("\\\\.\\")
}

fn path_resolves_remote(path: &Path) -> bool {
    matches!(symlink_target_status(path), LinkStatus::Remote)
}

pub(crate) fn local_link(path: &Path) -> bool {
    matches!(symlink_target_status(path), LinkStatus::Local)
}

fn symlink_target_status(path: &Path) -> LinkStatus {
    let mut current = path.to_owned();
    for _ in 0..MAX_SYMLINK_DEPTH {
        if is_remote_path(&current) {
            return LinkStatus::Remote;
        }
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) => {
                return if error.kind() == io::ErrorKind::NotFound {
                    LinkStatus::Dangling
                } else {
                    LinkStatus::Inaccessible
                };
            }
        };
        if !metadata.file_type().is_symlink() {
            return LinkStatus::Local;
        }
        let target = match fs::read_link(&current) {
            Ok(target) => target,
            Err(error) => {
                return if error.kind() == io::ErrorKind::NotFound {
                    LinkStatus::Dangling
                } else {
                    LinkStatus::Inaccessible
                };
            }
        };
        if is_remote_path(&target) {
            return LinkStatus::Remote;
        }
        current = if target.is_absolute() {
            target
        } else {
            current
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(target)
        };
    }
    LinkStatus::Inaccessible
}

fn is_scan_access_error(error: &io::Error) -> bool {
    !matches!(
        error.kind(),
        io::ErrorKind::NotFound
            | io::ErrorKind::NotADirectory
            | io::ErrorKind::InvalidInput
            | io::ErrorKind::InvalidData
    )
}

fn split_path(path: &OsStr) -> Vec<PathBuf> {
    env::split_paths(path).collect()
}

fn parse_pathext(value: &OsStr) -> Vec<String> {
    let text = value.to_string_lossy();
    let mut result = Vec::new();
    for extension in text.split(';') {
        let extension = extension.trim();
        if extension.is_empty() {
            continue;
        }
        let extension = if extension.starts_with('.') {
            extension.to_owned()
        } else {
            format!(".{extension}")
        };
        let extension = extension.to_ascii_lowercase();
        if !result.contains(&extension) {
            result.push(extension);
        }
    }
    if result.is_empty() {
        parse_pathext(OsStr::new(DEFAULT_PATHEXT))
    } else {
        result
    }
}

fn executable_stem(name: &str, extensions: &[String]) -> Option<(String, usize)> {
    let lower = name.to_ascii_lowercase();
    extensions.iter().enumerate().find_map(|(rank, extension)| {
        if lower.ends_with(extension) && name.len() > extension.len() {
            Some((name[..name.len() - extension.len()].to_owned(), rank))
        } else {
            None
        }
    })
}

fn inspect_executable(entry: &fs::DirEntry, extensions: &[String]) -> EntryInspection {
    // Filter by PATHEXT before following a link. This keeps the ordinary
    // directory path cheap and avoids touching unrelated (possibly remote)
    // symlink targets.
    let name = entry.file_name().to_string_lossy().into_owned();
    let Some((stem, extension_rank)) = executable_stem(&name, extensions) else {
        return EntryInspection::Ignore;
    };
    if stem.is_empty() {
        return EntryInspection::Ignore;
    }

    let file_type = match entry.file_type() {
        Ok(file_type) => file_type,
        Err(error) => {
            return if is_scan_access_error(&error) {
                EntryInspection::AccessError
            } else {
                EntryInspection::Ignore
            };
        }
    };
    let path = entry.path();
    if file_type.is_symlink() {
        match symlink_target_status(&path) {
            LinkStatus::Remote | LinkStatus::Dangling => return EntryInspection::Ignore,
            LinkStatus::Inaccessible => return EntryInspection::AccessError,
            LinkStatus::Local => {}
        }
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return EntryInspection::Ignore,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return EntryInspection::Ignore;
            }
            Err(_) => return EntryInspection::AccessError,
        }
    } else if !file_type.is_file() {
        return EntryInspection::Ignore;
    }

    EntryInspection::Candidate(FoundCommand {
        extension_rank,
        lower_name: name.to_ascii_lowercase(),
        stem,
        executable_path: path,
    })
}

fn shell_kind(kind: &str) -> CandidateKind {
    match kind.trim().to_ascii_lowercase().as_str() {
        "alias" => CandidateKind::Alias,
        "function" | "filter" => CandidateKind::Function,
        "cmdlet" => CandidateKind::Cmdlet,
        "subcommand" => CandidateKind::Subcommand,
        "option" | "parameter" => CandidateKind::Option,
        _ => CandidateKind::Command,
    }
}

fn command_priority(kind: &CandidateKind) -> u8 {
    match kind {
        CandidateKind::Alias => 4,
        CandidateKind::Function => 3,
        CandidateKind::Cmdlet => 2,
        CandidateKind::Command => 1,
        _ => 0,
    }
}

fn command_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn command_lookup_key(name: &str) -> String {
    let key = command_key(name);
    without_program_extension(&key)
        .map(ToOwned::to_owned)
        .unwrap_or(key)
}

fn without_program_extension(name: &str) -> Option<&str> {
    [".com", ".exe", ".bat", ".cmd"]
        .iter()
        .find_map(|extension| name.strip_suffix(extension))
}

fn first_definition_token(definition: &str) -> Option<String> {
    lex_line(definition)
        .into_iter()
        .find(|token| !token.separator)
        .map(|token| decode_power_shell(&token.raw))
}

fn definition_path(definition: &str) -> Option<PathBuf> {
    let token = first_definition_token(definition)?;
    let lower = token.to_ascii_lowercase();
    if token.contains(['/', '\\'])
        || [".com", ".exe", ".bat", ".cmd"]
            .iter()
            .any(|extension| lower.ends_with(extension))
    {
        Some(PathBuf::from(token))
    } else {
        None
    }
}

fn override_value(overrides: &BTreeMap<String, String>, key: &str) -> Option<String> {
    // Spec keys are case-sensitive because option spelling can carry meaning
    // (for example, git -C and git -c). Command canonicalization happens
    // before this helper is called.
    overrides.get(key).cloned()
}

fn preceding_arguments(
    tokens: &[Token],
    segment_start: usize,
    current_start: usize,
) -> Vec<String> {
    tokens
        .iter()
        .filter(|token| {
            !token.separator && token.start >= segment_start && token.start < current_start
        })
        .map(|token| decode_power_shell(&token.raw))
        .skip(1)
        .collect()
}

fn clamp_cursor(line: &str, cursor: usize) -> usize {
    let mut cursor = cursor.min(line.len());
    while cursor > 0 && !line.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

#[derive(Clone, Debug)]
struct Token {
    start: usize,
    end: usize,
    raw: String,
    separator: bool,
}

fn lex_line(line: &str) -> Vec<Token> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let character = line[cursor..].chars().next().unwrap();
        if character.is_whitespace() {
            cursor += character.len_utf8();
            continue;
        }
        if is_separator_at(bytes, cursor) {
            let start = cursor;
            if cursor + 1 < bytes.len()
                && ((bytes[cursor] == b'&' && bytes[cursor + 1] == b'&')
                    || (bytes[cursor] == b'|' && bytes[cursor + 1] == b'|'))
            {
                cursor += 2;
            } else {
                cursor += 1;
            }
            tokens.push(Token {
                start,
                end: cursor,
                raw: line[start..cursor].to_owned(),
                separator: true,
            });
            continue;
        }

        let start = cursor;
        let mut quote = 0u8;
        while cursor < bytes.len() {
            let byte = bytes[cursor];
            if quote == 0 {
                if line[cursor..]
                    .chars()
                    .next()
                    .map(char::is_whitespace)
                    .unwrap_or(false)
                    || is_separator_at(bytes, cursor)
                {
                    break;
                }
                if byte == b'\'' || byte == b'"' {
                    quote = byte;
                    cursor += 1;
                    continue;
                }
                if byte == b'`' {
                    cursor += 1;
                    skip_one_char(bytes, &mut cursor);
                    continue;
                }
                cursor += line[cursor..].chars().next().unwrap().len_utf8();
                continue;
            }
            if quote == b'\'' {
                if byte == b'\'' {
                    if cursor + 1 < bytes.len() && bytes[cursor + 1] == b'\'' {
                        cursor += 2;
                    } else {
                        quote = 0;
                        cursor += 1;
                    }
                } else {
                    cursor += line[cursor..].chars().next().unwrap().len_utf8();
                }
            } else if byte == b'`' {
                cursor += 1;
                skip_one_char(bytes, &mut cursor);
            } else if byte == b'"' {
                quote = 0;
                cursor += 1;
            } else {
                cursor += line[cursor..].chars().next().unwrap().len_utf8();
            }
        }
        tokens.push(Token {
            start,
            end: cursor,
            raw: line[start..cursor].to_owned(),
            separator: false,
        });
    }
    tokens
}

fn skip_one_char(bytes: &[u8], cursor: &mut usize) {
    if *cursor < bytes.len() {
        if let Ok(text) = std::str::from_utf8(&bytes[*cursor..])
            && let Some(character) = text.chars().next()
        {
            *cursor += character.len_utf8();
            return;
        }
        *cursor = (*cursor + 1).min(bytes.len());
    }
}

fn is_separator_at(bytes: &[u8], cursor: usize) -> bool {
    matches!(
        bytes.get(cursor),
        Some(b';' | b'|' | b'&' | b'<' | b'>' | b'(' | b')' | b'{' | b'}')
    )
}

fn current_token(line: &str, tokens: &[Token], cursor: usize) -> Token {
    if let Some(token) = tokens
        .iter()
        .find(|token| !token.separator && token.start <= cursor && cursor <= token.end)
    {
        return token.clone();
    }
    // At a separator, completion belongs to the segment after that separator;
    // at whitespace, an empty token starts exactly at the cursor.
    let _ = line;
    Token {
        start: cursor,
        end: cursor,
        raw: String::new(),
        separator: false,
    }
}

fn segment_start(tokens: &[Token], current_start: usize) -> usize {
    tokens
        .iter()
        .filter(|token| token.separator && token.end <= current_start)
        .map(|token| token.end)
        .max()
        .unwrap_or(0)
}

#[derive(Clone, Debug)]
struct TokenContext {
    value_before: String,
    quote_char: Option<u8>,
    quoted: bool,
    closed: bool,
}

impl TokenContext {
    fn from(line: &str, token: &Token, cursor: usize) -> Self {
        let before_end = cursor.min(token.end).max(token.start);
        let prefix_raw = if before_end >= token.start {
            &line[token.start..before_end]
        } else {
            ""
        };
        let (quote_char, closed) = analyze_quotes(&token.raw);
        Self {
            value_before: decode_power_shell(prefix_raw),
            quote_char,
            quoted: token
                .raw
                .as_bytes()
                .first()
                .is_some_and(|byte| *byte == b'\'' || *byte == b'"'),
            closed,
        }
    }
}

fn analyze_quotes(raw: &str) -> (Option<u8>, bool) {
    let bytes = raw.as_bytes();
    let mut quote = 0u8;
    let mut first_quote = None;
    let mut cursor = 0;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if quote == 0 {
            if byte == b'\'' || byte == b'"' {
                first_quote.get_or_insert(byte);
                quote = byte;
                cursor += 1;
                continue;
            }
            if byte == b'`' {
                cursor += 1;
                skip_one_char(bytes, &mut cursor);
                continue;
            }
            cursor += raw[cursor..].chars().next().unwrap().len_utf8();
        } else if quote == b'\'' {
            if byte == b'\'' {
                if cursor + 1 < bytes.len() && bytes[cursor + 1] == b'\'' {
                    cursor += 2;
                } else {
                    quote = 0;
                    cursor += 1;
                }
            } else {
                cursor += raw[cursor..].chars().next().unwrap().len_utf8();
            }
        } else if byte == b'`' {
            cursor += 1;
            skip_one_char(bytes, &mut cursor);
        } else if byte == b'"' {
            quote = 0;
            cursor += 1;
        } else {
            cursor += raw[cursor..].chars().next().unwrap().len_utf8();
        }
    }
    (first_quote, quote == 0 && first_quote.is_some())
}

pub fn space_after(text: &str) -> bool {
    let (quote, closed) = analyze_quotes(text);
    quote.is_none() || closed
}

fn decode_power_shell(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut result = String::new();
    let mut quote = 0u8;
    let mut cursor = 0;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if quote == 0 {
            if byte == b'\'' || byte == b'"' {
                quote = byte;
                cursor += 1;
            } else if byte == b'`' {
                cursor += 1;
                if cursor < bytes.len() {
                    append_one_char(raw, &mut cursor, &mut result);
                }
            } else {
                append_one_char(raw, &mut cursor, &mut result);
            }
        } else if quote == b'\'' {
            if byte == b'\'' {
                if cursor + 1 < bytes.len() && bytes[cursor + 1] == b'\'' {
                    result.push('\'');
                    cursor += 2;
                } else {
                    quote = 0;
                    cursor += 1;
                }
            } else {
                append_one_char(raw, &mut cursor, &mut result);
            }
        } else if byte == b'`' {
            cursor += 1;
            if cursor < bytes.len() {
                append_one_char(raw, &mut cursor, &mut result);
            }
        } else if byte == b'"' {
            quote = 0;
            cursor += 1;
        } else {
            append_one_char(raw, &mut cursor, &mut result);
        }
    }
    result
}

fn append_one_char(raw: &str, cursor: &mut usize, result: &mut String) {
    if let Some(character) = raw[*cursor..].chars().next() {
        result.push(character);
        *cursor += character.len_utf8();
    } else {
        *cursor += 1;
    }
}

pub fn format_value(value: &str, line: &str, context: &InputContext, cursor: usize) -> String {
    let Some(raw) = line.get(context.replace_start..context.replace_end) else {
        return String::new();
    };
    let token = Token {
        start: context.replace_start,
        end: context.replace_end,
        raw: raw.into(),
        separator: false,
    };
    format_insert(value, &TokenContext::from(line, &token, cursor), true)
}

fn comment_at_cursor(line: &str, cursor: usize) -> bool {
    let before = &line[..cursor];
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    let mut boundary = true;
    for character in before.chars() {
        if comment {
            if character == '\n' {
                comment = false;
                boundary = true;
            }
            continue;
        }
        if escaped {
            escaped = false;
            boundary = false;
            continue;
        }
        if character == '`' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        if character == '\'' || character == '"' {
            quote = Some(character);
        } else if character == '#' && boundary {
            comment = true;
        }
        boundary = character.is_whitespace() || matches!(character, ';' | '|' | '{' | '}');
    }
    comment
}

fn format_insert(text: &str, context: &TokenContext, path_value: bool) -> String {
    if context.quoted {
        let quote = context.quote_char.unwrap_or(b'\'');
        let mut result = String::new();
        result.push(quote as char);
        result.push_str(&escape_for_quote(text, quote));
        if context.closed {
            result.push(quote as char);
        }
        return result;
    }
    if path_value && needs_literal_quote(text) {
        return format!("'{}'", escape_for_quote(text, b'\''));
    }
    text.to_owned()
}

fn escape_for_quote(text: &str, quote: u8) -> String {
    if quote == b'\'' {
        text.replace('\'', "''")
    } else {
        let mut result = String::with_capacity(text.len());
        for character in text.chars() {
            if matches!(character, '`' | '"' | '$') {
                result.push('`');
            }
            result.push(character);
        }
        result
    }
}

fn needs_literal_quote(text: &str) -> bool {
    text.is_empty()
        || text.chars().any(|character| {
            character.is_whitespace()
                || matches!(
                    character,
                    ';' | '|'
                        | '&'
                        | '<'
                        | '>'
                        | '('
                        | ')'
                        | '{'
                        | '}'
                        | '\''
                        | '"'
                        | '`'
                        | '*'
                        | '?'
                        | '['
                        | ']'
                        | ','
                        | '$'
                        | '@'
                )
        })
        || text.starts_with('#')
}

fn filesystem_candidates(
    cwd: &Path,
    value: &str,
    context: &TokenContext,
    limit: usize,
) -> Vec<Candidate> {
    if limit == 0 {
        return Vec::new();
    }
    let (directory_text, name_prefix) = match value.rfind(['/', '\\']) {
        Some(index) => (&value[..index + 1], &value[index + 1..]),
        None => ("", value),
    };
    let directory = resolve_directory(cwd, directory_text);
    if is_remote_path(cwd)
        || is_remote_path(&directory)
        || path_resolves_remote(cwd)
        || path_resolves_remote(&directory)
    {
        return Vec::new();
    }
    let read_dir = match fs::read_dir(&directory) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let prefix_lower = name_prefix.to_ascii_lowercase();
    let mut entries = Vec::new();
    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().starts_with(&prefix_lower) {
            continue;
        }
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            match symlink_target_status(&path) {
                LinkStatus::Remote | LinkStatus::Dangling | LinkStatus::Inaccessible => continue,
                LinkStatus::Local => {}
            }
        }
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        entries.push((name.to_ascii_lowercase(), name, metadata.is_dir()));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    let separator = value
        .rfind(['/', '\\'])
        .and_then(|index| value[index..].chars().next())
        .unwrap_or_else(path_separator);
    let mut candidates = Vec::new();
    for (_, name, is_directory) in entries {
        let mut insert = String::with_capacity(directory_text.len() + name.len() + 1);
        insert.push_str(directory_text);
        insert.push_str(&name);
        if is_directory {
            insert.push(separator);
        }
        candidates.push(Candidate {
            label: name.clone(),
            insert_text: format_insert(&insert, context, true),
            description: if is_directory {
                "目录".to_owned()
            } else {
                "文件".to_owned()
            },
            kind: if is_directory {
                CandidateKind::Directory
            } else {
                CandidateKind::File
            },
            id: format!("path:{}", directory.join(&name).display()),
            source: directory.display().to_string(),
            ..Default::default()
        });
        if candidates.len() >= limit {
            break;
        }
    }
    candidates
}

fn resolve_directory(cwd: &Path, directory_text: &str) -> PathBuf {
    if directory_text.is_empty() {
        return cwd.to_owned();
    }
    let trailing_separator = directory_text
        .chars()
        .last()
        .filter(|character| matches!(character, '/' | '\\'));
    let mut path_text = directory_text.trim_end_matches(['/', '\\']).to_owned();
    if path_text.is_empty() {
        path_text.push(trailing_separator.unwrap_or_else(path_separator));
    } else if path_text.len() == 2
        && path_text.as_bytes()[1] == b':'
        && trailing_separator.is_some()
    {
        // Trimming C:\\ to C: turns a drive-relative path into a drive root.
        path_text.push(trailing_separator.unwrap());
    }
    if (path_text == "~" || path_text.starts_with("~/") || path_text.starts_with("~\\"))
        && let Some(home) = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME"))
    {
        let suffix = path_text[1..].trim_start_matches(['/', '\\']);
        return if suffix.is_empty() {
            PathBuf::from(home)
        } else {
            PathBuf::from(home).join(suffix)
        };
    }
    let candidate = PathBuf::from(&path_text);
    if candidate.is_absolute() {
        candidate
    } else {
        cwd.join(candidate)
    }
}

fn path_separator() -> char {
    if cfg!(windows) { '\\' } else { '/' }
}

pub fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let source_w: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination_w: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
        const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
        let result = unsafe {
            MoveFileExW(
                source_w.as_ptr(),
                destination_w.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, destination)
    }
}

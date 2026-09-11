//! A small, deterministic completion engine for the native host.
//!
//! The engine deliberately does not execute a shell while completing.  The
//! command index comes from PATH (and from the command list supplied by the
//! PowerShell adapter), while the few command specifications below are static
//! data.  This keeps completion predictable and keeps the hot path bounded.

use crate::model::{Candidate, CandidateKind, Completion, ShellCommand};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CACHE_VERSION: u32 = 1;
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";
const MAX_RESULTS: usize = 1_000;
const MAX_DIRECTORY_ENTRIES: usize = 8_192;
const DIRECTORY_SCAN_BUDGET: Duration = Duration::from_millis(50);

/// An ordered command index.  Entries are kept in discovery order, so the
/// first matching PATH entry wins both indexing and display order.
#[derive(Clone, Default)]
pub struct CommandIndex {
    entries: Vec<IndexedCommand>,
    by_name: HashMap<String, usize>,
    context: CacheContext,
    context_valid: bool,
    /// Commands discovered from PATH.  Keeping this baseline lets a session
    /// alias shadow an application temporarily and restores the application
    /// when the next complete session snapshot no longer contains that alias.
    base_entries: Vec<IndexedCommand>,
    session_commands: HashMap<String, IndexedCommand>,
    session_order: Vec<String>,
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
    /// Directory modification time in nanoseconds since the Unix epoch.  A
    /// missing value means the directory could not be inspected; discovery
    /// already ignores such directories.
    modified: Option<u128>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    context: CacheContext,
    commands: Vec<IndexedCommand>,
}

impl CommandIndex {
    /// Discover executable commands using the process PATH and PATHEXT.
    pub fn discover() -> Self {
        let path = env::var_os("PATH").unwrap_or_default();
        let pathext =
            env::var_os("PATHEXT").unwrap_or_else(|| OsStr::new(DEFAULT_PATHEXT).to_os_string());
        Self::discover_with_env(&path, &pathext)
    }

    /// Discover commands with an explicit environment snapshot.
    ///
    /// The host uses this when a child PowerShell has established a different
    /// PATH.  It also makes discovery straightforward to test without changing
    /// the process environment.
    pub fn discover_with_env(path: &OsStr, pathext: &OsStr) -> Self {
        let mut index = Self {
            entries: Vec::new(),
            by_name: HashMap::new(),
            context: context_for_env(path, pathext),
            context_valid: true,
            base_entries: Vec::new(),
            session_commands: HashMap::new(),
            session_order: Vec::new(),
        };

        let extensions = parse_pathext(pathext);
        for directory in split_path(path) {
            // Network/UNC PATH entries can block while a server is offline.
            // The native host intentionally keeps completion local and
            // predictable, so those entries are skipped.
            if is_remote_path(&directory) {
                continue;
            }
            let mut found = Vec::new();
            let read_dir = match fs::read_dir(&directory) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let started = Instant::now();

            for (entry_number, entry) in read_dir.flatten().enumerate() {
                if entry_number >= MAX_DIRECTORY_ENTRIES
                    || started.elapsed() >= DIRECTORY_SCAN_BUDGET
                {
                    break;
                }
                let path = entry.path();
                let is_file = entry
                    .metadata()
                    .map(|value| value.is_file())
                    .unwrap_or(false);
                if !is_file {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some((stem, extension_rank)) = executable_stem(&name, &extensions) else {
                    continue;
                };
                if stem.is_empty() {
                    continue;
                }
                found.push((extension_rank, name.to_ascii_lowercase(), stem, path));
            }

            // read_dir order is unspecified.  PATHEXT order is meaningful on
            // Windows (foo.cmd and foo.exe can coexist), followed by name for
            // deterministic output.
            found.sort_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| left.1.cmp(&right.1))
                    .then_with(|| {
                        left.2
                            .to_ascii_lowercase()
                            .cmp(&right.2.to_ascii_lowercase())
                    })
            });
            for (_, _, stem, executable_path) in found {
                index.insert_command(
                    IndexedCommand {
                        name: stem,
                        kind: CandidateKind::Command,
                        description: "Executable".to_owned(),
                        definition: String::new(),
                        executable_path: Some(executable_path),
                    },
                    false,
                );
            }
        }
        index.base_entries = index.entries.clone();
        index
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
        if cache.context != context {
            bail!("command cache PATH/PATHEXT context is stale");
        }

        let mut index = Self {
            entries: Vec::new(),
            by_name: HashMap::new(),
            context: cache.context,
            context_valid: true,
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
        let cache = CacheFile {
            version: CACHE_VERSION,
            context: if self.context_valid {
                self.context.clone()
            } else {
                current_context()
            },
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
            atomic_replace(&temporary, path)
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

    /// Complete a line at a UTF-8 byte cursor.  The returned replacement range
    /// always spans the complete current token, including its suffix after the
    /// cursor, when a cursor is placed in the middle of a token.
    pub fn complete(&self, line: &str, cursor: usize, cwd: &Path, limit: usize) -> Completion {
        let cursor = clamp_cursor(line, cursor);
        let tokens = lex_line(line);
        let current = current_token(line, &tokens, cursor);
        let replacement = Completion {
            replace_start: current.start,
            replace_end: current.end,
            candidates: Vec::new(),
        };
        let effective_limit = limit.min(MAX_RESULTS);
        if effective_limit == 0 {
            return replacement;
        }

        let token_context = TokenContext::from(line, &current, cursor);
        let prefix = token_context.value_before.clone();
        let segment_start = segment_start(&tokens, current.start);
        let command_token = tokens.iter().find(|token| {
            !token.separator && token.start >= segment_start && token.start <= current.start
        });
        let is_command_position = command_token
            .map(|command| command.start == current.start)
            .unwrap_or(true);

        if is_command_position {
            let candidates = self.command_candidates(&prefix, &token_context, effective_limit);
            return Completion {
                candidates,
                ..replacement
            };
        }

        let command_name = command_token
            .map(|command| decode_power_shell(&command.raw))
            .unwrap_or_default();
        let command_name_lower = command_name.to_ascii_lowercase();
        let arg_position = argument_position(&tokens, segment_start, current.start);
        let spec = self.spec_for_command(&command_name);
        let mut candidates = Vec::new();

        if let Some(spec) = spec {
            // Subcommands are only valid as the first argument.  Options stay
            // available at every argument position and are useful for the
            // common PowerShell cmdlets too.
            if arg_position <= 1 && !prefix.starts_with('-') {
                add_static_candidates(
                    &mut candidates,
                    spec.subcommands,
                    CandidateKind::Subcommand,
                    spec.name,
                    &token_context,
                    effective_limit,
                );
            }
            if prefix.starts_with('-') || (prefix.is_empty() && spec.name == "pwsh") {
                add_static_candidates(
                    &mut candidates,
                    spec.options,
                    CandidateKind::Option,
                    spec.name,
                    &token_context,
                    effective_limit,
                );
            }
        }

        let path_command = is_path_command(&command_name_lower)
            || spec
                .map(|spec| is_path_command(&spec.name.to_ascii_lowercase()))
                .unwrap_or(false);
        let no_spec = spec.is_none();
        let path_like = looks_like_path(&prefix);
        if candidates.is_empty()
            && !prefix.starts_with('-')
            && (path_command || no_spec || path_like)
        {
            candidates = filesystem_candidates(cwd, &prefix, &token_context, effective_limit);
        }

        Completion {
            candidates,
            ..replacement
        }
    }

    fn insert_command(&mut self, command: IndexedCommand, replace: bool) {
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
        let indexed = IndexedCommand {
            name: name.to_owned(),
            kind: shell_kind(&command.kind),
            description: if command.definition.trim().is_empty() {
                command.kind.clone()
            } else {
                command.definition.clone()
            },
            definition: command.definition,
            executable_path: None,
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
    ) -> Vec<Candidate> {
        let prefix_lower = prefix.to_ascii_lowercase();
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();

        for command in &self.entries {
            if !command.name.to_ascii_lowercase().starts_with(&prefix_lower) {
                continue;
            }
            if seen.insert(command_key(&command.name)) {
                candidates.push(Candidate {
                    label: command.name.clone(),
                    insert_text: format_insert(&command.name, context, false),
                    description: command.description.clone(),
                    kind: command.kind.clone(),
                });
            }
        }
        // PATH resolves duplicate names; display ranking is independent.
        // A short exact command (git) should precede git-gui for the prefix gi.
        candidates.sort_by_cached_key(|c| (c.label.len(), c.label.to_ascii_lowercase()));
        candidates.truncate(limit);
        candidates
    }

    fn spec_for_command(&self, command: &str) -> Option<&'static CommandSpec> {
        let mut current = command
            .trim_matches(|character| character == '\'' || character == '"')
            .to_owned();
        let mut visited = HashSet::new();
        for _ in 0..4 {
            let key = current.to_ascii_lowercase();
            if !visited.insert(key.clone()) {
                break;
            }
            if let Some(spec) = command_spec(&key) {
                return Some(spec);
            }
            if let Some(stem) = without_program_extension(&key) {
                if let Some(spec) = command_spec(stem) {
                    return Some(spec);
                }
            }
            let lookup_key = if self.by_name.contains_key(&key) {
                key.clone()
            } else if let Some(stem) = without_program_extension(&key) {
                stem.to_owned()
            } else {
                key.clone()
            };
            let Some(index) = self
                .by_name
                .get(&lookup_key)
                .and_then(|index| self.entries.get(*index))
            else {
                break;
            };
            let Some(definition) = first_definition_token(&index.definition) else {
                break;
            };
            current = definition.to_owned();
        }
        None
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
            .map(|directory| DirectoryStamp {
                path: directory.to_string_lossy().into_owned(),
                modified: directory_mtime(&directory),
            })
            .collect(),
    }
}

fn directory_mtime(path: &Path) -> Option<u128> {
    if is_remote_path(path) {
        return None;
    }
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos())
}

fn is_remote_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.starts_with(r"\\") || text.starts_with("//")
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
        // A session-defined name is the most specific command available to
        // the user.  Keep aliases ahead of functions, then cmdlets, then
        // external applications discovered from PATH.
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

fn without_program_extension(name: &str) -> Option<&str> {
    [".com", ".exe", ".bat", ".cmd"]
        .iter()
        .find_map(|extension| name.strip_suffix(extension))
}

fn first_definition_token(definition: &str) -> Option<&str> {
    let definition = definition.trim();
    if definition.is_empty() {
        return None;
    }
    let token = definition.split_whitespace().next()?;
    Some(token.trim_matches(|character| character == '\'' || character == '"'))
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
        if let Ok(text) = std::str::from_utf8(&bytes[*cursor..]) {
            if let Some(character) = text.chars().next() {
                *cursor += character.len_utf8();
                return;
            }
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

fn argument_position(tokens: &[Token], segment_start: usize, current_start: usize) -> usize {
    tokens
        .iter()
        .filter(|token| {
            !token.separator && token.start >= segment_start && token.start < current_start
        })
        .count()
        // The command itself is the first token in the segment.  Counting
        // prior non-separators therefore gives the one-based position of the
        // current argument (the current token is the next token).
        .saturating_sub(1)
        + 1
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

#[derive(Clone, Copy)]
struct CommandSpec {
    name: &'static str,
    names: &'static [&'static str],
    subcommands: &'static [&'static str],
    options: &'static [&'static str],
}

const GIT_SUBCOMMANDS: &[&str] = &[
    "add",
    "am",
    "archive",
    "bisect",
    "branch",
    "checkout",
    "cherry-pick",
    "clean",
    "clone",
    "commit",
    "config",
    "diff",
    "fetch",
    "format-patch",
    "grep",
    "init",
    "log",
    "merge",
    "mv",
    "pull",
    "push",
    "rebase",
    "reflog",
    "remote",
    "rename",
    "reset",
    "restore",
    "revert",
    "rm",
    "show",
    "sparse-checkout",
    "stash",
    "status",
    "switch",
    "tag",
    "worktree",
];
const GIT_OPTIONS: &[&str] = &[
    "--help",
    "--version",
    "--exec-path",
    "--git-dir",
    "--work-tree",
    "--bare",
    "--config-env",
    "-C",
    "-c",
    "-p",
    "--paginate",
    "--no-pager",
    "--no-replace-objects",
];
const CARGO_SUBCOMMANDS: &[&str] = &[
    "add",
    "bench",
    "build",
    "check",
    "clean",
    "clippy",
    "doc",
    "fetch",
    "fix",
    "fmt",
    "generate-lockfile",
    "install",
    "metadata",
    "new",
    "publish",
    "remove",
    "report",
    "run",
    "rustc",
    "rustdoc",
    "search",
    "test",
    "tree",
    "uninstall",
    "update",
    "vendor",
    "version",
    "locate-project",
];
const CARGO_OPTIONS: &[&str] = &[
    "--help",
    "--version",
    "--verbose",
    "--quiet",
    "--locked",
    "--offline",
    "--frozen",
    "--manifest-path",
    "--package",
    "--workspace",
    "--exclude",
    "--features",
    "--all-features",
    "--no-default-features",
    "--target",
];
const NPM_SUBCOMMANDS: &[&str] = &[
    "access",
    "audit",
    "bugs",
    "cache",
    "ci",
    "completion",
    "config",
    "dedupe",
    "deprecate",
    "diff",
    "dist-tag",
    "doctor",
    "docs",
    "exec",
    "explain",
    "explore",
    "fund",
    "help",
    "hook",
    "init",
    "install",
    "install-ci-test",
    "install-test",
    "link",
    "ll",
    "login",
    "logout",
    "ls",
    "org",
    "outdated",
    "owner",
    "pack",
    "ping",
    "pkg",
    "prefix",
    "profile",
    "prune",
    "publish",
    "query",
    "rebuild",
    "repo",
    "restart",
    "root",
    "run",
    "search",
    "set",
    "shrinkwrap",
    "start",
    "stop",
    "team",
    "test",
    "token",
    "uninstall",
    "unpublish",
    "update",
    "version",
    "view",
];
const NPM_OPTIONS: &[&str] = &[
    "--help",
    "--version",
    "--global",
    "--save",
    "--save-dev",
    "--save-exact",
    "--prefix",
    "--workspace",
    "--workspaces",
    "--include-workspace-root",
    "--ignore-scripts",
    "--production",
    "--json",
    "--silent",
    "--registry",
    "--yes",
];
const DOCKER_SUBCOMMANDS: &[&str] = &[
    "build",
    "builder",
    "buildx",
    "checkpoint",
    "commit",
    "compose",
    "config",
    "container",
    "context",
    "cp",
    "create",
    "diff",
    "events",
    "exec",
    "export",
    "history",
    "image",
    "images",
    "info",
    "init",
    "inspect",
    "kill",
    "load",
    "login",
    "logout",
    "logs",
    "manifest",
    "network",
    "node",
    "pause",
    "plugin",
    "port",
    "ps",
    "pull",
    "push",
    "rename",
    "restart",
    "rm",
    "rmi",
    "run",
    "save",
    "search",
    "secret",
    "service",
    "stack",
    "start",
    "stats",
    "stop",
    "swarm",
    "system",
    "tag",
    "top",
    "trust",
    "unpause",
    "update",
    "version",
    "volume",
    "wait",
];
const DOCKER_OPTIONS: &[&str] = &[
    "--help",
    "--version",
    "--config",
    "--context",
    "--debug",
    "--host",
    "--log-level",
    "--tls",
    "--tlscacert",
    "--tlscert",
    "--tlskey",
    "--tlsverify",
];
const PWSH_SUBCOMMANDS: &[&str] = &[];
const PWSH_OPTIONS: &[&str] = &[
    "-Command",
    "-EncodedCommand",
    "-EncodedArguments",
    "-ExecutionPolicy",
    "-File",
    "-InputFormat",
    "-Login",
    "-Mta",
    "-NoExit",
    "-NoLogo",
    "-NonInteractive",
    "-NoProfile",
    "-OutputFormat",
    "-Sta",
    "-Version",
    "-WindowStyle",
    "-WorkingDirectory",
    "--help",
    "--version",
];
const GH_SUBCOMMANDS: &[&str] = &[
    "alias",
    "api",
    "attestation",
    "auth",
    "browse",
    "codespace",
    "config",
    "copilot",
    "extension",
    "gist",
    "issue",
    "label",
    "org",
    "pr",
    "project",
    "release",
    "repo",
    "ruleset",
    "run",
    "search",
    "secret",
    "ssh-key",
    "status",
    "variable",
    "workflow",
];
const GH_OPTIONS: &[&str] = &[
    "--help",
    "--version",
    "--hostname",
    "--repo",
    "--json",
    "--jq",
    "--template",
    "--web",
    "--paginate",
    "--slurp",
];
const LOCATION_OPTIONS: &[&str] = &[
    "-Path",
    "-LiteralPath",
    "-Force",
    "-PassThru",
    "-StackName",
    "-UseTransaction",
    "-Verbose",
    "-ErrorAction",
    "-ErrorVariable",
];
const CHILD_ITEM_OPTIONS: &[&str] = &[
    "-Path",
    "-LiteralPath",
    "-Filter",
    "-Include",
    "-Exclude",
    "-Recurse",
    "-Force",
    "-Name",
    "-Directory",
    "-File",
    "-Depth",
    "-Attributes",
    "-FollowSymlink",
    "-Hidden",
    "-ReadOnly",
    "-System",
    "-ErrorAction",
    "-ErrorVariable",
    "-Verbose",
];

const COMMAND_SPECS: &[CommandSpec] = &[
    CommandSpec {
        name: "git",
        names: &["git"],
        subcommands: GIT_SUBCOMMANDS,
        options: GIT_OPTIONS,
    },
    CommandSpec {
        name: "cargo",
        names: &["cargo"],
        subcommands: CARGO_SUBCOMMANDS,
        options: CARGO_OPTIONS,
    },
    CommandSpec {
        name: "npm",
        names: &["npm"],
        subcommands: NPM_SUBCOMMANDS,
        options: NPM_OPTIONS,
    },
    CommandSpec {
        name: "docker",
        names: &["docker"],
        subcommands: DOCKER_SUBCOMMANDS,
        options: DOCKER_OPTIONS,
    },
    CommandSpec {
        name: "pwsh",
        names: &["pwsh", "powershell"],
        subcommands: PWSH_SUBCOMMANDS,
        options: PWSH_OPTIONS,
    },
    CommandSpec {
        name: "gh",
        names: &["gh"],
        subcommands: GH_SUBCOMMANDS,
        options: GH_OPTIONS,
    },
    CommandSpec {
        name: "Set-Location",
        names: &["set-location", "cd", "chdir", "sl"],
        subcommands: &[],
        options: LOCATION_OPTIONS,
    },
    CommandSpec {
        name: "Get-ChildItem",
        names: &["get-childitem", "gci", "dir", "ls"],
        subcommands: &[],
        options: CHILD_ITEM_OPTIONS,
    },
];

fn command_spec(command: &str) -> Option<&'static CommandSpec> {
    COMMAND_SPECS
        .iter()
        .find(|spec| spec.names.contains(&command))
}

fn add_static_candidates(
    output: &mut Vec<Candidate>,
    names: &[&str],
    kind: CandidateKind,
    command_name: &str,
    context: &TokenContext,
    limit: usize,
) {
    let prefix = context.value_before.to_ascii_lowercase();
    let mut seen = output
        .iter()
        .map(|candidate| command_key(&candidate.label))
        .collect::<HashSet<_>>();
    for name in names {
        if !name.to_ascii_lowercase().starts_with(&prefix) || !seen.insert(command_key(name)) {
            continue;
        }
        output.push(Candidate {
            label: (*name).to_owned(),
            insert_text: format_insert(name, context, false),
            description: format!("{command_name} command"),
            kind: kind.clone(),
        });
        if output.len() >= limit {
            break;
        }
    }
}

fn is_path_command(command: &str) -> bool {
    matches!(
        command,
        "cd" | "chdir"
            | "sl"
            | "set-location"
            | "get-childitem"
            | "gci"
            | "dir"
            | "ls"
            | "get-location"
            | "pwd"
            | "pushd"
            | "popd"
    )
}

fn looks_like_path(value: &str) -> bool {
    value.is_empty()
        || value.starts_with('.')
        || value.starts_with('~')
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('/')
        || value.contains('\\')
        || (value.len() >= 2 && value.as_bytes()[1] == b':')
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
    if is_remote_path(cwd) || is_remote_path(&directory) {
        return Vec::new();
    }
    let mut entries = Vec::new();
    let read_dir = match fs::read_dir(&directory) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let prefix_lower = name_prefix.to_ascii_lowercase();
    let started = Instant::now();
    for (entry_number, entry) in read_dir.flatten().enumerate() {
        if entry_number >= MAX_DIRECTORY_ENTRIES || started.elapsed() >= DIRECTORY_SCAN_BUDGET {
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().starts_with(&prefix_lower) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(value) => value,
            Err(_) => continue,
        };
        entries.push((name.to_ascii_lowercase(), name, metadata.is_dir()));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    let separator = value
        .rfind(['/', '\\'])
        .and_then(|index| value[index..].chars().next())
        .unwrap_or(path_separator());
    let mut candidates = Vec::new();
    for (_, name, is_directory) in entries {
        let mut insert = String::with_capacity(directory_text.len() + name.len() + 1);
        insert.push_str(directory_text);
        insert.push_str(&name);
        if is_directory {
            insert.push(separator);
        }
        candidates.push(Candidate {
            label: name,
            insert_text: format_insert(&insert, context, true),
            description: if is_directory {
                "Directory".to_owned()
            } else {
                "File".to_owned()
            },
            kind: if is_directory {
                CandidateKind::Directory
            } else {
                CandidateKind::File
            },
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
        // Trimming `C:\\` to `C:` turns an absolute drive root into a
        // drive-relative path on Windows.  Restore the root separator.
        path_text.push(trailing_separator.unwrap());
    }
    if path_text == "~" || path_text.starts_with("~/") || path_text.starts_with("~\\") {
        if let Some(home) = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")) {
            let suffix = path_text[1..].trim_start_matches(['/', '\\']);
            return if suffix.is_empty() {
                PathBuf::from(home)
            } else {
                PathBuf::from(home).join(suffix)
            };
        }
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

fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
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

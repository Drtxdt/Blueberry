//! In-memory directory snapshots; no file-system work is performed by the UI.
use crate::model::{Candidate, CandidateKind, InputContext};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::SystemTime,
};

#[derive(Default)]
pub struct PathCache {
    directories: HashMap<PathBuf, Directory>,
    clock: u64,
}
struct Directory {
    modified: Option<SystemTime>,
    entries: Vec<(String, bool)>,
    used: u64,
}
#[derive(Default)]
pub struct PathResult {
    pub candidates: Vec<Candidate>,
    pub incomplete: bool,
    pub diagnostic: Option<String>,
}

impl PathCache {
    pub fn invalidate(&mut self) {
        self.directories.clear();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete(
        &mut self,
        line: &str,
        _cursor: usize,
        context: &InputContext,
        cwd: &Path,
        environment: &BTreeMap<String, String>,
        directories_only: bool,
        cancel: &AtomicBool,
    ) -> PathResult {
        let mut result = PathResult::default();
        if context.suppressed {
            return result;
        }
        let original_prefix = context.prefix.as_str();
        let (head, prefix) = if original_prefix.starts_with('-') {
            original_prefix
                .find('=')
                .map(|i| (&original_prefix[..i + 1], &original_prefix[i + 1..]))
                .unwrap_or(("", original_prefix))
        } else {
            ("", original_prefix)
        };
        let raw = line
            .get(context.replace_start..context.replace_end)
            .unwrap_or("");
        let raw_value = if head.is_empty() {
            raw
        } else {
            raw_value_part(raw)
        };
        let (quote, _) = quote_state(raw_value);
        if let Some((braced, fragment)) = environment_name_fragment(prefix) {
            // A single-quoted `$env:` token is literal text. Do not expose
            // environment names here and do not expand it as a path below.
            if quote == Some('\'') {
                return result;
            }
            for name in environment.keys().filter(|name| {
                name.to_ascii_lowercase()
                    .starts_with(&fragment.to_ascii_lowercase())
            }) {
                let value = environment_name(name, braced);
                // Keep a non-braced display label prefix-compatible with the
                // user's `$env:...` input even when the legal insertion must
                // switch to `${env:...}` for a name containing parentheses.
                let label = if !braced && !simple_environment_name(name) {
                    format!("$env:{name}")
                } else {
                    value.clone()
                };
                let insert = format_environment_name(&value, raw_value);
                result.candidates.push(Candidate {
                    label,
                    insert_text: insert,
                    description: "环境变量名称".into(),
                    kind: CandidateKind::Value,
                    id: format!("environment:{name}"),
                    source: "当前 PowerShell 环境".into(),
                    ..Default::default()
                });
            }
            return result;
        }

        let single_quoted = quote == Some('\'');
        let expanded = (!single_quoted)
            .then(|| expand(prefix, environment))
            .flatten();
        let scan_value = expanded.as_deref().unwrap_or(prefix);
        // PowerShell expands `~` for cmdlets and directory navigation. Native
        // commands on pwsh 7.4 do not consistently receive that expansion, so
        // keep the spelling for known PowerShell path commands and use the
        // absolute scan value for native commands.
        let display_value = if !single_quoted
            && expanded.is_some()
            && starts_with_tilde(prefix)
            && !preserves_tilde(context, directories_only)
        {
            scan_value
        } else {
            prefix
        };
        let (scan_parent, scan_needle) = split_parent(scan_value);
        let display_parent = if display_value == "~" {
            if cfg!(windows) { "~\\" } else { "~/" }
        } else {
            split_parent(display_value).0
        };
        let directory = if scan_parent.is_empty() {
            cwd.to_path_buf()
        } else if Path::new(scan_parent).is_absolute() {
            PathBuf::from(scan_parent)
        } else {
            cwd.join(scan_parent)
        };
        if remote(&directory) || remote(cwd) {
            result.diagnostic = Some("网络路径请使用手动原生补全".into());
            return result;
        }
        let stamp = fs::metadata(&directory)
            .ok()
            .and_then(|m| m.modified().ok());
        self.clock = self.clock.wrapping_add(1);
        let needs_scan = self
            .directories
            .get(&directory)
            .is_none_or(|cached| cached.modified != stamp);
        let mut partial = None;
        if needs_scan {
            let mut entries = Vec::new();
            let read = match fs::read_dir(&directory) {
                Ok(read) => read,
                Err(error) => {
                    result.diagnostic = Some(format!("无法读取目录：{}", error.kind()));
                    return result;
                }
            };
            for (index, entry) in read.enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    result.incomplete = true;
                    return result;
                }
                if index % 256 == 0 {
                    std::thread::yield_now();
                }
                let Ok(entry) = entry else {
                    result.incomplete = true;
                    continue;
                };
                let Ok(kind) = entry.file_type() else {
                    result.incomplete = true;
                    continue;
                };
                let is_dir = if kind.is_symlink() {
                    // Resolve before following, so an explicit remote link is not opened.
                    if !crate::engine::local_link(&entry.path()) {
                        continue;
                    }
                    let Ok(metadata) = fs::metadata(entry.path()) else {
                        continue;
                    };
                    if !metadata.is_file() && !metadata.is_dir() {
                        continue;
                    }
                    metadata.is_dir()
                } else if kind.is_file() || kind.is_dir() {
                    kind.is_dir()
                } else {
                    continue;
                };
                entries.push((entry.file_name().to_string_lossy().into_owned(), is_dir));
            }
            entries.sort_by_cached_key(|(name, is_dir)| (!*is_dir, name.to_lowercase()));
            if !result.incomplete {
                if self.directories.len() >= 64
                    && !self.directories.contains_key(&directory)
                    && let Some(old) = self
                        .directories
                        .iter()
                        .min_by_key(|(_, v)| v.used)
                        .map(|(k, _)| k.clone())
                {
                    self.directories.remove(&old);
                }
                self.directories.insert(
                    directory.clone(),
                    Directory {
                        modified: stamp,
                        entries,
                        used: self.clock,
                    },
                );
            } else {
                partial = Some(entries);
            }
        }
        let entries = if let Some(entries) = partial.as_ref() {
            entries
        } else {
            let Some(snapshot) = self.directories.get_mut(&directory) else {
                return result;
            };
            snapshot.used = self.clock;
            &snapshot.entries
        };
        let needle = scan_needle.to_lowercase();
        let separator = if display_value.contains('/') && !display_value.contains('\\') {
            '/'
        } else if display_value.contains('\\') {
            '\\'
        } else {
            std::path::MAIN_SEPARATOR
        };
        let preserve_env_prefix = if display_value == prefix {
            display_value.len()
        } else {
            0
        };
        for (name, is_dir) in entries {
            if cancel.load(Ordering::Relaxed) {
                result.incomplete = true;
                break;
            }
            if (directories_only && !is_dir) || !name.to_lowercase().starts_with(&needle) {
                continue;
            }
            let insert = format!(
                "{display_parent}{name}{}",
                if *is_dir {
                    separator.to_string()
                } else {
                    String::new()
                }
            );
            let formatted = format_path_value(
                head,
                &insert,
                preserve_env_prefix.min(display_value.len()),
                raw_value,
                *is_dir,
            );
            result.candidates.push(Candidate {
                label: name.clone(),
                insert_text: formatted,
                description: if *is_dir { "目录" } else { "文件" }.into(),
                kind: if *is_dir {
                    CandidateKind::Directory
                } else {
                    CandidateKind::File
                },
                id: format!("path:{}", directory.join(name).display()),
                source: "本地路径".into(),
                ..Default::default()
            });
        }
        result
    }
}

fn raw_value_part(raw: &str) -> &str {
    raw.find('=').map(|index| &raw[index + 1..]).unwrap_or(raw)
}

/// Return the first quote on a token and whether that quote is closed. The
/// parser intentionally mirrors the small lexical rules used by the engine:
/// doubled single quotes and backtick-escaped characters do not close a token.
fn quote_state(raw: &str) -> (Option<char>, bool) {
    let Some(first) = raw.as_bytes().first().copied() else {
        return (None, false);
    };
    let quote = match first {
        b'\'' => '\'',
        b'"' => '"',
        _ => return (None, false),
    };
    let bytes = raw.as_bytes();
    let mut index = 1;
    while index < bytes.len() {
        if quote == '\'' {
            if bytes[index] == b'\'' {
                if index + 1 < bytes.len() && bytes[index + 1] == b'\'' {
                    index += 2;
                    continue;
                }
                return (Some(quote), true);
            }
        } else {
            if bytes[index] == b'`' {
                index += 1;
                if index < bytes.len() {
                    index += raw[index..].chars().next().map(char::len_utf8).unwrap_or(1);
                }
                continue;
            }
            if bytes[index] == b'"' {
                return (Some(quote), true);
            }
        }
        index += raw[index..].chars().next().map(char::len_utf8).unwrap_or(1);
    }
    (Some(quote), false)
}

fn starts_with_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// Return the end of a PowerShell environment expression at `start`, but only
/// when the expression is wholly inside `limit`. This lets the formatter keep
/// `$env:NAME` or `${env:NAME}` executable while escaping literal `$` signs in
/// the candidate's newly inserted filename.
fn environment_expression_end(text: &str, start: usize, limit: usize) -> Option<usize> {
    let limit = limit.min(text.len());
    if start >= limit {
        return None;
    }
    if starts_with_ascii_case(&text[start..limit], "${env:") {
        let close = text[start + 6..limit].find('}')?;
        let end = start + 6 + close + 1;
        return (end <= limit).then_some(end);
    }
    if starts_with_ascii_case(&text[start..limit], "$env:") {
        let mut end = start + 5;
        while end < limit {
            let character = text[end..].chars().next()?;
            if character == '/' || character == '\\' || character.is_whitespace() {
                break;
            }
            end += character.len_utf8();
        }
        return (end > start + 5).then_some(end);
    }
    None
}

fn environment_name_fragment(prefix: &str) -> Option<(bool, &str)> {
    if starts_with_ascii_case(prefix, "${env:") {
        let rest = &prefix[6..];
        if rest.contains(['/', '\\']) {
            return None;
        }
        return Some((true, rest.strip_suffix('}').unwrap_or(rest)));
    }
    if starts_with_ascii_case(prefix, "$env:") {
        let rest = &prefix[5..];
        if rest.contains(['/', '\\']) {
            return None;
        }
        return Some((false, rest));
    }
    None
}

fn simple_environment_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn environment_name(name: &str, braced: bool) -> String {
    if braced || !simple_environment_name(name) {
        format!("${{env:{name}}}")
    } else {
        format!("$env:{name}")
    }
}

fn format_environment_name(value: &str, raw: &str) -> String {
    let (quote, closed) = quote_state(raw);
    if quote == Some('"') {
        let mut result = String::from("\"");
        result.push_str(&escape_double_preserving_env(value, value.len()));
        if closed {
            result.push('"');
        }
        result
    } else {
        value.to_owned()
    }
}

fn split_parent(value: &str) -> (&str, &str) {
    value
        .rfind(['/', '\\'])
        .map(|index| (&value[..index + 1], &value[index + 1..]))
        .unwrap_or(("", value))
}

fn starts_with_tilde(value: &str) -> bool {
    value == "~" || value.starts_with("~/") || value.starts_with("~\\")
}

fn preserves_tilde(context: &InputContext, directories_only: bool) -> bool {
    if directories_only {
        return true;
    }
    let command = context
        .command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&context.command)
        .to_ascii_lowercase();
    matches!(
        command.as_str(),
        "cd" | "chdir"
            | "pushd"
            | "push-location"
            | "set-location"
            | "sl"
            | "get-childitem"
            | "gci"
            | "dir"
            | "ls"
            | "get-content"
            | "gc"
            | "cat"
            | "type"
            | "get-item"
            | "gi"
            | "copy-item"
            | "cp"
            | "copy"
            | "move-item"
            | "mi"
            | "mv"
            | "move"
            | "remove-item"
            | "ri"
            | "rm"
            | "del"
            | "erase"
            | "rd"
            | "mkdir"
            | "md"
            | "new-item"
            | "ni"
            | "rename-item"
            | "rni"
            | "ren"
            | "resolve-path"
            | "join-path"
            | "split-path"
            | "test-path"
            | "get-acl"
            | "set-acl"
            | "invoke-item"
            | "ii"
    )
}

fn escape_single(text: &str) -> String {
    text.replace('\'', "''")
}

fn escape_double_preserving_env(text: &str, preserve_prefix: usize) -> String {
    let limit = preserve_prefix.min(text.len());
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        if index < limit
            && let Some(end) = environment_expression_end(text, index, limit)
        {
            result.push_str(&text[index..end]);
            index = end;
            continue;
        }
        let character = text[index..].chars().next().unwrap_or('\0');
        if matches!(character, '`' | '"' | '$') {
            result.push('`');
        }
        result.push(character);
        index += character.len_utf8();
    }
    result
}

fn needs_double_quote(text: &str, preserve_prefix: usize) -> bool {
    let limit = preserve_prefix.min(text.len());
    let mut index = 0;
    while index < text.len() {
        if index < limit
            && let Some(end) = environment_expression_end(text, index, limit)
        {
            index = end;
            continue;
        }
        let character = text[index..].chars().next().unwrap_or('\0');
        if character.is_whitespace()
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
            || (index == 0 && character == '#')
        {
            return true;
        }
        index += character.len_utf8();
    }
    false
}

fn escape_unquoted(text: &str, preserve_prefix: usize) -> String {
    let limit = preserve_prefix.min(text.len());
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        if index < limit
            && let Some(end) = environment_expression_end(text, index, limit)
        {
            result.push_str(&text[index..end]);
            index = end;
            continue;
        }
        let character = text[index..].chars().next().unwrap_or('\0');
        if matches!(character, '`' | '$') {
            result.push('`');
        }
        result.push(character);
        index += character.len_utf8();
    }
    result
}

fn format_path_value(
    head: &str,
    text: &str,
    preserve_env_prefix: usize,
    raw: &str,
    is_dir: bool,
) -> String {
    let (quote, closed) = quote_state(raw);
    let close = closed || !is_dir;
    let body = match quote {
        Some('\'') => {
            let mut value = String::from("'");
            value.push_str(&escape_single(text));
            if close {
                value.push('\'');
            }
            value
        }
        Some('"') => {
            let mut value = String::from("\"");
            value.push_str(&escape_double_preserving_env(text, preserve_env_prefix));
            if close {
                value.push('"');
            }
            value
        }
        Some(_) => escape_unquoted(text, preserve_env_prefix),
        None if needs_double_quote(text, preserve_env_prefix) => {
            let mut value = String::from("\"");
            value.push_str(&escape_double_preserving_env(text, preserve_env_prefix));
            if !is_dir {
                value.push('"');
            }
            value
        }
        None => escape_unquoted(text, preserve_env_prefix),
    };
    format!("{head}{body}")
}

fn expand(prefix: &str, environment: &BTreeMap<String, String>) -> Option<String> {
    if starts_with_tilde(prefix) {
        let home = environment
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("USERPROFILE"))
            .or_else(|| {
                environment
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case("HOME"))
            })?
            .1;
        return Some(format!(
            "{home}{}",
            if prefix == "~" {
                if cfg!(windows) { "\\" } else { "/" }
            } else {
                &prefix[1..]
            }
        ));
    }
    let (end, name) = if starts_with_ascii_case(prefix, "${env:") {
        let close = prefix[6..].find('}')?;
        (6 + close + 1, &prefix[6..6 + close])
    } else if starts_with_ascii_case(prefix, "$env:") {
        let suffix = &prefix[5..];
        let end = suffix
            .find(['/', '\\'])
            .map(|index| index + 5)
            .unwrap_or(prefix.len());
        (end, &prefix[5..end])
    } else {
        return None;
    };
    let value = environment
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))?
        .1;
    Some(format!("{value}{}", &prefix[end..]))
}
fn remote(path: &Path) -> bool {
    let text = path.to_string_lossy();
    (text.starts_with("\\\\") && !text.starts_with("\\\\?\\"))
        || text.starts_with("//")
        || text.to_ascii_lowercase().starts_with("\\\\?\\unc\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context_for(line: &str, prefix: &str, command: &str) -> InputContext {
        let mut start = line.find(prefix).unwrap();
        if start > 0 && matches!(line.as_bytes()[start - 1], b'\'' | b'"') {
            start -= 1;
        }
        InputContext {
            command: command.to_owned(),
            prefix: prefix.to_owned(),
            replace_start: start,
            replace_end: line.len(),
            ..Default::default()
        }
    }

    fn candidate<'a>(result: &'a PathResult, label: &str) -> &'a Candidate {
        result
            .candidates
            .iter()
            .find(|candidate| candidate.label == label)
            .unwrap_or_else(|| panic!("missing candidate {label}: {:?}", result.candidates))
    }

    #[test]
    fn directories_environment_and_invalidation() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("中文目录")).unwrap();
        fs::write(tmp.path().join("file.txt"), "").unwrap();
        let mut cache = PathCache::default();
        let cancel = AtomicBool::new(false);
        let context = InputContext::default();
        let result = cache.complete("", 0, &context, tmp.path(), &BTreeMap::new(), true, &cancel);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].label, "中文目录");
        let context = InputContext {
            prefix: "$env:PA".into(),
            replace_end: 7,
            ..Default::default()
        };
        let result = cache.complete(
            "$env:PA",
            7,
            &context,
            tmp.path(),
            &BTreeMap::from([("PATH".into(), "secret".into())]),
            false,
            &cancel,
        );
        assert_eq!(result.candidates[0].insert_text, "$env:PATH");
        assert!(!format!("{:?}", result.candidates).contains("secret"));
        cancel.store(true, Ordering::Relaxed);
        cache.invalidate();
        assert!(
            cache
                .complete(
                    "",
                    0,
                    &InputContext::default(),
                    tmp.path(),
                    &BTreeMap::new(),
                    false,
                    &cancel
                )
                .incomplete
        );
    }

    #[test]
    fn environment_paths_keep_expression_and_quote_spaces_with_double_quotes() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("child");
        fs::create_dir(&child).unwrap();
        fs::write(child.join("space file.txt"), "").unwrap();
        fs::create_dir(root.path().join("child dir")).unwrap();
        let separator = std::path::MAIN_SEPARATOR;
        let environment = BTreeMap::from([(
            "ROOT".to_owned(),
            root.path().to_string_lossy().into_owned(),
        )]);
        let prefix = format!("$env:ROOT{separator}child{separator}spa");
        let line = format!("Get-ChildItem {prefix}");
        let context = context_for(&line, &prefix, "Get-ChildItem");
        let mut cache = PathCache::default();
        let result = cache.complete(
            &line,
            line.len(),
            &context,
            root.path(),
            &environment,
            false,
            &AtomicBool::new(false),
        );
        let file = candidate(&result, "space file.txt");
        assert_eq!(
            file.insert_text,
            format!("\"$env:ROOT{separator}child{separator}space file.txt\"")
        );
        assert!(!file.insert_text.contains("`$env:"));

        let prefix = format!("$env:ROOT{separator}child");
        let line = format!("Get-ChildItem {prefix}");
        let context = context_for(&line, &prefix, "Get-ChildItem");
        let result = cache.complete(
            &line,
            line.len(),
            &context,
            root.path(),
            &environment,
            false,
            &AtomicBool::new(false),
        );
        let directory = candidate(&result, "child dir");
        assert_eq!(
            directory.insert_text,
            format!("\"$env:ROOT{separator}child dir{separator}")
        );
        assert!(!directory.insert_text.ends_with('"'));
    }

    #[test]
    fn single_quoted_environment_path_is_literal_and_not_expanded() {
        let cwd = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("child")).unwrap();
        fs::write(root.path().join("child").join("space file.txt"), "").unwrap();
        let separator = std::path::MAIN_SEPARATOR;
        let environment = BTreeMap::from([(
            "ROOT".to_owned(),
            root.path().to_string_lossy().into_owned(),
        )]);
        let prefix = format!("$env:ROOT{separator}child{separator}spa");
        let line = format!("Get-ChildItem '{prefix}");
        let context = context_for(&line, &prefix, "Get-ChildItem");
        let result = PathCache::default().complete(
            &line,
            line.len(),
            &context,
            cwd.path(),
            &environment,
            false,
            &AtomicBool::new(false),
        );
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn braced_environment_names_and_paths_are_valid_power_shell_text() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("hello.txt"), "").unwrap();
        let separator = std::path::MAIN_SEPARATOR;
        let environment = BTreeMap::from([(
            "ProgramFiles(x86)".to_owned(),
            root.path().to_string_lossy().into_owned(),
        )]);
        let prefix = format!("${{env:ProgramFiles(x86)}}{separator}hel");
        let line = format!("Get-ChildItem {prefix}");
        let context = context_for(&line, &prefix, "Get-ChildItem");
        let result = PathCache::default().complete(
            &line,
            line.len(),
            &context,
            root.path(),
            &environment,
            false,
            &AtomicBool::new(false),
        );
        let file = candidate(&result, "hello.txt");
        assert_eq!(
            file.insert_text,
            format!("${{env:ProgramFiles(x86)}}{separator}hello.txt")
        );
        assert!(!file.insert_text.contains("`$"));

        let prefix = "$env:Program";
        let line = format!("Get-ChildItem {prefix}");
        let context = context_for(&line, prefix, "Get-ChildItem");
        let result = PathCache::default().complete(
            &line,
            line.len(),
            &context,
            root.path(),
            &environment,
            false,
            &AtomicBool::new(false),
        );
        let variable = candidate(&result, "$env:ProgramFiles(x86)");
        assert_eq!(variable.insert_text, "${env:ProgramFiles(x86)}");
    }

    #[test]
    fn tilde_is_kept_for_power_shell_paths_but_expanded_for_native_commands() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("tilde-dir")).unwrap();
        let environment = BTreeMap::from([(
            "USERPROFILE".to_owned(),
            root.path().to_string_lossy().into_owned(),
        )]);
        let separator = std::path::MAIN_SEPARATOR;
        let prefix = format!("~{separator}tilde");
        let line = format!("Get-ChildItem {prefix}");
        let context = context_for(&line, &prefix, "Get-ChildItem");
        let result = PathCache::default().complete(
            &line,
            line.len(),
            &context,
            root.path(),
            &environment,
            false,
            &AtomicBool::new(false),
        );
        let powershell = candidate(&result, "tilde-dir");
        assert_eq!(
            powershell.insert_text,
            format!("~{separator}tilde-dir{separator}")
        );

        let prefix = "~";
        let line = format!("Get-ChildItem {prefix}");
        let context = context_for(&line, prefix, "Get-ChildItem");
        let result = PathCache::default().complete(
            &line,
            line.len(),
            &context,
            root.path(),
            &environment,
            false,
            &AtomicBool::new(false),
        );
        let powershell = candidate(&result, "tilde-dir");
        assert_eq!(
            powershell.insert_text,
            format!("~{separator}tilde-dir{separator}")
        );

        let line = format!("rg {prefix}");
        let context = context_for(&line, prefix, "rg");
        let result = PathCache::default().complete(
            &line,
            line.len(),
            &context,
            root.path(),
            &environment,
            false,
            &AtomicBool::new(false),
        );
        let native = candidate(&result, "tilde-dir");
        assert!(
            native
                .insert_text
                .starts_with(root.path().to_string_lossy().as_ref())
        );
        assert!(!native.insert_text.starts_with('~'));
    }

    #[test]
    fn files_close_open_quotes_while_directories_stay_open() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("中文😀 file.txt"), "").unwrap();
        fs::create_dir(root.path().join("中文😀 folder")).unwrap();
        for quote in ['\'', '"'] {
            let prefix = "中文😀";
            let line = format!("Get-ChildItem {quote}{prefix}");
            let context = context_for(&line, prefix, "Get-ChildItem");
            let result = PathCache::default().complete(
                &line,
                line.len(),
                &context,
                root.path(),
                &BTreeMap::new(),
                false,
                &AtomicBool::new(false),
            );
            let file = candidate(&result, "中文😀 file.txt");
            assert_eq!(file.insert_text, format!("{quote}中文😀 file.txt{quote}"));
        }
        let line = "Get-ChildItem \"中文😀";
        let context = context_for(line, "中文😀", "Get-ChildItem");
        let result = PathCache::default().complete(
            line,
            line.len(),
            &context,
            root.path(),
            &BTreeMap::new(),
            false,
            &AtomicBool::new(false),
        );
        let directory = candidate(&result, "中文😀 folder");
        assert_eq!(
            directory.insert_text,
            format!("\"中文😀 folder{}", std::path::MAIN_SEPARATOR)
        );
    }

    #[test]
    fn environment_path_suffix_does_not_duplicate_existing_quote() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("child");
        fs::create_dir(&child).unwrap();
        fs::write(child.join("space file.txt"), "").unwrap();
        let separator = std::path::MAIN_SEPARATOR;
        let environment = BTreeMap::from([(
            "ROOT".to_owned(),
            root.path().to_string_lossy().into_owned(),
        )]);
        let prefix = format!("$env:ROOT{separator}child{separator}spa");
        let line = format!("Get-ChildItem \"{prefix}XYZ\"");
        let cursor = line.find("spa").unwrap() + "spa".len();
        let context = context_for(&line, &prefix, "Get-ChildItem");
        let mut result = PathCache::default().complete(
            &line,
            cursor,
            &context,
            root.path(),
            &environment,
            false,
            &AtomicBool::new(false),
        );
        let mut file = candidate(&result, "space file.txt").clone();
        crate::completion::preserve_suffix(
            &mut file,
            &line,
            cursor,
            context.replace_start,
            context.replace_end,
        );
        assert_eq!(
            file.insert_text,
            format!("\"$env:ROOT{separator}child{separator}space file.txtXYZ\"")
        );
        result.candidates.clear();
    }
}

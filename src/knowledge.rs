//! Local command help, with bounded execution and entry-specific caches.
use crate::model::Candidate;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub fn clean(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.next() {
                Some('[') => {
                    for x in chars.by_ref() {
                        if ('@'..='~').contains(&x) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(x) = chars.next() {
                        if x == '\x07' || (x == '\x1b' && chars.next() == Some('\\')) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if c == '\n' || c == '\t' || !c.is_control() {
            result.push(c);
        }
    }
    result
}
pub fn synopsis(label: &str, text: &str) -> Option<String> {
    let text = clean(text);
    let line = text.lines().map(str::trim).find(|s| !s.is_empty())?;
    if line.eq_ignore_ascii_case(label)
        || line == "参数选项"
        || line.starts_with("用途暂未收录")
        || line.starts_with("\\\\")
        || line.starts_with("/:")
        || (line.as_bytes().get(1) == Some(&b':')
            && matches!(line.as_bytes().get(2), Some(b'\\' | b'/')))
        || line.starts_with('/') && !line.contains(' ')
    {
        return None;
    }
    let end = line
        .find('。')
        .map(|i| i + '。'.len_utf8())
        .or_else(|| line.find(". ").map(|i| i + 1))
        .unwrap_or(line.len());
    Some(line[..end].chars().take(240).collect())
}
pub fn annotate(candidate: &mut Candidate) {
    if candidate.description_source.is_empty() {
        candidate.description_source = if candidate.source.is_empty() {
            "builtin".into()
        } else {
            candidate.source.clone()
        };
    }
    candidate.language = if candidate
        .description
        .chars()
        .any(|c| ('\u{3400}'..='\u{9fff}').contains(&c))
    {
        "zh"
    } else {
        "en"
    }
    .into();
}
pub fn native_description(candidate: &mut Candidate) {
    let original = clean(&candidate.description);
    candidate.description =
        synopsis(&candidate.label, &original).unwrap_or_else(|| "原生补全".into());
    candidate.detail = original;
    candidate.description_source = "PowerShell ToolTip".into();
    annotate(candidate);
}

const PARSER_VERSION: u32 = 2;
const OUTPUT_LIMIT: usize = 256 * 1024;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HelpOption {
    pub names: Vec<String>,
    pub description: String,
    pub detail: String,
    pub value_name: String,
    pub values: Vec<String>,
    pub repeatable: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HelpPage {
    pub description: String,
    pub options: Vec<HelpOption>,
    pub commands: BTreeMap<String, String>,
    #[serde(default)]
    pub command_aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub options_complete: bool,
    #[serde(default)]
    pub commands_complete: bool,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub adapter: String,
    /// Legacy protocol/cache field. Version 1 pages are additive only.
    #[serde(default)]
    pub complete: bool,
}

/// Parse only documented option/command sections; prose is never executable input.
pub fn parse_help(text: &str) -> HelpPage {
    let text = clean(text);
    let mut page = HelpPage::default();
    let mut section = "";
    let mut active: Option<usize> = None;
    let mut saw_options = false;
    let mut saw_commands = false;
    let mut options_uncertain = false;
    let mut commands_uncertain = false;
    let mut command_indent = None;
    let mut enum_list = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        if [
            "options:",
            "optional arguments:",
            "选项:",
            "选项：",
            "以下选项可用：",
        ]
        .contains(&lower.as_str())
        {
            section = "options";
            active = None;
            saw_options = true;
            enum_list = false;
            continue;
        }
        if [
            "commands:",
            "available commands:",
            "subcommands:",
            "命令:",
            "命令：",
            "以下命令可用：",
        ]
        .contains(&lower.as_str())
        {
            section = "commands";
            active = None;
            saw_commands = true;
            continue;
        }
        if !raw.starts_with(char::is_whitespace) && line.ends_with(':') {
            section = "";
            active = None;
        }
        if page.description.is_empty()
            && !lower.starts_with("usage:")
            && !lower.starts_with("warning:")
            && !line.ends_with(':')
        {
            page.description = line.to_owned();
        }
        // Git -h and some argparse programs omit an Options heading.
        if line.starts_with('-')
            && (section == "options" || section.is_empty())
            && !line.starts_with("-- ")
        {
            let boundary = line
                .as_bytes()
                .windows(2)
                .position(|w| w == b"  ")
                .unwrap_or(line.len());
            let syntax = &line[..boundary];
            if syntax.contains("[=") || syntax.contains("[<") {
                options_uncertain = true;
                continue;
            }
            let names: Vec<String> = syntax
                .split(|c: char| c.is_whitespace() || c == ',' || c == '=' || c == '|')
                .filter(|s| {
                    s.starts_with('-')
                        && s.len() > 1
                        && s.chars()
                            .all(|c| c.is_ascii_alphanumeric() || "-_?".contains(c))
                })
                .map(str::to_owned)
                .collect();
            if !names.is_empty() {
                let value_name = syntax
                    .find('<')
                    .and_then(|a| syntax[a..].find('>').map(|b| syntax[a..=a + b].to_owned()))
                    .or_else(|| {
                        syntax
                            .split_whitespace()
                            .find(|s| {
                                s.len() > 1 && s.chars().all(|c| c.is_ascii_uppercase() || c == '_')
                            })
                            .map(|s| format!("<{s}>"))
                    })
                    .unwrap_or_default();
                let desc = line[boundary..].trim().to_owned();
                page.options.push(HelpOption {
                    names,
                    description: desc.clone(),
                    detail: desc,
                    value_name,
                    repeatable: syntax.ends_with("..."),
                    values: Vec::new(),
                });
                active = Some(page.options.len() - 1);
                enum_list = false;
                continue;
            }
        }
        if section == "commands" {
            let indent = raw.len() - raw.trim_start().len();
            if command_indent.is_some_and(|old| indent > old) {
                continue;
            }
            if line == "..." || line.starts_with("... ") {
                commands_uncertain = true;
                continue;
            }
            let boundary = line
                .as_bytes()
                .windows(2)
                .position(|window| window == b"  ")
                .unwrap_or(line.len());
            let syntax = line[..boundary].trim();
            let desc = line[boundary..].trim();
            let syntax = syntax
                .strip_suffix(')')
                .and_then(|value| value.split_once(" ("))
                .map(|(name, aliases)| format!("{name}, {aliases}"))
                .unwrap_or_else(|| syntax.to_owned());
            let names = syntax
                .split([',', '|'])
                .flat_map(str::split_whitespace)
                .map(|name| name.trim_matches(['(', ')']))
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>();
            let valid = |name: &str| {
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            };
            if indent > 0 && !names.is_empty() && names.iter().all(|name| valid(name)) {
                command_indent = Some(indent);
                let name = names[0];
                page.commands.insert(
                    name.to_owned(),
                    if desc.is_empty() {
                        "子命令".into()
                    } else {
                        desc.to_owned()
                    },
                );
                for alias in names.into_iter().skip(1) {
                    page.command_aliases.insert(alias.to_owned(), name.to_owned());
                }
            } else if indent > 0 {
                commands_uncertain = true;
            }
        } else if let Some(index) = active {
            let option = &mut page.options[index];
            if lower == "possible values:" {
                enum_list = true;
            }
            if enum_list
                && let Some(value) = line
                    .strip_prefix("- ")
                    .and_then(|s| s.split_once(':').map(|(v, _)| v))
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                option.values.push(value.into());
            }
            if !option.detail.is_empty() {
                option.detail.push('\n');
            }
            option.detail.push_str(line);
            if option.description.is_empty() {
                option.description = line.to_owned();
            }
            if let Some(start) = lower.find("[possible values:")
                && let Some(end) = line[start..].find(']')
            {
                option.values = line[start + 17..start + end]
                    .split(',')
                    .map(|v| v.trim().to_owned())
                    .filter(|v| !v.is_empty())
                    .collect();
            }
        }
    }
    for option in &mut page.options {
        if option.description.is_empty() {
            option.description = format!("{} {}", option.names.join(", "), option.value_name)
                .trim()
                .to_owned();
        }
    }
    // Authority is section-specific. A missing or uncertain section is
    // additive, so malformed help can never delete curated built-ins.
    page.options_complete = saw_options && !page.options.is_empty() && !options_uncertain;
    page.commands_complete = saw_commands && !page.commands.is_empty() && !commands_uncertain;
    page.complete = page.options_complete && (!saw_commands || page.commands_complete);
    page.adapter = "sectioned-help".into();
    page
}

fn parse_cargo_list(text: &str) -> HelpPage {
    let mut page = HelpPage {
        adapter: "cargo-list".into(),
        ..Default::default()
    };
    let mut header = false;
    let mut uncertain = false;
    for raw in clean(text).lines() {
        let line = raw.trim();
        if line.eq_ignore_ascii_case("Installed Commands:") {
            header = true;
            continue;
        }
        if !header || line.is_empty() {
            continue;
        }
        let boundary = line
            .as_bytes()
            .windows(2)
            .position(|window| window == b"  ")
            .unwrap_or(line.len());
        let name = line[..boundary].trim();
        let description = line[boundary..].trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            uncertain = true;
            continue;
        }
        if let Some(target) = description.strip_prefix("alias:").map(str::trim) {
            if !target.is_empty() {
                page.command_aliases.insert(name.into(), target.into());
            } else {
                uncertain = true;
            }
        } else if !description.starts_with("REMOVED:") {
            page.commands.insert(
                name.into(),
                if description.is_empty() {
                    "本机安装的 Cargo 子命令".into()
                } else {
                    description.into()
                },
            );
        }
    }
    page.commands_complete = header && !page.commands.is_empty() && !uncertain;
    page.complete = page.commands_complete;
    page
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub command: String,
    pub path: PathBuf,
    pub target: PathBuf,
    pub fingerprint: String,
    pub trusted: bool,
    #[serde(default)]
    pub script: Option<PathBuf>,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn stamp(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    Some(format!(
        "{}:{}:{}",
        path.display(),
        meta.len(),
        meta.modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos()
    ))
}
pub fn entry(command: &str, path: &Path, cwd: &Path) -> Option<Entry> {
    // Preserve argv[0] for multicall launchers such as rustup symlinks.
    let launch_path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let path = fs::canonicalize(path).ok()?;
    if !path.is_file() {
        return None;
    }
    let root = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .to_ascii_lowercase();
    let invoked_root = root
        .trim_end_matches(".exe")
        .trim_end_matches(".cmd")
        .trim_end_matches(".ps1")
        .to_owned();
    let definition = crate::tool_registry::definition(&invoked_root);
    let root = definition
        .map(|tool| tool.name.to_ascii_lowercase())
        .unwrap_or(invoked_root);
    let known = definition.is_some_and(|tool| tool.help != "powershell");
    let mut target = launch_path;
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let mut packaged = false;
    let mut script = None;
    if ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("ps1") {
        let wrapper = fs::read_to_string(&path).ok()?;
        // Recognize the standard npm-generated shim and package, never interpret it.
        let package = if root == "codex" {
            "@openai/codex"
        } else {
            root.as_str()
        };
        let base = path.parent()?.join("node_modules").join(package);
        let manifest_path = base.join("package.json");
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).ok()?).ok()?;
        if manifest["name"].as_str() != Some(package) {
            return None;
        }
        let bin = manifest["bin"][&root]
            .as_str()
            .or_else(|| manifest["bin"].as_str())?;
        let expected = format!("node_modules/{package}/{bin}");
        if !wrapper.replace('\\', "/").contains(&expected) {
            return None;
        }
        // Codex's package wraps a native binary. Resolve that binary directly;
        // other script packages require explicit learning, not shell evaluation.
        if root == "codex" {
            let arch = if cfg!(target_arch = "aarch64") {
                "aarch64-pc-windows-msvc"
            } else {
                "x86_64-pc-windows-msvc"
            };
            let suffix = if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x64"
            };
            let candidates = [
                base.join(format!(
                    "node_modules/@openai/codex-win32-{suffix}/vendor/{arch}/bin/codex.exe"
                )),
                path.parent()?.join(format!(
                    "node_modules/@openai/codex-win32-{suffix}/vendor/{arch}/bin/codex.exe"
                )),
                base.join(format!("vendor/{arch}/bin/codex.exe")),
            ];
            target = candidates.into_iter().find(|p| p.is_file())?;
            packaged = true;
        } else {
            let resolved_script = fs::canonicalize(base.join(bin)).ok()?;
            if !resolved_script.is_file()
                || !resolved_script.starts_with(fs::canonicalize(&base).ok()?)
            {
                return None;
            }
            script = Some(resolved_script);
            target = path.parent()?.join("node.exe");
            if !target.is_file() {
                target = std::env::split_paths(&std::env::var_os("PATH")?)
                    .map(|p| p.join("node.exe"))
                    .find(|p| p.is_file())?;
            }
            packaged = true;
        }
    }
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default();
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_default();
    let programs = std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_default();
    let roots: Vec<PathBuf> = match root.as_str() {
        "cargo" | "rustc" => vec![home.join(".cargo/bin")],
        "codex" => vec![local.join("OpenAI/Codex/bin")],
        "git" => vec![programs.join("Git")],
        "gh" => vec![programs.join("GitHub CLI")],
        "pwsh" => vec![programs.join("PowerShell")],
        "dotnet" => vec![programs.join("dotnet")],
        "python" | "python3" => vec![local.join("Programs/Python")],
        "uv" => vec![
            home.join(".local/bin"),
            local.join("Microsoft/WinGet/Packages"),
        ],
        "winget" => vec![programs.join("WindowsApps")],
        "docker" => vec![programs.join("Docker")],
        _ => Vec::new(),
    };
    let known_install = roots
        .iter()
        .filter_map(|r| fs::canonicalize(r).ok())
        .any(|r| path.starts_with(r));
    let in_project = fs::canonicalize(cwd)
        .ok()
        .is_some_and(|p| path.starts_with(p));
    let npm_install = path.parent().is_some_and(|parent| {
        parent.join("node.exe").is_file()
            || std::env::var_os("APPDATA")
                .and_then(|p| fs::canonicalize(PathBuf::from(p).join("npm")).ok())
                .is_some_and(|p| parent == p)
    });
    let fingerprint = hash(
        format!(
            "{PARSER_VERSION}:{}:{}:{}",
            stamp(&path)?,
            stamp(&target)?,
            script.as_ref().and_then(|p| stamp(p)).unwrap_or_default()
        )
        .as_bytes(),
    );
    let trusted = known
        && !in_project
        && ((packaged && npm_install) || known_install || script.is_none());
    Some(Entry {
        command: root,
        path,
        target,
        fingerprint,
        script,
        trusted,
    })
}

pub fn metadata_description(entry: &Entry) -> Option<String> {
    if entry.script.is_some() {
        let package = entry
            .path
            .parent()?
            .join("node_modules")
            .join(&entry.command)
            .join("package.json");
        let json: serde_json::Value = serde_json::from_slice(&fs::read(package).ok()?).ok()?;
        return synopsis(&entry.command, json["description"].as_str()?);
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::*;
        let path: Vec<u16> = entry
            .target
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe {
            let size = GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut());
            if size == 0 || size > 1048576 {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr() as _) == 0 {
                return None;
            }
            let query = |key: &str| -> Option<Vec<u16>> {
                let wide: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
                let mut pointer = std::ptr::null_mut();
                let mut length = 0;
                if VerQueryValueW(data.as_ptr() as _, wide.as_ptr(), &mut pointer, &mut length) == 0
                    || pointer.is_null()
                {
                    return None;
                }
                let start = pointer as usize;
                let begin = data.as_ptr() as usize;
                if start < begin || start.checked_add(length as usize * 2)? > begin + data.len() {
                    return None;
                }
                Some(std::slice::from_raw_parts(pointer as *const u16, length as usize).to_vec())
            };
            // Common Unicode and Windows code-page resource tables.
            for table in ["040904b0", "040904e4", "080404b0", "000004b0"] {
                if let Some(wide) = query(&format!("\\StringFileInfo\\{table}\\FileDescription")) {
                    let text = String::from_utf16_lossy(&wide)
                        .trim_end_matches('\0')
                        .to_owned();
                    if let Some(value) = synopsis(&entry.command, &text) {
                        return Some(value);
                    }
                }
            }
        }
    }
    None
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub parser_version: u32,
    pub entry: Entry,
    pub context: Vec<String>,
    pub page: HelpPage,
    pub fetched_at: u64,
    pub error: Option<String>,
}
pub fn cache_dir() -> PathBuf {
    crate::config::cache_dir().join("help")
}
pub fn records(dir: &Path) -> Vec<Record> {
    let mut result = Vec::new();
    if let Ok(files) = fs::read_dir(dir) {
        for file in files.flatten().take(512) {
            if file.path().extension().is_some_and(|s| s == "json")
                && file
                    .metadata()
                    .is_ok_and(|m| m.len() <= 2 * OUTPUT_LIMIT as u64)
                && let Ok(bytes) = fs::read(file.path())
                && let Ok(record) = serde_json::from_slice::<Record>(&bytes)
            {
                result.push(record);
            }
        }
    }
    result
}
pub fn current(record: &Record) -> bool {
    record.parser_version == PARSER_VERSION
        && stamp(&record.entry.path)
            .zip(stamp(&record.entry.target))
            .is_some_and(|(a, b)| {
                hash(
                    format!(
                        "{PARSER_VERSION}:{a}:{b}:{}",
                        record
                            .entry
                            .script
                            .as_ref()
                            .and_then(|p| stamp(p))
                            .unwrap_or_default()
                    )
                    .as_bytes(),
                ) == record.entry.fingerprint
            })
}
fn record_path(dir: &Path, entry: &Entry, context: &[String]) -> PathBuf {
    dir.join(format!(
        "{}.json",
        hash(format!("{}:{context:?}", entry.fingerprint).as_bytes())
    ))
}
pub fn forget(dir: &Path, command: &str) -> std::io::Result<usize> {
    let mut count = 0;
    for record in records(dir) {
        if record.entry.command.eq_ignore_ascii_case(command) {
            fs::remove_file(record_path(dir, &record.entry, &record.context))?;
            count += 1;
        }
    }
    Ok(count)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// One hidden child, with a Job Object so timeout closes all descendants too.
pub fn capture(
    target: &Path,
    args: &[String],
    cwd: &Path,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    let mut command = Command::new(target);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .env("AWS_PAGER", "")
        .env("AZURE_CORE_ONLY_SHOW_ERRORS", "true")
        .env("CLOUDSDK_CORE_DISABLE_PROMPTS", "1")
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000004);
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let _job = match Job::attach(&child) {
        Ok(job) => job,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    #[cfg(windows)]
    if let Err(error) = resume_child(child.id()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let stdout = child.stdout.take().ok_or("stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("stderr unavailable")?;
    let (tx, rx) = std::sync::mpsc::sync_channel(2);
    for mut stream in [Box::new(stdout) as Box<dyn Read + Send>, Box::new(stderr)] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stream
                .by_ref()
                .take(OUTPUT_LIMIT as u64 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = tx.send(result);
        });
    }
    drop(tx);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    let mut streams = 0;
    loop {
        if cancelled.load(Ordering::Relaxed) || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("帮助查询已取消或超过 2 秒".into());
        }
        if streams < 2 {
            match rx.try_recv() {
                Ok(Ok(output)) => {
                    if output.len() + bytes.len() > OUTPUT_LIMIT {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("帮助输出超过 256 KiB".into());
                    }
                    bytes.extend_from_slice(&output);
                    bytes.push(b'\n');
                    streams += 1;
                }
                Ok(Err(e)) => return Err(e.to_string()),
                _ => {}
            }
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() && streams == 2 {
            return String::from_utf8(bytes).map_err(|_| "帮助不是 UTF-8 文本".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[cfg(windows)]
fn resume_child(process_id: u32) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::*,
        System::{Diagnostics::ToolHelp::*, Threading::*},
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        let mut found = Thread32First(snapshot, &mut entry);
        let mut resumed = false;
        while found != 0 {
            if entry.th32OwnerProcessID == process_id {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if !thread.is_null() {
                    resumed = ResumeThread(thread) != u32::MAX;
                    CloseHandle(thread);
                }
                break;
            }
            found = Thread32Next(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        if resumed {
            Ok(())
        } else {
            Err("无法恢复帮助进程".into())
        }
    }
}
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Job {
    fn attach(child: &std::process::Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let job = Self(handle);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as _,
                std::mem::size_of_val(&info) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle() as _) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

pub fn learn(
    entry: Entry,
    context: Vec<String>,
    dir: &Path,
    cancelled: &AtomicBool,
) -> Result<Record, String> {
    if context.len() > 4
        || context.iter().any(|s| {
            s.is_empty()
                || !s
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
    {
        return Err("无效帮助上下文".into());
    }
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let neutral = dir.join("work");
    fs::create_dir_all(&neutral).map_err(|e| e.to_string())?;
    let arguments = |suffix: Vec<String>| {
        let mut args = Vec::new();
        if let Some(script) = &entry.script {
            args.push(script.to_string_lossy().into_owned());
        }
        if entry.command == "python" || entry.command == "python3" {
            args.extend(["-I".into(), "-S".into()]);
        }
        args.extend(suffix);
        args
    };
    let adapter = crate::tool_registry::definition(&entry.command)
        .map(|tool| tool.help)
        .unwrap_or("sectioned");
    let mut help_args = match adapter {
        "scoop" => std::iter::once("help".to_owned())
            .chain(context.iter().cloned())
            .collect(),
        "aws" => context
            .iter()
            .cloned()
            .chain(std::iter::once("help".to_owned()))
            .collect(),
        _ => context.clone(),
    };
    if adapter != "scoop" && adapter != "aws" {
        help_args.push(
            match adapter {
                "windows-slash" => "/?",
                "sevenzip" => "-h",
                "choco" => "-?",
                "sqlite" => "-help",
                _ if entry.command == "git" && !context.is_empty() => "-h",
                _ => "--help",
            }
            .into(),
        );
    }
    let result = capture(
        &entry.target,
        &arguments(help_args),
        &neutral,
        cancelled,
    )
    .and_then(|text| {
        let mut page = parse_help(&text);
        page.adapter = adapter.to_owned();
        if entry.command == "cargo" && context.is_empty() {
            let listing = capture(
                &entry.target,
                &arguments(vec!["--list".into()]),
                &neutral,
                cancelled,
            )?;
            let commands = parse_cargo_list(&listing);
            page.commands = commands.commands;
            page.command_aliases = commands.command_aliases;
            page.commands_complete = commands.commands_complete;
            page.adapter = "cargo-list+help".into();
            page.complete = page.options_complete && page.commands_complete;
        }
        if page.options.is_empty() && page.commands.is_empty() {
            Err("未识别到可用的命令或选项".into())
        } else {
            Ok(page)
        }
    });
    let record = Record {
        parser_version: PARSER_VERSION,
        entry,
        context,
        page: result.clone().unwrap_or_default(),
        fetched_at: now(),
        error: result.err(),
    };
    let path = record_path(dir, &record.entry, &record.context);
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
    let mut file = fs::File::create(&temporary).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    drop(file);
    crate::engine::replace_file(&temporary, &path).map_err(|e| e.to_string())?;
    Ok(record)
}

#[derive(Default)]
struct Pending {
    query: Option<(Entry, Vec<String>, Instant)>,
    stop: bool,
    reset: bool,
    cancel: Option<Arc<AtomicBool>>,
}
pub struct Learner {
    shared: Arc<(Mutex<Pending>, Condvar)>,
}
impl Learner {
    pub fn new(dir: PathBuf, notify: impl Fn(Record) + Send + 'static) -> Self {
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let worker = shared.clone();
        std::thread::spawn(move || {
            let mut attempted = BTreeMap::<String, u64>::new();
            loop {
                let (lock, wake) = &*worker;
                let mut pending = lock.lock().unwrap();
                while pending.query.is_none() && !pending.stop && !pending.reset {
                    pending = wake.wait(pending).unwrap();
                }
                if pending.stop {
                    break;
                }
                if pending.reset {
                    attempted.clear();
                    pending.reset = false;
                    if pending.query.is_none() {
                        continue;
                    }
                }
                let until = pending.query.as_ref().unwrap().2;
                if let Some(wait) = until.checked_duration_since(Instant::now()) {
                    let _ = wake.wait_timeout(pending, wait).unwrap();
                    continue;
                }
                let (entry, context, _) = pending.query.take().unwrap();
                let cancelled = Arc::new(AtomicBool::new(false));
                pending.cancel = Some(cancelled.clone());
                drop(pending);
                let key = format!("{}:{context:?}", entry.fingerprint);
                if attempted.get(&key).is_some_and(|until| *until > now()) {
                    continue;
                }
                if let Ok(bytes) = fs::read(record_path(&dir, &entry, &context))
                    && let Ok(record) = serde_json::from_slice::<Record>(&bytes)
                    && current(&record)
                    && (record.error.is_none() || now() < record.fetched_at + 300)
                {
                    attempted.insert(
                        key,
                        if record.error.is_none() {
                            u64::MAX
                        } else {
                            record.fetched_at + 300
                        },
                    );
                    notify(record);
                    continue;
                }
                if !entry.trusted {
                    attempted.insert(key, u64::MAX);
                    if context.is_empty()
                        && let Some(description) = metadata_description(&entry)
                    {
                        notify(Record {
                            parser_version: PARSER_VERSION,
                            entry,
                            context,
                            page: HelpPage {
                                description,
                                ..Default::default()
                            },
                            fetched_at: now(),
                            error: None,
                        });
                    }
                    continue;
                }
                if let Ok(record) = learn(entry, context, &dir, &cancelled) {
                    attempted.insert(
                        key,
                        if record.error.is_none() {
                            u64::MAX
                        } else {
                            now() + 300
                        },
                    );
                    notify(record);
                }
            }
        });
        Self { shared }
    }
    pub fn submit(&self, entry: Entry, context: Vec<String>) {
        let mut pending = self.shared.0.lock().unwrap();
        pending.query = Some((entry, context, Instant::now() + Duration::from_millis(300)));
        self.shared.1.notify_one();
    }
    pub fn invalidate(&self) {
        self.shared.0.lock().unwrap().reset = true;
        self.shared.1.notify_one();
    }
    pub fn cancel(&self) {
        let mut pending = self.shared.0.lock().unwrap();
        pending.query = None;
        if let Some(cancel) = pending.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.shared.1.notify_one();
    }
}
impl Drop for Learner {
    fn drop(&mut self) {
        self.cancel();
        self.shared.0.lock().unwrap().stop = true;
        self.shared.1.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tooltip_survives_and_paths_do_not() {
        assert_eq!(
            synopsis("x", "\x1b[31m真实用途。更多内容\x1b[0m"),
            Some("真实用途。".into())
        );
        assert!(synopsis("x", "C:\\bin\\x.exe").is_none());
        assert!(synopsis("x", "x").is_none());
    }
    #[test]
    fn clap_options_values_and_commands() {
        let p = parse_help(
            "Tool\nCommands:\n  exec  Run things\nOptions:\n  -s, --sandbox <MODE>\n      Select mode\n      [possible values: read-only, workspace-write]\n  --flag  Enable it\n",
        );
        assert!(p.complete);
        assert_eq!(p.commands["exec"], "Run things");
        assert_eq!(p.options[0].names, ["-s", "--sandbox"]);
        assert_eq!(p.options[0].values, ["read-only", "workspace-write"]);
        assert_eq!(p.options[0].value_name, "<MODE>");
    }
    #[test]
    fn prose_is_not_a_command() {
        let p = parse_help("A paragraph mentioning --delete and --force\nRead the docs.");
        assert!(p.options.is_empty());
        assert!(!p.complete);
    }
}

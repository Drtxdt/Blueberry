use crate::{
    config::{self},
    knowledge,
    spec_catalog::Catalog,
    terminal_ui::{ScreenGuard, fit},
};
use anyhow::{Context, Result};
use crossterm::{
    cursor::MoveTo,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind},
    execute,
};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    env, fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};
use toml_edit::{DocumentMut, value};

#[path = "tools_tasks.rs"]
mod tasks;

#[derive(Clone, Serialize)]
struct ToolInfo {
    command: String,
    installed: bool,
    paths: Vec<PathBuf>,
    contexts: usize,
    dynamic: bool,
    help: String,
    help_adapter: String,
    providers: Vec<String>,
    resources: String,
    error: Option<String>,
    #[serde(skip)]
    version: Option<String>,
}

fn locate_all(command: &str) -> Vec<PathBuf> {
    let Some(path) = env::var_os("PATH") else {
        return Vec::new();
    };
    let suffixes = if cfg!(windows) {
        [".exe", ".cmd", ".bat", ""]
    } else {
        ["", "", "", ""]
    };
    let mut seen = BTreeSet::new();
    env::split_paths(&path)
        .flat_map(|dir| {
            suffixes
                .iter()
                .map(move |suffix| dir.join(format!("{command}{suffix}")))
        })
        .filter(|path| path.is_file() && seen.insert(path.clone()))
        .collect()
}

fn collect(settings: &config::Config, config_path: Option<&Path>) -> Result<Vec<ToolInfo>> {
    let catalog = Catalog::load_user_dir(config::specs_dir(settings, config_path))?;
    let summaries = catalog.list();
    let records = knowledge::records(&knowledge::cache_dir());
    let mut names = BTreeSet::new();
    for tool in crate::tool_registry::TOOLS {
        names.insert(tool.name.to_owned());
    }
    for item in &summaries {
        if let Some(name) = item.path.split_whitespace().next() {
            names.insert(name.to_owned());
        }
    }
    for record in &records {
        names.insert(record.entry.command.clone());
    }
    let mut result = Vec::new();
    for command in names {
        let paths = locate_all(&command);
        let matching = records
            .iter()
            .filter(|r| r.entry.command.eq_ignore_ascii_case(&command))
            .collect::<Vec<_>>();
        let help = if matching
            .iter()
            .any(|r| r.error.is_none() && knowledge::current(r))
        {
            "已学习"
        } else if matching.iter().any(|r| r.error.is_some()) {
            "失败"
        } else if !matching.is_empty() {
            "已过期"
        } else {
            "未学习"
        };
        let error = matching.iter().rev().find_map(|r| r.error.clone());
        let contexts = summaries
            .iter()
            .filter(|n| n.path == command || n.path.starts_with(&format!("{command} ")))
            .count();
        let definition = crate::tool_registry::definition(&command);
        let providers: Vec<String> = definition
            .map(|tool| {
                tool.providers
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect()
            })
            .unwrap_or_default();
        let dynamic = !providers.is_empty()
            || summaries.iter().any(|n| {
                (n.path == command || n.path.starts_with(&format!("{command} ")))
                    && n.provider.is_some()
            });
        result.push(ToolInfo {
            command,
            installed: !paths.is_empty(),
            paths,
            contexts,
            dynamic,
            help: help.into(),
            help_adapter: definition
                .map(|tool| tool.help)
                .unwrap_or("用户规格")
                .into(),
            providers,
            resources: definition
                .map(|tool| tool.resources)
                .unwrap_or("none")
                .into(),
            error,
            version: None,
        });
    }
    Ok(result)
}

pub fn run(config_path: Option<&Path>, json: bool) -> Result<u32> {
    let settings = config::load(config_path)?;
    let mut tools = collect(&settings, config_path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&tools)?);
        return Ok(0);
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        print_text(&tools);
        return Ok(0);
    }
    let _screen = ScreenGuard::enter(false)?;
    let mut query = String::new();
    let mut selected = 0usize;
    let mut notice = String::new();
    let mut tasks = tasks::Worker::new();
    let mut dirty = true;
    loop {
        if let Some(completed) = tasks.poll() {
            dirty = true;
            let command = completed.tool.command;
            match completed.output {
                Ok(tasks::Output::Version(version)) => {
                    if let Some(tool) = tools
                        .iter_mut()
                        .find(|tool| tool.command == command && tool.paths == completed.tool.paths)
                    {
                        tool.version = Some(version.clone());
                    }
                    notice = format!("{command}：{version}");
                }
                Ok(tasks::Output::Learned(record)) => {
                    let published = knowledge::publish(&record, &knowledge::cache_dir());
                    let saved = published.is_ok();
                    notice = match published {
                        Ok(()) => record
                            .error
                            .as_ref()
                            .map(|e| format!("{command}：学习失败：{e}"))
                            .unwrap_or_else(|| format!("{command}：帮助学习完成")),
                        Err(error) => format!("{command}：缓存保存失败：{error}"),
                    };
                    if saved
                        && let Some(tool) = tools.iter_mut().find(|tool| {
                            tool.command == command && tool.paths == completed.tool.paths
                        })
                    {
                        tool.help = if record.error.is_none() {
                            "已学习"
                        } else {
                            "失败"
                        }
                        .into();
                        tool.error.clone_from(&record.error);
                    }
                }
                Err(error) => notice = format!("{command}：{error}"),
            }
        }
        let visible = filtered(&tools, &query);
        selected = selected.min(visible.len().saturating_sub(1));
        if dirty {
            draw(&tools, &visible, selected, &query, &notice, &settings)?;
            dirty = false;
        }
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        dirty = true;
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc => break,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Char('l' | 'L') if key.modifiers.is_empty() => {
                    if let Some(index) = visible.get(selected).copied()
                        && tasks.submit(&tools[index], tasks::Kind::Learn)
                    {
                        notice = format!("{}：正在学习…", tools[index].command);
                    }
                }
                KeyCode::Delete => {
                    tasks.cancel();
                    if let Some(index) = visible.get(selected).copied() {
                        let count =
                            knowledge::forget(&knowledge::cache_dir(), &tools[index].command)?;
                        notice = format!("已清除 {count} 条帮助缓存");
                        tools[index].help = "未学习".into();
                        tools[index].error = None;
                    }
                }
                KeyCode::Char('v' | 'V') if key.modifiers.is_empty() => {
                    if let Some(index) = visible.get(selected).copied()
                        && tasks.submit(&tools[index], tasks::Kind::Version)
                    {
                        notice = format!("{}：正在读取版本…", tools[index].command);
                    }
                }
                KeyCode::Char('h' | 'H') if key.modifiers.is_empty() => {
                    if let Some(index) = visible.get(selected).copied() {
                        let current = settings
                            .tools
                            .get(&tools[index].command)
                            .and_then(|t| t.help)
                            .unwrap_or(settings.help.enabled);
                        set_override(config_path, &tools[index].command, "help", !current)?;
                        notice = format!(
                            "{}：帮助学习 {}",
                            tools[index].command,
                            if current { "关闭" } else { "开启" }
                        )
                    }
                }
                KeyCode::Char('d' | 'D') if key.modifiers.is_empty() => {
                    if let Some(index) = visible.get(selected).copied() {
                        let current = settings
                            .tools
                            .get(&tools[index].command)
                            .and_then(|t| t.dynamic)
                            .unwrap_or(settings.completion.dynamic);
                        set_override(config_path, &tools[index].command, "dynamic", !current)?;
                        notice = format!(
                            "{}：动态候选 {}",
                            tools[index].command,
                            if current { "关闭" } else { "开启" }
                        )
                    }
                }
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    query.push(ch);
                    selected = 0
                }
                KeyCode::Backspace => {
                    query.pop();
                    selected = 0
                }
                KeyCode::Up => {
                    selected = selected
                        .checked_sub(1)
                        .unwrap_or(visible.len().saturating_sub(1))
                }
                KeyCode::Down => {
                    selected = if visible.is_empty() {
                        0
                    } else {
                        (selected + 1) % visible.len()
                    }
                }
                KeyCode::PageUp => selected = selected.saturating_sub(10),
                KeyCode::PageDown => {
                    selected = (selected + 10).min(visible.len().saturating_sub(1))
                }
                _ => {}
            },
            Event::Paste(text) => {
                query.push_str(&text.replace(['\r', '\n'], " "));
                selected = 0
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => selected = selected.saturating_sub(3),
                MouseEventKind::ScrollDown => {
                    selected = (selected + 3).min(visible.len().saturating_sub(1))
                }
                _ => continue,
            },
            _ => {}
        }
    }
    Ok(0)
}

fn filtered(tools: &[ToolInfo], query: &str) -> Vec<usize> {
    let words = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    tools
        .iter()
        .enumerate()
        .filter(|(_, tool)| {
            let text = format!(
                "{} {} {} {}",
                tool.command,
                tool.help,
                tool.contexts,
                tool.paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .to_lowercase();
            words.iter().all(|word| text.contains(word))
        })
        .map(|(index, _)| index)
        .collect()
}
fn print_text(tools: &[ToolInfo]) {
    println!("Blueberry 工具管理\n");
    for tool in tools {
        println!(
            "{:<16} {:<6} 规则 {:>3} · 帮助 {}",
            tool.command,
            if tool.installed {
                "已安装"
            } else {
                "未发现"
            },
            tool.contexts,
            tool.help
        )
    }
}
fn draw(
    tools: &[ToolInfo],
    visible: &[usize],
    selected: usize,
    query: &str,
    notice: &str,
    settings: &config::Config,
) -> Result<()> {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((100, 30));
    let mut out = io::stdout();
    write!(out, "\x1b[?2026h")?;
    execute!(out, MoveTo(0, 0))?;
    writeln!(out, "Blueberry 工具管理")?;
    writeln!(
        out,
        "搜索：{}",
        if query.is_empty() {
            "输入工具、路径或状态"
        } else {
            query
        }
    )?;
    writeln!(out, "{}", "─".repeat(cols as usize))?;
    let width = if cols >= 100 {
        cols as usize / 2
    } else {
        cols as usize
    };
    let count = rows.saturating_sub(7) as usize;
    let start = selected.saturating_sub(count.saturating_sub(1));
    for (row, index) in visible.iter().skip(start).take(count).enumerate() {
        let tool = &tools[*index];
        let line = format!(
            "{} {:<15} {:<6} 规则 {:>3} · {}",
            if start + row == selected { "▶" } else { " " },
            tool.command,
            if tool.installed {
                "已安装"
            } else {
                "未发现"
            },
            tool.contexts,
            tool.help
        );
        writeln!(out, "{}", fit(&line, width))?
    }
    if let Some(index) = visible.get(selected) {
        let tool = &tools[*index];
        if cols >= 100 {
            execute!(out, MoveTo(cols / 2, 3))?;
            write!(out, "详情 · {}", tool.command)?;
            execute!(out, MoveTo(cols / 2, 4))?;
            write!(
                out,
                "版本：{}",
                tool.version.as_deref().unwrap_or("按 V 读取")
            )?;
            execute!(out, MoveTo(cols / 2, 5))?;
            write!(
                out,
                "动态能力：{} · 资源：{}",
                if tool.dynamic { "有" } else { "无" },
                tool.resources
            )?;
            execute!(out, MoveTo(cols / 2, 6))?;
            write!(
                out,
                "入口：{}",
                fit(
                    &tool
                        .paths
                        .first()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "未发现".into()),
                    cols as usize / 2
                )
            )?;
            let override_ = settings
                .tools
                .get(&tool.command)
                .cloned()
                .unwrap_or_default();
            execute!(out, MoveTo(cols / 2, 7))?;
            write!(
                out,
                "帮助：{} · 动态：{} · 适配器：{}",
                flag(override_.help),
                flag(override_.dynamic),
                tool.help_adapter
            )?;
            execute!(out, MoveTo(cols / 2, 8))?;
            write!(
                out,
                "数据源：{}",
                fit(&tool.providers.join(", "), cols as usize / 2)
            )?;
            if let Some(error) = &tool.error {
                execute!(out, MoveTo(cols / 2, 9))?;
                write!(out, "最近错误：{}", fit(error, cols as usize / 2))?
            }
        }
    }
    execute!(out, MoveTo(0, rows.saturating_sub(2)))?;
    write!(out, "{}", fit(notice, cols as usize))?;
    execute!(out, MoveTo(0, rows.saturating_sub(1)))?;
    write!(
        out,
        "↑↓ 选择 · L 学习 · Del 清缓存 · V 版本 · H/D 切换 · Esc 返回"
    )?;
    write!(out, "\x1b[?2026l")?;
    out.flush()?;
    Ok(())
}
fn flag(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "开启",
        Some(false) => "关闭",
        None => "跟随全局",
    }
}
fn learn(tool: &ToolInfo, cancelled: &AtomicBool) -> Result<knowledge::Record, String> {
    let path = tool.paths.first().ok_or("找不到可学习的程序入口")?;
    let cwd = env::current_dir().map_err(|e| e.to_string())?;
    let entry = knowledge::entry(&tool.command, path, &cwd).ok_or("此入口不符合已知学习规则")?;
    knowledge::learn_uncached(entry, Vec::new(), &knowledge::cache_dir(), cancelled)
}
fn query_version(tool: &ToolInfo, cancelled: &AtomicBool) -> Result<String, String> {
    let path = tool.paths.first().ok_or("找不到程序入口")?;
    let cwd = env::current_dir().map_err(|e| e.to_string())?;
    let output = crate::bounded_process::capture(path, &["--version".into()], &cwd, cancelled)?;
    let first_line = |bytes: &[u8]| {
        knowledge::clean(&String::from_utf8_lossy(bytes))
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_owned())
    };
    first_line(&output.stdout)
        .or_else(|| first_line(&output.stderr))
        .ok_or("无法读取版本：输出为空".into())
}

fn set_override(config_path: Option<&Path>, command: &str, key: &str, enabled: bool) -> Result<()> {
    let path = config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(config::default_path);
    let text = fs::read_to_string(&path).unwrap_or_else(|_| config::example().to_owned());
    let mut doc = text.parse::<DocumentMut>().context("配置 TOML 无法解析")?;
    doc["tools"][command][key] = value(enabled);
    let temp = path.with_extension("toml.tmp");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?
    }
    fs::write(&temp, doc.to_string())?;
    if path.exists() {
        let backup = path.with_extension("toml.bak");
        let _ = fs::remove_file(&backup);
        fs::rename(&path, &backup)?;
        if let Err(error) = fs::rename(&temp, &path) {
            let _ = fs::rename(&backup, &path);
            return Err(error.into());
        }
        let _ = fs::remove_file(backup);
    } else {
        fs::rename(temp, path)?;
    }
    Ok(())
}

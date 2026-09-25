use crate::{
    config,
    input::{self, Input},
    terminal_ui::{ScreenGuard, fit},
};
use anyhow::{Context, Result};
use crossterm::{
    cursor::MoveTo,
    event::{self, Event, KeyCode, KeyEventKind, MouseEventKind},
    execute,
    terminal::{Clear, ClearType},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::Path,
};
use toml_edit::{DocumentMut, value};

const SETUP_VERSION: u32 = 2;
#[derive(Default, Deserialize, Serialize)]
struct State {
    #[serde(default)]
    setup_hint_seen: bool,
    #[serde(default)]
    setup_completed: bool,
    #[serde(default)]
    setup_version: u32,
}
fn load() -> State {
    fs::read_to_string(config::state_path())
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}
fn save(state: &State) -> Result<()> {
    let path = config::state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?
    }
    fs::write(&path, toml::to_string_pretty(state)?)
        .with_context(|| format!("无法写入 {}", path.display()))
}
pub fn take_first_hint() -> bool {
    let mut state = load();
    if state.setup_hint_seen && state.setup_version >= SETUP_VERSION {
        return false;
    }
    state.setup_hint_seen = true;
    state.setup_version = SETUP_VERSION;
    save(&state).is_ok()
}

pub fn run(config_path: Option<&Path>) -> Result<u32> {
    let path = config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(config::default_path);
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        println!("Blueberry 初次使用向导");
        println!("Ctrl+Space 补全 · Ctrl+Alt+F 用途搜索 · F1 详情 · Ctrl+Alt+P 工作台");
        println!(
            "交互终端中运行 blueberry setup 以修改基础设置。\n配置路径：{}",
            path.display()
        );
        return Ok(0);
    }
    let mut choice = Choices::default();
    let keys = config::load(Some(&path))
        .map(|value| value.keys)
        .unwrap_or_default();
    let screen = ScreenGuard::enter(false)?;
    let mut step = 0usize;
    loop {
        draw(step, &choice, &path, &keys)?;
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if step == 0 {
                    match input::configured(&Event::Key(key), &keys) {
                        Some(Input::Trigger) => choice.shortcut_seen[0] = true,
                        Some(Input::Search) => choice.shortcut_seen[1] = true,
                        Some(Input::Hub) => choice.shortcut_seen[2] = true,
                        _ => {}
                    }
                }
                match key.code {
                    KeyCode::Esc => return Ok(0),
                    KeyCode::Up | KeyCode::Left => match step {
                        1 => choice.nerd = false,
                        2 => choice.auto = true,
                        3 => choice.startup = false,
                        _ => {}
                    },
                    KeyCode::Down | KeyCode::Right => match step {
                        1 => choice.nerd = true,
                        2 => choice.auto = false,
                        3 => choice.startup = true,
                        _ => {}
                    },
                    KeyCode::Char(' ') => match step {
                        1 => choice.nerd = !choice.nerd,
                        2 => choice.auto = !choice.auto,
                        3 => choice.startup = !choice.startup,
                        _ => {}
                    },
                    KeyCode::Backspace if step > 0 => step -= 1,
                    KeyCode::Enter if step < 4 => step += 1,
                    KeyCode::Enter => break,
                    _ => {}
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => step = step.saturating_sub(1),
                MouseEventKind::ScrollDown => step = (step + 1).min(4),
                _ => continue,
            },
            _ => {}
        }
    }
    drop(screen);
    write_config(&path, &choice)?;
    if choice.startup {
        crate::startup::run("enable", None, false).context("配置自动启动失败")?;
    }
    let mut state = load();
    state.setup_hint_seen = true;
    state.setup_completed = true;
    state.setup_version = SETUP_VERSION;
    save(&state)?;
    println!("已保存设置：{}", path.display());
    Ok(0)
}

#[derive(Default)]
struct Choices {
    nerd: bool,
    auto: bool,
    startup: bool,
    shortcut_seen: [bool; 3],
}
impl Choices {
    fn default() -> Self {
        Self {
            nerd: false,
            auto: true,
            startup: false,
            shortcut_seen: [false; 3],
        }
    }
}
fn draw(step: usize, choice: &Choices, path: &Path, keys: &config::KeyBindings) -> Result<()> {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((100, 30));
    let mut out = io::stdout();
    write!(out, "\x1b[?2026h")?;
    execute!(out, MoveTo(0, 0), Clear(ClearType::All))?;
    writeln!(out, "Blueberry 设置向导  {}/5", step + 1)?;
    writeln!(out, "{}", "─".repeat(cols as usize))?;
    match step {
        0 => {
            writeln!(out, "欢迎使用 Blueberry")?;
            writeln!(out, "\n请在此页实际按下以下快捷键：")?;
            writeln!(
                out,
                "{}  {}  打开补全",
                if choice.shortcut_seen[0] {
                    "✓"
                } else {
                    "○"
                },
                keys.trigger
            )?;
            writeln!(
                out,
                "{}  {}  按用途搜索",
                if choice.shortcut_seen[1] {
                    "✓"
                } else {
                    "○"
                },
                keys.search
            )?;
            writeln!(
                out,
                "{}  {}  打开命令工作台",
                if choice.shortcut_seen[2] {
                    "✓"
                } else {
                    "○"
                },
                keys.hub
            )?;
            writeln!(out, "未点亮的组合键可能被终端占用；可在设置中改键。")?
        }
        1 => {
            writeln!(out, "图标风格")?;
            writeln!(
                out,
                "\n{}  Unicode  ✓  📁  ⚙",
                if !choice.nerd { "▶" } else { " " }
            )?;
            writeln!(
                out,
                "{}  Nerd Font    󰉋  ",
                if choice.nerd { "▶" } else { " " }
            )?;
            writeln!(out, "\n终端未配置 Nerd Font 时请选择 Unicode。")?
        }
        2 => {
            writeln!(out, "补全显示方式")?;
            writeln!(out, "\n{} 自动显示", if choice.auto { "▶" } else { " " })?;
            writeln!(
                out,
                "{} 按 Ctrl+Space 显示",
                if !choice.auto { "▶" } else { " " }
            )?
        }
        3 => {
            writeln!(out, "随 PowerShell 自动启动")?;
            writeln!(
                out,
                "\n{} 保持关闭",
                if !choice.startup { "▶" } else { " " }
            )?;
            writeln!(
                out,
                "{} 为当前用户启用",
                if choice.startup { "▶" } else { " " }
            )?
        }
        _ => {
            writeln!(out, "保存前预览")?;
            writeln!(
                out,
                "\n图标：{}",
                if choice.nerd { "Nerd Font" } else { "Unicode" }
            )?;
            writeln!(
                out,
                "补全：{}",
                if choice.auto {
                    "自动显示"
                } else {
                    "手动触发"
                }
            )?;
            writeln!(
                out,
                "自动启动：{}",
                if choice.startup { "启用" } else { "关闭" }
            )?;
            writeln!(
                out,
                "配置：{}",
                fit(
                    &path.display().to_string(),
                    (cols as usize).saturating_sub(6)
                )
            )?;
            writeln!(
                out,
                "快捷键实按：{}/3",
                choice.shortcut_seen.iter().filter(|seen| **seen).count()
            )?
        }
    }
    execute!(out, MoveTo(0, rows.saturating_sub(1)))?;
    write!(
        out,
        "{}",
        if matches!(step, 1..=3) {
            "↑↓/←→ 选择 · Space 切换 · Enter 下一步 · Backspace 上一步 · Esc 退出"
        } else {
            "Enter 下一步/保存 · Backspace 上一步 · Esc 退出"
        }
    )?;
    write!(out, "\x1b[?2026l")?;
    out.flush()?;
    Ok(())
}
fn write_config(path: &Path, choice: &Choices) -> Result<()> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => config::example().to_owned(),
        Err(error) => return Err(error).with_context(|| format!("无法读取 {}", path.display())),
    };
    let mut doc = text
        .parse::<DocumentMut>()
        .context("配置 TOML 无法解析；请先运行 blueberry config check")?;
    doc["ui"]["icon_style"] = value(if choice.nerd { "nerd" } else { "unicode" });
    doc["completion"]["auto_trigger"] = value(choice.auto);
    let temp = path.with_extension("toml.tmp");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?
    }
    fs::write(&temp, doc.to_string())?;
    if path.exists() {
        let backup = path.with_extension("toml.bak");
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup)?;
        if let Err(error) = fs::rename(&temp, path) {
            let _ = fs::rename(&backup, path);
            return Err(error.into());
        }
        let _ = fs::remove_file(backup);
    } else {
        fs::rename(temp, path)?;
    }
    Ok(())
}

use crate::config;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, io::{self, IsTerminal, Write}, path::Path};

#[derive(Default, Deserialize, Serialize)]
struct State { setup_hint_seen: bool, setup_completed: bool }

fn load() -> State { fs::read_to_string(config::state_path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default() }
fn save(state: &State) -> Result<()> {
    let path=config::state_path();
    if let Some(parent)=path.parent(){fs::create_dir_all(parent)?;}
    fs::write(&path,toml::to_string_pretty(state)?).with_context(||format!("无法写入 {}",path.display()))
}

pub fn take_first_hint() -> bool {
    let mut state=load();
    if state.setup_hint_seen { return false; }
    state.setup_hint_seen=true;
    save(&state).is_ok()
}

pub fn run(config_path: Option<&Path>) -> Result<u32> {
    println!("Blueberry 初次使用向导");
    println!("  Ctrl+Space 补全 · Ctrl+Alt+F 用途搜索 · F1 详情 · blueberry config edit 设置");
    let path=config_path.map(Path::to_path_buf).unwrap_or_else(config::default_path);
    if !io::stdin().is_terminal() { println!("当前不是交互终端；配置路径：{}",path.display()); return Ok(0); }
    print!("使用 Nerd Font 图标？终端未配置字体时请选择 N [y/N] "); io::stdout().flush()?;
    let mut answer=String::new(); io::stdin().read_line(&mut answer)?;
    let nerd=answer.trim().eq_ignore_ascii_case("y")||answer.trim().eq_ignore_ascii_case("yes");
    let mut document=if path.is_file(){fs::read_to_string(&path)?}else{config::example().to_owned()};
    document=document.replace("icon_style = \"nerd\"", if nerd{"icon_style = \"nerd\""}else{"icon_style = \"unicode\""});
    if let Some(parent)=path.parent(){fs::create_dir_all(parent)?;}
    fs::write(&path,document)?;
    answer.clear(); print!("随 PowerShell 自动启动？默认关闭 [y/N] "); io::stdout().flush()?; io::stdin().read_line(&mut answer)?;
    let startup=answer.trim().eq_ignore_ascii_case("y")||answer.trim().eq_ignore_ascii_case("yes");
    if startup { crate::startup::run("enable",None,false).context("配置自动启动失败")?; }
    let mut state=load(); state.setup_hint_seen=true; state.setup_completed=true; save(&state)?;
    println!("已保存设置：{}",path.display());
    if !startup { println!("自动启动保持关闭；需要时运行 blueberry startup enable。"); }
    Ok(0)
}

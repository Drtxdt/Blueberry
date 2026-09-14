use crate::{config, model::{Candidate, CandidateKind}};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, env, fs, io::{self, IsTerminal, Write}, path::Path};

#[derive(Clone,Default,Deserialize,Serialize)]
struct SavedCommand { name:String, command:String, #[serde(default)] description:String, #[serde(default)] tags:Vec<String> }
#[derive(Default,Deserialize,Serialize)]
struct Commands { #[serde(default)] favorites:Vec<SavedCommand>, #[serde(default)] templates:Vec<SavedCommand> }

fn save_commands(path:&Path,commands:&Commands)->Result<()> { if let Some(parent)=path.parent(){fs::create_dir_all(parent)?;}fs::write(path,toml::to_string_pretty(commands)?).with_context(||format!("无法写入 {}",path.display())) }

pub fn add_favorite(name:&str,command:&str)->Result<()> {
    let path=config::commands_path();let mut saved:Commands=fs::read_to_string(&path).ok().and_then(|s|toml::from_str(&s).ok()).unwrap_or_default();
    if let Some(item)=saved.favorites.iter_mut().find(|item|item.name==name){item.command=command.into();}else{saved.favorites.push(SavedCommand{name:name.into(),command:command.into(),description:"用户收藏".into(),tags:Vec::new()});}
    save_commands(&path,&saved)?;println!("已保存收藏：{name}");Ok(())
}
pub fn remove_favorite(name:&str)->Result<()> {
    let path=config::commands_path();let mut saved:Commands=fs::read_to_string(&path).ok().and_then(|s|toml::from_str(&s).ok()).unwrap_or_default();let before=saved.favorites.len();saved.favorites.retain(|item|item.name!=name);save_commands(&path,&saved)?;println!("已删除 {} 个同名收藏。",before-saved.favorites.len());Ok(())
}

fn builtins()->Vec<SavedCommand>{[
    ("创建 Git 分支","git switch -c <BRANCH>"),("运行项目测试","cargo test"),("进入 Conda 环境","conda activate <ENV>"),
    ("查看 Compose 日志","docker compose logs -f <SERVICE>"),("查看 Kubernetes 日志","kubectl logs <RESOURCE>")
].into_iter().map(|(n,c)|SavedCommand{name:n.into(),command:c.into(),description:"内置命令模板".into(),tags:Vec::new()}).collect()}

fn history(limit:usize)->Vec<SavedCommand>{
    let root=env::var_os("APPDATA").map(std::path::PathBuf::from); let Some(root)=root else{return Vec::new()};
    let dir=root.join("Microsoft/Windows/PowerShell/PSReadLine"); let Ok(entries)=fs::read_dir(dir) else{return Vec::new()};
    let mut lines=Vec::new(); for entry in entries.flatten(){if entry.path().extension().and_then(|x|x.to_str())==Some("txt"){if let Ok(text)=fs::read_to_string(entry.path()){let mut current=String::new();for line in text.lines(){current.push_str(line);if current.ends_with('`'){current.pop();current.push('\n');}else if !current.is_empty(){lines.push(std::mem::take(&mut current));}}if !current.is_empty(){lines.push(current);}}}}
    let mut seen=HashSet::new(); lines.into_iter().rev().filter(|line|!line.trim().is_empty()&&seen.insert(line.clone())).take(limit).map(|command|SavedCommand{name:command.lines().next().unwrap_or("").to_owned(),command,description:"PSReadLine 历史".into(),tags:Vec::new()}).collect()
}

pub fn run(config_path:Option<&Path>, query:Option<&str>)->Result<u32>{
    let settings=config::load(config_path)?; let path=config::commands_path();
    let saved:Commands=fs::read_to_string(&path).ok().and_then(|s|toml::from_str(&s).ok()).unwrap_or_default();
    let mut items=saved.favorites; items.extend(saved.templates); items.extend(builtins()); items.extend(history(settings.workbench.history_limit));
    let query=query.unwrap_or("").trim().to_lowercase(); if !query.is_empty(){items.retain(|i|format!("{} {} {}",i.name,i.description,i.tags.join(" ")).to_lowercase().contains(&query));}
    if !io::stdin().is_terminal(){for item in items.iter().take(100){println!("{}\t{}",item.command,item.name);}return Ok(0)}
    println!("Blueberry 命令工作台（输入序号，将命令填到输出；直接回车退出）\n"); for (i,item) in items.iter().take(30).enumerate(){println!("{:>2}. {:<24} {}",i+1,item.name,item.command)}
    print!("选择：");io::stdout().flush()?;let mut answer=String::new();io::stdin().read_line(&mut answer)?;
    if let Ok(index)=answer.trim().parse::<usize>() && let Some(item)=items.get(index.saturating_sub(1)){println!("{}",item.command);}
    if !path.exists(){if let Some(parent)=path.parent(){fs::create_dir_all(parent)?;}fs::write(&path,"# Blueberry 收藏与自定义模板\n\nfavorites = []\ntemplates = []\n").with_context(||format!("无法创建 {}",path.display()))?;}
    Ok(0)
}

pub fn candidates(settings:&config::Config, query:&str)->Vec<Candidate>{
    let path=config::commands_path();
    let saved:Commands=fs::read_to_string(path).ok().and_then(|s|toml::from_str(&s).ok()).unwrap_or_default();
    let mut items=saved.favorites;items.extend(saved.templates);items.extend(builtins());items.extend(history(settings.workbench.history_limit));
    let words=query.to_lowercase().split_whitespace().map(str::to_owned).collect::<Vec<_>>();
    items.into_iter().filter(|item|{let text=format!("{} {} {} {}",item.name,item.description,item.command,item.tags.join(" ")).to_lowercase();words.iter().all(|word|text.contains(word))}).take(settings.completion.max_results).map(|item|Candidate{label:item.name,insert_text:item.command,description:item.description,kind:CandidateKind::Value,source:"Blueberry 工作台".into(),append_space:false,..Default::default()}).collect()
}

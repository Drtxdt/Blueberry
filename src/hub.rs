use crate::{config, model::{Candidate, CandidateKind}};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap,HashSet}, env, fs, io::{self, IsTerminal, Write}, path::{Path, PathBuf}};

#[derive(Clone,Default,Deserialize,Serialize)]
struct SavedCommand { name:String, command:String, #[serde(default)] description:String, #[serde(default)] tags:Vec<String> }
#[derive(Default,Deserialize,Serialize)]
struct Commands { #[serde(default)] schema_version:u32, #[serde(default)] favorites:Vec<SavedCommand>, #[serde(default)] templates:Vec<CommandTemplate> }

#[derive(Clone,Default,Deserialize,Serialize)]
pub struct CommandTemplate { #[serde(default)] pub id:String, pub name:String, #[serde(default)] pub command:String, #[serde(default)] pub tokens:Vec<String>, #[serde(default)] pub description:String, #[serde(default)] pub tags:Vec<String>, #[serde(default)] pub parameters:Vec<TemplateParameter> }
#[derive(Clone,Default,Deserialize,Serialize)]
pub struct TemplateParameter { pub name:String, #[serde(default)] pub label:String, #[serde(default="text_kind")] pub kind:String, #[serde(default)] pub required:bool, #[serde(default)] pub default:String, #[serde(default)] pub values:Vec<String>, #[serde(default)] pub source:String }
fn text_kind()->String{"text".into()}

impl CommandTemplate {
    fn preview(&self)->String{if self.tokens.is_empty(){self.command.clone()}else{self.tokens.join(" ")}}
    pub fn render(&self,values:&BTreeMap<String,String>)->Result<String>{
        if self.tokens.is_empty(){return Ok(self.command.clone())}
        let mut output=Vec::new();
        for token in &self.tokens {if let Some(name)=token.strip_prefix('{').and_then(|s|s.strip_suffix('}')) {let Some(parameter)=self.parameters.iter().find(|p|p.name==name) else{anyhow::bail!("模板字段 {name} 没有参数定义")};let value=values.get(name).filter(|v|!v.is_empty()).unwrap_or(&parameter.default);if parameter.required&&value.is_empty(){anyhow::bail!("参数 {} 不能为空",if parameter.label.is_empty(){name}else{&parameter.label})}if parameter.kind=="enum"&&!value.is_empty()&&!parameter.values.iter().any(|v|v==value){anyhow::bail!("参数 {name} 不在枚举值中")}output.push(quote_powershell_token(value));}else if token.contains('{')||token.contains('}'){anyhow::bail!("模板占位符必须占据完整 token")}else{output.push(token.clone())}}
        Ok(output.join(" "))
    }
}
pub fn quote_powershell_token(value:&str)->String{if !value.is_empty()&&value.chars().all(|c|c.is_ascii_alphanumeric()||"_./:@+,-\\".contains(c)){value.into()}else{format!("'{}'",value.replace('\'',"''"))}}
pub fn placeholder_names(command:&str)->Vec<String>{
    let mut names=Vec::new();let chars=command.char_indices().collect::<Vec<_>>();let mut index=0;
    while index<chars.len(){let (start,ch)=chars[index];let close=match ch{'{'=>'}','<'=>'>',_=>{index+=1;continue}};if let Some((offset,(end,_)))=chars[index+1..].iter().enumerate().find(|(_,(_,c))|*c==close){let name=&command[start+ch.len_utf8()..*end];if !name.is_empty()&&name.chars().all(|c|c.is_ascii_alphanumeric()||c=='_'||c=='-')&&!names.iter().any(|n|n==name){names.push(name.to_owned())}index+=offset+2}else{index+=1}}
    names
}
pub fn fill_placeholders(command:&str,values:&BTreeMap<String,String>)->String{let mut result=command.to_owned();for (name,value) in values{let value=quote_powershell_token(value);result=result.replace(&format!("{{{name}}}"),&value).replace(&format!("<{name}>"),&value);}result}
fn templates_as_commands(values:Vec<CommandTemplate>)->Vec<SavedCommand>{values.into_iter().map(|item|{let command=item.preview();SavedCommand{name:item.name,command,description:if item.description.is_empty(){"用户模板".into()}else{item.description},tags:item.tags}}).collect()}

fn save_commands(path:&Path,commands:&Commands)->Result<()> { if let Some(parent)=path.parent(){fs::create_dir_all(parent)?;}fs::write(path,toml::to_string_pretty(commands)?).with_context(||format!("无法写入 {}",path.display())) }

pub fn add_favorite(name:&str,command:&str)->Result<()> {
    let path=config::commands_path();let mut saved:Commands=fs::read_to_string(&path).ok().and_then(|s|toml::from_str(&s).ok()).unwrap_or_default();
    saved.schema_version=2;
    if let Some(item)=saved.favorites.iter_mut().find(|item|item.name==name){item.command=command.into();}else{saved.favorites.push(SavedCommand{name:name.into(),command:command.into(),description:"用户收藏".into(),tags:Vec::new()});}
    save_commands(&path,&saved)?;println!("已保存收藏：{name}");Ok(())
}
pub fn remove_favorite(name:&str)->Result<()> {
    let path=config::commands_path();let mut saved:Commands=fs::read_to_string(&path).ok().and_then(|s|toml::from_str(&s).ok()).unwrap_or_default();let before=saved.favorites.len();saved.favorites.retain(|item|item.name!=name);save_commands(&path,&saved)?;println!("已删除 {} 个同名收藏。",before-saved.favorites.len());Ok(())
}

fn builtins()->Vec<SavedCommand>{[
    ("创建 Git 分支","git switch -c <BRANCH>"),("切换 Git 分支","git switch <BRANCH>"),("运行 Cargo 测试","cargo test"),
    ("运行 dotnet 测试","dotnet test <PROJECT>"),("运行 Go 测试","go test ./..."),("运行 Python 测试","uv run pytest"),
    ("运行 npm 脚本","npm run <SCRIPT>"),("运行 pnpm 脚本","pnpm run <SCRIPT>"),("运行 Yarn 脚本","yarn run <SCRIPT>"),("运行 Bun 脚本","bun run <SCRIPT>"),
    ("进入 Conda 环境","conda activate <ENV>"),("在 Conda 环境运行","conda run -n <ENV> <COMMAND>"),
    ("启动 Compose 服务","docker compose up -d <SERVICE>"),("查看 Compose 日志","docker compose logs -f <SERVICE>"),("进入 Compose 服务","docker compose exec <SERVICE> <COMMAND>"),
    ("查看 Kubernetes 日志","kubectl logs <POD>"),("进入 Kubernetes 容器","kubectl exec -it <POD> -- <COMMAND>"),("连接 SSH 主机","ssh <HOST>")
].into_iter().map(|(n,c)|SavedCommand{name:n.into(),command:c.into(),description:"内置命令模板".into(),tags:Vec::new()}).collect()}

fn actions()->Vec<SavedCommand>{[
    ("打开设置","blueberry config edit","调整外观、补全和快捷键"),
    ("管理工具","blueberry tools","查看工具入口、规则和学习状态"),
    ("运行诊断","blueberry doctor","检查配置、适配器和安装状态"),
    ("重新运行引导","blueberry setup","重新选择常用初始设置")
].into_iter().map(|(n,c,d)|SavedCommand{name:n.into(),command:c.into(),description:d.into(),tags:vec!["Blueberry 操作".into()]}).collect()}

pub fn read_history_file(path:&Path)->Vec<String>{
    let Ok(text)=fs::read_to_string(path) else{return Vec::new()};
    let mut lines=Vec::new();let mut current=String::new();
    for line in text.lines(){current.push_str(line);if current.ends_with('`'){current.pop();current.push('\n');}else if !current.trim().is_empty(){lines.push(std::mem::take(&mut current));}}
    if !current.trim().is_empty(){lines.push(current)}
    lines
}

fn history_files()->Vec<PathBuf>{
    let Some(root)=env::var_os("APPDATA").map(PathBuf::from) else{return Vec::new()};
    let Ok(entries)=fs::read_dir(root.join("Microsoft/Windows/PowerShell/PSReadLine")) else{return Vec::new()};
    entries.flatten().map(|e|e.path()).filter(|p|p.extension().and_then(|x|x.to_str())==Some("txt")).collect()
}

pub fn merged_history(limit:usize,session:&[String],exact_path:Option<&Path>)->Vec<String>{
    let files=exact_path.map(|p|vec![p.to_path_buf()]).unwrap_or_else(history_files);
    let mut lines=files.iter().flat_map(|p|read_history_file(p)).collect::<Vec<_>>();lines.extend(session.iter().cloned());
    let mut seen=HashSet::new();lines.into_iter().rev().filter(|line|!line.trim().is_empty()&&seen.insert(line.clone())).take(limit).collect()
}

fn history(limit:usize)->Vec<SavedCommand>{
    merged_history(limit,&[],None).into_iter().map(|command|SavedCommand{name:command.lines().next().unwrap_or("").to_owned(),command,description:"PSReadLine 历史".into(),tags:Vec::new()}).collect()
}

pub fn run(config_path:Option<&Path>, query:Option<&str>)->Result<u32>{
    let settings=config::load(config_path)?; let path=config::commands_path();
    let saved:Commands=fs::read_to_string(&path).ok().and_then(|s|toml::from_str(&s).ok()).unwrap_or_default();
    let mut items=saved.favorites; items.extend(templates_as_commands(saved.templates)); items.extend(builtins());items.extend(actions()); items.extend(history(settings.workbench.history_limit));
    let query=query.unwrap_or("").trim().to_lowercase(); if !query.is_empty(){items.retain(|i|format!("{} {} {}",i.name,i.description,i.tags.join(" ")).to_lowercase().contains(&query));}
    if !io::stdin().is_terminal(){for item in items.iter().take(100){println!("{}\t{}",item.command,item.name);}return Ok(0)}
    println!("Blueberry 命令工作台（输入序号，将命令填到输出；直接回车退出）\n"); for (i,item) in items.iter().take(30).enumerate(){println!("{:>2}. {:<24} {}",i+1,item.name,item.command)}
    print!("选择：");io::stdout().flush()?;let mut answer=String::new();io::stdin().read_line(&mut answer)?;
    if let Ok(index)=answer.trim().parse::<usize>() && let Some(item)=items.get(index.saturating_sub(1)){println!("{}",item.command);}
    if !path.exists(){if let Some(parent)=path.parent(){fs::create_dir_all(parent)?;}fs::write(&path,"# Blueberry 收藏与自定义模板\nschema_version = 2\n\nfavorites = []\ntemplates = []\n").with_context(||format!("无法创建 {}",path.display()))?;}
    Ok(0)
}

pub fn candidates(settings:&config::Config, query:&str)->Vec<Candidate>{
    candidates_with_history(settings,query,&[],None)
}

pub fn candidates_with_history(settings:&config::Config,query:&str,session:&[String],history_path:Option<&Path>)->Vec<Candidate>{
    let path=config::commands_path();
    let saved:Commands=fs::read_to_string(path).ok().and_then(|s|toml::from_str(&s).ok()).unwrap_or_default();
    let mut items=saved.favorites;items.extend(templates_as_commands(saved.templates));items.extend(builtins());items.extend(actions());items.extend(merged_history(settings.workbench.history_limit,session,history_path).into_iter().map(|command|SavedCommand{name:command.lines().next().unwrap_or("").to_owned(),command,description:"PowerShell 历史".into(),tags:Vec::new()}));
    let words=query.to_lowercase().split_whitespace().map(str::to_owned).collect::<Vec<_>>();
    items.into_iter().enumerate().filter(|(_,item)|{let text=format!("{} {} {} {}",item.name,item.description,item.command,item.tags.join(" ")).to_lowercase();words.iter().all(|word|text.contains(word))}).take(settings.completion.max_results).map(|(index,item)|Candidate{label:item.name,insert_text:item.command.clone(),description:item.description.clone(),kind:CandidateKind::Value,id:format!("hub:{index}:{}",item.command),source:if item.description.contains("历史"){"Blueberry 工作台/历史".into()}else if item.tags.iter().any(|t|t.contains("操作")){"Blueberry 工作台/操作".into()}else{"Blueberry 工作台".into()},detail:format!("{}\n\n命令\n  {}\n\n来源\n  {}",item.description,item.command,item.tags.join("、")),append_space:false,..Default::default()}).collect()
}

pub fn suggestions(settings:&config::Config,line:&str,cwd:&Path,session:&[String],history_path:Option<&Path>)->Vec<Candidate>{
    if !settings.workbench.suggestions||line.trim().is_empty(){return Vec::new()}
    let needle=line.to_lowercase();let saved:Commands=fs::read_to_string(config::commands_path()).ok().and_then(|s|toml::from_str(&s).ok()).unwrap_or_default();
    let mut values=saved.favorites.into_iter().map(|item|(item.command,item.description,"收藏".to_owned())).collect::<Vec<_>>();
    values.extend(crate::providers::project_actions(cwd,line).into_iter().map(|item|(item.value,item.description,"当前项目".to_owned())));
    values.extend(merged_history(settings.workbench.history_limit,session,history_path).into_iter().map(|command|(command,"最近使用".into(),"历史".into())));
    let mut seen=HashSet::new();values.into_iter().filter(|(command,_,_)|command.to_lowercase().starts_with(&needle)&&command!=line&&seen.insert(command.clone())).take(settings.workbench.suggestion_limit).enumerate().map(|(index,(command,description,source))|Candidate{
        label:command.clone(),insert_text:command.clone(),description,kind:CandidateKind::Command,id:format!("suggestion:{source}:{index}:{command}"),source:format!("Blueberry/{source}"),match_reason:format!("来自{source}"),replacement:Some(crate::model::Replacement{start:0,end:line.len()}),append_space:false,..Default::default()
    }).collect()
}

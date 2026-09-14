use crate::{config, knowledge, spec_catalog::Catalog};
use anyhow::Result;
use std::{collections::BTreeMap, env, path::{Path, PathBuf}};

const TOOLS: &[(&str,&str)] = &[
    ("python","Python"),("pip","pip"),("uv","uv"),("conda","Conda / Mamba"),("poetry","Poetry"),
    ("npm","npm"),("pnpm","pnpm"),("yarn","Yarn"),("bun","Bun"),("git","Git"),("cargo","Cargo"),
    ("rustup","rustup"),("go","Go"),("dotnet",".NET"),("cmake","CMake"),("docker","Docker"),
    ("kubectl","kubectl"),("helm","Helm"),("ssh","SSH")
];

fn locate(command:&str)->Option<PathBuf>{
    let path=env::var_os("PATH")?;
    let suffixes=if cfg!(windows){vec![".exe",".cmd",".bat",""]}else{vec![""]};
    env::split_paths(&path).find_map(|dir|suffixes.iter().map(|s|dir.join(format!("{command}{s}"))).find(|p|p.is_file()))
}

pub fn run(config_path:Option<&Path>, json:bool)->Result<u32>{
    let settings=config::load(config_path)?;
    let catalog=Catalog::load_user_dir(config::specs_dir(&settings,config_path))?;
    let help=knowledge::records(&knowledge::cache_dir()).into_iter().fold(BTreeMap::new(),|mut map,r|{map.insert(r.entry.command.clone(),r);map});
    let values=TOOLS.iter().map(|(command,label)|{
        let path=locate(command); let record=help.get(*command);
        serde_json::json!({"command":command,"label":label,"installed":path.is_some(),"path":path,"contexts":catalog.list().iter().filter(|n|n.path==*command||n.path.starts_with(&format!("{command} "))).count(),"help":record.map(|r|if r.error.is_some(){"失败"}else if knowledge::current(r){"已学习"}else{"已过期"}).unwrap_or("未学习")})
    }).collect::<Vec<_>>();
    if json { println!("{}",serde_json::to_string_pretty(&values)?); }
    else { println!("Blueberry 工具管理\n"); for value in values {println!("{:<14} {:<6} 规则 {:>3} 个 · 帮助 {}{}",value["label"].as_str().unwrap_or(""),if value["installed"].as_bool()==Some(true){"已安装"}else{"未发现"},value["contexts"],value["help"].as_str().unwrap_or(""),value["path"].as_str().map(|p|format!(" · {p}")).unwrap_or_default());} println!("\n学习：blueberry specs learn <命令> · 清除：blueberry specs forget <命令> · 设置：blueberry config edit"); }
    Ok(0)
}

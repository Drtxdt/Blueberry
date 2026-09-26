use crate::{
    config,
    model::{Candidate, CandidateKind},
};
use anyhow::{Context, Result, bail};
use crossterm::{
    cursor::MoveTo,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind},
    execute,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    env, fs,
    io::{self, IsTerminal, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    sync::{Arc, Condvar, Mutex, OnceLock, RwLock},
    thread,
    time::Duration,
};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum HubKind {
    #[default]
    Favorite,
    Template,
    Builtin,
    Action,
    History,
    Project,
    Guided,
}
#[derive(Clone, Default, Deserialize, Serialize)]
struct SavedCommand {
    name: String,
    command: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(skip)]
    kind: HubKind,
    #[serde(skip)]
    template: Option<CommandTemplate>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
struct Commands {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    favorites: Vec<SavedCommand>,
    #[serde(default)]
    templates: Vec<CommandTemplate>,
}
#[derive(Default)]
struct CommandsCacheState {
    commands: Commands,
    error: Option<String>,
    ready: bool,
}
struct CommandsCache {
    state: RwLock<CommandsCacheState>,
    wake: (Mutex<bool>, Condvar),
    listener: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}
static COMMANDS_CACHE: OnceLock<Arc<CommandsCache>> = OnceLock::new();
fn commands_cache() -> &'static Arc<CommandsCache> {
    COMMANDS_CACHE.get_or_init(|| {
        let cache = Arc::new(CommandsCache {
            state: RwLock::new(CommandsCacheState::default()),
            wake: (Mutex::new(false), Condvar::new()),
            listener: Mutex::new(None),
        });
        let worker = cache.clone();
        thread::spawn(move || {
            let mut previous: Option<std::result::Result<Option<String>, String>> = None;
            loop {
                let path = config::commands_path();
                let current = read_commands_text(&path).map_err(|error| format!("{error:#}"));
                if previous.as_ref() != Some(&current) {
                    let loaded = match &current {
                        Ok(Some(text)) => parse_commands(&path, text),
                        Ok(None) => Ok(Commands::default()),
                        Err(error) => Err(anyhow::anyhow!("{error}")),
                    };
                    {
                        let mut state = worker.state.write().unwrap();
                        state.ready = true;
                        match loaded {
                            Ok(commands) => {
                                state.commands = commands;
                                state.error = None;
                            }
                            Err(error) => state.error = Some(format!("{error:#}")),
                        }
                    }
                    if let Some(listener) = worker.listener.lock().unwrap().as_ref() {
                        listener();
                    }
                    previous = Some(current);
                }
                let signal = worker.wake.0.lock().unwrap();
                let _ = worker
                    .wake
                    .1
                    .wait_timeout(signal, Duration::from_millis(750))
                    .unwrap();
            }
        });
        cache
    })
}
pub fn warm_commands_cache() {
    let _ = commands_cache();
}
pub fn refresh_commands_cache() {
    commands_cache().wake.1.notify_one();
}
pub fn on_commands_change(listener: impl Fn() + Send + Sync + 'static) {
    let cache = commands_cache();
    *cache.listener.lock().unwrap() = Some(Box::new(listener));
    if cache.state.read().unwrap().ready
        && let Some(listener) = cache.listener.lock().unwrap().as_ref()
    {
        listener();
    }
}
pub fn commands_status() -> (bool, Option<String>) {
    let state = commands_cache().state.read().unwrap();
    (state.ready, state.error.clone())
}
pub fn inspect_commands() -> Result<usize> {
    let commands = load_commands(&config::commands_path())?;
    Ok(commands.favorites.len() + commands.templates.len())
}
fn cached_commands() -> Commands {
    commands_cache().state.read().unwrap().commands.clone()
}

fn read_commands_text(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("无法读取 {}", path.display())),
    }
}
fn parse_commands(path: &Path, text: &str) -> Result<Commands> {
    let commands: Commands =
        toml::from_str(text).with_context(|| format!("无法解析 {}", path.display()))?;
    if commands.schema_version > 2 {
        bail!(
            "{} 使用不支持的 schema_version {}",
            path.display(),
            commands.schema_version
        );
    }
    Ok(commands)
}
fn load_commands(path: &Path) -> Result<Commands> {
    read_commands_text(path)?
        .map(|text| parse_commands(path, &text))
        .unwrap_or_else(|| Ok(Commands::default()))
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct CommandTemplate {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub tokens: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<TemplateParameter>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct TemplateParameter {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "text_kind")]
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub source: String,
}
fn text_kind() -> String {
    "text".into()
}

impl CommandTemplate {
    fn preview(&self) -> String {
        if self.tokens.is_empty() {
            self.command.clone()
        } else {
            self.tokens.join(" ")
        }
    }
    pub fn render(&self, values: &BTreeMap<String, String>) -> Result<String> {
        if self.tokens.is_empty() {
            return Ok(fill_placeholders(&self.command, values));
        }
        let mut output = Vec::new();
        for token in &self.tokens {
            if let Some(name) = token.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                let Some(parameter) = self.parameters.iter().find(|p| p.name == name) else {
                    anyhow::bail!("模板字段 {name} 没有参数定义")
                };
                let value = values
                    .get(name)
                    .filter(|v| !v.is_empty())
                    .unwrap_or(&parameter.default);
                if parameter.required && value.is_empty() {
                    anyhow::bail!(
                        "参数 {} 不能为空",
                        if parameter.label.is_empty() {
                            name
                        } else {
                            &parameter.label
                        }
                    )
                }
                if parameter.kind == "enum"
                    && !value.is_empty()
                    && !parameter.values.iter().any(|v| v == value)
                {
                    anyhow::bail!("参数 {name} 不在枚举值中")
                }
                if value.is_empty() {
                    continue;
                }
                output.push(quote_powershell_token(value));
            } else if token.contains('{') || token.contains('}') {
                anyhow::bail!("模板占位符必须占据完整 token")
            } else {
                output.push(token.clone())
            }
        }
        Ok(output.join(" "))
    }
}
#[derive(Clone)]
pub struct TemplateForm {
    template: CommandTemplate,
    fields: Vec<TemplateParameter>,
    index: usize,
    values: BTreeMap<String, String>,
}
impl TemplateForm {
    pub fn new(template: CommandTemplate) -> Result<Self> {
        let names = if template.tokens.is_empty() {
            placeholder_names(&template.command)
        } else {
            template
                .tokens
                .iter()
                .filter_map(|token| token.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let mut fields = Vec::new();
        for name in names {
            if fields
                .iter()
                .any(|field: &TemplateParameter| field.name == name)
            {
                continue;
            }
            let mut field = template
                .parameters
                .iter()
                .find(|field| field.name == name)
                .cloned()
                .or_else(|| {
                    template.tokens.is_empty().then(|| TemplateParameter {
                        name: name.clone(),
                        required: true,
                        ..Default::default()
                    })
                })
                .with_context(|| format!("模板字段 {name} 没有参数定义"))?;
            if field.kind.is_empty() {
                field.kind = text_kind();
            }
            if !matches!(
                field.kind.as_str(),
                "text" | "file" | "directory" | "enum" | "dynamic"
            ) {
                bail!("参数 {} 使用未知类型 {}", field.name, field.kind);
            }
            fields.push(field);
        }
        Ok(Self {
            template,
            fields,
            index: 0,
            values: BTreeMap::new(),
        })
    }
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
    pub fn position(&self) -> (usize, usize) {
        (self.index + 1, self.fields.len())
    }
    pub fn current(&self) -> Option<&TemplateParameter> {
        self.fields.get(self.index)
    }
    pub fn set_values(&mut self, name: &str, values: Vec<String>) {
        if let Some(field) = self.fields.iter_mut().find(|field| field.name == name) {
            field.values = values;
        }
    }
    pub fn preview(&self) -> String {
        fill_placeholders(&self.template.preview(), &self.values)
    }
    pub fn submit(&mut self, entered: &str) -> Result<bool> {
        let field = self.current().context("参数表单已经结束")?;
        let value = if entered.is_empty() {
            field.default.as_str()
        } else {
            entered
        };
        if field.required && value.is_empty() {
            bail!("参数 {} 不能为空", field.label_or_name());
        }
        if field.kind == "enum" && !value.is_empty() && !field.values.iter().any(|v| v == value) {
            bail!("参数 {} 不在可选值中", field.label_or_name());
        }
        self.values.insert(field.name.clone(), value.to_owned());
        self.index += 1;
        Ok(self.index == self.fields.len())
    }
    pub fn finish(&self) -> Result<String> {
        if self.index != self.fields.len() {
            bail!("参数尚未填写完毕");
        }
        self.template.render(&self.values)
    }
}

/// The same form frame is used by both terminal hosts. Validation remains in
/// TemplateForm::submit; display never mutates the user's original buffer.
pub fn form_completion(
    form: &TemplateForm,
    entered: &str,
    original_len: usize,
) -> crate::model::Completion {
    let Some(field) = form.current() else {
        return Default::default();
    };
    let name = if field.label.is_empty() {
        &field.name
    } else {
        &field.label
    };
    let (position, count) = form.position();
    crate::model::Completion {
        replace_start: 0,
        replace_end: original_len,
        candidates: vec![Candidate {
            label: format!("填写参数：{name}"),
            insert_text: form.preview(),
            description: if !field.values.is_empty() {
                format!(
                    "↑↓ 选值：{}",
                    field
                        .values
                        .iter()
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("、")
                )
            } else if entered.is_empty() {
                if field.default.is_empty() {
                    if field.required {
                        "输入参数值后按 Enter".into()
                    } else {
                        "可跳过；按 Enter 继续".into()
                    }
                } else {
                    format!("默认值：{} · Enter 使用默认值", field.default)
                }
            } else {
                format!("当前值：{entered}")
            },
            kind: CandidateKind::Value,
            id: format!("hub-form:{name}"),
            source: "Blueberry 工作台/参数表单".into(),
            append_space: false,
            ..Default::default()
        }],
        incomplete: false,
        argument_hint: format!("参数 {position}/{count} · {name} · Enter 下一项 · Esc 取消"),
    }
}
impl TemplateParameter {
    fn label_or_name(&self) -> &str {
        if self.label.is_empty() {
            &self.name
        } else {
            &self.label
        }
    }
}
pub fn parameter_suggestions(
    field: &TemplateParameter,
    line: &str,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> Vec<String> {
    let cancelled = AtomicBool::new(false);
    if matches!(field.kind.as_str(), "file" | "directory") {
        let mut cache = crate::paths::PathCache::default();
        let context = crate::model::InputContext::default();
        let mut values = Vec::new();
        for _ in 0..64 {
            let result = cache.complete(
                "",
                0,
                &context,
                cwd,
                environment,
                field.kind == "directory",
                &cancelled,
            );
            values = result
                .candidates
                .into_iter()
                .map(|candidate| candidate.label)
                .take(100)
                .collect();
            if !result.scan_pending {
                break;
            }
        }
        return values;
    }
    if !matches!(
        field.source.as_str(),
        "cargo.packages" | "cargo.features" | "cargo.tests" | "docker.compose.service"
    ) {
        return field.values.clone();
    }
    let mut words = line.split_whitespace();
    let Some(command) = words.next() else {
        return Vec::new();
    };
    let mut environment = environment.clone();
    environment.remove("BLUEBERRY_REMOTE_REQUESTED");
    let mut query = crate::providers::ProjectQuery::new(0, command, words, "", cwd, environment);
    query.provider = Some(field.source.clone());
    crate::providers::collect(&query, &cancelled)
        .candidates
        .into_iter()
        .map(|candidate| candidate.value)
        .take(100)
        .collect()
}
pub fn quote_powershell_token(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./:@+,-\\".contains(c))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}
pub fn placeholder_names(command: &str) -> Vec<String> {
    let mut names = Vec::new();
    let chars = command.char_indices().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        let (start, ch) = chars[index];
        let close = match ch {
            '{' => '}',
            '<' => '>',
            _ => {
                index += 1;
                continue;
            }
        };
        if let Some((offset, (end, _))) = chars[index + 1..]
            .iter()
            .enumerate()
            .find(|(_, (_, c))| *c == close)
        {
            let name = &command[start + ch.len_utf8()..*end];
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && !names.iter().any(|n| n == name)
            {
                names.push(name.to_owned())
            }
            index += offset + 2
        } else {
            index += 1
        }
    }
    names
}
pub fn fill_placeholders(command: &str, values: &BTreeMap<String, String>) -> String {
    let mut result = command.to_owned();
    for (name, value) in values {
        let value = quote_powershell_token(value);
        result = result
            .replace(&format!("{{{name}}}"), &value)
            .replace(&format!("<{name}>"), &value);
    }
    result
}
fn templates_as_commands(values: Vec<CommandTemplate>) -> Vec<SavedCommand> {
    values
        .into_iter()
        .map(|item| {
            let command = item.preview();
            SavedCommand {
                name: item.name.clone(),
                command,
                description: if item.description.is_empty() {
                    "用户模板".into()
                } else {
                    item.description.clone()
                },
                tags: item.tags.clone(),
                kind: HubKind::Template,
                template: Some(item),
            }
        })
        .collect()
}
fn project_templates_as_commands(values: Vec<CommandTemplate>) -> Vec<SavedCommand> {
    templates_as_commands(values)
        .into_iter()
        .map(|mut item| {
            item.kind = HubKind::Project;
            item
        })
        .collect()
}
fn guided_as_command(template: CommandTemplate) -> SavedCommand {
    let mut item = templates_as_commands(vec![template]).remove(0);
    item.kind = HubKind::Guided;
    item
}
pub fn guided_template(line: &str, cursor: usize) -> Option<CommandTemplate> {
    if cursor > line.len() || !line.is_char_boundary(cursor) {
        return None;
    }
    let before = &line[..cursor];
    let after = &line[cursor..];
    if (!after.is_empty() && !after.starts_with(char::is_whitespace))
        || line
            .chars()
            .any(|ch| ch.is_control() || "'\";|&`{}<>".contains(ch))
    {
        return None;
    }
    let words = before.split_whitespace().collect::<Vec<_>>();
    let (id, name, parameter, label, kind, source) = match words.as_slice() {
        ["git", "switch", "-c"] => (
            "git-branch",
            "继续填写 Git 分支",
            "BRANCH",
            "新分支名称",
            "text",
            "",
        ),
        ["cargo", "test", "-p"] => (
            "cargo-package",
            "继续填写 Cargo 包",
            "PACKAGE",
            "工作区包",
            "dynamic",
            "cargo.packages",
        ),
        ["cargo", "test", "--features"] => (
            "cargo-features",
            "继续填写 Cargo features",
            "FEATURES",
            "Feature",
            "dynamic",
            "cargo.features",
        ),
        ["cargo", "test", "--test"] => (
            "cargo-test",
            "继续填写 Cargo 测试目标",
            "TARGET",
            "测试目标",
            "dynamic",
            "cargo.tests",
        ),
        ["docker", "compose", "up"] | ["docker", "compose", "up", "-d"] => (
            "compose-up",
            "继续填写 Compose 服务",
            "SERVICE",
            "服务",
            "dynamic",
            "docker.compose.service",
        ),
        ["docker", "compose", "logs"] | ["docker", "compose", "logs", "-f"] => (
            "compose-logs",
            "继续填写 Compose 日志服务",
            "SERVICE",
            "服务",
            "dynamic",
            "docker.compose.service",
        ),
        _ => return None,
    };
    let separator = if before.ends_with(char::is_whitespace) {
        ""
    } else {
        " "
    };
    Some(CommandTemplate {
        id: format!("guided:{id}"),
        name: name.into(),
        command: format!("{before}{separator}{{{parameter}}}{after}"),
        description: "从当前命令继续；确认只填回，不执行".into(),
        parameters: vec![TemplateParameter {
            name: parameter.into(),
            label: label.into(),
            kind: kind.into(),
            source: source.into(),
            required: true,
            ..Default::default()
        }],
        ..Default::default()
    })
}

fn edit_commands(path: &Path, edit: impl FnOnce(&mut DocumentMut) -> Result<bool>) -> Result<bool> {
    let parent = path.parent().context("收藏文件没有父目录")?;
    fs::create_dir_all(parent).with_context(|| format!("无法创建 {}", parent.display()))?;
    // The operating system releases this lock when a process exits, including
    // when it is interrupted between the read and the replacement.
    let lock_path = path.with_extension("toml.lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("无法打开锁文件 {}", lock_path.display()))?;
    lock.lock()
        .with_context(|| format!("无法锁定 {}", lock_path.display()))?;
    let original = read_commands_text(path)?;
    let mut document: DocumentMut = if let Some(text) = original.as_deref() {
        parse_commands(path, text)?;
        text.parse()
            .with_context(|| format!("无法解析 {}", path.display()))?
    } else {
        DocumentMut::new()
    };
    if !edit(&mut document)? {
        return Ok(false);
    }
    document["schema_version"] = value(2);
    let bytes = document.to_string().into_bytes();
    let temporary = parent.join(format!(".commands-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        output.write_all(&bytes)?;
        output.sync_all()?;
        drop(output);
        #[cfg(test)]
        if let Some(marker) = std::env::var_os("BLUEBERRY_TEST_STORE_PAUSE") {
            fs::write(marker, b"synced")?;
            loop {
                thread::park_timeout(Duration::from_millis(100));
            }
        }
        // An editor that does not take our lock may have changed the file.
        if read_commands_text(path)? != original {
            bail!("{} 在保存期间被外部修改；未覆盖原文件", path.display());
        }
        crate::engine::replace_file(&temporary, path)
            .with_context(|| format!("无法替换 {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    refresh_commands_cache();
    Ok(true)
}

fn favorites_table(document: &mut DocumentMut) -> Result<&mut ArrayOfTables> {
    if document.get("favorites").is_none() {
        document["favorites"] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    document["favorites"]
        .as_array_of_tables_mut()
        .context("favorites 必须是表数组")
}
fn add_favorite_at(path: &Path, name: &str, command: &str) -> Result<()> {
    edit_commands(path, |document| {
        let favorites = favorites_table(document)?;
        if let Some(item) = favorites
            .iter_mut()
            .find(|item| item.get("name").and_then(Item::as_str) == Some(name))
        {
            item["command"] = value(command);
        } else {
            let mut item = Table::new();
            item["name"] = value(name);
            item["command"] = value(command);
            item["description"] = value("用户收藏");
            favorites.push(item);
        }
        Ok(true)
    })?;
    Ok(())
}
fn remove_favorite_at(path: &Path, name: &str) -> Result<usize> {
    let mut removed = 0;
    edit_commands(path, |document| {
        if let Some(favorites) = document.get_mut("favorites") {
            let favorites = favorites
                .as_array_of_tables_mut()
                .context("favorites 必须是表数组")?;
            for index in (0..favorites.len()).rev() {
                if favorites
                    .get(index)
                    .and_then(|item| item.get("name"))
                    .and_then(Item::as_str)
                    == Some(name)
                {
                    favorites.remove(index);
                    removed += 1;
                }
            }
        }
        Ok(removed > 0)
    })?;
    Ok(removed)
}
pub fn add_favorite(name: &str, command: &str) -> Result<()> {
    add_favorite_at(&config::commands_path(), name, command)?;
    println!("已保存收藏：{name}");
    Ok(())
}
pub fn remove_favorite(name: &str) -> Result<()> {
    let removed = remove_favorite_at(&config::commands_path(), name)?;
    println!("已删除 {removed} 个同名收藏。");
    Ok(())
}

fn builtins() -> Vec<SavedCommand> {
    [
        ("创建 Git 分支", "git switch -c <BRANCH>"),
        ("切换 Git 分支", "git switch <BRANCH>"),
        ("运行 Cargo 测试", "cargo test"),
        ("运行 dotnet 测试", "dotnet test <PROJECT>"),
        ("运行 Go 测试", "go test ./..."),
        ("运行 Python 测试", "uv run pytest"),
        ("运行 npm 脚本", "npm run <SCRIPT>"),
        ("运行 pnpm 脚本", "pnpm run <SCRIPT>"),
        ("运行 Yarn 脚本", "yarn run <SCRIPT>"),
        ("运行 Bun 脚本", "bun run <SCRIPT>"),
        ("进入 Conda 环境", "conda activate <ENV>"),
        ("在 Conda 环境运行", "conda run -n <ENV> <COMMAND>"),
        ("启动 Compose 服务", "docker compose up -d <SERVICE>"),
        ("查看 Compose 日志", "docker compose logs -f <SERVICE>"),
        (
            "进入 Compose 服务",
            "docker compose exec <SERVICE> <COMMAND>",
        ),
        ("查看 Kubernetes 日志", "kubectl logs <POD>"),
        (
            "进入 Kubernetes 容器",
            "kubectl exec -it <POD> -- <COMMAND>",
        ),
        ("连接 SSH 主机", "ssh <HOST>"),
    ]
    .into_iter()
    .map(|(n, c)| SavedCommand {
        name: n.into(),
        command: c.into(),
        description: "内置命令模板".into(),
        tags: Vec::new(),
        kind: HubKind::Builtin,
        template: None,
    })
    .collect()
}

fn actions() -> Vec<SavedCommand> {
    [
        (
            "打开设置",
            "blueberry config edit",
            "调整外观、补全和快捷键",
        ),
        (
            "管理工具",
            "blueberry tools",
            "查看工具入口、规则和学习状态",
        ),
        ("运行诊断", "blueberry doctor", "检查配置、适配器和安装状态"),
        (
            "审查项目命令包",
            "blueberry packs review",
            "预览当前仓库的声明式操作与变更",
        ),
        ("重新运行引导", "blueberry setup", "重新选择常用初始设置"),
    ]
    .into_iter()
    .map(|(n, c, d)| SavedCommand {
        name: n.into(),
        command: c.into(),
        description: d.into(),
        tags: vec!["Blueberry 操作".into()],
        kind: HubKind::Action,
        template: None,
    })
    .collect()
}

const HISTORY_BYTE_BUDGET: u64 = 16 * 1024 * 1024;
fn read_history_file_limited(path: &Path, limit: usize) -> (Vec<String>, bool) {
    read_history_file_budget(path, limit, HISTORY_BYTE_BUDGET)
}
fn read_history_file_budget(path: &Path, limit: usize, budget: u64) -> (Vec<String>, bool) {
    let Ok(mut file) = fs::File::open(path) else {
        return (Vec::new(), false);
    };
    let Ok(length) = file.metadata().map(|meta| meta.len()) else {
        return (Vec::new(), false);
    };
    let start = length.saturating_sub(budget);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return (Vec::new(), false);
    }
    let mut bytes = Vec::new();
    if file.take(budget).read_to_end(&mut bytes).is_err() {
        return (Vec::new(), false);
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut skip_partial = start > 0;
    for line in text.lines() {
        if skip_partial {
            if !line.ends_with('`') {
                skip_partial = false;
            }
            continue;
        }
        current.push_str(line);
        if current.ends_with('`') {
            current.pop();
            current.push('\n');
        } else if !current.trim().is_empty() {
            lines.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        lines.push(current)
    }
    let partial = start > 0 || lines.len() > limit;
    if lines.len() > limit {
        lines.drain(..lines.len() - limit);
    }
    (lines, partial)
}
pub fn read_history_file(path: &Path) -> Vec<String> {
    read_history_file_limited(path, 20_000).0
}

fn history_files() -> Vec<PathBuf> {
    let Some(root) = env::var_os("APPDATA").map(PathBuf::from) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(root.join("Microsoft/Windows/PowerShell/PSReadLine")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("txt"))
        .collect()
}

pub fn merged_history(limit: usize, session: &[String], exact_path: Option<&Path>) -> Vec<String> {
    merged_history_with_status(limit, session, exact_path).0
}
pub fn merged_history_with_status(
    limit: usize,
    session: &[String],
    exact_path: Option<&Path>,
) -> (Vec<String>, bool) {
    let files = exact_path
        .map(|p| vec![p.to_path_buf()])
        .unwrap_or_default();
    let mut partial = false;
    let mut lines = Vec::new();
    for path in &files {
        let (values, truncated) = read_history_file_limited(path, limit);
        lines.extend(values);
        partial |= truncated;
    }
    lines.extend(session.iter().cloned());
    let mut seen = HashSet::new();
    let values = lines
        .into_iter()
        .rev()
        .filter(|line| !line.trim().is_empty() && seen.insert(line.clone()))
        .take(limit)
        .collect();
    (values, partial)
}

fn history(limit: usize) -> Vec<SavedCommand> {
    let mut values = history_files()
        .iter()
        .flat_map(|path| read_history_file_limited(path, limit).0)
        .collect::<Vec<_>>();
    let mut seen = HashSet::new();
    values.reverse();
    values
        .into_iter()
        .filter(|line| !line.trim().is_empty() && seen.insert(line.clone()))
        .take(limit)
        .map(|command| SavedCommand {
            name: command.lines().next().unwrap_or("").to_owned(),
            command,
            description: "PSReadLine 历史".into(),
            tags: Vec::new(),
            kind: HubKind::History,
            template: None,
        })
        .collect()
}

pub fn run(config_path: Option<&Path>, query: Option<&str>) -> Result<u32> {
    let settings = config::load(config_path)?;
    let saved = load_commands(&config::commands_path())?;
    let mut items = saved.favorites;
    items.extend(templates_as_commands(saved.templates));
    items.extend(project_templates_as_commands(
        crate::packs::approved_templates(&std::env::current_dir()?)?,
    ));
    items.extend(builtins());
    items.extend(actions());
    items.extend(history(settings.workbench.history_limit));
    let mut query = query.unwrap_or("").trim().to_owned();
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        let words = query.to_lowercase();
        for item in items.iter().filter(|i| hub_match(i, &words)).take(100) {
            println!("{}\t{}", item.command, item.name);
        }
        return Ok(0);
    }
    let screen = crate::terminal_ui::ScreenGuard::enter(false)?;
    let mut selected = 0usize;
    let mut preview = false;
    let command = loop {
        let visible = items
            .iter()
            .enumerate()
            .filter(|(_, item)| hub_match(item, &query.to_lowercase()))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        selected = selected.min(visible.len().saturating_sub(1));
        draw_hub(&items, &visible, selected, &query, preview)?;
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc => break None,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break None,
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    query.push(ch);
                    selected = 0;
                    preview = false
                }
                KeyCode::Backspace => {
                    query.pop();
                    selected = 0;
                    preview = false
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
                KeyCode::Tab | KeyCode::F(1) => preview = !preview,
                KeyCode::Enter => {
                    if let Some(index) = visible.get(selected) {
                        if items[*index].kind == HubKind::Project {
                            crate::packs::verify_approval(&std::env::current_dir()?)?;
                        }
                        if let Some(command) = fill_interactive(&items[*index])? {
                            if items[*index].kind == HubKind::Project {
                                crate::packs::verify_approval(&std::env::current_dir()?)?;
                            }
                            break Some(command);
                        }
                    }
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
    };
    drop(screen);
    if let Some(command) = command {
        copy_or_print(&command)?
    }
    Ok(0)
}

fn hub_match(item: &SavedCommand, query: &str) -> bool {
    let text = format!(
        "{} {} {} {}",
        item.name,
        item.command,
        item.description,
        item.tags.join(" ")
    )
    .to_lowercase();
    query.split_whitespace().all(|word| text.contains(word))
}
fn draw_hub(
    items: &[SavedCommand],
    visible: &[usize],
    selected: usize,
    query: &str,
    preview: bool,
) -> Result<()> {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((100, 30));
    let mut out = io::stdout();
    write!(out, "\x1b[?2026h")?;
    execute!(
        out,
        MoveTo(0, 0),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
    )?;
    writeln!(out, "Blueberry 命令工作台")?;
    writeln!(
        out,
        "搜索：{}",
        if query.is_empty() {
            "输入名称、命令、说明或标签".to_owned()
        } else {
            crate::terminal_ui::fit(query, cols.saturating_sub(4) as usize)
        }
    )?;
    writeln!(out, "{}", "─".repeat(cols.saturating_sub(1) as usize))?;
    let count = rows.saturating_sub(7) as usize;
    let start = selected.saturating_sub(count.saturating_sub(1));
    for (row, index) in visible.iter().skip(start).take(count).enumerate() {
        let item = &items[*index];
        writeln!(
            out,
            "{}",
            crate::terminal_ui::fit(
                &format!(
                    "{} {:<24}  {}",
                    if start + row == selected { "▶" } else { " " },
                    item.name,
                    item.command
                ),
                cols as usize
            )
        )?
    }
    if preview && let Some(index) = visible.get(selected) {
        let item = &items[*index];
        execute!(out, MoveTo(0, rows.saturating_sub(4)))?;
        write!(
            out,
            "预览：{}\n{}",
            crate::terminal_ui::fit(&item.command.replace('\n', " ↵ "), cols as usize),
            crate::terminal_ui::fit(&item.description, cols as usize)
        )?
    }

    execute!(out, MoveTo(0, rows.saturating_sub(1)))?;
    write!(
        out,
        "{}",
        crate::terminal_ui::fit(
            "工作台：Enter 填写/复制，不执行 · Tab/F1 预览 · Esc 返回",
            cols as usize
        )
    )?;
    write!(out, "\x1b[?2026l")?;
    out.flush()?;
    Ok(())
}
fn fill_interactive(item: &SavedCommand) -> Result<Option<String>> {
    let template = item.template.clone().unwrap_or_else(|| CommandTemplate {
        command: item.command.clone(),
        ..Default::default()
    });
    let mut form = TemplateForm::new(template)?;
    if form.is_empty() {
        return Ok(Some(form.finish()?));
    }
    loop {
        if let Some(field) = form.current().cloned()
            && (matches!(field.kind.as_str(), "file" | "directory") || !field.source.is_empty())
        {
            let values = parameter_suggestions(
                &field,
                &item.command,
                &std::env::current_dir()?,
                &std::env::vars().collect(),
            );
            form.set_values(&field.name, values);
        }
        let field = form.current().cloned().context("参数表单已经结束")?;
        let name = field.label_or_name().to_owned();
        let (position, count) = form.position();
        let mut value = String::new();
        let mut error = String::new();
        loop {
            let (cols, rows) = crossterm::terminal::size().unwrap_or((100, 30));
            let mut out = io::stdout();
            write!(out, "\x1b[?2026h")?;
            execute!(
                out,
                MoveTo(0, 0),
                crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
            )?;
            writeln!(out, "填写模板参数  {position}/{count}")?;
            writeln!(
                out,
                "\n{}：{}",
                crate::terminal_ui::fit(&name, cols as usize),
                crate::terminal_ui::fit(&value, cols as usize)
            )?;
            if !field.default.is_empty() {
                writeln!(
                    out,
                    "默认：{}",
                    crate::terminal_ui::fit(&field.default, cols as usize)
                )?;
            }
            if !field.values.is_empty() {
                writeln!(
                    out,
                    "↑↓ 选值：{}",
                    crate::terminal_ui::fit(&field.values.join("、"), cols as usize)
                )?;
            }
            if !error.is_empty() {
                writeln!(out, "{error}")?;
            }
            writeln!(
                out,
                "\n预览：{}",
                crate::terminal_ui::fit(&form.preview(), cols as usize)
            )?;
            execute!(out, MoveTo(0, rows.saturating_sub(1)))?;
            write!(out, "输入参数 · Enter 下一项 · Esc 取消")?;
            write!(out, "\x1b[?2026l")?;
            out.flush()?;
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                    KeyCode::Esc => return Ok(None),
                    KeyCode::Enter => match form.submit(&value) {
                        Ok(finished) if finished => return Ok(Some(form.finish()?)),
                        Ok(_) => break,
                        Err(problem) => error = format!("{problem:#}"),
                    },
                    KeyCode::Char(ch)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        value.push(ch)
                    }
                    KeyCode::Backspace => {
                        value.pop();
                    }
                    KeyCode::Up | KeyCode::Down if !field.values.is_empty() => {
                        let found = field.values.iter().position(|item| item == &value);
                        let next = match (key.code, found) {
                            (KeyCode::Down, Some(index)) => (index + 1) % field.values.len(),
                            (KeyCode::Up, Some(0)) => field.values.len() - 1,
                            (KeyCode::Up, Some(index)) => index - 1,
                            (KeyCode::Down, None) => 0,
                            _ => field.values.len() - 1,
                        };
                        value = field.values[next].clone();
                    }
                    _ => {}
                },
                Event::Paste(text) => value.push_str(&text.replace(['\r', '\n'], " ")),
                Event::Mouse(_) => continue,
                _ => {}
            }
        }
    }
}
fn copy_or_print(command: &str) -> Result<()> {
    #[cfg(windows)]
    {
        use std::process::{Command, Stdio};
        if let Ok(mut child) = Command::new("clip.exe").stdin(Stdio::piped()).spawn() {
            if let Some(mut input) = child.stdin.take() {
                input.write_all(command.as_bytes())?
            }
            if child.wait().is_ok_and(|status| status.success()) {
                println!("已复制命令：{command}");
                return Ok(());
            }
        }
    }
    println!("{command}");
    Ok(())
}

pub fn candidates(settings: &config::Config, query: &str) -> Vec<Candidate> {
    let history = history(settings.workbench.history_limit)
        .into_iter()
        .map(|item| item.command)
        .collect::<Vec<_>>();
    candidates_with_history(settings, query, &history, None)
}

pub fn candidates_with_history(
    settings: &config::Config,
    query: &str,
    session: &[String],
    history_path: Option<&Path>,
) -> Vec<Candidate> {
    search_with_history(settings, query, session, history_path).0
}
pub fn search_with_history(
    settings: &config::Config,
    query: &str,
    session: &[String],
    history_path: Option<&Path>,
) -> (Vec<Candidate>, BTreeMap<String, CommandTemplate>) {
    search_with_history_guided(settings, query, session, history_path, None)
}
pub fn search_with_history_guided(
    settings: &config::Config,
    query: &str,
    session: &[String],
    history_path: Option<&Path>,
    guided: Option<CommandTemplate>,
) -> (Vec<Candidate>, BTreeMap<String, CommandTemplate>) {
    let saved = cached_commands();
    let mut items = saved.favorites;
    if let Some(guided) = guided {
        items.insert(0, guided_as_command(guided));
    }
    items.extend(templates_as_commands(saved.templates));
    items.extend(project_templates_as_commands(
        crate::packs::cached_templates(),
    ));
    items.extend(builtins());
    items.extend(actions());
    items.extend(
        merged_history(settings.workbench.history_limit, session, history_path)
            .into_iter()
            .map(|command| SavedCommand {
                name: command.lines().next().unwrap_or("").to_owned(),
                command,
                description: "PowerShell 历史".into(),
                tags: Vec::new(),
                kind: HubKind::History,
                template: None,
            }),
    );
    let words = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut templates = BTreeMap::new();
    let candidates = items
        .into_iter()
        .enumerate()
        .filter(|(_, item)| {
            let text = format!(
                "{} {} {} {}",
                item.name,
                item.description,
                item.command,
                item.tags.join(" ")
            )
            .to_lowercase();
            words.iter().all(|word| text.contains(word))
        })
        .take(settings.completion.max_results)
        .map(|(index, item)| {
            let id = if let Some(template) = &item.template {
                let key = if template.id.is_empty() {
                    format!("{}:{}", template.name, item.command)
                } else {
                    template.id.clone()
                };
                format!(
                    "hub:{}:{key}",
                    match item.kind {
                        HubKind::Project => "project",
                        HubKind::Guided => "guided",
                        _ => "template",
                    }
                )
            } else {
                format!("hub:{index}:{}", item.command)
            };
            if let Some(template) = item.template {
                templates.insert(id.clone(), template);
            }
            let review = if item.kind == HubKind::Action && item.command == "blueberry packs review"
            {
                crate::packs::cached_review()
            } else {
                None
            };
            Candidate {
                label: item.name,
                insert_text: item.command.clone(),
                description: if review.is_some() {
                    "项目命令包来源、校验与批准差异；F1 查看".into()
                } else {
                    item.description.clone()
                },
                kind: CandidateKind::Value,
                id,
                source: if item.kind == HubKind::History {
                    "Blueberry 工作台/历史".into()
                } else if item.kind == HubKind::Action {
                    "Blueberry 工作台/操作".into()
                } else if item.kind == HubKind::Template {
                    "Blueberry 工作台/模板".into()
                } else if item.kind == HubKind::Project {
                    "Blueberry 工作台/项目".into()
                } else if item.kind == HubKind::Guided {
                    "Blueberry 工作台/当前命令".into()
                } else {
                    "Blueberry 工作台".into()
                },
                detail: review.unwrap_or_else(|| {
                    format!(
                        "{}\n\n命令\n  {}\n\n来源\n  {}",
                        item.description,
                        item.command,
                        item.tags.join("、")
                    )
                }),
                append_space: false,
                ..Default::default()
            }
        })
        .collect();
    (candidates, templates)
}

pub fn suggestions(settings: &config::Config, line: &str, session: &[String]) -> Vec<Candidate> {
    if !settings.workbench.suggestions || line.trim().is_empty() {
        return Vec::new();
    }
    let needle = line.to_lowercase();
    let saved = cached_commands();
    let mut values = saved
        .favorites
        .into_iter()
        .map(|item| (item.command, item.description, "收藏".to_owned()))
        .collect::<Vec<_>>();
    let scan_limit = settings
        .workbench
        .suggestion_limit
        .saturating_mul(20)
        .max(50)
        .min(settings.workbench.history_limit);
    values.extend(
        session
            .iter()
            .take(scan_limit)
            .cloned()
            .map(|command| (command, "最近使用".into(), "历史".into())),
    );
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|(command, _, _)| {
            command.to_lowercase().starts_with(&needle)
                && command != line
                && seen.insert(command.clone())
        })
        .take(settings.workbench.suggestion_limit)
        .enumerate()
        .map(|(index, (command, description, source))| Candidate {
            label: command.clone(),
            insert_text: command.clone(),
            description,
            kind: CandidateKind::Command,
            id: format!("suggestion:{source}:{index}:{command}"),
            source: format!("Blueberry/{source}"),
            match_reason: format!("来自{source}"),
            replacement: Some(crate::model::Replacement {
                start: 0,
                end: line.len(),
            }),
            append_space: false,
            ..Default::default()
        })
        .collect()
}

#[cfg(test)]
mod store_tests {
    use super::*;
    use std::{process::Command, time::Instant};

    #[test]
    fn malformed_or_future_commands_are_never_replaced() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("commands.toml");
        for text in [
            "schema_version = 2\n[[favorites]]\nname = 'old'\ncommand = 'git status'\n[[templates]]\nname = '",
            "schema_version = 99\n",
        ] {
            fs::write(&path, text)?;
            assert!(add_favorite_at(&path, "new", "cargo test").is_err());
            assert!(remove_favorite_at(&path, "old").is_err());
            assert_eq!(fs::read_to_string(&path)?, text);
        }
        Ok(())
    }

    #[test]
    fn edits_preserve_templates_comments_and_unknown_fields() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("commands.toml");
        fs::write(
            &path,
            "# user note\nschema_version = 2\ncustom = 'keep'\n\n[[templates]]\nid = 'build'\nname = 'Build'\ntokens = ['cargo', 'build']\n",
        )?;
        add_favorite_at(&path, "first", "git status")?;
        let text = fs::read_to_string(&path)?;
        assert!(text.contains("# user note"));
        assert!(text.contains("custom = 'keep'"));
        assert!(text.contains("tokens = ['cargo', 'build']"));
        let before = text.clone();
        assert_eq!(remove_favorite_at(&path, "missing")?, 0);
        assert_eq!(fs::read_to_string(&path)?, before);
        assert_eq!(remove_favorite_at(&path, "first")?, 1);
        assert_eq!(load_commands(&path)?.favorites.len(), 0);
        Ok(())
    }

    #[test]
    fn concurrent_writers_keep_every_favorite() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("commands.toml");
        let threads = (0..12)
            .map(|index| {
                let path = path.clone();
                thread::spawn(move || {
                    add_favorite_at(&path, &format!("favorite-{index}"), "git status")
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap()?;
        }
        assert_eq!(load_commands(&path)?.favorites.len(), 12);
        Ok(())
    }

    #[test]
    fn external_edit_between_read_and_replace_aborts_without_overwrite() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("commands.toml");
        fs::write(&path, "schema_version = 2\n# original\n")?;
        let external = "schema_version = 2\n# external editor\n";
        let result = edit_commands(&path, |document| {
            document["favorites"] = Item::ArrayOfTables(ArrayOfTables::new());
            fs::write(&path, external)?;
            Ok(true)
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path)?, external);
        assert!(fs::read_dir(directory.path())?.all(|entry| {
            !entry
                .map(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
                .unwrap_or(false)
        }));
        Ok(())
    }

    #[test]
    fn interrupted_writer_child() -> Result<()> {
        let Some(path) = std::env::var_os("BLUEBERRY_TEST_STORE_CHILD_PATH") else {
            return Ok(());
        };
        add_favorite_at(&PathBuf::from(path), "interrupted", "git status")
    }

    #[test]
    fn killed_process_after_sync_keeps_original_and_releases_lock() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("commands.toml");
        let marker = directory.path().join("writer-synced.signal");
        let original = "# preserve me\nschema_version = 2\n";
        fs::write(&path, original)?;
        let mut child = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "hub::store_tests::interrupted_writer_child",
                "--nocapture",
            ])
            .env("BLUEBERRY_TEST_STORE_CHILD_PATH", &path)
            .env("BLUEBERRY_TEST_STORE_PAUSE", &marker)
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() {
            if let Some(status) = child.try_wait()? {
                bail!("writer exited before sync point: {status}");
            }
            if Instant::now() >= deadline {
                child.kill()?;
                let _ = child.wait();
                bail!("writer did not reach the synced temp file");
            }
            thread::sleep(Duration::from_millis(5));
        }
        child.kill()?;
        child.wait()?;
        assert_eq!(fs::read_to_string(&path)?, original);
        add_favorite_at(&path, "after-crash", "cargo test")?;
        let text = fs::read_to_string(&path)?;
        assert!(text.contains("# preserve me"));
        assert!(text.contains("after-crash"));
        assert!(!text.contains("interrupted"));
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn denied_replacement_preserves_original() -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("commands.toml");
        let original = "# preserve on sharing failure\nschema_version = 2\n";
        fs::write(&path, original)?;
        // An external reader that does not share delete models a Windows
        // access/sharing failure at the atomic replacement boundary.
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)?;
        assert!(add_favorite_at(&path, "blocked", "git status").is_err());
        assert_eq!(fs::read_to_string(&path)?, original);
        drop(held);
        add_favorite_at(&path, "allowed", "git status")?;
        assert_eq!(load_commands(&path)?.favorites.len(), 1);
        Ok(())
    }
}

#[cfg(test)]
mod template_tests {
    use super::*;

    #[test]
    fn structured_form_applies_defaults_optional_fields_and_enum_rules() -> Result<()> {
        let template = CommandTemplate {
            tokens: vec![
                "cargo".into(),
                "test".into(),
                "-p".into(),
                "{package}".into(),
                "{mode}".into(),
                "{optional}".into(),
            ],
            parameters: vec![
                TemplateParameter {
                    name: "package".into(),
                    label: "包".into(),
                    required: true,
                    ..Default::default()
                },
                TemplateParameter {
                    name: "mode".into(),
                    kind: "enum".into(),
                    default: "fast".into(),
                    values: vec!["fast".into(), "slow".into()],
                    ..Default::default()
                },
                TemplateParameter {
                    name: "optional".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut form = TemplateForm::new(template)?;
        assert!(form.submit("").is_err());
        assert!(!form.submit("my package")?);
        assert!(form.submit("invalid").is_err());
        assert!(!form.submit("")?);
        assert!(form.submit("")?);
        assert_eq!(form.finish()?, "cargo test -p 'my package' fast");
        Ok(())
    }

    #[test]
    fn legacy_command_form_keeps_placeholder_behavior() -> Result<()> {
        let mut form = TemplateForm::new(CommandTemplate {
            command: "git switch -c <BRANCH>".into(),
            ..Default::default()
        })?;
        assert!(!form.is_empty());
        assert!(form.submit("feature's work")?);
        assert_eq!(form.finish()?, "git switch -c 'feature''s work'");
        Ok(())
    }

    #[test]
    fn guided_form_preserves_existing_text_and_suffix() -> Result<()> {
        let line = "cargo test -p --release";
        let cursor = "cargo test -p".len();
        let template = guided_template(line, cursor).context("guided package form")?;
        let mut form = TemplateForm::new(template)?;
        assert!(form.submit("my crate")?);
        assert_eq!(form.finish()?, "cargo test -p 'my crate' --release");
        assert!(guided_template("cargo test -p | rm", cursor).is_none());
        assert!(guided_template("cargo test -pX", cursor).is_none());
        Ok(())
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn tail_budget_discards_an_incomplete_multiline_record() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("history.txt");
        fs::write(
            &path,
            "old command with a very long prefix`\ncontinued old line\nnew`\npart\nlatest\n",
        )?;
        let (history, partial) = read_history_file_budget(&path, 10, 23);
        assert!(partial);
        assert_eq!(history, vec!["new\npart", "latest"]);
        Ok(())
    }
}

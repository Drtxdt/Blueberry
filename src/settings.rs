//! Interactive editing of the existing configuration document.
use crate::{
    config::{self, Config},
    model::{Candidate, CandidateKind},
};
use anyhow::{Context, Result, bail};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{
    collections::BTreeMap,
    env,
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, value};

#[derive(Clone, Copy)]
enum Kind {
    Toggle,
    Choice(&'static [(&'static str, &'static str)]),
    Number(i64, i64),
    Key,
}
struct Field {
    section: &'static str,
    key: &'static str,
    group: &'static str,
    label: &'static str,
    help: &'static str,
    kind: Kind,
}
const THEMES: &[(&str, &str)] = &[
    ("dark", "深色"),
    ("light", "浅色"),
    ("high_contrast", "高对比度"),
];
const ICONS: &[(&str, &str)] = &[
    ("unicode", "兼容图标"),
    ("nerd", "Nerd Font 图标（需要终端字体支持）"),
];
const BORDERS: &[(&str, &str)] = &[("rounded", "圆角"), ("square", "直角"), ("none", "无边框")];
macro_rules! field {
    ($s:literal,$k:literal,$g:literal,$l:literal,$h:literal,$t:expr) => {
        Field {
            section: $s,
            key: $k,
            group: $g,
            label: $l,
            help: $h,
            kind: $t,
        }
    };
}
const FIELDS: &[Field] = &[
    field!(
        "ui",
        "theme",
        "外观",
        "主题",
        "改变菜单整体配色；自定义颜色仍优先使用。",
        Kind::Choice(THEMES)
    ),
    field!(
        "ui",
        "icons",
        "外观",
        "显示图标",
        "在候选项前显示命令、文件和文件夹图标。",
        Kind::Toggle
    ),
    field!(
        "ui",
        "icon_style",
        "外观",
        "图标风格",
        "Nerd Font 需要自行配置终端字体；看不到图标时请选择兼容图标。",
        Kind::Choice(ICONS)
    ),
    field!(
        "ui",
        "max_rows",
        "外观",
        "菜单行数",
        "菜单最多显示多少行候选项。",
        Kind::Number(1, 100)
    ),
    field!(
        "ui",
        "width",
        "外观",
        "菜单宽度",
        "以终端字符列计算；0 表示自动适应窗口。",
        Kind::Number(0, 512)
    ),
    field!(
        "ui",
        "border",
        "外观",
        "边框",
        "选择菜单边框样式。",
        Kind::Choice(BORDERS)
    ),
    field!(
        "ui",
        "descriptions",
        "外观",
        "显示说明",
        "显示候选命令或参数的用途说明。",
        Kind::Toggle
    ),
    field!(
        "ui",
        "status_bar",
        "外观",
        "显示状态栏",
        "在菜单底部显示状态和操作提示。",
        Kind::Toggle
    ),
    field!(
        "completion",
        "auto_trigger",
        "补全",
        "自动显示补全",
        "输入命令时自动显示候选；关闭后使用补全快捷键。",
        Kind::Toggle
    ),
    field!(
        "completion",
        "fuzzy",
        "补全",
        "模糊匹配",
        "允许使用不完整的名称寻找候选。",
        Kind::Toggle
    ),
    field!(
        "completion",
        "dynamic",
        "补全",
        "动态候选",
        "根据当前目录和工具上下文提供候选。",
        Kind::Toggle
    ),
    field!(
        "completion",
        "max_results",
        "补全",
        "候选数量",
        "一次查询最多保留的候选数量。",
        Kind::Number(1, 1000)
    ),
    field!(
        "completion",
        "append_space",
        "补全",
        "接受后添加空格",
        "接受适用的候选后添加空格，方便继续输入。",
        Kind::Toggle
    ),
    field!(
        "completion",
        "up_arrow_history",
        "补全",
        "上箭头浏览历史",
        "保留上箭头调用 PowerShell 历史的习惯。",
        Kind::Toggle
    ),
    field!(
        "keys",
        "trigger",
        "快捷键",
        "显示补全",
        "按 Enter 后直接按下希望使用的组合键。",
        Kind::Key
    ),
    field!(
        "keys",
        "native",
        "快捷键",
        "原生补全",
        "手动调用 PowerShell 原生补全。",
        Kind::Key
    ),
    field!(
        "keys",
        "search",
        "快捷键",
        "用途搜索",
        "按中文用途搜索当前可用的命令。",
        Kind::Key
    ),
    field!(
        "keys",
        "details",
        "快捷键",
        "查看详情",
        "查看选中候选的完整说明和示例。",
        Kind::Key
    ),
    field!(
        "keys",
        "refresh",
        "快捷键",
        "刷新命令",
        "重新读取当前会话的可用命令。",
        Kind::Key
    ),
    field!(
        "keys",
        "reload",
        "快捷键",
        "重载配置",
        "重新读取配置与规格。",
        Kind::Key
    ),
    field!(
        "keys", "resources", "快捷键", "读取远程资源",
        "在 Docker、Kubernetes 或 Helm 上下文中主动读取远程候选。", Kind::Key
    ),
    field!(
        "keys", "hub", "快捷键", "打开命令工作台",
        "打开收藏、模板、历史和工具管理入口。", Kind::Key
    ),
    field!(
        "help",
        "enabled",
        "学习",
        "工具帮助后台学习",
        "为识别的本机工具读取帮助，补充命令和参数。",
        Kind::Toggle
    ),
    field!(
        "learning",
        "enabled",
        "学习",
        "记录选择习惯",
        "使用本地选择次数调整候选排序。",
        Kind::Toggle
    ),
    field!(
        "resources", "local_automatic", "资源", "自动读取本机资源",
        "自动读取项目清单、本机环境和本机 Docker 上下文。", Kind::Toggle
    ),
    field!(
        "resources", "remote_on_demand", "资源", "远程资源按需读取",
        "远程 Docker、Kubernetes 和 Helm 数据只在主动触发后读取。", Kind::Toggle
    ),
    field!(
        "resources", "cache_seconds", "资源", "资源缓存秒数",
        "本机程序查询结果保留的秒数。", Kind::Number(1, 86400)
    ),
    field!(
        "workbench", "history_limit", "工作台", "历史读取数量",
        "历史选择器最多读取多少条 PSReadLine 历史。", Kind::Number(1, 20000)
    ),
];
const ACTIONS: &[&str] = &[
    "保存",
    "恢复当前项默认值",
    "恢复主题默认配色",
    "预览",
    "打开 TOML",
    "重新加载",
    "退出",
];
const COLORS: &[&str] = &[
    "foreground",
    "background",
    "selected_foreground",
    "selected_background",
    "border_color",
    "description_color",
    "match_color",
];

#[derive(Clone, Copy)]
enum Next {
    Exit,
    Raw,
    Reload,
}
enum Mode {
    Form,
    Search(String),
    Diff,
    Number(String, bool),
    Key,
    Preview,
    Confirm(Next, usize),
}
struct Editor {
    path: PathBuf,
    original: Option<Vec<u8>>,
    baseline: Vec<u8>,
    doc: DocumentMut,
    settings: Config,
    load_error: Option<String>,
    validation: Option<String>,
    notice: String,
    selected: usize,
    last_field: usize,
    scroll: usize,
    mode: Mode,
    hits: Vec<(u16, u16, u16, usize)>,
    bom: bool,
    filter: String,
    modified_only: bool,
    key_conflicts: BTreeMap<String, String>,
}

impl Editor {
    fn open(path: PathBuf) -> Self {
        let mut editor = Self {
            path,
            original: None,
            baseline: Vec::new(),
            doc: DocumentMut::new(),
            settings: Config::default(),
            load_error: None,
            validation: None,
            notice: String::new(),
            selected: 0,
            last_field: 0,
            scroll: 0,
            mode: Mode::Form,
            hits: Vec::new(),
            bom: false,
            filter: String::new(),
            modified_only: false,
            key_conflicts: BTreeMap::new(),
        };
        let result = (|| -> Result<()> {
            editor.original = match fs::read(&editor.path) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            if let Some(bytes) = &editor.original {
                let text = std::str::from_utf8(bytes).context("配置文件需要使用 UTF-8 编码")?;
                editor.bom = text.starts_with('\u{feff}');
                editor.doc = text
                    .trim_start_matches('\u{feff}')
                    .parse()
                    .context("TOML 解析失败")?;
            } else {
                editor.doc = config::example()
                    .replace("icon_style = \"nerd\"", "icon_style = \"unicode\"")
                    .parse()?;
                editor.notice = "配置尚未创建；点击保存后写入默认配置。".into();
            }
            editor.refresh()?;
            Ok(())
        })();
        if let Err(e) = result {
            editor.load_error = Some(format!("{e:#}"));
        }
        editor.baseline = editor.bytes();
        if let Some(session)=env::var_os("BLUEBERRY_SESSION_DIR") {
            let adapter=PathBuf::from(session).join("adapter.json");
            if let Ok(bytes)=fs::read(adapter) && let Ok(value)=serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(keys)=value.pointer("/capabilities/public_keys").and_then(serde_json::Value::as_object) {
                    for (name,available) in keys { if available==false { editor.key_conflicts.insert(name.clone(),"当前 PSReadLine 已占用此快捷键".into()); } }
                }
            }
        } else if editor.notice.is_empty() { editor.notice="未连接 Blueberry 会话，PSReadLine 快捷键冲突尚未检查。".into(); }
        editor
    }

    fn bytes(&self) -> Vec<u8> {
        let mut text = self.doc.to_string();
        if self.bom {
            text.insert(0, '\u{feff}');
        }
        text.into_bytes()
    }
    fn dirty(&self) -> bool {
        self.load_error.is_none() && self.baseline != self.bytes()
    }
    fn refresh(&mut self) -> Result<()> {
        let text = self.doc.to_string();
        let mut settings: Config = toml::from_str(&text).context("无法读取配置字段")?;
        let raw: toml::Value = toml::from_str(&text)?;
        settings.apply_theme(raw.get("ui").and_then(toml::Value::as_table));
        self.validation = settings.validate().err().map(|e| e.to_string());
        self.settings = settings;
        Ok(())
    }
    fn current(&self, index: usize) -> toml::Value {
        let field = &FIELDS[index];
        toml::Value::try_from(&self.settings).unwrap()[field.section][field.key].clone()
    }
    fn display(&self, index: usize) -> String {
        let v = self.current(index);
        match FIELDS[index].kind {
            Kind::Toggle => if v.as_bool() == Some(true) {
                "开启"
            } else {
                "关闭"
            }
            .into(),
            Kind::Choice(choices) => choices
                .iter()
                .find(|(key, _)| Some(*key) == v.as_str())
                .map(|(_, label)| (*label).to_owned())
                .unwrap_or_else(|| v.to_string()),
            Kind::Number(..) if FIELDS[index].key == "width" && v.as_integer() == Some(0) => {
                "自动适应窗口".into()
            }
            Kind::Key => v.as_str().unwrap_or("").to_owned(),
            _ => v.to_string(),
        }
    }
    fn set(&mut self, item: Item) -> Result<()> {
        let f = &FIELDS[self.selected];
        // Retain inline comments attached to the original value.
        let mut item = item;
        if let Some(old) = self
            .doc
            .get(f.section)
            .and_then(|s| s.get(f.key))
            .and_then(Item::as_value)
            && let Some(new) = item.as_value_mut()
        {
            *new.decor_mut() = old.decor().clone();
        }
        self.doc[f.section][f.key] = item;
        self.notice.clear();
        self.refresh()
    }
    fn set_named(&mut self, section: &str, key: &str, item: Item) -> Result<()> {
        self.doc[section][key] = item;
        self.refresh()
    }
    fn apply_preset(&mut self, preset: u8) -> Result<()> {
        match preset {
            1 => {
                let defaults = Config::default();
                self.set_named("completion", "auto_trigger", value(defaults.completion.auto_trigger))?;
                self.set_named("ui", "max_rows", value(defaults.ui.max_rows as i64))?;
                self.set_named("ui", "descriptions", value(defaults.ui.descriptions))?;
                self.set_named("ui", "status_bar", value(defaults.ui.status_bar))?;
            }
            2 => {
                self.set_named("completion", "auto_trigger", value(true))?;
                self.set_named("ui", "max_rows", value(5))?;
                self.set_named("ui", "descriptions", value(false))?;
                self.set_named("ui", "status_bar", value(false))?;
            }
            _ => self.set_named("completion", "auto_trigger", value(false))?,
        }
        self.notice = format!("已应用{}预设；Ctrl+D 查看差异，Ctrl+S 保存。", ["", "默认", "精简", "手动触发"][preset as usize]);
        Ok(())
    }
    fn field_visible(&self, index: usize) -> bool {
        let field=&FIELDS[index];
        let matches=self.filter.is_empty() || format!("{} {} {}.{} {}",field.group,field.label,field.section,field.key,field.help).to_lowercase().contains(&self.filter.to_lowercase());
        if !matches { return false; }
        if !self.modified_only { return true; }
        let defaults=toml::Value::try_from(Config::default()).unwrap();
        self.current(index) != defaults[field.section][field.key]
    }
    fn move_field(&mut self, delta:i32) {
        if self.selected>=FIELDS.len(){self.selected=self.last_field;}
        for _ in 0..FIELDS.len(){self.selected=(self.selected as i32+delta).rem_euclid(FIELDS.len() as i32) as usize;if self.field_visible(self.selected){self.last_field=self.selected;break;}}
    }
    fn save(&mut self) -> Result<()> {
        if let Some(e) = &self.load_error {
            bail!("请先在 TOML 中修正文件：{e}");
        }
        self.settings.validate().context("请修正配置后再保存")?;
        let disk = match fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if disk != self.original {
            bail!("文件已被其他程序修改，请选择“重新加载”后再编辑。");
        }
        let bytes = self.bytes();
        if disk.as_ref() != Some(&bytes) {
            write_document(&self.path, &bytes)?;
        }
        self.original = Some(bytes);
        self.baseline = self.bytes();
        self.notice = "已保存；正在运行的 Blueberry 会通过配置热重载应用修改。".into();
        Ok(())
    }
    fn request(&mut self, next: Next) -> Option<Next> {
        if self.dirty() {
            self.mode = Mode::Confirm(next, 2);
            None
        } else {
            Some(next)
        }
    }
    fn activate(&mut self, delta: i64) -> Result<Option<Next>> {
        if self.selected >= FIELDS.len() {
            match self.selected - FIELDS.len() {
                0 => self.save()?,
                1 => {
                    if self.load_error.is_some() {
                        bail!("请先修复 TOML 文件。");
                    }
                    let f = &FIELDS[self.last_field];
                    if let Some(table) = self
                        .doc
                        .get_mut(f.section)
                        .and_then(Item::as_table_like_mut)
                    {
                        table.remove(f.key);
                    }
                    self.refresh()?;
                }
                2 => {
                    if self.load_error.is_some() {
                        bail!("请先修复 TOML 文件。");
                    }
                    if let Some(table) = self.doc.get_mut("ui").and_then(Item::as_table_like_mut) {
                        for color in COLORS {
                            table.remove(color);
                        }
                    }
                    self.refresh()?;
                }
                3 => self.mode = Mode::Preview,
                4 => return Ok(self.request(Next::Raw)),
                5 => return Ok(self.request(Next::Reload)),
                _ => return Ok(self.request(Next::Exit)),
            }
        } else {
            if self.load_error.is_some() {
                bail!("请使用“打开 TOML”修复文件，然后重新加载。");
            }
            match FIELDS[self.selected].kind {
                Kind::Toggle => self.set(value(
                    !self.current(self.selected).as_bool().unwrap_or(false),
                ))?,
                Kind::Choice(choices) => {
                    let current = self.current(self.selected);
                    let index = choices
                        .iter()
                        .position(|(key, _)| Some(*key) == current.as_str())
                        .unwrap_or(0);
                    self.set(value(
                        choices[(index as i64 + delta).rem_euclid(choices.len() as i64) as usize].0,
                    ))?;
                }
                Kind::Number(..) => {
                    self.mode = Mode::Number(self.current(self.selected).to_string(), true)
                }
                Kind::Key => self.mode = Mode::Key,
            }
        }
        Ok(None)
    }
    fn select(&mut self, index: usize) {
        self.selected = index;
        if index < FIELDS.len() {
            self.last_field = index;
        }
    }
    fn handle(&mut self, event: Event) -> Result<Option<Next>> {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                return Ok(None);
            }
            if let Mode::Confirm(next, choice) = &mut self.mode {
                if key.code == KeyCode::Esc {
                    self.mode = Mode::Form;
                    return Ok(None);
                }
                match key.code {
                    KeyCode::Left | KeyCode::Up | KeyCode::BackTab => *choice = (*choice + 2) % 3,
                    KeyCode::Right | KeyCode::Down | KeyCode::Tab => *choice = (*choice + 1) % 3,
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        let (next, choice) = (*next, *choice);
                        self.mode = Mode::Form;
                        if choice == 0 {
                            self.save()?;
                            return Ok(Some(next));
                        }
                        if choice == 1 {
                            return Ok(Some(next));
                        }
                    }
                    _ => {}
                }
                return Ok(None);
            }
            if let Mode::Number(text, fresh) = &mut self.mode {
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Form,
                    KeyCode::Backspace => {
                        text.pop();
                        *fresh = false;
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() && (*fresh || text.len() < 8) => {
                        if *fresh {
                            text.clear();
                        }
                        *fresh = false;
                        text.push(c);
                    }
                    KeyCode::Enter => {
                        let Kind::Number(min, max) = FIELDS[self.selected].kind else {
                            unreachable!()
                        };
                        let n = text
                            .parse::<i64>()
                            .ok()
                            .filter(|n| (min..=max).contains(n))
                            .with_context(|| format!("请输入 {min} 到 {max} 之间的整数。"))?;
                        self.set(value(n))?;
                        self.mode = Mode::Form;
                    }
                    _ => {}
                }
                return Ok(None);
            }
            if matches!(self.mode, Mode::Key) {
                if key.code == KeyCode::Esc {
                    self.mode = Mode::Form;
                } else {
                    let chord = chord(key)?;
                    crate::input::parse_chord(&chord).map_err(anyhow::Error::msg)?;
                    self.set(value(chord))?;
                    self.mode = Mode::Form;
                }
                return Ok(None);
            }
            if matches!(self.mode, Mode::Preview) {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                    self.mode = Mode::Form;
                }
                return Ok(None);
            }
            if let Mode::Search(text) = &mut self.mode {
                match key.code {
                    KeyCode::Esc => { text.clear(); self.filter.clear(); self.mode=Mode::Form; }
                    KeyCode::Enter => { self.filter=text.clone(); self.mode=Mode::Form; if !self.field_visible(self.selected){self.move_field(1);} }
                    KeyCode::Backspace => { text.pop(); }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => text.push(c),
                    _ => {}
                }
                return Ok(None);
            }
            if matches!(self.mode, Mode::Diff) {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter) { self.mode=Mode::Form; }
                return Ok(None);
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
                self.save()?;
                return Ok(None);
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('m') => { self.modified_only=!self.modified_only; if !self.field_visible(self.selected){self.move_field(1);} return Ok(None); }
                    KeyCode::Char('d') => { self.mode=Mode::Diff; return Ok(None); }
                    KeyCode::Char('1') => { self.apply_preset(1)?; return Ok(None); }
                    KeyCode::Char('2') => { self.apply_preset(2)?; return Ok(None); }
                    KeyCode::Char('3') => { self.apply_preset(3)?; return Ok(None); }
                    _ => {}
                }
            }
            match key.code {
                KeyCode::Char('/') => { self.mode=Mode::Search(self.filter.clone()); }
                KeyCode::Esc => return Ok(self.request(Next::Exit)),
                KeyCode::Down | KeyCode::Tab if self.selected<FIELDS.len() => self.move_field(1),
                KeyCode::Up | KeyCode::BackTab if self.selected<FIELDS.len() => self.move_field(-1),
                KeyCode::Down | KeyCode::Tab => self.select((self.selected + 1) % (FIELDS.len() + ACTIONS.len())),
                KeyCode::Up | KeyCode::BackTab => self.select((self.selected + FIELDS.len() + ACTIONS.len() - 1) % (FIELDS.len() + ACTIONS.len())),
                KeyCode::Left => return self.activate(-1),
                KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => return self.activate(1),
                KeyCode::Home => self.select(0),
                KeyCode::End => self.select(FIELDS.len()),
                _ => {}
            }
        } else if let Event::Mouse(mouse) = event {
            if !matches!(self.mode, Mode::Form) {
                if let Mode::Confirm(next, _) = self.mode
                    && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                    && let Some((_, _, _, choice)) = self.hits.iter().find(|(y, x, w, _)| {
                        *y == mouse.row && (*x..x.saturating_add(*w)).contains(&mouse.column)
                    })
                {
                    let choice = *choice;
                    self.mode = Mode::Form;
                    if choice == 0 {
                        self.save()?;
                        return Ok(Some(next));
                    }
                    if choice == 1 {
                        return Ok(Some(next));
                    }
                }
                return Ok(None);
            }
            match mouse.kind {
                MouseEventKind::ScrollDown => {
                    self.select((self.selected + 1).min(FIELDS.len() - 1))
                }
                MouseEventKind::ScrollUp => self.select(self.selected.saturating_sub(1)),
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some((_, _, _, index)) = self.hits.iter().find(|(y, x, w, _)| {
                        *y == mouse.row && (*x..x.saturating_add(*w)).contains(&mouse.column)
                    }) {
                        let index = *index;
                        self.select(index);
                        return self.activate(1);
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn draw(&mut self, out: &mut impl Write) -> Result<()> {
        let (cols, rows) = terminal::size()?;
        execute!(out, MoveTo(0, 0), Clear(ClearType::All))?;
        self.hits.clear();
        if cols < 45 || rows < 16 {
            line(
                out,
                0,
                0,
                cols,
                "请将窗口扩大至至少 45 列、16 行；Esc 可退出。",
                false,
            )?;
            if matches!(self.mode, Mode::Confirm(..)) {
                line(
                    out,
                    0,
                    2,
                    cols,
                    "未保存：← → 选择，Enter 确认，Esc 返回",
                    false,
                )?;
                if let Mode::Confirm(_, choice) = self.mode {
                    line(
                        out,
                        0,
                        3,
                        cols,
                        ["保存并退出", "放弃修改", "继续编辑"][choice],
                        true,
                    )?;
                }
            }
            out.flush()?;
            return Ok(());
        }
        line(
            out,
            0,
            0,
            cols,
            &format!(
                "Blueberry 设置{}",
                if self.dirty() { " · 未保存" } else { "" }
            ),
            true,
        )?;
        line(out, 0, 1, cols, &self.path.display().to_string(), false)?;
        let error = self.load_error.as_ref().or(self.validation.as_ref());
        line(
            out,
            0,
            2,
            cols,
            if self.notice.is_empty() {
                error.map(String::as_str).unwrap_or("")
            } else {
                &self.notice
            },
            false,
        )?;
        if let Mode::Confirm(_, choice) = self.mode {
            line(out, 2, 5, cols - 4, "有未保存的修改，请选择：", false)?;
            for (i, label) in ["保存并继续", "放弃修改并继续", "继续编辑"]
                .iter()
                .enumerate()
            {
                line(out, 2, 7 + i as u16, cols - 4, label, i == choice)?;
                self.hits.push((7 + i as u16, 2, cols - 4, i));
            }
        } else if matches!(self.mode, Mode::Preview) {
            self.preview(out, 0, 4, cols, rows - 6)?;
            line(out, 0, rows - 1, cols, "Esc / Enter 返回设置", false)?;
        } else if matches!(self.mode, Mode::Diff) {
            line(out,0,4,cols,"未保存修改（当前值 → 默认值）",true)?;
            let defaults=toml::Value::try_from(Config::default()).unwrap();
            let mut row=6;
            for (index,field) in FIELDS.iter().enumerate(){let current=self.current(index);let default=&defaults[field.section][field.key];if current!=*default && row<rows-2 {line(out,2,row,cols-4,&format!("{}.{}: {} → {}",field.section,field.key,current,default),false)?;row+=1;}}
            line(out,0,rows-1,cols,"Esc / Enter 返回设置",false)?;
        } else {
            let width = if cols >= 100 { cols / 2 } else { cols };
            let visible = usize::from(rows - 11);
            if self.selected < FIELDS.len() {
                if self.selected < self.scroll {
                    self.scroll = self.selected;
                }
                if self.selected >= self.scroll + visible {
                    self.scroll = self.selected + 1 - visible;
                }
            }
            if let Some(error) = &self.load_error {
                for (offset, text) in error.lines().take(visible).enumerate() {
                    line(out, 0, 4 + offset as u16, width - 1, text, false)?;
                }
            } else {
                let visible_fields=(self.scroll..FIELDS.len()).filter(|index|self.field_visible(*index)).take(visible).collect::<Vec<_>>();
                for (offset, index) in visible_fields.into_iter().enumerate() {
                    let f = &FIELDS[index];
                    let key = format!("{}.{}", f.section, f.key);
                    let invalid = self.validation.as_ref().is_some_and(|e| e.contains(&key));
                    let conflict = f.section=="keys" && self.key_conflicts.contains_key(f.key);
                    let label = format!(
                        "{} · {}：{}{}",
                        f.group,
                        f.label,
                        self.display(index),
                        if invalid || conflict { "  [需修改]" } else { "" }
                    );
                    line(
                        out,
                        0,
                        4 + offset as u16,
                        width - 1,
                        &label,
                        self.selected == index,
                    )?;
                    self.hits.push((4 + offset as u16, 0, width - 1, index));
                }
            }
            if cols >= 100 {
                self.preview(out, width + 1, 4, cols - width - 1, rows - 11)?;
            }
            let f = &FIELDS[self.last_field];
            let help = match &self.mode {
                Mode::Number(text, _) => {
                    format!("输入 {}：{}  （Enter 确认，Esc 取消）", f.label, text)
                }
                Mode::Key => format!("请按下“{}”的组合键；Esc 取消。", f.label),
                _ => {
                    let key = format!("{}.{}", f.section, f.key);
                    if let Some(error) = self.validation.as_ref().filter(|e| e.contains(&key)) {
                        format!("{} · {error}", f.label)
                    } else {
                        format!("{} · {}", f.label, f.help)
                    }
                }
            };
            let search=match &self.mode {Mode::Search(text)=>format!("搜索：{text}_"),_=>format!("筛选：{}{} · / 搜索 · Ctrl+M 仅修改 · Ctrl+1/2/3 预设 · Ctrl+D 差异",if self.filter.is_empty(){"全部"}else{&self.filter},if self.modified_only{" · 仅修改"}else{""})};
            line(out,0,rows-8,cols,&search,false)?;
            line(out, 0, rows - 7, cols, &help, false)?;
            for (i, label) in ACTIONS.iter().enumerate() {
                // Three short rows stay usable in a narrow terminal.
                let (row, col, span) = if i < 2 {
                    (rows - 5, i as u16 * (cols / 2), cols / 2)
                } else if i < 4 {
                    (rows - 4, (i - 2) as u16 * (cols / 2), cols / 2)
                } else {
                    (rows - 3, (i - 4) as u16 * (cols / 3), cols / 3)
                };
                line(
                    out,
                    col,
                    row,
                    span,
                    &format!("[ {label} ]"),
                    self.selected == FIELDS.len() + i,
                )?;
                self.hits.push((row, col, span, FIELDS.len() + i));
            }
            line(
                out,
                0,
                rows - 1,
                cols,
                "↑↓/Tab 选择 · Enter 修改 · Ctrl+S 保存 · Esc 返回",
                false,
            )?;
        }
        out.flush()?;
        Ok(())
    }
    fn preview(&self, out: &mut impl Write, x: u16, y: u16, width: u16, height: u16) -> Result<()> {
        line(out, x, y, width, "效果预览（示例数据）", false)?;
        let candidates = [
            ("Get-ChildItem", "列出目录中的项目", CandidateKind::Cmdlet),
            ("Get-Content", "读取文件内容", CandidateKind::Cmdlet),
            ("Get-Command", "查找可用命令", CandidateKind::Cmdlet),
        ]
        .into_iter()
        .map(|(label, description, kind)| Candidate {
            label: label.into(),
            description: description.into(),
            kind,
            ..Default::default()
        })
        .collect::<Vec<_>>();
        let mut config = self.settings.clone();
        // Invalid existing numeric settings must not affect rendering bounds.
        config.ui.max_rows = config.ui.max_rows.clamp(1, 100);
        config.ui.width = config.ui.width.min(512);
        let frame = crate::menu::render_with_state(
            &candidates,
            0,
            "Get-",
            width,
            &config,
            crate::menu::MenuState::default(),
            height.saturating_sub(2) as usize,
        );
        for (i, text) in frame
            .lines
            .iter()
            .take(height.saturating_sub(2) as usize)
            .enumerate()
        {
            execute!(out, MoveTo(x, y + 1 + i as u16))?;
            write!(out, "{text}\x1b[0m")?;
        }
        if self
            .doc
            .get("ui")
            .is_some_and(|ui| COLORS.iter().any(|key| ui.get(key).is_some()))
        {
            line(
                out,
                x,
                y + height.saturating_sub(1),
                width,
                "自定义颜色正在覆盖部分主题配色",
                false,
            )?;
        }
        Ok(())
    }
}

fn chord(key: KeyEvent) -> Result<String> {
    let mut parts = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl".to_owned());
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("Alt".to_owned());
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("Shift".to_owned());
    }
    let name = match key.code {
        KeyCode::Char(' ') => "Space".into(),
        KeyCode::Char(c) => c.to_uppercase().to_string(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => {
            if !parts.iter().any(|p| p == "Shift") {
                parts.push("Shift".into());
            }
            "Tab".into()
        }
        KeyCode::Enter => "Enter".into(),
        KeyCode::F(n) => format!("F{n}"),
        _ => bail!("请选择字母、数字、空格、Tab、Enter 或 F1–F12，可组合 Ctrl / Alt / Shift。"),
    };
    parts.push(name);
    Ok(parts.join("+"))
}

fn line(
    out: &mut impl Write,
    x: u16,
    y: u16,
    width: u16,
    text: &str,
    selected: bool,
) -> Result<()> {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    execute!(out, MoveTo(x, y))?;
    if selected {
        write!(out, "\x1b[7m")?;
    }
    let mut used = 0;
    for segment in text.graphemes(true) {
        if segment.chars().any(char::is_control) {
            continue;
        }
        let n = segment.width();
        if used + n > width.saturating_sub(1) as usize {
            break;
        }
        write!(out, "{segment}")?;
        used += n;
    }
    write!(out, "\x1b[0m")?;
    Ok(())
}

fn write_document(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("配置文件没有父目录")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".blueberry-config-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
            };
            let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe {
                MoveFileExW(
                    from.as_ptr(),
                    to.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(io::Error::last_os_error().into());
            }
        }
        #[cfg(not(windows))]
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("无法保存 {}", path.display()))
}

struct Screen {
    #[cfg(windows)]
    output_mode: u32,
}
impl Screen {
    fn enter() -> Result<Self> {
        let screen = {
            #[cfg(windows)]
            {
                use windows_sys::Win32::System::Console::*;
                let mut output_mode = 0;
                unsafe {
                    let output = GetStdHandle(STD_OUTPUT_HANDLE);
                    if GetConsoleMode(output, &mut output_mode) == 0 {
                        return Err(io::Error::last_os_error().into());
                    }
                    if SetConsoleMode(output, output_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) == 0
                    {
                        return Err(io::Error::last_os_error().into());
                    }
                }
                Self { output_mode }
            }
            #[cfg(not(windows))]
            Self {}
        };
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(screen)
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = execute!(
            out,
            crossterm::event::DisableMouseCapture,
            Show,
            LeaveAlternateScreen
        );
        let _ = out.flush();
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Console::*;
            SetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), self.output_mode);
        }
    }
}

/// The first interactive settings release targets Windows Terminal.
pub fn run(path: Option<&Path>) -> Result<u32> {
    if !cfg!(windows) {
        bail!("交互设置页目前支持 Windows；可使用 config init 和 config check 管理 TOML。");
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("请在交互终端中运行 blueberry config edit。");
    }
    let path = path
        .map(Path::to_path_buf)
        .unwrap_or_else(config::default_path);
    let path = std::path::absolute(path)?;
    let mut editor = Editor::open(path.clone());
    loop {
        let next = {
            let _screen = Screen::enter()?;
            let mut raw = crate::host::RawMode::enable()?;
            raw.mouse(true)?;
            let mut out = io::stdout();
            write!(out, "\x1b[?1000h\x1b[?1006h")?;
            #[cfg(windows)]
            let mut reader = crate::windows_input::Reader::new()?;
            let mut redraw = true;
            'page: loop {
                if redraw {
                    let mut frame = Vec::new();
                    editor.draw(&mut frame)?;
                    out.write_all(&frame)?;
                    out.flush()?;
                }
                #[cfg(windows)]
                let events = reader.read_batch(64)?;
                #[cfg(not(windows))]
                let events = vec![crossterm::event::read()?];
                redraw = false;
                #[cfg(windows)]
                if reader.take_paste_rejection().is_some() {
                    editor.notice = "粘贴内容过长，请使用按键编辑当前字段。".into();
                    redraw = true;
                }
                for event in events {
                    if matches!(
                        event,
                        Event::Key(KeyEvent {
                            kind: KeyEventKind::Release,
                            ..
                        }) | Event::Mouse(crossterm::event::MouseEvent {
                            kind: MouseEventKind::Moved | MouseEventKind::Up(_),
                            ..
                        })
                    ) {
                        continue;
                    }
                    redraw = true;
                    match editor.handle(event) {
                        Ok(Some(next)) => break 'page next,
                        Ok(None) => {}
                        Err(e) => editor.notice = format!("{e:#}"),
                    }
                }
            }
        };
        match next {
            Next::Exit => return Ok(0),
            Next::Raw => {
                let result = std::process::Command::new("notepad.exe")
                    .arg(&path)
                    .spawn()
                    .and_then(|mut child| child.wait());
                editor = Editor::open(path.clone());
                editor.notice = match result {
                    Ok(_) => "已返回设置页；记事本仍在编辑时，保存后选择“重新加载”。".into(),
                    Err(e) => format!("无法打开记事本：{e}"),
                };
            }
            Next::Reload => editor = Editor::open(path.clone()),
        }
    }
}

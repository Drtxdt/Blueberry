//! Declarative project commands. Repository files are inert until the exact
//! contents have been reviewed and approved in the user's own config area.
use crate::{
    config, engine,
    hub::{CommandTemplate, TemplateForm, TemplateParameter},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock, RwLock},
    thread,
    time::Duration,
};

const MAX_PACK_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackFile {
    schema_version: u32,
    pack: PackMeta,
    #[serde(default)]
    operations: Vec<PackOperation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackMeta {
    id: String,
    name: String,
    version: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackOperation {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tags: Vec<String>,
    tokens: Vec<String>,
    #[serde(default)]
    parameters: Vec<PackParameter>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackParameter {
    name: String,
    #[serde(default)]
    label: String,
    #[serde(default = "text_kind")]
    kind: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    default: String,
    #[serde(default)]
    values: Vec<String>,
    #[serde(default)]
    source: String,
}
fn text_kind() -> String {
    "text".into()
}
impl PackOperation {
    fn template(&self, pack: &PackMeta) -> CommandTemplate {
        CommandTemplate {
            id: format!("{}:{}", pack.id, self.id),
            name: self.name.clone(),
            tokens: self.tokens.clone(),
            description: self.description.clone(),
            tags: self.tags.clone(),
            parameters: self
                .parameters
                .iter()
                .map(|p| TemplateParameter {
                    name: p.name.clone(),
                    label: p.label.clone(),
                    kind: p.kind.clone(),
                    required: p.required,
                    default: p.default.clone(),
                    values: p.values.clone(),
                    source: p.source.clone(),
                })
                .collect(),
            ..Default::default()
        }
    }
}
impl PackFile {
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!("项目命令包 schema_version 必须为 1");
        }
        if self.pack.id.trim().is_empty()
            || self.pack.name.trim().is_empty()
            || self.pack.version.trim().is_empty()
        {
            bail!("项目命令包必须提供 id、name 和 version");
        }
        let mut ids = HashSet::new();
        for operation in &self.operations {
            if operation.id.trim().is_empty()
                || operation.name.trim().is_empty()
                || operation.tokens.is_empty()
                || !ids.insert(&operation.id)
            {
                bail!("项目操作需要非空且唯一的 id、name 和 tokens");
            }
            let names = operation
                .parameters
                .iter()
                .map(|p| p.name.as_str())
                .collect::<HashSet<_>>();
            if names.len() != operation.parameters.len() {
                bail!("操作 {} 的参数名称重复", operation.id);
            }
            for token in &operation.tokens {
                if let Some(name) = token
                    .strip_prefix('{')
                    .and_then(|value| value.strip_suffix('}'))
                {
                    if !names.contains(name) {
                        bail!("操作 {} 的字段 {name} 未定义", operation.id);
                    }
                } else if token.is_empty()
                    || token.chars().any(|ch| {
                        ch.is_control() || ch.is_whitespace() || ";|&<>`$\"'{}".contains(ch)
                    })
                {
                    bail!("操作 {} 含有不允许的命令 token", operation.id);
                }
            }
            let template = operation.template(&self.pack);
            let _ = TemplateForm::new(template)?;
            for parameter in &operation.parameters {
                if !parameter.source.is_empty()
                    && !matches!(
                        parameter.source.as_str(),
                        "cargo.packages"
                            | "cargo.features"
                            | "cargo.tests"
                            | "docker.compose.service"
                    )
                {
                    bail!(
                        "操作 {} 的参数 {} 使用了不支持的数据源",
                        operation.id,
                        parameter.name
                    );
                }
                if parameter.kind == "enum"
                    && (!parameter.default.is_empty()
                        && !parameter.values.contains(&parameter.default))
                {
                    bail!(
                        "操作 {} 的参数 {} 默认值不在枚举值中",
                        operation.id,
                        parameter.name
                    );
                }
            }
        }
        Ok(())
    }
    fn templates(&self) -> Vec<CommandTemplate> {
        self.operations
            .iter()
            .map(|operation| operation.template(&self.pack))
            .collect()
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct ApprovalFile {
    schema_version: u32,
    #[serde(default)]
    entries: BTreeMap<String, ApprovedPack>,
}
#[derive(Clone, Deserialize, Serialize)]
struct ApprovedPack {
    digest: String,
    file: PackFile,
}
#[derive(Clone)]
struct LoadedPack {
    root: PathBuf,
    digest: String,
    file: PackFile,
}

pub fn git_root(cwd: &Path) -> Option<PathBuf> {
    let start = cwd.canonicalize().ok()?;
    start
        .ancestors()
        .find(|path| path.join(".git").exists())
        .map(Path::to_path_buf)
}
fn load_pack(root: &Path) -> Result<Option<LoadedPack>> {
    let path = root.join(".blueberry/commands.toml");
    let mut file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("无法读取 {}", path.display())),
    };
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_PACK_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PACK_BYTES {
        bail!("{} 超过 1 MiB 上限", path.display());
    }
    let text =
        std::str::from_utf8(&bytes).with_context(|| format!("{} 不是 UTF-8", path.display()))?;
    let pack: PackFile =
        toml::from_str(text).with_context(|| format!("无法解析 {}", path.display()))?;
    pack.validate()
        .with_context(|| format!("无法校验 {}", path.display()))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    Ok(Some(LoadedPack {
        root: root.to_path_buf(),
        digest,
        file: pack,
    }))
}
fn approvals_at(path: &Path) -> Result<ApprovalFile> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(ApprovalFile::default()),
        Err(error) => return Err(error).with_context(|| format!("无法读取 {}", path.display())),
    };
    let approvals: ApprovalFile =
        serde_json::from_str(&text).with_context(|| format!("无法解析 {}", path.display()))?;
    if approvals.schema_version > 1 {
        bail!("{} 使用不支持的版本", path.display());
    }
    Ok(approvals)
}
fn edit_approvals(path: &Path, edit: impl FnOnce(&mut ApprovalFile) -> Result<bool>) -> Result<()> {
    let parent = path.parent().context("项目审批文件没有父目录")?;
    fs::create_dir_all(parent)?;
    let lock_path = path.with_extension("json.lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    lock.lock()?;
    let original = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let mut approvals = approvals_at(path)?;
    if !edit(&mut approvals)? {
        return Ok(());
    }
    approvals.schema_version = 1;
    let temporary = parent.join(format!(".project-packs-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        output.write_all(&serde_json::to_vec_pretty(&approvals)?)?;
        output.sync_all()?;
        drop(output);
        let current = match fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if current != original {
            bail!("{} 在保存期间被外部修改", path.display());
        }
        engine::replace_file(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn root_key(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}
fn current_at(cwd: &Path, approvals_path: &Path) -> Result<(Vec<CommandTemplate>, Option<String>)> {
    let Some(root) = git_root(cwd) else {
        return Ok((Vec::new(), None));
    };
    let Some(pack) = load_pack(&root)? else {
        return Ok((Vec::new(), None));
    };
    let approvals = approvals_at(approvals_path)?;
    if approvals
        .entries
        .get(&root_key(&root))
        .is_some_and(|approved| approved.digest == pack.digest)
    {
        Ok((pack.file.templates(), None))
    } else {
        Ok((
            Vec::new(),
            Some(format!(
                "发现项目命令包 {} v{}；运行 blueberry packs review 审查",
                pack.file.pack.name, pack.file.pack.version
            )),
        ))
    }
}
pub fn approved_templates(cwd: &Path) -> Result<Vec<CommandTemplate>> {
    Ok(current_at(cwd, &config::project_packs_path())?.0)
}
pub fn verify_approval(cwd: &Path) -> Result<()> {
    verify_approval_at(cwd, &config::project_packs_path())
}
fn verify_approval_at(cwd: &Path, approvals_path: &Path) -> Result<()> {
    let (templates, status) = current_at(cwd, approvals_path)?;
    if let Some(status) = status {
        bail!("{status}");
    }
    if templates.is_empty() {
        bail!("当前仓库没有已批准的项目操作");
    }
    Ok(())
}

#[derive(Default)]
struct CacheState {
    templates: Vec<CommandTemplate>,
    status: Option<String>,
    review: Option<String>,
}
struct PackCache {
    cwd: Mutex<PathBuf>,
    state: RwLock<CacheState>,
    wake: Condvar,
    listener: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}
static CACHE: OnceLock<Arc<PackCache>> = OnceLock::new();
fn cache() -> &'static Arc<PackCache> {
    CACHE.get_or_init(|| {
        let cache = Arc::new(PackCache {
            cwd: Mutex::new(PathBuf::new()),
            state: RwLock::new(CacheState::default()),
            wake: Condvar::new(),
            listener: Mutex::new(None),
        });
        let worker = cache.clone();
        thread::spawn(move || {
            let mut previous = None;
            loop {
                let cwd = worker.cwd.lock().unwrap().clone();
                let (templates, status) = if cwd.as_os_str().is_empty() {
                    (Vec::new(), None)
                } else {
                    match current_at(&cwd, &config::project_packs_path()) {
                        Ok(value) => value,
                        Err(error) => (Vec::new(), Some(format!("项目命令包错误：{error:#}"))),
                    }
                };
                let review = if cwd.as_os_str().is_empty() {
                    None
                } else {
                    review_text_at(&cwd, &config::project_packs_path())
                        .ok()
                        .flatten()
                };
                let key = (
                    cwd.clone(),
                    templates
                        .iter()
                        .map(|t| format!("{}:{}", t.id, t.tokens.join(" ")))
                        .collect::<Vec<_>>(),
                    status.clone(),
                    review.clone(),
                );
                if previous.as_ref() != Some(&key) && *worker.cwd.lock().unwrap() == cwd {
                    *worker.state.write().unwrap() = CacheState {
                        templates,
                        status,
                        review,
                    };
                    if let Some(listener) = worker.listener.lock().unwrap().as_ref() {
                        listener();
                    }
                    previous = Some(key);
                }
                let lock = worker.cwd.lock().unwrap();
                let _ = worker
                    .wake
                    .wait_timeout(lock, Duration::from_millis(750))
                    .unwrap();
            }
        });
        cache
    })
}
pub fn set_cwd(cwd: PathBuf) {
    let cache = cache();
    *cache.cwd.lock().unwrap() = cwd;
    cache.wake.notify_one();
}
pub fn on_change(listener: impl Fn() + Send + Sync + 'static) {
    *cache().listener.lock().unwrap() = Some(Box::new(listener));
}
pub fn cached_templates() -> Vec<CommandTemplate> {
    cache().state.read().unwrap().templates.clone()
}
pub fn cached_status() -> Option<String> {
    cache().state.read().unwrap().status.clone()
}
pub fn cached_review() -> Option<String> {
    cache().state.read().unwrap().review.clone()
}

pub fn list() -> Result<()> {
    let approvals = approvals_at(&config::project_packs_path())?;
    if approvals.entries.is_empty() {
        println!("尚未批准任何项目命令包。");
    }
    for (root, approved) in approvals.entries {
        let status = match load_pack(Path::new(&root)) {
            Ok(Some(pack)) if pack.digest == approved.digest => "已启用".to_owned(),
            Ok(Some(_)) => "内容已变更，待重新审查".to_owned(),
            Ok(None) => "文件已移除".to_owned(),
            Err(error) => format!("校验失败：{error:#}"),
        };
        println!(
            "{}\t{} v{}\t{}\t{}",
            root, approved.file.pack.name, approved.file.pack.version, approved.digest, status
        );
    }
    Ok(())
}
pub fn review() -> Result<()> {
    let cwd = std::env::current_dir()?;
    print!(
        "{}",
        review_text_at(&cwd, &config::project_packs_path())?
            .context("仓库根目录没有 .blueberry/commands.toml")?
    );
    Ok(())
}
fn review_text_at(cwd: &Path, approvals_path: &Path) -> Result<Option<String>> {
    let root = git_root(cwd).context("当前目录不在 Git 仓库中")?;
    let Some(pack) = load_pack(&root)? else {
        return Ok(None);
    };
    let approvals = approvals_at(approvals_path)?;
    let previous = approvals.entries.get(&root_key(&root));
    let mut output = format!(
        "来源：{}\n",
        root.join(".blueberry/commands.toml").display()
    );
    output.push_str(&format!(
        "项目命令包：{} ({}) v{}",
        pack.file.pack.name, pack.file.pack.id, pack.file.pack.version
    ));
    output.push('\n');
    output.push_str(&format!("SHA-256：{}\n", pack.digest));
    output.push_str(&format!(
        "状态：{}",
        if previous.is_some_and(|approved| approved.digest == pack.digest) {
            "已批准"
        } else {
            "待批准"
        }
    ));
    output.push('\n');
    let old = previous
        .map(|approved| {
            approved
                .file
                .operations
                .iter()
                .map(|op| (op.id.as_str(), op))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    for operation in &pack.file.operations {
        let mark = match old.get(operation.id.as_str()) {
            None => '+',
            Some(older) if *older != operation => '~',
            Some(_) => ' ',
        };
        output.push_str(&format!(
            "{mark} {}：{}\n",
            operation.name,
            operation.tokens.join(" ")
        ));
    }
    for operation in old.values().filter(|old| {
        !pack
            .file
            .operations
            .iter()
            .any(|current| current.id == old.id)
    }) {
        output.push_str(&format!(
            "- {}：{}\n",
            operation.name,
            operation.tokens.join(" ")
        ));
    }
    Ok(Some(output))
}
pub fn approve(digest: &str) -> Result<()> {
    let root = git_root(&std::env::current_dir()?).context("当前目录不在 Git 仓库中")?;
    let pack = load_pack(&root)?.context("仓库根目录没有 .blueberry/commands.toml")?;
    if digest != pack.digest {
        bail!("文件内容与审查时的 SHA-256 不一致；请重新运行 packs review");
    }
    edit_approvals(&config::project_packs_path(), |approvals| {
        approvals.entries.insert(
            root_key(&pack.root),
            ApprovedPack {
                digest: pack.digest.clone(),
                file: pack.file.clone(),
            },
        );
        Ok(true)
    })?;
    println!(
        "已批准 {} v{}。",
        pack.file.pack.name, pack.file.pack.version
    );
    Ok(())
}
pub fn revoke() -> Result<()> {
    let root = git_root(&std::env::current_dir()?).context("当前目录不在 Git 仓库中")?;
    edit_approvals(&config::project_packs_path(), |approvals| {
        Ok(approvals.entries.remove(&root_key(&root)).is_some())
    })?;
    println!("已撤销当前仓库的项目命令包批准。");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_is_bound_to_exact_repository_contents() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::create_dir(root.join(".git"))?;
        fs::create_dir(root.join(".blueberry"))?;
        let path = root.join(".blueberry/commands.toml");
        fs::write(
            &path,
            "schema_version = 1\n[pack]\nid = 'team'\nname = 'Team'\nversion = '1'\n[[operations]]\nid = 'test'\nname = 'Test'\ntokens = ['cargo', 'test', '{package}']\n[[operations.parameters]]\nname = 'package'\nrequired = true\n",
        )?;
        let approvals_path = root.join("approvals.json");
        let canonical_root = git_root(root).context("git root")?;
        let pack = load_pack(&canonical_root)?.context("pack is present")?;
        assert!(current_at(root, &approvals_path)?.0.is_empty());
        edit_approvals(&approvals_path, |approvals| {
            approvals.entries.insert(
                root_key(&pack.root),
                ApprovedPack {
                    digest: pack.digest.clone(),
                    file: pack.file.clone(),
                },
            );
            Ok(true)
        })?;
        assert_eq!(current_at(root, &approvals_path)?.0.len(), 1);
        verify_approval_at(root, &approvals_path)?;
        fs::write(
            &path,
            fs::read_to_string(&path)?.replace("version = '1'", "version = '2'"),
        )?;
        let (templates, status) = current_at(root, &approvals_path)?;
        assert!(templates.is_empty());
        assert!(status.is_some());
        assert!(verify_approval_at(root, &approvals_path).is_err());
        Ok(())
    }

    #[test]
    fn scripts_and_shell_operators_are_rejected() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::create_dir(root.join(".blueberry"))?;
        let path = root.join(".blueberry/commands.toml");
        let prefix = "schema_version = 1\n[pack]\nid = 'team'\nname = 'Team'\nversion = '1'\n[[operations]]\nid = 'test'\nname = 'Test'\n";
        fs::write(
            &path,
            format!("{prefix}tokens = ['cargo', 'test']\nscript = 'evil'\n"),
        )?;
        assert!(load_pack(root).is_err());
        fs::write(&path, format!("{prefix}tokens = ['cargo', ';', 'test']\n"))?;
        assert!(load_pack(root).is_err());
        Ok(())
    }
}

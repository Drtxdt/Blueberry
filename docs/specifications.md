# ShellSense 规格目录

ShellSense 的内置命令知识来自 [`specs/builtin.toml`](../specs/builtin.toml)。该文件使用版本 1 的声明式格式，构建时由 `build.rs` 校验并生成 Rust 静态数据。运行时不会解析内置 TOML，也不会执行其中的脚本。用户规格只从用户配置目录读取 `*.toml`，不能引用任意脚本、程序或在线服务。

## 版本 1 格式

文件至少需要声明 `schema_version = 1`。一个规格文件可以定义选项集合、命令上下文和根命令别名：

```toml
schema_version = 1
name = "我的离线规格"
version = "1"

[aliases]
mytool = ["mytool", "mytool.exe"]

[[option_sets]]
id = "mytool.common"

[[option_sets.options]]
name = "--format"
description = "选择输出格式"
detail = "选择输出格式；示例：mytool list --format=json。"
value_kind = "text"
values = [
  { name = "json", description = "输出 JSON 数据" },
  { name = "text", description = "输出纯文本" },
]
repeatable = false
value_delimiter = "="

[[nodes]]
path = "mytool"
description = "运行我的离线工具"
detail = "运行我的离线工具；示例：mytool list。"
positional = "none"
children = ["mytool list"]
option_sets = ["mytool.common"]
examples = ["mytool list --format=json"]

[[nodes]]
path = "mytool list"
description = "列出项目"
detail = "列出项目；示例：mytool list。"
positional = "path"
provider = "powershell.paths"
```

`nodes.path` 是完整的规范上下文，使用空格分隔根命令和子命令。`children` 可以写完整路径，也可以写当前节点下的短名称；完整路径更适合覆盖已有节点。`positional` 可取 `none`、`value` 或 `path`。

选项支持 `value_kind`（`none`、`text`、`path`）、固定 `values`、`optional`、`repeatable`、`conflicts`、`requires`、`positional`、`value_delimiter`、`short_cluster` 和 `append_space`。`optional = true` 的选项只有在输入 `--name=value` 或短选项附加值时才展开固定值候选；裸选项不会吞掉后面的未知 token。未知选项始终只占用自身 token，规格不会猜测它的参数个数。

选项可以用 `names = ["-f", "--format"]` 声明别名；`name` 是候选显示和规格键使用的主拼写。`repeatable = false` 的选项在已出现后会从菜单移除。`conflicts` 中的任一选项已经出现时，该选项会被移除；`requires` 中的选项全部出现后才会显示。`value_delimiter = ","` 可描述 `--features=a,b` 这类值分隔符，`=` 适合长选项的内联值。

## 固定动态数据源

`provider` 只接受内置的只读标识，规格本身不运行任何命令。当前标识包括：

- Git：`git` 是 Git 命令节点使用的通用数据源；它根据当前子命令、选项作用域、位置参数和 `--` 分隔符选择引用、远端、工作树或状态路径。`git.refs`、`git.branches`、`git.remotes`、`git.tags`、`git.worktrees`、`git.status`、`git.paths` 仍作为显式的细分数据源保留。
- Cargo：`cargo.packages`、`cargo.features`、`cargo.bins`、`cargo.examples`、`cargo.tests`、`cargo.benches`
- npm/pnpm：`npm.scripts`、`npm.workspaces`、`npm.dependencies`、`pnpm.scripts`、`pnpm.workspaces`、`pnpm.dependencies`
- PowerShell：`powershell.env`、`powershell.paths`、`powershell.redirects`

内置规格将 `git` 通用数据源挂在适用的 Git 子命令节点上，例如 `add`、`branch`、`checkout`、`diff`、`fetch`、`log`、`merge`、`pull`、`push`、`rebase`、`remote`、`reset`、`restore`、`show`、`status`、`switch`、`tag` 和 `worktree`。Git provider 会在 Rust 中固定调用只读查询；它不会触发 fetch、构建、脚本、依赖安装或凭据提示。

节点的 `provider` 只描述位置参数或该命令节点的动态值。正在等待值的选项，以及 `--name=value` 形式的内联选项，只使用该选项自己的 `option.provider`；不会从节点 provider 继承。因此没有显式 provider 的 `git log --format <值>` 不会启动 Git 动态查询，而标明 `provider = "git.refs"` 的 `--source`、`--onto` 等选项才会请求引用候选。选项 provider 同样只能指向上述固定只读数据源。

## 覆盖与热重载

用户目录中的文件按文件名排序并逐个应用。节点按完整 `path` 精确匹配内置上下文：省略的字段继承原节点，显式的数组字段（`children`、`option_sets`、`options`、`examples`）替换原数组；选项集合本身按 `id` 在当前文件中解析。节点中不能同时使用 `option_sets` 和内联 `options`。新增子节点会自动挂到已存在的父节点，父节点显式提供 `children` 时以显式列表为准。

每个文件独立校验。解析失败、未知 provider、重复节点、未知 option set 和字段错误都会进入诊断列表，文件不会改变当前快照；其余有效文件仍可应用。因此宿主可以保留上一份有效的 `Catalog`，在后台热重载时避免半成品目录。描述覆盖仍在补全结果的最后阶段应用，现有 `[descriptions]` 配置拥有最高优先级。

`Catalog::load_user_dir(path)` 返回内置目录加上有效用户文件的快照；目录级读取错误返回 `CatalogLoadError`。文件级错误保存在 `Catalog::diagnostics()`。`Catalog::check_dir(path)` 只校验并返回每个文件的节点数量、选项集合数量和具体错误，适合 `specs check`；`Catalog::list()` 返回完整上下文、来源、provider、选项和子命令数量，适合 `specs list`。

规格查找提供两个入口：`complete` 保持 0.2 的前缀过滤行为，`complete_unfiltered` 保留当前选项、值、短选项附加值和替换上下文的全部合法候选，供宿主执行精确、前缀、模糊和本地使用习惯排序。内置函数 `canonical_command`、`describe_command`、`all_builtin_descriptions` 的行为继续兼容旧接口。

## CLI 诊断

用户规格目录可以用以下命令检查和查看。未指定 `--directory` 时使用配置中的 `specs.directory`；没有创建目录时按空目录处理，内置规格仍然可用。

```text
shellsense specs check
shellsense specs check --directory .\shellsense-specs --json
shellsense specs list
shellsense specs list --directory .\shellsense-specs --json
```

`specs check` 会列出每个文件的有效性、节点数和 option set 数量，并在错误时以退出码 1 结束。`specs list` 展示合并后的完整上下文、来源和固定 provider。JSON 输出包含 `files`、`nodes` 或 `diagnostics`，便于编辑器和 CI 使用。

补全诊断可以附加 `--explain`：

```text
shellsense complete --line "git switch fea" --explain
shellsense complete --line "cargo test --features=" --json --explain
```

解释结果包含 UTF-8 替换范围、解析后的命令上下文、规格目录诊断、动态数据源状态及每个候选的来源。普通 `complete --json` 的原有字段保持不变。

`doctor` 会同时报告配置位置、快捷键绑定、规格目录和诊断、Git/Cargo/npm/pnpm/PowerShell 可执行文件状态，以及本地统计文件位置。选择统计只保存散列标识；需要清除时执行 `shellsense learning clear`。

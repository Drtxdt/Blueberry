# Blueberry

[![CI](https://github.com/Drtxdt/Blueberry/actions/workflows/ci.yml/badge.svg)](https://github.com/Drtxdt/Blueberry/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Drtxdt/Blueberry?include_prereleases)](https://github.com/Drtxdt/Blueberry/releases)

Blueberry 是一个使用rust主要构建的高性能终端IDE风格补全工具，可以为 PowerShell 提供带中文说明的命令补全菜单。输入命令后，可以查看参数、浏览路径、阅读帮助，或按用途查找命令。

使用 Rust 编写，目前交互运行环境为 **Windows x64、Windows PowerShell 5.1 / PowerShell 7、PSReadLine 2.0 及以上**。未来计划支持更多类型的终端。

本项目的灵感来源来自：[microsoft/inshellisense: IDE style command line auto complete](https://github.com/microsoft/inshellisense)

`0.5.0-beta.7` 仍在候选验收中。源码增加实验性单层宿主：`blueberry run --host-mode direct`，PowerShell 直接继承终端，传输固定为 pipe；`--host-mode direct --transport osc` 会报错。当前默认仍为 nested，显式 `--transport osc|pipe` 的旧调用保持嵌套路径。单层通过正确性和性能门槛后才切换默认，进度见 [验收记录](docs/beta-7-validation.md)。

## 安装

在 PowerShell 中运行：

```powershell
(irm 'https://raw.githubusercontent.com/Drtxdt/Blueberry/main/install.ps1').TrimStart([char]0xFEFF) | iex
```

安装脚本下载 GitHub Release，校验文件后安装到当前用户目录，并询问是否随 PowerShell 启动。默认选择最新稳定版；只有 Beta 版本时安装最新公开 Beta。

也可以从 [Releases](https://github.com/Drtxdt/Blueberry/releases) 下载 Windows x64 ZIP，解压后运行 `blueberry.exe`。指定版本、升级、回滚和卸载步骤见 [安装指南](docs/installation.md)。

## 快速开始

在你的终端中输入：

```powershell
blueberry
```

进入会话后，试着输入你常用的命令，例如：

```powershell
git log --
codex exec --
uv python
Get-ChildItem .\
```

菜单出现后，用方向键选择，按 **Tab** 接受，按 **Esc** 关闭，按 **F1** 阅读详情。

自动启动可以随时调整：

```powershell
blueberry startup enable
blueberry startup disable
blueberry startup status
```

## 功能与示例

| 功能 | 用法 |
|---|---|
| 命令与参数补全 | 按当前子命令和已输入参数筛选候选，支持明确的枚举值和目录参数 |
| 中文说明 | 内置说明优先使用中文，本机帮助提供英文兜底；用户可自行覆盖 |
| 本机帮助学习 | 对已登记且入口可信的工具按需获取帮助，缓存绑定实际安装入口和文件指纹 |
| 参数引导 | `codex --model` 等待模型值时显示参数格式，提示文字不会被插入 |
| 用途搜索 | 输入“查看分支”，按 Ctrl+Alt+F 搜索；在命令后搜索时限定当前上下文 |
| 详情页 | F1 查看说明、参数格式、示例和来源，长内容可翻页 |
| 原生补全 | Ctrl+Alt+Space 手动调用当前 PowerShell 会话的补全 |
| 图标与主题 | Unicode / Nerd Font 图标，支持深色、浅色及高对比度主题 |
| 命令工作台 | Ctrl+Alt+P 搜索收藏、结构化模板、当前会话历史、项目操作和 Blueberry 管理入口 |
| 常用命令建议 | 输入完整命令前缀时，在普通菜单中补充收藏、项目操作和最近历史 |

内置规则涵盖 PowerShell、Git/GitHub、Rust、Python、Conda、前端工具、Go、.NET、CMake、容器与集群工具、SSH、Codex 和 winget。本轮还加入 Deno、Java/Maven/Gradle、Make/Ninja/Just/Task、Terraform/OpenTofu、Ansible、常用 Windows 命令、AWS/Azure/Google Cloud CLI，以及 PostgreSQL、MySQL、SQLite、Redis 和 MongoDB 客户端。项目补全可读取脚本、依赖、环境、工作区、构建目标、基础设施变量、Compose 服务、Kubernetes 上下文和 SSH Host。

本机项目、环境、云配置名称和数据库连接配置自动更新。远程 Docker、Kubernetes、Helm、云资源和数据库对象使用 **Ctrl+Alt+D** 主动读取。Gradle 任务查询和 Ansible 插件解析等可能执行项目代码的操作也只允许手动刷新。

### 补全知识从哪里来

Blueberry 将几类知识合并后生成菜单：用户 TOML 规格与说明覆盖拥有最高优先级；内置规格提供稳定的中文命令、参数、格式和示例；本机程序帮助补充当前安装版本及插件命令；项目和本机动态提供器读取脚本、环境、目标和资源；PowerShell 原生补全的有效 ToolTip 可作为当前会话说明。

本机帮助按工具登记固定调用方式。Cargo 根命令分别读取 `cargo --list` 和 `cargo --help`，Git、pnpm、Docker 等使用各自适配器。只有帮助适配器和解析器都确认命令段或选项段完整时，Blueberry 才会隐藏本机版本不存在的内置项目；输出截断、格式未知或标题缺失时只追加知识。别名会指向规范命令，例如 `cargo b` 使用 `cargo build` 的规则。

学习缓存绑定实际程序入口、文件指纹、帮助适配器和解析器版本。升级程序或解析器后缓存会自动失效。以下命令可以查看每个候选的来源、学习状态、动态提供器和隐藏依据：

```powershell
blueberry complete --explain "cargo b"
blueberry specs list
blueberry doctor --json
blueberry tools
```

未知工具可以手动学习：

```powershell
blueberry specs learn mytool
blueberry specs list
blueberry specs forget mytool
```

学习在本机完成，后台任务有超时和输出大小限制。更复杂的参数关系可通过 [TOML 规格](docs/specifications.md) 补充。

## 快捷键

| 按键 | 操作 |
|---|---|
| Ctrl+Space | 打开补全菜单 |
| Tab | 接受选中候选 |
| ↑ / ↓ | 移动菜单选项 |
| Esc | 关闭菜单或退出用途搜索 |
| F1 | 打开或关闭详情 |
| PageUp / PageDown | 翻页 |
| Ctrl+Alt+F | 按用途搜索当前编辑词 |
| Ctrl+Alt+Space | 请求 PowerShell 原生补全 |
| Ctrl+Alt+C | 刷新命令索引 |
| Ctrl+Alt+R | 重载配置 |
| Ctrl+Alt+D | 刷新当前工具的资源候选 |
| Ctrl+Alt+P | 打开收藏、模板和历史工作台 |

Enter 保持 PowerShell 的执行行为。快捷键发生冲突时，可运行 `blueberry doctor` 查看诊断并修改配置。

## 配置

默认配置为 `%APPDATA%\Blueberry\config.toml`，安装目录为 `%LOCALAPPDATA%\Blueberry\bin`，缓存为 `%LOCALAPPDATA%\Blueberry\cache`。

```powershell
blueberry config edit
blueberry config init
blueberry config check
blueberry theme
blueberry doctor
blueberry doctor --json
blueberry tools
blueberry setup
blueberry hub
```

`blueberry config edit` 打开中文设置页，可调整外观、补全、快捷键、资源和学习开关。方向键或 Tab 选择，Enter 修改，`/` 搜索，Ctrl+M 只看修改项，Ctrl+1／2／3 应用默认、精简或手动触发预设，Ctrl+D 查看差异，Ctrl+S 保存。修改外观时可以预览效果。

`blueberry tools` 打开可搜索的工具管理页，可查看入口、规则、动态能力和帮助学习状态，并按工具调整帮助及动态候选。`blueberry setup` 使用分步页面选择图标、补全方式和自动启动。
引导第一页可实际按下补全、用途搜索和工作台快捷键；页面只标记终端送达的组合键，未点亮时可在设置中改键。

`blueberry hub` 汇总收藏、模板、当前 PowerShell 会话历史、准确的 PSReadLine 历史文件和项目操作。会话中按 Ctrl+Alt+P 后直接输入关键词，Enter 将命令填回但不会执行，Tab 或 F1 查看预览，Esc 保留原编辑行。独立运行时，确认后的命令复制到剪贴板。详细格式见 [命令工作台](docs/workbench.md)。

工作台可从已输入的 `git switch -c`、`cargo test -p/--features/--test` 和 `docker compose up/logs` 打开参数表单。选择“继续填写”后输入或选择参数，确认只填回当前行。仓库根目录的 `.blueberry/commands.toml` 可声明项目命令包；先用 `blueberry packs review` 查看来源和变化，再用 `blueberry packs approve <SHA-256>` 批准。文件变化会立即停用包；`blueberry packs list/revoke` 可查看或撤销。格式见 [命令工作台](docs/workbench.md)。

```powershell
blueberry hub --add "启动开发服务" --command "npm run dev"
blueberry hub --remove "启动开发服务"
```

设置页保留 TOML 注释和高级配置。需要编辑自定义颜色、规格目录或命令说明时，选择“打开 TOML”。独立配置使用 `blueberry --config .\my-config.toml config edit`。Nerd Font 图标需要终端已配置相应字体。

普通命令行编辑期间，右键粘贴和文字选择沿用 Windows Terminal 的设置。多行右键粘贴会作为一次 PSReadLine 编辑插入，管道首行、空行、中文和光标右侧内容都会保留；外部程序请求鼠标输入时，Blueberry 将鼠标操作转交给该程序。

安装时使用兼容性较好的 Unicode 图标。终端已配置 Nerd Font 时，可修改：

```toml
[ui]
icons = true
icon_style = "nerd"
```

工作台和按工具覆盖示例：

```toml
[workbench]
history_limit = 2000
suggestions = true
suggestion_limit = 6

[tools.docker]
dynamic = true
help = true
resources = "manual"
```

[什么是Nerd Font?](https://www.nerdfonts.com/)

完整选项见 [配置示例](config.example.toml)。使用独立配置或指定 PowerShell：

```powershell
blueberry --config .\my-config.toml
blueberry run --shell powershell.exe
blueberry run --shell pwsh.exe
```

## 开发与测试

构建需要 Rust 1.94 或更新版本：

```powershell
cargo build --release --locked
.\scripts\verify-local.ps1 -Shell pwsh.exe
```

GitHub Actions 检查 Windows、Linux、macOS 的构建和核心测试，并在 Windows PowerShell 5.1 与 PowerShell 7 上运行适配器和 ConPTY 回归。Windows Terminal 的中文输入、字体、缩放和视觉体验按 [本机检查表](docs/installation.md#本机验收) 验收。

发布者从 [发版指南](docs/releasing.md) 开始：推送版本标签后，Actions 生成带附件的 Release 草稿，再由发布者检查并公开。

## 未来展望

- Linux/macOS 交互会话，以及 Bash、Zsh、Fish 等 Shell 适配。
- 更深入的云账号资源、在线包索引和语言服务器候选。
- 在线规格市场、AI 翻译和更多界面语言。
- 行内预测、更完整的历史建议和 VS Code 集成。
- 持续降低启动和菜单响应开销，目标为启动增量 P50 ≤50 ms、热态菜单 P95 ≤20 ms。
- Scoop、WinGet 分发和发布签名。

## 许可证

[MIT](LICENSE)。依赖许可证见 [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt)。

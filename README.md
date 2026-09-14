# Blueberry

[![CI](https://github.com/Drtxdt/Blueberry/actions/workflows/ci.yml/badge.svg)](https://github.com/Drtxdt/Blueberry/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Drtxdt/Blueberry?include_prereleases)](https://github.com/Drtxdt/Blueberry/releases)

Blueberry 是一个使用rust主要构建的高性能终端IDE风格补全工具，可以为 PowerShell 提供带中文说明的命令补全菜单。输入命令后，可以查看参数、浏览路径、阅读帮助，或按用途查找命令。

使用 Rust 编写，目前交互运行环境为 **Windows x64、Windows PowerShell 5.1 / PowerShell 7、PSReadLine 2.0 及以上**。未来计划支持更多类型的终端。

本项目的灵感来源来自：[microsoft/inshellisense: IDE style command line auto complete](https://github.com/microsoft/inshellisense)

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

内置规则涵盖 Git、Cargo、rustup、Python、pip、uv、Conda／Mamba、Poetry、npm、pnpm、Yarn、Bun、Go、dotnet、CMake、Docker Compose、kubectl、Helm、SSH、Codex 和 winget。项目补全可读取脚本、依赖、环境、工作区、解决方案、CMake 预设、Compose 服务、Kubernetes 上下文和 SSH Host。

本机项目与环境数据自动更新。远程 Docker、Kubernetes 和 Helm 资源使用 **Ctrl+Alt+D** 主动读取，避免输入普通命令时连接远程服务。

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

`blueberry tools` 查看工具入口、内置规则数量和帮助学习状态。首次启动会提示一次 `blueberry setup`；向导不会阻塞 PowerShell。`blueberry hub` 汇总收藏、内置模板和 PSReadLine 历史；会话中按 Ctrl+Alt+P 后，Tab 将选中命令填回当前编辑行，Esc 保留原内容。

```powershell
blueberry hub --add "启动开发服务" --command "npm run dev"
blueberry hub --remove "启动开发服务"
```

设置页保留 TOML 注释和高级配置。需要编辑自定义颜色、规格目录或命令说明时，选择“打开 TOML”。独立配置使用 `blueberry --config .\my-config.toml config edit`。Nerd Font 图标需要终端已配置相应字体。

普通命令行编辑期间，右键粘贴和文字选择沿用 Windows Terminal 的设置；外部程序请求鼠标输入时，Blueberry 将鼠标操作转交给该程序。

安装时使用兼容性较好的 Unicode 图标。终端已配置 Nerd Font 时，可修改：

```toml
[ui]
icons = true
icon_style = "nerd"
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

GitHub Actions 检查 Windows、Linux、macOS 的构建和核心测试，并在 Windows PowerShell 5.1 上运行适配器、安装和 ConPTY 回归。Windows Terminal 的中文输入、字体、缩放和视觉体验按 [本机检查表](docs/installation.md#本机验收) 验收。

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

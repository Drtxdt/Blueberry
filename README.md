# ShellSense

Windows Terminal + PowerShell 7 的 Rust 补全宿主。Beta 开发版提供项目感知候选、离线中文说明和可配置菜单，不需要 Node、在线翻译或模型。架构是 Windows Terminal → ShellSense → 一个 pwsh，保留用户 profile 和 PSReadLine 的行内预测。

本地构建未签名，尚未公开发布。功能、真实终端验证和性能是不同验收项，具体状态见 [Beta 验收记录](docs/beta-progress.md) 和 [性能报告](docs/performance.md)。启动增量 P50 ≤50 ms、热态菜单 P95 ≤20 ms 的目标不随版本推进而降低。

## 启动

需要支持 ConPTY 的 Windows 10/11、Windows Terminal 和 PowerShell 7。源码构建另需 Rust stable、MSVC C++ Build Tools：

```powershell
cargo build --release --locked
.\target\release\shellsense.exe run
```

本地交付目录提供 `dist/shellsense.exe`，Git 不跟踪生成的 exe。建议让 Windows Terminal 直接启动 ShellSense，避免在 profile 中再启动外层 pwsh。`shellsense terminal-profile` 只打印配置；发布包提供带预览、备份、升级和回滚的 [安装脚本](docs/beta-installation.md)。

从现有 shell 手动启动时会清空当前可视区域以建立菜单坐标。子 pwsh 使用 UTF-8 控制台输入输出，依赖旧代码页的程序需要单独验证。正常模式保留 profile；`run --no-profile` 仅用于诊断。

## 补全内容

| 场景 | 内容 |
| --- | --- |
| 首词 | 实际 PATH/PATHEXT 程序、会话 alias/function/cmdlet，包括 cargo/rustc 等本地链接 |
| Git | 上下文选项、本地/远端分支、标签、远端、worktree、适用的状态文件 |
| Cargo | workspace 包、features、bin/example/test/bench 目标 |
| npm / pnpm | 本地脚本、workspace 包名和依赖名称 |
| PowerShell | $env: 名称、路径表达式、目录专用位置与重定向目标 |
| 自定义工具 | 用户目录中的版本化 TOML 规格、中文说明、示例与内置数据源 |

`git log --o` 包含 `--oneline`，`git status --o` 不包含；`git -C "含空格目录" log --o` 保留作用域。支持 `--name=value`、规格定义的短选项组合、互斥/依赖/重复规则与 `cargo build --features a,b`。未知选项不猜测取值个数，`--` 后停止选项补全。

简单输入由 Rust 解析；复杂输入从当前 PSReadLine AST 提取必要上下文。注释、here-string 正文和不能确认的表达式不猜测。插入校验真实缓冲区与 Unicode 范围，保留光标右侧文本；ShellSense 不执行补全文本。

Git 使用固定只读查询；Cargo/JS 读取本地清单，不执行构建、项目脚本、安装、fetch 或凭据交互。最多两个后台数据任务，静态候选立即可用，未完成结果显示加载状态。缓存更新会重算当前查询，保留选中的具体候选。

## 按键

| 操作 | 默认按键 |
| --- | --- |
| 显式显示菜单 | Ctrl+Space |
| 进入菜单导航 / 下一项 | ↓ |
| 上一项（进入菜单导航后） | ↑ / Shift+Tab |
| 接受候选 | Tab |
| 执行当前输入，不接受菜单 | Enter |
| 关闭菜单 | Esc |
| 中文详情、参数格式、离线示例 | F1 |
| 手动请求本会话原生补全 | Ctrl+Alt+Space |
| 刷新命令和项目候选 | Ctrl+Alt+C |
| 重新加载配置和规格 | Ctrl+Alt+R |

自动菜单默认保留上箭头的历史操作。菜单未显示时 Tab 和方向键交回 shell。外部程序运行期间不注入查询键，全屏与鼠标控制按当前终端模式透传。

原生补全可能运行用户已有的 PowerShell 补全脚本，因此只手动触发，使用独立菜单和真实替换范围，耗时不属于自动菜单承诺。ShellSense 不能强制终止任意补全脚本。

内部使用 `F12,s/a/c/n/e/l` 组合键，前缀可配置。冲突时保留原绑定并提示配置项。公共快捷键变化支持重新仲裁；内部前缀下次启动生效。

## 配置与说明

```powershell
shellsense config init
shellsense config check
shellsense theme
```

默认配置为 `%APPDATA%\shellsense\config.toml`，`--config <路径>` 可指定其他文件；初始化不覆盖已有文件。完整字段见 [config.example.toml](config.example.toml)。

```toml
[ui]
theme = "dark" # dark / light / high_contrast
width = 0      # 自动适配窗口
descriptions = true
status_bar = true

[completion]
fuzzy = true
dynamic = true
up_arrow_history = true

[learning]
enabled = true

[keys]
native = "Ctrl+Alt+Space"

[descriptions]
"git log --oneline" = "每条提交显示为一行"
"mytool" = "运行我的本地工具"
```

手动颜色优先于主题，接受 `default` 或 `#RRGGBB`，支持 `NO_COLOR`。变更自动重载，也可按 Ctrl+Alt+R；错误配置保留上一份有效设置。

内置主命令、子命令和参数有简短中文用途；未知程序显示“用途暂未收录”及来源，alias 优先展示目标用途。`[descriptions]` 按完整上下文覆盖，优先于用户规格和内置说明。文件、目录分别显示中文类型。

规格只从用户配置目录加载，不加载项目脚本或任意程序插件。[TOML 规格说明](docs/specifications.md) 包含格式、覆盖规则和固定数据源。可在 `[specs]` 中指定 `directory`。

本地排序仅在适配器确认成功插入后记录候选/项目的加盐散列、次数和时间，不读取完整历史，不记录环境变量值或输出。最多一万条，清理九十天未使用项；只调整同等匹配质量的顺序。关闭后不再记录或使用统计，`shellsense learning clear` 清除已有统计。

## 诊断与验证

```powershell
shellsense complete --line "git switch fea" --json --explain
shellsense specs check
shellsense specs list
shellsense doctor
shellsense learning clear
cargo test --locked -- --test-threads=1
pwsh -NoProfile -NonInteractive -File tests/adapter.tests.ps1
cargo clippy --all-targets --locked -- -D warnings
```

`complete --json` 保留原字段和 UTF-8 字节替换范围，增加可选的标识、来源、详情及完整性状态。`doctor` 在 ShellSense 会话内还读取当前适配器能力和键位仲裁结果。

`run --trace <新文件>` 默认关闭，只记录阶段、请求编号、数量和时间，不记录命令正文。`run --transport pipe` 是显式 named pipe 对照路径；默认 OSC，失败回退。只有完整宿主测量和兼容测试证明收益，才考虑改变默认。

历史 `probe` 与 `probe --host` 的口径见性能报告：前者测适配器回显，后者包含外层 ConPTY 宿主。不能把 Rust 查找时间当作按键到菜单可见的总延迟，也不能把组件测试通过当作真实输入法已验收。

本轮不提供自己的行内预测、完整历史建议、在线规格市场、AI 或 i18n。后续顺序为 Python/uv、winget/dotnet、Docker/Kubernetes、VS Code、i18n、WSL/其他 Shell。

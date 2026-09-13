# ShellSense 0.5 Beta 开发与验收记录

当前版本为 `0.5.0-beta.1`。交互、项目功能和本地交付工具已实现并通过本机自动回归；**性能验收未通过**。本轮准备未签名的本地 Beta 候选包，不执行公开发布。真实 Windows Terminal 的输入法/颜色/视觉验收及远程 CI 仍待完成。

## 已实现的功能

### 0.3 基础与输入可靠性

- Rust 核心、一个 PowerShell 7 会话、ConPTY 宿主；保留 profile、PSReadLine 行内预测、原 JSON 字段和 UTF-8 替换范围。
- PATH/PATHEXT 按实际顺序发现程序，跟随本地链接，保留 alias/function/cmdlet 优先级；扫描及 shell 枚举分批推进，完整快照才删除旧缺项，旧版本和不完整索引缓存自动重建。
- 简单输入由 Rust 解析，复杂输入取当前 PSReadLine AST 的必要上下文；注释、here-string 正文和无法确认的表达式不猜测。多行编辑与真实执行使用不同状态。
- Windows 输入读取器增量解码 Win32 CSI_ 外壳和内层 VT，覆盖中文、emoji、重复键、AltGr/Alt-code、物理 Esc、整段粘贴及错误序列恢复。粘贴按会话 FIFO 插入实际 PSReadLine 缓冲区，不自动执行；超过上限时拒绝整段输入。
- 插入前校验实际行、UTF-16 光标与替换范围；保留右侧文本、引号和撤销行为，确认成功后才记录候选选择。
- 版本化 TOML 规格在构建时校验并生成 Rust 数据；用户规格仅从配置目录后台加载，按完整上下文覆盖；中文说明覆盖优先级最高，保留最后有效快照。

### 0.4 项目功能与菜单

- Git：本地/远端分支、标签、远端、worktree、状态路径和 `-C` 上下文；按参数位置、选项和 `--` 区分引用与路径。固定只读 Git 子进程查询共用 1 秒预算，不执行 fetch、凭据交互或修改索引。查询前的文件探测和单次文件系统调用不能强制中断，不宣称整个 provider 有硬超时。
- Cargo：workspace 包、features、bin/example/test/bench；npm/pnpm：本地脚本、workspace 与依赖。只读清单和标准目录，不运行构建、脚本或安装。
- PowerShell：环境变量名称、路径表达式、目录专用参数和重定向目标。最多两个项目/路径数据任务，合并相同请求、取消过期任务，完整性状态随候选返回。
- 缺失的配置/specs 目录可逐级重新挂载监听；Git 状态请求递归监听工作树，切换 provider 时清理旧根；workspace 新成员及缺失目标目录可触发失效，提示符阶段和手动刷新作为补查路径。
- 精确、前缀、可关闭的模糊和低优先级纠错；规格限制重复、互斥、依赖、位置、短选项组合、内联值及逗号分隔值。未知参数不推断取值个数。
- 自动菜单保留上箭头历史；向下键进入导航、Tab 接受、Enter 执行现有输入。保留所选候选身份，支持空格/目录分隔符、三种主题、手动颜色、候选数量/来源/加载状态和 F1 中文详情。
- Ctrl+Alt+Space 手动请求本会话原生补全，使用真实替换范围单独展示；可能运行用户已有 completer，不计入自动菜单延迟，也不宣称能强制终止任意脚本。
- 本地排序只保存加盐散列、次数和时间，最多一万条、九十天过期，可关闭/清除；活动会话不能把已清除记录重新写回。排序不越过匹配质量、作用域及 shell 命令优先级。
- `complete --explain`、增强 `doctor`、`specs check/list`、配置和用途说明热重载、能力协商/编辑状态/插入确认均已接入。

## 本机验证

环境为 Windows 11 `10.0.26200`、PowerShell `7.6.6`、PSReadLine `2.4.5`、Rust `1.94.1`。release 使用 MSVC 静态 CRT，导入表仅依赖 Windows 系统 DLL。

| 检查 | 结果 |
| --- | --- |
| 完整 Rust 回归 | 144 passed，0 failed；1 个 helper 标记 ignored，由实际鼠标用例作为子进程启动 |
| pipe 模式的 terminal_beta 与 terminal_modes | 8 passed，0 failed；同一鼠标 helper 单独启动 |
| PowerShell adapter 回归 | 通过，包括绑定冲突、分批枚举、真实范围和 LASTEXITCODE 保留 |
| 本地发布生命周期自测 | 通过：安装、升级失败恢复、回滚、保留配置、卸载、设置预览/备份及包校验 |
| release CLI 回归 | 6 项通过：Cargo 链接、Git/Cargo 参数上下文、中文说明及 `--` 边界 |
| fmt / clippy（all-targets、deny warnings）/ release build | 通过 |
| Windows Terminal 中文输入法、颜色和视觉 | 尚未完成 |
| 远程 Windows CI 版本矩阵 | 已配置，尚未运行 |

真实 ConPTY 已覆盖：中文/emoji、单行与多行粘贴、CRLF、选择替换、撤销、未聚焦上箭头历史、多行继续编辑、光标中间插入、引号/右侧后缀、手动原生范围、接受确认、历史搜索、外部全屏恢复、实际鼠标、连续缩放和大量输出。旧版 terminal_modes 的首次完整运行曾超时一次，原因未确定；后续完整回归通过，增加启动阶段诊断及 ready/transport 校验后，OSC 与 pipe 定向回归也通过。

```powershell
cargo test --locked -- --test-threads=1
pwsh -NoProfile -File tests/adapter.tests.ps1
$env:SHELLSENSE_TEST_TRANSPORT = 'pipe'
cargo test --locked --test terminal_beta --test terminal_modes -- --test-threads=1
Remove-Item Env:SHELLSENSE_TEST_TRANSPORT
```

## 性能结果

正式运行保留 profile、关闭 trace，OS 缓存未清空；30 对启动样本及三组各 3,600 次热态查询的完整方法和原始数据见 [性能报告](performance-v0.5.md)。

| 指标 | 结果 | 门槛 | 验收 |
| --- | ---: | ---: | --- |
| 适配器首次输入增量 P50 | 143.40 ms | ≤50 ms | 未通过 |
| OSC、中文说明开启，热态 P95 | 36.80 ms | ≤20 ms | 未通过 |
| OSC、说明关闭，热态 P95 | 37.24 ms | ≤20 ms | 未通过 |
| named pipe、说明开启，热态 P95 | 37.09 ms | ≤20 ms | 未通过 |

hit/miss 仅表示落盘命令索引状态；进程内项目缓存在每个新会话重新建立，首次完整动态结果另行计时。各场景、每种缓存状态都单独验收，不能用总体分位数掩盖失败组。

OSC 继续作为默认通道。named pipe 未表现出明确的端到端收益；预编译 C# 的早期 30 对实验也未达到至少降低 20 ms 的保留标准，不进入生产路径或发布包。集中输出、共享快照与按变化重绘已实现，但不能因此宣称性能门槛完成。

## 本地交付与剩余工作

交付工具包括 Windows x64 ZIP、SHA-256/文件清单、离线依赖许可证和版本说明；本地安装、升级、卸载及回滚保留配置和上一版文件。Windows Terminal 设置修改须先 Preview，Apply 备份原字节；复杂 JSONC 明确拒绝自动改写。自测与最终包记录见 [发布检查表](release-checklist.md)，用法见 [安装说明](beta-installation.md)。

E 盘已离线，当前源码和本地交付物在 C 盘临时工作副本。交付后清理临时编译缓存；唯一源码、发布包、原始测量和回滚版本保留，待 E 盘恢复后迁移到独立的 `E:\OpenSource\shellsense-rs`。原 inshellisense 仓库不作修改。

Beta 的剩余验收项是性能门槛、真实 Windows Terminal 输入法/视觉及远程 CI 矩阵；公开发布另行处理。后续平台顺序仍为 Python/uv、winget/dotnet、Docker/Kubernetes、VS Code、i18n、WSL/其他 shell；完整历史建议、自有行内预测、在线市场和 AI 功能不进入本轮。

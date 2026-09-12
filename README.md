# ShellSense

Windows + PowerShell 7 的原生终端补全宿主，当前为 **0.2 Alpha**。独立 Rust 工程，不依赖 Node、npm 或 TypeScript 运行时。PowerShell 适配脚本嵌入可执行文件，负责读取 PSReadLine 输入和应用经过校验的替换。

本版修复 Cargo 等链接命令漏项，加入按上下文补全的 Git/Cargo 规格和离线中文用途说明。**启动与热态菜单的性能目标仍未通过**，实测结果、原始数据和剩余瓶颈见 [性能验收](docs/performance.md)。

## 使用

本地交付已包含 `dist/shellsense.exe`，可直接运行：

```powershell
.\dist\shellsense.exe run
```

Git 仓库不跟踪生成的二进制；从源码克隆后按以下步骤构建。

要求 Windows 10/11（支持 ConPTY）、PowerShell 7、Windows Terminal。源码构建需要 Rust stable 和 MSVC C++ Build Tools：

```powershell
cargo build --release --locked
.\target\release\shellsense.exe run
```

更推荐直接让 Windows Terminal 启动 ShellSense：

```powershell
.\target\release\shellsense.exe terminal-profile
```

将输出的 JSON 对象加入 Windows Terminal 的 `profiles.list`。该命令只打印配置，不修改现有设置。正式使用时先将 exe 放在固定位置，再导出配置；移动 exe 后需要更新路径。

Windows Terminal → ShellSense → 一个 pwsh。不要在 PowerShell profile 中启动 ShellSense，否则外层 pwsh 的启动成本仍然存在。正常模式保留用户 profile 的一次加载；`run --no-profile` 可用于排查 profile 的耗时。

从已有 shell 手动启动时，会清空当前可视区域以建立可靠的菜单坐标；更适合在单独的 Windows Terminal 标签页中使用。

宿主内的 pwsh 使用 UTF-8 控制台输入/输出，确保中文与 emoji 重绘正确。这只作用于子进程；依赖旧代码页的程序需要另行验证。

## 已实现

- 首词补全：实际 PATH/PATHEXT 可执行文件、当前已加载的 alias/function/cmdlet；支持管道或分号后的命令位置。
- 参数补全：内置 Git、Cargo、npm、Docker、pwsh、gh 等常用命令的静态子命令和选项，以及本地路径候选。
- 参数按子命令和取值规则过滤：`git log --o` 提供 `--oneline`，`git status --o` 不提供；支持 `git -C "含空格目录" log --o`，`--` 后停止选项补全。
- 内置主命令、子命令、选项和静态取值均有中文说明。未知程序显示“用途暂未收录”及来源，可自行补充；alias 优先展示目标用途。
- 原生菜单：上下选择、分页、选中项颜色、描述、边框、图标、匹配高亮；支持中文宽字符，候选文本会移除终端控制字符。
- 后台命令索引与缓存、过期请求丢弃。输入线程不等待目录扫描；空闲时阻塞等待事件。
- 当前 pwsh 的 PATH 变更在下一次提示符时更新索引。编辑操作使用 PSReadLine.Replace；不会执行补全文本。

| 操作 | 按键 |
| --- | --- |
| 输入时自动显示 | 默认开启 |
| 显示补全，包括空行 | Ctrl+Space |
| 选择候选 | ↑ / ↓ / Shift+Tab |
| 接受候选 | Tab |
| 关闭菜单 | Esc |
| 执行当前输入 | Enter |
| 刷新命令索引及当前 shell 命令 | Ctrl+Alt+C |
| 重新加载主题和用途说明 | Ctrl+Alt+R |

菜单未显示时，Tab 和方向键仍交给 shell。程序运行期间及备用屏幕中停用补全。内部占用 PSReadLine 的 `F12,s`、`F12,a`、`F12,c` 组合键；已有绑定冲突时停用适配，保留原绑定。

## 个性化

```powershell
shellsense config init
shellsense config check
shellsense theme
```

默认文件：`%APPDATA%\shellsense\config.toml`。可用全局 `--config <路径>` 指定其他文件。初始化不会覆盖已有文件。完整示例见 [config.example.toml](config.example.toml)：

```toml
[ui]
max_rows = 8
width = 80
border = "rounded"
icons = true
descriptions = true
selected_background = "#264f78"
selected_foreground = "#ffffff"
match_color = "#ffcc66"

[completion]
max_results = 100
auto_trigger = true

[descriptions]
"git log --oneline" = "每条提交显示为一行"
"cargo" = "构建项目并管理 Rust 依赖"
"mytool" = "运行我的本地工具"
```

颜色接受 `default` 或 `#RRGGBB`；支持 `NO_COLOR`。修改后按 Ctrl+Alt+R 生效，配置无效时保留上一次有效配置；使用 `config check` 查看具体错误。

用途说明的键使用完整命令上下文；例如 `"git log --oneline"` 与 `"git show --oneline"` 可以不同，`-C` 和 `-c` 区分大小写。使用别名补全参数时仍采用目标命令的规范键。主命令可以直接按程序名或 alias 名覆盖。说明随程序离线提供，输入时不启动帮助命令、翻译服务或模型。`ui.descriptions = false` 可隐藏说明列。

可选值使用附加写法补全，例如 `--ignored=mat` → `--ignored=matching`；它的取值说明键写作 `"git status --ignored matching"`。

升级会自动重建旧索引缓存，旧配置仍然有效。本地交付的上一版保存在 `dist/previous/shellsense-v0.1.0.exe`，不会覆盖用户配置。已运行的旧进程需关闭后从新 exe 启动。

若回退到 0.1，已加入 `[descriptions]` 的配置需另存一份旧格式副本并通过 `--config` 指定；0.1 不识别这个新配置段。从源码回退提交后需重新构建 exe。

## 验证与性能

```powershell
cargo test --locked
pwsh -NoProfile -NonInteractive -File tests/adapter.tests.ps1
cargo clippy --all-targets --locked -- -D warnings
shellsense probe --iterations 30 --output benchmark.json
shellsense probe --with-profile --iterations 30 --output profile-benchmark.json
shellsense probe --host --iterations 30 --output host-benchmark.json
shellsense probe --host --no-descriptions --iterations 30 --output without-descriptions.json
shellsense complete --line "gi" --json
```

`probe` 创建真实 ConPTY 会话，交替测量原生 pwsh 与适配后的 pwsh，记录提示符时间、首次输入回显时间、PSReadLine 查询往返和 Rust 查找时间，验证输入读取、替换、命令枚举及执行后恢复。默认不加载用户 profile；`--with-profile` 测试实际 profile。它不会修改 profile，测试会关闭历史写入。

`probe --host` 分别测量应用缓存未命中/命中时的首个提示符、首次菜单、每个会话 10 次热态菜单和自身 working set。它使用固定配置和临时命令目录，不加载 profile；内存统计排除 pwsh。缓存命中必须通过完整性验证，且不重复写盘。测试用两层 ConPTY 模拟运行环境，OS 缓存不清空。中位数采用中间两项均值，P95 采用 nearest-rank。

定位性能可运行 `shellsense run --trace timing.jsonl`，路径须为新文件。trace 默认关闭，只记录阶段、请求编号、时间和数量，不记录命令正文。正式计时请关闭 trace。

性能数字只适用于被测机器。提示符出现、首次输入可用和菜单出现是三个不同指标；`probe` 不测外层菜单渲染、冷启动磁盘缓存或内存占用。请勿把静态查找耗时解释为总输入延迟，也不要把 Alpha 当作全部性能目标已经达成。

## 当前边界

- 首版只验收 Windows + pwsh 的默认 Windows 键位，不承诺 Bash/Zsh、远程 SSH、多路复用器、Vi 编辑模式或所有终端程序兼容。鼠标事件透传尚未实现。
- 未移植全部 Fig/inshellisense 规格，不运行 JavaScript 生成器；Git 分支等动态数据、外部规格转换工具后续实现。
- 支持常见引号和命令分隔符，但不是完整 PowerShell 语法分析器。复杂子表达式、here-string、嵌套语言及多行编辑仍需扩展。回车后至下一次提示符期间不会注入查询键。
- shell 命令仅枚举已加载内容，避免为补全自动导入模块。新建/删除 alias、function 或导入模块后可按 Ctrl+Alt+C 刷新。
- 首版避免扫描显式 UNC 目录；映射网络盘仍可能延迟后台 I/O。高级 VT 控制、复杂 emoji 和大量异步输出需要继续兼容性验证。
- 原 inshellisense 工程保持不变。这是新实现，尚不声称功能完全等价。

实现设计见 [docs/architecture.md](docs/architecture.md)，适配协议见 [docs/powershell-adapter.md](docs/powershell-adapter.md)。

## 许可证

MIT。新编写的补全逻辑与静态规格不包含 Fig/inshellisense 源码的直接复制。依赖分别遵循各自许可证。

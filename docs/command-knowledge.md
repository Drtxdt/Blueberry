# 命令知识、自动帮助和用途搜索

本轮源码在 0.5.0-beta.1 基础上实现，已按用户要求更新 dist/shellsense.exe；用户配置保持不变。

## 实施批次与回退

1. 说明：保留 ToolTip 完整文本，清理重复路径/名字与自指 F1 文案，未知来源显示类型；校验孤立选项集、空白/占位说明和无效选项。
2. 学习：knowledge 模块独立管理入口、解析器、进程边界与 help 缓存；主机负责 300 ms 去抖和重算。关闭 `[help] enabled` 可独立停用自动执行。
3. 规格：新增六组工具及主要一级上下文，保留中文说明与人工约束，用户节点不被学习覆盖。
4. 菜单：`ui.icon_style` 可回退 Unicode，`icons` 总开关仍有效；参数提示只展示，不进入接受候选；F1 用 PgUp/PgDn 翻页。
5. 搜索：`keys.search` 默认 Ctrl+Alt+F；离线子串匹配命令名、说明和关键词，匹配原因显示在搜索页脚。Esc 回到普通补全。

旧版 dist/shellsense-before-knowledge.exe 保留作二进制回退。旧版不能读取新增配置字段，切换回旧版时使用原配置。未提交自动修改用户 profile、字体或终端设置。

## 数据与边界

规格 schema_version 仍为 1。节点可选 `keywords = ["查看分支"]`，选项可选 `value_name = "<MODEL>"`。JSON 保留旧字段和 UTF-8 字节替换范围，增加 `argument_hint`、`description_source`、`language`、`match_reason`。会话通过 `command_metadata` 能力协商读取已加载函数/命令的静态元数据；不导入模块，不执行 dynamicparam、用户补全器或生成的 shell 补全脚本。

帮助使用独立工作目录，stdin 关闭，固定逐工具帮助参数；Git 子命令使用 `-h`。stdout/stderr 合计限 256 KiB，2 秒超时，Windows Job Object 约束并终止子进程树。失败在会话内退避 5 分钟，刷新可重新尝试。帮助不是 UTF-8 时报告失败，不猜测编码。

缓存身份包括入口、包装目标、可选脚本、大小和修改时间，以及解析器版本。它用于版本变化检测，不是对抗篡改的内容证明。npm 包装入口只解析 manifest 与已知入口，不执行 shim；Codex 直接调用该 npm 包对应的原生二进制，与桌面应用目录中的 codex.exe 分开缓存。非标准脚本须指定实际可执行入口显式学习。

保守解析不能证明完整的帮助只补充，不删除离线规则；复杂互斥与依赖仍以人工规则为准。未提供中文的新增选项保留本机英文。用途搜索是离线文本匹配，不进行语义推理或在线翻译。

## 验收

- Rust 回归各测试组共 158 项通过（含最终重新运行的终端测试）。辅助进程测试 1 项由鼠标测试启动，标为 ignored。
- pipe 下终端、模式切换、编辑缓冲区和帮助专项回归通过。最终静态元数据、原生补全、多行输入、粘贴和模式切换在 OSC 与 pipe 两种通道下均通过。
- PowerShell 适配器测试、Clippy（`-D warnings`）、格式与 diff 检查通过。
- 覆盖中文用途搜索、多 token 插入、右侧文本保留、无候选参数提示、动态参数不执行、stderr 非零帮助输出、超大输出、超时、缓存身份变化，以及 Unicode/Nerd 两种渲染路径。
- Windows Terminal 的字形、缩放、窄窗口视觉与真实 IME 验收尚未完成。可用电脑控制技能的 guidance 明确禁止自动操作终端应用；ConPTY 结果不替代该项。

最终终端修复包括：命令枚举与元数据请求串行化；元数据处理中允许排队手动原生补全；抑制复杂多行位置的元数据查询；原生结果到达后立即重绘。元数据测试实际接受 `fast` 并验证 PSReadLine 缓冲区，避免历史屏幕文本形成假阳性。

## 本机帮助采集

最终 release 实测 npm Codex 根命令和 exec/review/resume/fork/mcp/completion 全部成功，mcp 的 7 个子命令（含只有名字的条目）保留。另采集桌面应用的 codex.exe，验证两个入口的缓存指纹不同；再通过 CLI 确认缓存中的 `codex exec --json` 可见。测试使用 artifacts 下隔离的 LOCALAPPDATA/APPDATA，未清理或修改用户帮助缓存。

显式 learn 单次总耗时：npm 根 633.83 ms、桌面入口根 646.40 ms，六个子上下文 98.23–119.23 ms。这些是单次 CLI 端到端观察，包含程序启动、命令发现、帮助子进程、解析与写缓存；不等于纯解析开销，不包含宿主的 300 ms 空闲等待。原始结果在 [learn-results.json](../artifacts/knowledge/learn-results.json)。

## 性能口径

2026-09-13，本机 Windows x64、PowerShell 7.6.6、PSReadLine 2.4.5，release，保留 profile，关闭 trace。旧版 exe SHA256 为 8064AC6293EE3CE3C7E909BFC56A605F3DAE8A413B914B7129B49740F4F4219A（以实际文件哈希为准）；最终版本 A4D82560EA8A1EE417A7621EEFC2A2E0468FB1403593E755CA126E269BFB6AC2。

热态菜单每场景/缓存状态 30 个样本，6 场景 × miss/hit = 360 个样本/版本；顺序运行、没有清理 OS 文件缓存，不与此前 C 盘报告直接比较。样本低于正式验收要求的每组 300 个，因此仅作为增量对比。使用兼容 Unicode 模式保证新旧口径一致；Nerd 字形的真实视觉成本未在此测量。

| 指标 | 旧版 | 最终版本 | 目标 |
| --- | ---: | ---: | ---: |
| 热态菜单 P95 | 32.2131 ms | 32.2732 ms | ≤20 ms，未达到 |
| 配对首次输入启动增量 P50（各 30 对） | 198.9779 ms | 201.66885 ms | ≤50 ms，未达到 |
| Rust 根查找 P50（组件） | 0.2452 ms | 0.25385 ms | 不代表端到端菜单 |

0.0601 ms 的差异不足以作因果判断。帮助后台等待没有作为“热缓存菜单完成”进行计时，也没有从原始样本中剔除异常值。所有运行保持 transport_degraded=false。原始数据：[旧版热态](../artifacts/knowledge/baseline-hot.json)、[最终热态](../artifacts/knowledge/final-hot.json)。


启动口径为每个版本分别与 plain PowerShell 配对的首次输入回显增量，旧版与新版之间顺序测量而非逐对交错。观察到的 P50 差值约 +2.69 ms，不能据此精确归因。原始数据：[旧版启动](../artifacts/knowledge/baseline-startup.json)、[最终启动](../artifacts/knowledge/final-startup.json)。两项原目标都没有达到，本轮不宣称性能验收通过。

## 使用与回退

本机提供独立的 `dist/shellsense-knowledge.exe` 和 `dist/knowledge-config.toml`（Nerd 图标）。可从 Windows Terminal 启动：

```powershell
.\dist\shellsense-knowledge.exe --config .\dist\knowledge-config.toml run
```

已按用户要求将新版覆盖到 `dist/shellsense.exe`，旧版备份为 `dist/shellsense-before-knowledge.exe`。新配置没有写入用户配置目录；退出新会话并启动备份版即可回退。`specs list` 缓存状态及 `specs forget codex` 已在隔离目录验证。

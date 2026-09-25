# 0.5.0-beta.7 候选验收记录

公开 beta.6 标签 `v0.5.0-beta.6` 指向 `decd47e0416524985c210dccc16cbced0403280a`；本次功能候选开始前的开发基线为其后的 `c5b0c09af692a028c51ecfd45b74d97defc94b82`。两者不能混作同一个版本。本文件跟随未提交的 beta.7 源码候选更新；正式验收必须在冻结提交后重测。历史 [0.5 性能报告](performance-v0.5.md) 测量的是更早的提交，不能代表本候选。

本机安装目录有一份 beta.6 可执行文件（SHA-256 `C27404F61D5C96AEB2EDF236DC0F25B033A45E50B825E7369D73521C73C7FB86`），但尚未与公开发布附件逐字节核对，不能作为正式 beta.6 对照。plain PowerShell／公开 beta.6 发布包／候选版／inshellisense 的交替对照仍待取得并冻结各文件哈希后执行。

本机 inshellisense 为 `@microsoft/inshellisense` `0.0.1-rc.21`；当前 `build/index.js` 的 SHA-256 为 `6F005109131EF301C15219CDE1E5B1994BA683D78FAF125CFCA90009D56E9713`，pnpm 启动脚本的 SHA-256 为 `E239747E939671C0AEDD0543CC6C6A527ECBD36067085F5799871DD74C822460`。正式对照前须再次确认文件未变化。公开 beta.6 附件查询因本机 GitHub TLS 连接失败，仍缺正式来源核验。

## 已完成的自动验证

- `cargo fmt --all`、`cargo clippy --all-targets --locked -- -D warnings` 通过。
- 库测试 83 项通过，包含损坏/未知 schema 不覆写、注释与无关字段保留、12 个并发写入者、外部修改冲突、进程写入中断、Windows 拒绝替换、历史尾部跨行记录、结构化参数、启动快捷键环境完整性与项目包哈希失效。独立进程收藏写入测试用 16 个进程验证，项目包 CLI 测试验证跨进程改动后批准失效。
- 本机 `cargo test --locked -- --test-threads=1` 全部通过（2 项按原设计 ignored）；真实 ConPTY 验证从 `cargo test -p --release` 打开表单、确认只填回、保留光标右侧文本，以及 Esc 取消保留原行。
- 本机 PowerShell 5.1 和 PowerShell 7 的 `terminal_beta` 与 `terminal_modes` 在 OSC 和 pipe 均通过；受限权限运行 pipe 会因无法创建命名管道而降级，正常权限复测通过。CI 增加固定 PSReadLine 版本的双 Shell、双传输矩阵；远程结果尚未取得。
- 发布证据校验的 9 项 Python 测试通过；缺原始样本、超标热态、额外包文件、外置与包内清单不一致、危险路径、人工验收缺项或用户使用了其他 EXE 均被拒绝。独立工作台真实 ConPTY 测试通过窄窗口、搜索缩短、中文 emoji 与结构化表单取消。

## 发布硬门槛与剩余验收

| 项目 | 门槛 | 当前状态 |
| --- | --- | --- |
| 首次输入增量 | P50 ≤50 ms，固定机器、profile 和交替顺序 | 最近 10 对探索样本 P50 199.8764 ms，未达标；冻结候选仍须正式复测 |
| 热态菜单 | 各场景 P95 ≤20 ms | 最近六场景各 10 次探索样本 P95 31.89–32.76 ms，未达标；冻结候选仍须正式复测 |
| 完整 ConPTY 回归 | PowerShell 5.1/7、OSC/pipe 全部通过 | 本机双 Shell、双传输通过；固定 PSReadLine 版本及远程矩阵待确认 |
| Windows Terminal | 输入法、字体缩放、选择、粘贴、嵌套程序 | 待人工验收 |
| 目标用户 | 8–12 位独立完成安装、解释、项目参数、模板、退出恢复 | 待候选达到技术门槛后执行 |

## 当前源码候选的探索性样本

- Release 可执行文件 SHA-256：`808CA259FFEEC7C074554828DA36A129680D3916088CAA71B522B904AA579AC3`。该二进制含未提交源码；`doctor` 的 Git commit 字段仍指向基线 `c5b0c09`，不能当作最终源码哈希。
- [首次输入原始样本](benchmarks/v0.5/beta7-startup-exploratory.json)：`probe --with-profile --iterations 10`，一进程内交替运行 plain 与嵌入适配器的 PowerShell 7，P50 148.35845 ms；样本数低于正式 30 对。
- [宿主原始样本](benchmarks/v0.5/beta7-host-exploratory.json)：`probe --host --iterations 3`，每种缓存状态 30 次热态查询，P95 hit 41.9412 ms、miss 33.6016 ms；场景范围和样本数均低于正式验收。
- [miss trace](benchmarks/v0.5/beta7-trace-miss-0.jsonl) 与 [hit trace](benchmarks/v0.5/beta7-trace-hit-0.jsonl) 来自相同 release 二进制的单会话诊断；只含阶段、时间与数量。`adapter_bootstrap` 分别为 153.25/149.69 ms，其中 `readline_init` 为 124.88/122.03 ms；后者包含约 27–29 ms 的公共快捷键审查、约 13 ms 的按键快照与约 10 ms 的内部键注册。最大热态相关 `query_response` 样本约 56–58 ms。阶段存在包含关系，不应相加为总启动时长。

本候选不发布。首次输入仍需降低脚本加载与 PSReadLine 初始化开销；热态分层诊断见下文。优化后按固定环境重做交替对照与各场景 300 样本验收。不能使用探索数据宣称达标。

## 追加诊断与未采用的优化

- [3 对启动诊断](benchmarks/v0.5/beta7-startup-diagnostic-3.json) 的首次输入增量 P50 为 214.7252 ms；这批小样本来自另一未提交构建，未纳入正式统计。
- [宿主诊断](benchmarks/v0.5/beta7-host-diagnostic-1.json) 的热态 P95 为 miss 48.1541 ms、hit 64.0589 ms。对应 [miss trace](benchmarks/v0.5/beta7-trace-diagnostic-miss.jsonl)、[hit trace](benchmarks/v0.5/beta7-trace-diagnostic-hit.jsonl) 显示 `script_source` 约 250.53 ms、`adapter_bootstrap` 约 179.82 ms、`readline_init` 约 149.2 ms；候选计算低于 1.5 ms、重绘低于 0.15 ms。trace 阶段相互包含，且会增加运行开销。
- 试过将输入与 PSReadLine 缓冲区查询合并为一次 ConPTY 写入。同探针各 2 个会话的热态 P95，改动前 miss 31.91 ms、hit 32.33 ms，改动后 miss 32.32 ms、hit 32.12 ms；证据不支持收益，已撤回。对应诊断文件保留在 `target/beta7-{before,after}-batch.json`，不是正式发布数据。
- 撤回该改动后重新构建的候选 EXE SHA-256 为 `CF731DF5F039657D4226DBA09809B5DB21EB592A300E64CC8B9A4F80920AE60F`。[当前启动 10 对样本](benchmarks/v0.5/beta7-current-startup-10.json) 的首次输入增量 P50 为 **199.8764 ms**；[当前六场景热态样本](benchmarks/v0.5/beta7-current-hot-10.json) 每组 10 次，P95 为 **31.89–32.76 ms**。采样不足以正式验收，但已足以判定目前未达到发布标准；正式计时关闭 trace，实际传输为 OSC 且未降级。

## 本轮分层回显诊断与数据安全

- 新增 `layer-probe`，以相同 30×120 外层 ConPTY 交替测普通 PowerShell、直接加载适配器、最小 Rust 嵌套透传和完整 Blueberry。报告将每个会话首个查询与后续热态查询分开；只测真实输入回显，不作为候选菜单的发布验收指标。
- [PowerShell 7 原始分层样本](benchmarks/v0.5/beta7-layer-diagnostic-30.json) 使用 EXE SHA-256 `01186A86FC7A36B28479E45C5CC0DD5A266CD34F9CC8EC939718A5AEA9541732`，每种路径 30 次、其中 27 次热态。热态回显 P95：普通 PowerShell **16.64 ms**，直接加载适配器 **16.38 ms**，最小双层透传 **32.44 ms**，完整 Blueberry **32.21 ms**。首个查询另有启动波动，不能混入热态结论。
- [PowerShell 5.1 原始分层样本](benchmarks/v0.5/beta7-layer-ps51-diagnostic-10.json) 每种路径 10 次、其中 9 次热态；对应 P95 为 **16.46 / 16.33 / 32.06 / 32.03 ms**。样本较少，仅用于交叉验证。两种 Shell 都指向第二层 ConPTY 造成的约 16 ms 额外回显；完整 Blueberry 相对最小双层透传的热态差异很小。单层路径目前没有 Rust 候选菜单，不能据此宣称产品达标。
- 收藏库增加临时文件同步后杀进程和 Windows 拒绝替换测试：原文件保留，锁自动释放，后续写入成功。真实 ConPTY 验证工作台在运行中接收外部修改；TOML 损坏时保留上一份有效候选并显示错误，修复后自动恢复。
- 发布证据校验现在要求人工验收记录和每位用户任务都绑定最终 EXE SHA-256，人工记录还绑定源码提交；不允许沿用旧构建的验收结果。
- 快捷键启动环境此前漏传 `SEARCH`，令适配器每次都回退解析 JSON，现已补齐。旧 EXE `01186A86FC7A36B28479E45C5CC0DD5A266CD34F9CC8EC939718A5AEA9541732` 与新 EXE `2639BA0AE966390920374DEF5BB336156EEDCB11117ABF8735FB303B6586FC20` 以 ABAB 顺序各测 10 对启动样本；原始数据为 [旧 A](benchmarks/v0.5/beta7-before-search-key-5a.json)、[新 A](benchmarks/v0.5/beta7-after-search-key-5a.json)、[旧 B](benchmarks/v0.5/beta7-before-search-key-5b.json)、[新 B](benchmarks/v0.5/beta7-after-search-key-5b.json)。合并 P50 分别为 200.49 与 209.38 ms，无稳定收益；这是一项完整性修复，启动门槛依然失败。
- 用当前探索性 release EXE 在隔离目录完成发布脚本自测、PowerShell 5.1 安装/升级/回滚/卸载及损坏包拒绝、PowerShell 5.1／7 本地下载选择测试。该本地包不是最终 main CI 工件，正式验收仍须在冻结 EXE 后重做。Windows Terminal 真实输入法与视觉验收、远程 CI 和目标用户任务尚未执行。

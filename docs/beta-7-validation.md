# 0.5.0-beta.7 候选验收记录

**最新状态（2026-09-26）：**按用户明确授权，候选已合并并推送 main；源码 `7efe7a3d89426bef50978e76c4c6b465a973ec91` 的[完整 main CI](https://github.com/Drtxdt/Blueberry/actions/runs/36232787492) 十个作业全部通过，已取得并校验该次 Windows x64 包。被测 CI EXE SHA-256 为 `76B02B562E6F6B2E5F37D7AA808F9CE15216697D875A870E4398186ADA395D50`，干净 release 构建。该 EXE 的 PS7 短组启动 P50 **157.61 ms**，十二组热态完整菜单 P95 **29.63–350.22 ms**，仍不达标。隔离安装及真实公开 beta.6 EXE 升级回滚自测通过；Windows Terminal 人工验收、正式矩阵和四方对照未完成。默认仍 nested，direct 实验状态，未打标签或发布。后续证据提交只保存报告与原始数组，不改变这份包的源码身份；下文旧状态均属于历史记录。

公开 beta.6 标签 `v0.5.0-beta.6` 指向 `decd47e0416524985c210dccc16cbced0403280a`；本次功能候选开始前的开发基线为其后的 `c5b0c09af692a028c51ecfd45b74d97defc94b82`。两者不能混作同一个版本。beta.7 功能候选已提交于独立分支 `codex/beta7-candidate`，起点提交为 `7242c6a87c07bae17aa794cdd379f1629bad5812`。早期探索 EXE 在该提交之前构建；本轮后续 EXE 包含尚未冻结的诊断与启动改动。两者都不能当作最终候选；正式验收必须以最终 main CI 工件重测。历史 [0.5 性能报告](performance-v0.5.md) 测量的是更早的提交，不能代表本候选。

2026-09-26 已下载并核验[公开 beta.6 ZIP](https://github.com/Drtxdt/Blueberry/releases/download/v0.5.0-beta.6/blueberry-v0.5.0-beta.6-windows-x64.zip)：ZIP SHA-256 `4D40AE29FEA9B92820DCB72373FE1AFD6538F682EC58B72A337FC24A3B3B40BA`；包内 EXE 与本机安装版摘要均为 `C27404F61D5C96AEB2EDF236DC0F25B033A45E50B825E7369D73521C73C7FB86`。公开附件来源核验已完成；四方正式交替测量仍未执行。

本机 inshellisense 为 `@microsoft/inshellisense` `0.0.1-rc.21`；当前 `build/index.js` 的 SHA-256 为 `6F005109131EF301C15219CDE1E5B1994BA683D78FAF125CFCA90009D56E9713`，pnpm 启动脚本的 SHA-256 为 `E239747E939671C0AEDD0543CC6C6A527ECBD36067085F5799871DD74C822460`。正式对照前须再次确认文件未变化。先前 GitHub TLS 失败来自受限运行环境的 Schannel 凭证，正常权限取得公开附件成功。

## 已完成的自动验证

以下为历次记录。本轮从 `59dd80f0db3d9796d213e2ee781abbfb87cf5703` 继续的单层实现及其测量另见文末；历史 EXE 和嵌套样本不能替代单层正式验收。

- `cargo fmt --all`、`cargo clippy --all-targets --locked -- -D warnings` 通过。
- 库测试 83 项通过，包含损坏/未知 schema 不覆写、注释与无关字段保留、12 个并发写入者、外部修改冲突、进程写入中断、Windows 拒绝替换、历史尾部跨行记录、结构化参数、启动快捷键环境完整性与项目包哈希失效。独立进程收藏写入测试用 16 个进程验证，项目包 CLI 测试验证跨进程改动后批准失效。
- 本机 `cargo test --locked -- --test-threads=1` 全部通过（2 项按原设计 ignored）；真实 ConPTY 验证从 `cargo test -p --release` 打开表单、确认只填回、保留光标右侧文本，以及 Esc 取消保留原行。
- 本机 PowerShell 5.1 和 PowerShell 7 的 `terminal_beta` 与 `terminal_modes` 在 OSC 和 pipe 均通过；受限权限运行 pipe 会因无法创建命名管道而降级，正常权限复测通过。CI 增加固定 PSReadLine 版本的双 Shell、双传输矩阵；远程结果尚未取得。
- 发布证据校验的 12 项 Python 测试通过；缺原始样本、超标热态、四方对照缺项、公开 beta.6 包摘要不符、额外包文件、外置与包内清单不一致、危险路径、人工验收缺项或用户使用了其他 EXE 均被拒绝。独立工作台真实 ConPTY 测试通过窄窗口、搜索缩短、中文 emoji 与结构化表单取消。

## 发布硬门槛与剩余验收

| 项目 | 门槛 | 当前状态 |
| --- | --- | --- |
| 首次输入增量 | P50 ≤50 ms，固定机器、profile 和交替顺序 | 同一 main CI EXE 的 PS7 10 对探索 P50 157.61 ms，失败；三组合探索详见文末，正式验收未执行 |
| 热态菜单 | 正式单层各场景 P95 ≤20 ms，完整动态结果 | 同一 main CI EXE 的 PS7 各 30 个探索样本，十二组 P95 29.63–350.22 ms，全部失败；正式矩阵未执行 |
| 完整 ConPTY 回归 | 固定 PowerShell/PSReadLine 三组合、单层与嵌套兼容 | 源码 7efe7a3 的远程三固定组合单层、嵌套 OSC／pipe 及 native mouse 全部通过；剩余全流程覆盖见文末 |
| 安装升级回滚 | 同一 main CI EXE，保留用户配置、失败恢复与公开 beta.6 回滚 | 远程包自测及本机隔离公开 beta.6 EXE 升级回滚通过，绑定摘要见文末；不替代 Windows Terminal 人工验收 |
| Windows Terminal | 输入法、字体缩放、选择、粘贴、嵌套程序 | 待人工验收 |
| 目标用户 | 本版按用户明确要求豁免 | 用户要求豁免、未执行；不得记录成“通过” |

## 当前源码候选的探索性样本

- 当时的探索性 Release 可执行文件 SHA-256：`808CA259FFEEC7C074554828DA36A129680D3916088CAA71B522B904AA579AC3`。它构建于候选提交之前；`doctor` 的 Git commit 字段仍指向开发基线 `c5b0c09`，不能当作最终源码哈希。
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

## 单层直连诊断（仍非功能候选）

- `layer-probe` 现增加第五种路径：PowerShell 直接继承外层终端，Rust 作为后台服务通过会话专用 pipe 接收适配器事件，整个路径不建立第二层 ConPTY。每次输入回显后以内部快捷键读取真实 PSReadLine 缓冲区；每个会话另外用仅覆盖 `g` 的实验性处理函数委托原 `SelfInsert` 并确认缓冲区长度为 1。只在此隐藏的诊断模式安装该处理函数，遇到已有绑定则不覆盖。普通 `run` 的编辑和菜单行为未改变。
- [PowerShell 7 原始样本](benchmarks/v0.5/beta7-direct-layer-ps7-30.json) 与 [PowerShell 5.1 原始样本](benchmarks/v0.5/beta7-direct-layer-ps51-30.json) 各包含五种路径交替运行的 30 个回显样本，其中每种路径 27 个热态样本。对应 release EXE SHA-256 均为 `275861F6CC5356F07DBF322EE447FE885586AA7C73D326EB0F3A8B576EB1C9ED`；该 EXE 仍在当前新增诊断源码提交之前构建，不能视为正式候选。
- PowerShell 7 的热态回显 P95（plain／直接适配器／单层服务／最小双层／完整宿主）依次为 **23.20／29.46／23.67／48.88／38.04 ms**；PowerShell 5.1 依次为 **20.79／19.52／19.99／33.05／36.71 ms**。单层服务的“发送私有查询键→PSReadLine 确认缓冲区”P95 分别为 **6.05／7.37 ms**，“确认→Rust 收到 pipe 事件”P95 分别为 **3.38／4.93 ms**。这两段计时使用 Windows 进程间共用的性能计数器，报告不保留命令内容。
- 这批样本证明两种 Shell 都能在直连模式下送达真实缓冲区事件，并确认一个安全委托的编辑键；单层路径尚无 Rust 菜单、通用按键覆盖和插入流程。回显 P95 本身在部分组超过 20 ms，不能将其当作产品热态达标。先前 30 样本约 16／32 ms 的结果与本轮存在环境波动，下一步须固定电源策略、profile 和系统负载，继续定位屏幕可见延迟并完成菜单原型。
- 发布证据校验要求正式的 `comparison_reports`：每个支持组合均记录四方交替顺序、每种模式至少 30 个启动样本，以及六场景、两种缓存状态各至少 300 个热态样本。plain 和 inshellisense 的 Blueberry 传输标记为 `not_applicable`；beta.6 的文件摘要必须来自公开附件核验。当前没有正式四方报告，发布仍被阻断。

## PowerShell 5.1 JSON 初始化优化（探索）

- 构建时使用系统 Windows PowerShell 5.1 的 Automation 程序集编译 `shell/legacy-json.cs`，把程序集嵌入 EXE。安装后的适配器在 PowerShell 5.1 中加载这份程序集；单独运行源码适配器仍保留 `Add-Type` 回退。`probe` 现在单独报告 `json_initialization_ms`，可检查是否把开销转移到了首次按键。
- 旧 release EXE SHA-256 为 `275861F6CC5356F07DBF322EE447FE885586AA7C73D326EB0F3A8B576EB1C9ED`，新 release EXE SHA-256 为 `684C0BF530FB3EF4EE5311CE240589CEFA734CD2A66C4AB9B92C54A29CF5A7B8`。固定同一个本机 PowerShell 5.1、PSReadLine 2.4.5、profile 和 30×120 ConPTY，按旧／新／旧／新顺序每轮各采 5 对；OS 缓存未清除。原始报告：[旧 A](benchmarks/v0.5/beta7-before-embedded-legacy-5a.json)、[新 A](benchmarks/v0.5/beta7-after-embedded-legacy-5a.json)、[旧 B](benchmarks/v0.5/beta7-before-embedded-legacy-5b.json)、[新 B](benchmarks/v0.5/beta7-after-embedded-legacy-5b.json)。
- 首次输入增量的各轮 P50：旧 **427.74／458.58 ms**，新 **225.62／207.62 ms**；新构建 JSON 初始化各轮 P50 **11.50／10.61 ms**。旧构建的探针尚未单独输出 JSON 初始化时间；独立新进程的 `Add-Type` 阶段曾测得约 246 ms。优化消除了一个明确的启动大阶段，但首次输入仍明显超过 50 ms，且 PS7 热态菜单尚未因这项改动重测，不能用于发布验收。
- 新源码的 `cargo test --locked -- --test-threads=1` 全部通过（2 项按设计 ignored），`cargo clippy --all-targets --locked -- -D warnings`、PowerShell 5.1／7 的适配器与启动脚本测试、发布证据校验 12 项测试均通过。PowerShell 5.1 的 OSC 和 pipe，以及 PowerShell 7 的 pipe，`terminal_beta` 与 `hub_terminal` 各 11 项通过；全量 Rust 测试还覆盖默认 PowerShell 7 终端路径。启动脚本测试在受限文件系统中无法替换临时 profile，正常 Windows 权限复测通过。以上仍是本机回归，不代替远程 CI 和人工验收。

## 从 59dd80f 继续：可用单层候选与完整产品测量

本轮单层宿主已接入产品 `run --host-mode direct`，仍为实验模式。默认保持 nested；显式 `--transport osc|pipe` 且没有宿主参数继续选择 nested；direct 与 OSC 的组合报错。PowerShell 继承终端，Rust 不读取键盘或并发写终端。CLR 4 托管桥接在构建时编译并嵌入，启动只加载程序集；没有产品启动时 C# 编译。PSReadLine 2.0.0 和 2.4.5 共用公开编辑 API，保存并委托真实原处理函数，未按描述字符串重建处理函数，也未修改私有编辑状态。

- 自动菜单、共享工作台与参数表单由 Rust 原有 Worker、候选引擎和表单逻辑产生。Shell 编辑线程恢复覆盖区、委托原编辑动作、确认 UTF-16 整行／光标，再绘制 Rust 净化后的菜单。能力协商、递增 revision、帧身份和候选身份贯穿查询与接受；过期结果丢弃。接受只填回，表单取消不改变原行，替换通过 PSReadLine 公开 API 保留撤销。
- 管道独立接收线程在 ReadFile 后、JSON 解析前记录到达时间；高精度定时器配合非阻塞管道避免同步双向句柄互相阻塞。服务退出或协议失败后保留原 Shell，清理菜单并恢复编辑。`doctor --json` 保留旧字段，增加实际宿主、自动菜单状态与停用原因；会话外状态为空。
- 本机最初三固定组合的单层 7/7 回归通过；包含中文／组合 emoji、两次打开表单、取消和确认、光标右侧文本、撤销、上一帧接受、窄窗口、外部程序、Ctrl+C、运行中自定义绑定及服务断开。PS7 嵌套 OSC／pipe 各 21 项通过，2 项按原设计 ignored。共享逻辑完整 Rust 核心回归 171 项通过；库 86 项包含独立进程写入中断、权限失败、16 进程并发收藏写入及项目包失效。后续新增空行／关闭自动触发回归，最终组合结果另记下方。
- Windows PowerShell 5.1／PSReadLine 2.4.5 在 PSReadLine 启动前修改 Console 编码会丢失 emoji；在 plain 与最小桥接会话复现后，撤回提前改编码，由 PSReadLine 保持其编码生命周期。三个固定组合的 Unicode 回归均通过。
- 运行中绑定审查尝试过只看 Dictionary 版本计数，实测 PowerShell 7 替换 Backspace 前后计数均为 72，不能用于保护。该方案已撤回；现在核对处理器对象身份，并在必要时重新完整校验。编译桥接测试证明同数量自定义重绑被发现且原绑定保留。输入批次只在最终确认查询前审查，避免对每个 UTF-16 单元重复扫描约 6.5 万项。
- 单层历史从实际 Shell 历史路径在后台按既有预算读取；会话内新命令保留于内存。参数值后台更新受表单 generation 和字段身份校验，复用两种宿主的参数校验与渲染。

### 原始探索记录与下一阶段热点

下列 EXE 均由 `59dd80f` 加未提交改动构建，`source_dirty=true`，不是冻结的 main CI 工件。诊断 trace 与关闭 trace 的计时分别保存；所有时间均指接收线程所见输出形成 VT 观察终端的完整内容，不指物理屏幕像素显示。

| EXE SHA-256 | PS7 配对启动 10 对 P50 | 首键回显 P50 | 首次动态菜单 P50／P95 | 原始报告 |
| --- | --- | --- | --- | --- |
| `F68BAC042954B6974519ADDEB3E40F50141F501359878B3D7946E7E581DD08C4` | 201.14 ms | 57.87 ms | 17.92／31.42 ms | [优化前](benchmarks/v0.5/beta7-direct-product-ps7-10.json) |
| `95A5C5018BB1E35DEDB27CEA660E0327A4E6C353D321CC1DC7D1FC07F4EDD1BD` | 184.27 ms | 45.53 ms | 50.65／383.86 ms | [中间探索](benchmarks/v0.5/beta7-direct-product-ps7-after-10.json) |

两批都未达到启动 50 ms 门槛；首次键的成本仍需收口，不能只看提示符速度。第二批尚未包含最终批次审查、高精度响应等待和空行取消调整，不能代表之后的 EXE。

- [优化前 numeric trace](benchmarks/v0.5/beta7-direct-product-before-optimization-trace.jsonl) 来自第一份 EXE：桥接初始化 55.83 ms，其中约 6.5 万个按键注册 50.55 ms、初始绑定快照 1.19 ms；候选计算最大 20.91 ms。阶段相互包含，不能直接相加。
- [早期热态 30 样本](benchmarks/v0.5/beta7-direct-hot-ps7-30.json) 保留全部原始数组，已整批标记 `batch_validity.status=invalidated`：采样期间运行了 cargo clippy。排除整批，不挑选其中的快样本。后续诊断报告启用 trace，同样不能替代正式计时。
- 中间构建六场景 trace 的 Git 首次数据源约 40.85 ms 才返回，完整候选到菜单绘制又等待约 290 ms；命中缓存时数据源／合并通常不到 1 ms。空行候选规划约 11–12 ms 的重复工作也在 trace 中出现。下一构建改为收到不完整帧后使用剩余 4 ms 预算等完整帧，使用高精度响应定时器；空行或关闭 auto_trigger 时取消自动查询，与嵌套宿主行为一致。冷数据源超过 4 ms 时仍依赖 PSReadLine 安全回调，慢样本必须保留，不能提前显示未经确认的输入。

### 发布证据与仍未完成的门槛

发布证据校验器现在区分正式单层性能与嵌套兼容结果，核对干净 release 构建、源码、同一 CI 包、EXE、profile 摘要、机器、电源策略、30×120 终端与实际传输。完整产品启动必须提供 plain／candidate 原始数组、可复算配对差值、交替顺序、首键／首次静态／首次动态数组及自动菜单状态。诊断 trace、旧摘要、缺样本或不实的 plain 菜单指标被拒绝。Python 拒绝性测试已增加到 21 项，通过。CI 加入三个固定组合的单层回归，嵌套 OSC／pipe 保留。

目标用户试用记录为 **用户要求豁免、未执行**，不再作为本版缺项；历史段落中的未执行状态仅描述当时。安装升级回滚、Windows Terminal 输入法／缩放／选择／粘贴／嵌套程序人工验收仍未针对最终 CI EXE 完成。四方正式对照、进程树空闲资源、完整正式性能矩阵和成功 main CI 工件尚未取得。

单层所有可配置公共快捷键、Shell 自定义命令／别名元数据、真实剪贴板多行与输入法、项目包即时失效和乱序／协议故障的全流程回归仍需收口。已有测试证明原行与接受身份安全，但不能宣称用户计划中的全部功能覆盖完成。性能失败期间不切换默认、不合并 main、不打标签、不发布；下轮先针对最终探索 EXE 继续最大阶段优化与短组重测。

### 高精度等待、批次审查、空行取消后的探索

本轮最后被测 EXE SHA-256 为 `DB5AC1C13567730DA1D4523FB0742BE539CEAD378135F350ADFE834AC8665592`，release 构建仍标记 `59dd80f`、dirty。三个组合各 10 对，固定 profile、30×120、电源策略，关闭 trace，串行采样且无并发编译。PS5.1 最初一轮因探针误读回显中的版本命令而被版本一致性检查拒绝，未产生验收报告；修正为只解析完整元数据结束标记后重新整批采样。探针源码的后续解析测试不改变被测 EXE。

| Shell／PSReadLine | 启动增量 P50 | 首键回显 P50 | 首次静态完整菜单 P50 | 首次动态完整菜单 P50 | 原始数组 |
| --- | --- | --- | --- | --- | --- |
| PS5.1／2.0.0 | 171.57 ms | 36.25 ms | 360.46 ms | 19.67 ms | [10 对](benchmarks/v0.5/beta7-direct-product-ps51-200-final-exploration-10.json) |
| PS5.1／2.4.5 | 188.75 ms | 38.87 ms | 363.95 ms | 15.95 ms | [10 对](benchmarks/v0.5/beta7-direct-product-ps51-245-final-exploration-10.json) |
| PS7／2.4.5 | 166.56 ms | 41.44 ms | 367.57 ms | 16.11 ms | [10 对](benchmarks/v0.5/beta7-direct-product-ps7-final-exploration-10.json) |

三个启动组均失败。首次静态菜单变慢说明严格 4 ms 预算下首次计算无法即时交付、依赖安全回调的路径仍未解决；这是回归风险，不以较快的动态 P50 掩盖。下一阶段必须同时收口启动同步工作、首次静态候选与迟到安全刷新。

[同一 EXE 的 PS7 热态原始数组](benchmarks/v0.5/beta7-direct-hot-ps7-final-exploration-30.json) 关闭 trace，六场景每缓存条件各 30 个样本，三个 miss／hit 配对会话。这里 miss／hit 指命令索引持久缓存的会话启动状态；各会话首个数据源查询单独记录，不能把后续热态当作每键数据源冷启动或 OS 缓存冷启动。

| 场景 | 命令索引 miss 热态完整菜单 P95 | 命令索引 hit 热态完整菜单 P95 |
| --- | --- | --- |
| root | 362.53 ms | 35.78 ms |
| git | 37.37 ms | 47.69 ms |
| cargo | 47.06 ms | 353.72 ms |
| js | 360.14 ms | 357.19 ms |
| path | 352.16 ms | 361.88 ms |
| fuzzy | 369.06 ms | 349.19 ms |

十二组都失败；完整保留约 350 ms 的迟到样本，不能按首个加载帧或静态帧代替动态完成。root hit 回显 P50 为 16.24 ms、P95 28.87 ms，菜单 P50 为 18.64 ms；同条件 plain 诊断回显 P50 为 18.02 ms、P95 40.06 ms（仅 10 个控制样本），说明普通终端路径也有尾延迟，不能把所有慢样本归因于 Rust 候选计算。正式测量尚未开始，不扩大到 300 样本矩阵。

最终本机单层三固定组合各 8 项通过；最新库 87 项通过，包括防止把版本命令回显误当作实际版本的新测试。高精度等待遵守响应预算后暴露的迟到刷新问题尚未收口；代码留在候选分支，默认仍是 nested，远程结果另行记录，不合并 main 或发布。

提交前重跑剩余核心集，合计 172 项通过（87 库、4 CLI、3 收藏库进程、22 引擎、10 帮助知识、1 项目包 CLI、20 提供器、25 规格）。PS5.1／2.0.0 与 PS7／2.4.5 的嵌套 OSC／pipe 四组各 21 项通过，2 项按原设计 ignored；原生 Windows 11 鼠标仍交给专门 CI job。`cargo fmt --all`、Clippy 全 targets `-D warnings` 和 Python 21 项通过。另修正 doctor 在配置关闭 auto_trigger 时的状态与原因，单层测试验证真实会话状态；该诊断修正及观察探针解析测试在上述 DB5A 性能构建之后，未来正式 EXE 必须重测。

## 43c41e5 之后：根命令索引与 main 集成授权

候选已提交为 `43c41e597a149cc39277207b7a2b34f85030d4d1`，不再是未提交源码。用户随后明确要求“本地合并到 main 然后推送”，因此本轮将候选本地快进合并到 main 并推送以取得远程 CI；此授权只调整源码集成顺序，没有豁免性能、安装、Windows Terminal 人工验收、默认宿主切换或发布门槛。上述“不合并 main”描述保留为当时状态。默认继续 nested，版本继续候选，不创建 beta.7 标签。

诊断发现 `Catalog::canonical_command`／`describe_command` 在未知入口上逐个扫描所有规格节点。命令索引中多个未知入口把 root 规划放大到约 11–20 ms。现在加载时将所有根命令加入同一规范化索引，保留显式别名优先级、大小写、用户覆盖和帮助学习语义；按键查询直接查索引。[独立 numeric trace](benchmarks/v0.5/beta7-direct-product-root-index-trace.jsonl) 中 root `completion_plan` 约 0.8–1.9 ms，完整候选计算约 1.1–2.4 ms。诊断样本不计入正式计时。

本轮被测 EXE SHA-256 为 `2D1F182F2F406C83E604204C4C7D1D3F139B092DCCA21558944F325A7FFFE101`，实际源码为 `43c41e5` 加根索引未提交修改；EXE 的构建字段仍为 `59dd80f`／dirty，原因是 Cargo 只监控 `.git/HEAD`，同分支提交不改变该文件。本轮随后修正构建监控，覆盖实际 Git ref、packed-refs、index 和产品源码路径，支持 worktree。保留原始报告字段，不把这批带过期构建字段的本机探索提升为正式证据。

关闭 trace、无并发编译的 PS7／PSReadLine 2.4.5 [完整产品 10 对](benchmarks/v0.5/beta7-direct-product-ps7-root-index-10.json)：启动增量 P50 **179.45 ms**，首键回显 P50 **37.44 ms**，首次静态完整菜单 P50 **45.70 ms**（上一批 367.57 ms），首次动态完整菜单 P50／P95 **16.47／30.35 ms**。启动仍失败。

[同一 EXE 的热态 30 样本](benchmarks/v0.5/beta7-direct-hot-ps7-root-index-30.json) 保留十二组全部数组及慢样本；固定环境、实际 direct／pipe、说明开启。miss／hit 仍指会话命令索引缓存，OS 缓存没有清除。

| 场景 | miss 完整菜单 P95 | hit 完整菜单 P95 |
| --- | --- | --- |
| root | 47.55 ms | 32.06 ms |
| git | 46.92 ms | 47.33 ms |
| cargo | 342.70 ms | 371.39 ms |
| js | 46.92 ms | 47.12 ms |
| path | 45.71 ms | 31.03 ms |
| fuzzy | 35.85 ms | 41.14 ms |

十二组仍失败，未扩大到 300 样本。多数 root／path／fuzzy 热态中位数已约 16 ms，但尾延迟和 cargo 迟到安全回调仍未收口；不能用中位数代替 P95，也不能用静态条目代替动态完成。

另外修正字符批量注册覆盖原 `Spacebar` 绑定的风险，真实 PSReadLine API 回归证明预先绑定的 `ForwardChar` 不被改成 `SelfInsert`。该桥接修复和构建字段修复发生在被测 EXE 之后，之后的正式包必须重新测试。新增根索引行为回归后核心测试共 173 项；三个固定组合单层各 8 项通过，Clippy 全 targets `-D warnings`、格式检查及发布证据 Python 21 项通过；远程 CI 结果另记。

### 首次 main 推送与远程 CI 修复

用户授权的快进合并及 main 推送已完成，源码提交为 `db33807a05178ad6ea2551f897c9e1908d238796`。[首次 main CI](https://github.com/Drtxdt/Blueberry/actions/runs/36231575759) 的格式及证据测试通过，但 macOS／Ubuntu Clippy 发现 Windows 条件编译之外的 `unused_mut`；PS7 适配器脚本实际通过后，作业错误地把测试夹具刻意留下的 `$LASTEXITCODE=7` 当作失败。后续修复使用条件变量遮蔽，并按 PS5.1 相同方式在独立 PowerShell 进程运行 PS7 两份脚本、检查实际进程退出码。保持原 Clippy `-D warnings` 和全部回归，不放宽门槛。本机同一 CI 调用方式的 PS7 适配器／启动回归及 Clippy 已通过；远程结果继续跟踪，尚无成功 main 包可作为正式验收对象。

修复提交 `7e4087f` 的[第二轮 CI](https://github.com/Drtxdt/Blueberry/actions/runs/36231709142) 中，三平台核心、格式与 native mouse 通过；交互组在 `startup_host` 的回退 Shell 正常退出步骤超时，命令输出和下一提示符都已显示。探针会把 ConPTY 的光标位置应答错误地当成输入、取消已确认的空缓冲区状态，退出测试也没有等待命令之后的 `prompt_end` 边界。后续保留光标应答前的编辑状态，并在实际下一提示符确认后退出；仍要求 Shell 正常结束，不以 kill 替代退出、不跳过测试。远程运行不得计为成功包，需修复后重新执行矩阵。

提交 `b2124e4` 的[第三轮 CI](https://github.com/Drtxdt/Blueberry/actions/runs/36231943773) 中，回退退出测试全部通过，三平台核心、格式与 native mouse 通过。交互矩阵继续暴露了工作台覆盖、粘贴／多行和菜单接受的失败，包步骤仍跳过。检查发现嵌套宿主没有隔离普通 Worker 结果与工作台模式：打开工作台后迟到的普通 buffer／completion 仍会规划并覆盖表单条目。后续取消普通查询、禁止工作台期间普通规划和普通结果应用，保留已确认整行／光标；表单值仍由共享 FormValues 路径更新。接受测试增加持续检查选中操作身份，避免只看到短暂的工作台帧便宣称通过。

另外两份终端等待辅助函数每 500 ms 无输出就报错，实际未使用它们声明的 20 秒截止时间；后续改为使用剩余截止时间，仍逐帧检查同一条件、保留原截止值及正常退出断言。PS7 OSC／pipe 的本机完整 `terminal_beta` 11 项和 `terminal_modes` 3 项分别通过（各 2 项原设计 ignored）。所有这些都是功能回归调整，不更改 50／20 ms 性能门槛，也不将第三轮远程失败作业记成通过。

工作台隔离修复后，PS5.1／PSReadLine 2.4.5 的 OSC／pipe 本机 `terminal_beta` 11 项及 `terminal_modes` 3 项也各通过，包括新增的迟到候选身份保持检查；Clippy 全 targets 通过。远程需再运行，同一 CI 包尚未可用。

提交 `f05f729` 的[第四轮 CI](https://github.com/Drtxdt/Blueberry/actions/runs/36232445247) 中，PS7 OSC／pipe 两作业完整通过，包括 pipe 作业的单层回归；三平台核心、格式、native mouse 通过。PS5.1／2.4.5 在下载前发现运行器未注册 PSGallery，后续仅在缺失时注册默认库并继续严格选择原固定版本。PS5.1／2.0.0 的多行测试在空提示符状态下超时，其余 10 项 terminal_beta 通过；已有日志缺少初始化等待阶段上下文，不能宣称原因已消除。补充 capabilities、prompt、command snapshot 和 awaited event 的精确诊断，并定向复测；保留该失败，包步骤仍跳过。

## 7efe7a3：完整 main CI、不可变工件与重新测量

源码提交 `7efe7a3d89426bef50978e76c4c6b465a973ec91` 的[第五轮 main CI](https://github.com/Drtxdt/Blueberry/actions/runs/36232787492) **十个作业全部通过**：格式与发布证据 Python 21 项、Windows／Ubuntu／macOS 核心、PS5.1／PSReadLine 2.0.0 和 2.4.5 的适配器／启动／嵌套 OSC／pipe／单层、PS7／2.4.5 的 OSC／pipe（pipe 作业包含单层）、Windows 2025 native mouse，以及构建与包自测。第四轮 PS5.1／2.0.0 初始化超时保留为历史失败；本轮观察到完整矩阵成功，不据此声称已找出该间歇失败的唯一原因。

### 包身份与隔离安装升级回滚

[机器可读身份记录](benchmarks/v0.5/beta7-ci-7efe7a3-identity.json) 保存源码、CI 十个 job 身份、包摘要、自测绑定和未完成状态。它是候选身份与探索记录，**不是正式 release-evidence.json**，不能通过它绕过发布校验。

| 对象 | SHA-256 |
| --- | --- |
| GitHub artifact 10902873054／blueberry-windows-x64 的外层 ZIP | `0219A44E124E2000E688FDEF134153545B76BFD0EBFAEA43CD8D030873D3B762` |
| 包内发布 ZIP blueberry-v0.5.0-beta.7-windows-x64.zip | `5F40CBB9213EB411FD76584A0838490B201923460D606AAC658DAFEFEF990EA0` |
| 包内 blueberry.exe（7,378,944 bytes） | `76B02B562E6F6B2E5F37D7AA808F9CE15216697D875A870E4398186ADA395D50` |

外层 ZIP 与 GitHub API artifact digest 相同，发布 ZIP 与工件内 `.sha256` 相同；逐项核对 `release.json` 的 27 个文件摘要、字节数与路径范围。EXE `doctor --json` 报告上述完整源码提交、`dirty=false`、`profile=release`、构建时间 `1790415078`。后续所有本节测量均直接运行下载包内 EXE，没有用本机重新构建 EXE 替换它。工件记录的过期时间为 2026-12-25T09:26:37Z；须保留下载字节，不能在过期后重新构建并沿用本节结果。包的 `performance_artifacts=[]`，尚未形成通过验收的发布资产。

本机用该 EXE 执行 `scripts/test-release.ps1 -PreviousExePath <已核验公开 beta.6 EXE>`，正常退出码 **0**，隔离安装、升级、回滚、篡改拒绝、写入中断与卸载保留自测通过；回滚 EXE 校验为公开 beta.6 的 `C27404F6…C7FB86`，用户配置保留。旧 EXE 被测试主动锁定时的“升级失败并恢复”是预期负例，随后正常升级和回滚成功。Windows Terminal settings／JSONC 使用隔离夹具，不改用户真实 Terminal 设置。旧版自测包使用真实公开 beta.6 EXE 和本轮安装脚本构造；不把它称作对原始 beta.6 ZIP 的完整旧安装器验收。这项结果不替代真实 Windows Terminal 输入法与视觉人工验收。

### 同一 CI EXE 的探索性能（未达标）

固定机器 LIUHETONG、30×120、现有 profile 和平衡电源策略；三个组合各 10 对 plain／完整 direct 产品交替启动，实际传输 pipe、自动菜单开启、trace 关闭。串行测量期间无编译、安装自测或其他重负载任务；保存全部原始数组。下表是接收线程时间戳形成的 VT 观察终端完整内容，不是物理屏幕像素时间。

| Shell／PSReadLine | 启动增量 P50 | 首键回显 P50 | 首次静态完整菜单 P50 | 首次动态完整菜单 P50／P95 | 原始数组 |
| --- | --- | --- | --- | --- | --- |
| PS5.1／2.0.0 | 186.07 ms | 33.64 ms | 33.64 ms | 16.58／30.04 ms | [10 对](benchmarks/v0.5/beta7-ci-7efe7a3-product-ps51-200-10.json) |
| PS5.1／2.4.5 | 188.01 ms | 34.68 ms | 34.68 ms | 17.15／35.65 ms | [10 对](benchmarks/v0.5/beta7-ci-7efe7a3-product-ps51-245-10.json) |
| PS7／2.4.5 | 157.61 ms | 37.95 ms | 43.22 ms | 16.32／28.54 ms | [10 对](benchmarks/v0.5/beta7-ci-7efe7a3-product-ps7-10.json) |

三个启动组均失败，未扩大到正式 30 对。PS7／2.4.5 的[六场景热态数组](benchmarks/v0.5/beta7-ci-7efe7a3-hot-ps7-30.json) 每场景 miss／hit 各 30 个样本，实际 direct／pipe、说明开启，未降级，完整动态菜单才计为完成。命令索引 miss／hit 是会话缓存条件，不是每键提供器冷启动；OS 文件缓存保留，首次查询另列，不混入热态。

| 场景 | 命令索引 miss 热态 P95 | 命令索引 hit 热态 P95 |
| --- | --- | --- |
| root | 33.10 ms | 37.44 ms |
| git | 341.37 ms | 37.52 ms |
| cargo | 345.54 ms | 41.39 ms |
| js | 48.42 ms | 350.22 ms |
| path | 37.42 ms | 47.88 ms |
| fuzzy | 37.51 ms | 29.63 ms |

十二组均失败，保留约 350 ms 迟到样本；不扩大到 300 样本，也不将较快首个静态帧或中位数视作通过。该批只覆盖 PS7／说明开启探索，不是三组合、说明开／关正式矩阵，不替代四方对照或嵌套性能报告。

### 剩余发布阻断与下一轮入口

性能仍是主动工程阻断：先分离启动脚本与约 6.5 万键注册成本，再定位 4 ms 即时预算之后安全回调刷新、菜单输出与 VT 尾延迟；每次产品代码变化须获得新 CI EXE 并重测受影响结果。不能提前显示未经 PSReadLine 确认的输入或延长键处理等待来伪造达标。

此外仍欠正式三组合启动／热态矩阵、说明关闭变体、嵌套兼容性能、四方交替对照、进程树空闲资源与 Windows Terminal 人工验收，以及前文列出的部分单层快捷键／Shell 元数据／真实剪贴板输入法／协议故障全流程覆盖。目标用户试用为 `waived_by_user`、`executed=false`，不是通过。成功 CI 和隔离安装回滚已补齐，公开 beta.6 来源已核验，不再列作未取得。当前不创建 beta.7 标签、不公开发布、不切换默认宿主。

报告和原始数组作为后续证据提交；本节测量源码始终是 `7efe7a3d89426bef50978e76c4c6b465a973ec91`。后续 main 的文档提交和它们新生成的 CI 工件不自动继承本节结果；若未来发布改用另一包，必须重新绑定及验收。

# 0.5 Beta 性能与验收记录

本页记录当前 release 二进制的正式计时。两项门槛均未达到：首次输入增量 P50 为 **143.4021 ms**（门槛 ≤50 ms），三个宿主变体的热态菜单 P95 均超过 20 ms。报告保留完整原始数组、环境元数据和独立统计复核；没有剔除异常值，也没有把未配对的小差异解释为因果收益。

## 正式结论

| 指标 | 正式结果 | 门槛 | 结果 |
| --- | ---: | ---: | --- |
| probe --with-profile 配对首次输入增量 P50（30 对） | 143.4021 ms | ≤50 ms | 未达标 |
| OSC + descriptions 热态菜单 P95 | 36.7951 ms | ≤20 ms | 未达标 |
| OSC - descriptions 热态菜单 P95 | 37.2417 ms | ≤20 ms | 未达标 |
| pipe + descriptions 热态菜单 P95 | 37.0887 ms | ≤20 ms | 未达标 |

热态门槛按六个场景、两个缓存状态分别验收：root、git、cargo、js、path、fuzzy 各有 miss/hit 两组。每个变体的 12 组均为 failed，合计 36/36 组失败；每组都观察到 300/300 个样本，实际通道与目标通道一致，transport_degraded=false。上述 P95 是目标 hot_menu 的总体值；逐场景验收值见下表。

## 测量设计与环境

- 正式数据来自 release 构建，测量源码为 commit aa1457cd1a1d056931f68cd77dfbdb72e432dfd7（短号 aa1457c），可执行文件 SHA-256 为 8064AC6293EE3CE3C7E909BFC56A605F3DAE8A413B914B7129B49740F4F4219A。静态 CRT 已确认，运行时只依赖 Windows 系统 DLL。
- 环境为 Windows 10.0.26200.0、13th Gen Intel Core i9-13900HX（32 logical processors）、Rust 1.94.1、PowerShell 7.6.6、PSReadLine 2.4.5。release profile 为 lto thin; codegen-units=1。
- beta-environment.json 的时间戳是环境采集时间（2026-09-13T12:45:30.8340674+08:00），不是正式计时开始时间；正式计时日期为 2026-09-13。测试会话设置 PSReadLine HistorySaveStyle=SaveNothing，保留 profile；普通使用维持原历史策略。正式运行关闭 trace，OS 文件缓存未清空。
- 构建和夹具位于 C 盘；E 盘在本轮离线。源、包和回滚版留在 C 盘，等待 E 盘恢复后迁移。
- 每个变体包含 6 个场景 × 2 个缓存状态 × 300 个热态样本 = 3600 个样本，三变体合计 10800 个；每个场景/缓存状态建立 30 个会话，每会话 10 次查询。pipe 变体共 360 个真实 pipe 会话。十个固定候选（0..9）循环使用，夹具不代表超大仓库。
- miss/hit 只表示持久化的完整 commands.json：miss 使用空数据目录，hit 复用完整命令缓存并检查其完整性。Git/Cargo/npm 等 provider cache 位于进程内，每个新进程都是冷启动。三个变体顺序运行、相互不配对，差异小于 1 ms 不作因果归因。
- 统计对有限数值排序后使用标准 median；P95 使用 nearest-rank ceil(0.95 × n)（一基），没有删除离群样本。每次计时清除旧输入和菜单；完整动态结果还要求目标候选、状态栏可见且没有加载标记。

宿主数据经过外层测试 ConPTY，表示 PowerShell + ShellSense + ConPTY 的可观察交互时间，不能当作 Windows Terminal 视觉帧或真实 Windows Terminal IME 验收。记忆体是 Rust ShellSense host 自身 working set，不是完整进程树；吞吐和独立 pwsh 控制使用同一固定文本。

## 六场景 miss/hit P95

静态 root/fuzzy 使用 transport_eligible_first_candidate_menu；动态 git/cargo/js/path 使用 transport_eligible_dynamic_complete_menu。数值单位均为 ms，所有单元格的样本数均为 300，门槛为 20 ms。

| 场景 | OSC + desc miss | OSC + desc hit | OSC - desc miss | OSC - desc hit | pipe + desc miss | pipe + desc hit |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| root | 38.2518 | 36.9340 | 37.2869 | 36.6198 | 37.0887 | 36.7050 |
| git | 36.9425 | 35.6365 | 36.6352 | 36.4968 | 36.7074 | 34.6678 |
| cargo | 37.4235 | 37.5526 | 37.6826 | 37.5644 | 37.1924 | 37.7900 |
| js | 37.0048 | 36.3902 | 37.5810 | 37.8802 | 36.7634 | 37.0388 |
| path | 36.3632 | 36.4359 | 37.2213 | 37.6011 | 37.1655 | 37.3092 |
| fuzzy | 36.3889 | 36.5415 | 36.5771 | 36.4953 | 36.9579 | 38.0417 |

## 首查、回显与资源诊断

首个完整动态结果单独统计，不混入热态菜单：

| 变体 | 首个完整动态结果 n | median | P95 | 首个查询 n | median | P95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| OSC + descriptions | 240 | 63.9488 | 100.1595 | 360 | 61.3308 | 97.1034 |
| OSC - descriptions | 240 | 65.6057 | 98.8091 | 360 | 61.9132 | 96.8168 |
| pipe + descriptions | 240 | 104.5588 | 136.1048 | 360 | 100.5417 | 134.2970 |

整体 3600 次输入回显和 2400 次完整动态菜单的复核如下：

| 变体 | 输入回显 median / P95 (n=3600) | 完整动态菜单 median / P95 (n=2400) |
| --- | ---: | ---: |
| OSC + descriptions | 28.9830 / 36.7916 ms | 28.4202 / 36.6484 ms |
| OSC - descriptions | 30.4393 / 37.2383 ms | 30.6189 / 37.4461 ms |
| pipe + descriptions | 29.3893 / 37.0851 ms | 29.1075 / 36.9761 ms |

Rust host working set、200 ms 空闲 CPU 采样和固定文本吞吐：

| 变体 | prompt WSet median / P95 (n=360) | menu WSet median / P95 (n=3960) | idle CPU median / P95 / max (%) | 吞吐 median / P95 (bytes/s, n=360) |
| --- | ---: | ---: | ---: | ---: |
| OSC + descriptions | 13,201,408 / 14,356,480 bytes | 14,209,024 / 14,733,312 bytes | 0 / 0 / 7.802 | 1,678,774.01 / 1,778,736.77 |
| OSC - descriptions | 13,185,024 / 14,331,904 bytes | 14,221,312 / 14,749,696 bytes | 0 / 0 / 7.789 | 1,681,773.32 / 1,788,263.16 |
| pipe + descriptions | 13,668,352 / 14,811,136 bytes | 14,675,968 / 15,171,584 bytes | 0 / 0 / 7.799 | 1,766,125.66 / 1,835,284.76 |

CPU 数值由 Windows GetProcessTimes 在约 200 ms 窗口计算，并把一个 logical core 的 100% 作为基准；这是空闲窗口诊断，不是峰值 CPU 报告。working set 只属于 Rust 宿主进程，不能推断整个进程树或 Windows Terminal 内存。

独立 pwsh 控制（每场景 10 次输入回显；不含 ShellSense host，不代替正式验收）为：

| 变体 | pwsh 回显 median / P95 (n=60) | pwsh 吞吐 median / P95 (n=6, bytes/s) |
| --- | ---: | ---: |
| OSC + descriptions | 15.2615 / 30.5674 ms | 3,841,825.46 / 4,037,145.98 |
| OSC - descriptions | 15.4862 / 28.9834 ms | 4,003,911.99 / 4,080,900.67 |
| pipe + descriptions | 14.9083 / 28.2405 ms | 3,869,069.72 / 4,126,945.64 |

## probe --with-profile 计时边界

beta-05-startup-profile-30.json 的 30 对数据使用 release、probe --with-profile --iterations 30、no_profile=false、embedded adapter；这是一个 probe 进程内交替启动 30 对 fresh pwsh。paired_first_input_delta 的独立 median/P95 为 143.4021/203.0701 ms。这个 probe 测的是新 pwsh 适配器相对 plain pwsh 的首次输入增量，不是完整 ShellSense host 进程启动，也不是完整动态菜单。宿主的首查、回显和菜单结果由上面的 host + ConPTY 测量分别覆盖。

## trace 诊断（不用于验收）

[beta-05-trace-diagnostic.json](benchmarks/v0.5/beta-05-trace-diagnostic.json) 是从 miss/hit 两份 trace JSONL 合并出的纯 JSON，保留完整事件数组：miss 258 个事件、hit 223 个事件。它明确标记 trace_enabled=true、profile_mode=no_profile、formal_acceptance=false 和 causal_comparison=false，只用于定位阶段，不用于正式分位数或因果对比。readline_init 时长分别为 miss 113.4427 ms、hit 114.0013 ms；这不能被解释成正式测量收益或损失。

## 当前取舍与剩余瓶颈

正式门槛仍未通过。默认保留 OSC 和中文说明：本轮三个变体依次运行，不能把不足 1 ms 的差异解释为通道或说明开关的因果收益；pipe 还增加了本次首次完整动态结果的等待。C# 早期实验未满足保留条件，生产版本不加载该程序集。

诊断中 PSReadLine 初始化仍约 114 ms，其中按键快照、公共键仲裁和内部键注册合计约 45 ms，其余部分尚未完全分段；trace 的阶段存在包含关系，不能相加当作启动总耗时。下一步应细分初始化中的调用、脚本解析与模块准备成本，再决定是否能在首次提示符前安全减少工作，不能把它们集中推迟到第一次输入。

Rust 首词查询的中位数约 0.13 ms，明显小于实际宿主的输入回显与菜单等待；独立 pwsh 和嵌套宿主的对照提示，ConPTY 转发、调度和输出处理仍需进一步定位。当前证据不能证明 Windows 存在固定 16 ms 下限，也不能把菜单全部延迟归因于候选筛选。集中写出已减少每批写调用次数，但小样本没有证明延迟下降。

## 早期对照与历史背景

- [beta-bridge-ab-30.json](benchmarks/v0.5/beta-bridge-ab-30.json) 保留 C# bridge 的 30 对早期原始数据。它使用旧 key 配置和 proofIO/绑定证明文件 I/O；first_input_difference_ms（baseline - bridge）median 为 -37.88215 ms，表示该样本中 bridge 更慢，不能当作当前正式 A/B 结论。
- [beta-05-before-input-batch.json](benchmarks/v0.5/beta-05-before-input-batch.json) 与 [beta-05-after-input-batch.json](benchmarks/v0.5/beta-05-after-input-batch.json) 是小样本对照：热态 P95 约从 37.61 ms 变为 38.15 ms，没有证明延迟收益；after 版本完成了每批一次的集中写出。
- 0.2 历史参考为启动 119.7114 ms、热态 37.6 ms。当时 E 盘在线，本轮 E 盘离线，不能把两者当作同条件同比速度收益。

## 原始文件、元数据与复核

以下文件是交付包需要的 JSON/Markdown 清单；四份正式 host/startup 原始 JSON、环境 JSON、早期对照 JSON 均逐字复制并保留完整数组。统计和样本数的独立复核见 [beta-05-stats-recheck.json](benchmarks/v0.5/beta-05-stats-recheck.json)，环境、源码/可执行文件标识和复制 SHA-256 见 [beta-05-metadata.json](benchmarks/v0.5/beta-05-metadata.json)。

- [beta-05-startup-profile-30.json](benchmarks/v0.5/beta-05-startup-profile-30.json)
- [beta-05-host-osc-descriptions-300.json](benchmarks/v0.5/beta-05-host-osc-descriptions-300.json)
- [beta-05-host-osc-no-descriptions-300.json](benchmarks/v0.5/beta-05-host-osc-no-descriptions-300.json)
- [beta-05-host-pipe-descriptions-300.json](benchmarks/v0.5/beta-05-host-pipe-descriptions-300.json)
- [beta-environment.json](benchmarks/v0.5/beta-environment.json)
- [beta-05-metadata.json](benchmarks/v0.5/beta-05-metadata.json)
- [beta-05-stats-recheck.json](benchmarks/v0.5/beta-05-stats-recheck.json)
- [beta-05-trace-diagnostic.json](benchmarks/v0.5/beta-05-trace-diagnostic.json)
- [beta-bridge-ab-30.json](benchmarks/v0.5/beta-bridge-ab-30.json)
- [beta-05-before-input-batch.json](benchmarks/v0.5/beta-05-before-input-batch.json)
- [beta-05-after-input-batch.json](benchmarks/v0.5/beta-05-after-input-batch.json)

复核的正式计数为每变体 3600、合计 10800 个热态样本，pipe 360 个会话；36 个场景/缓存/变体验收子组均为 300 个观察样本且统计值与原始 JSON 内嵌值一致。原始复制共 8 个文件并逐字校验；早期对照共 3 个文件，诊断 trace 合并为 1 个纯 JSON。

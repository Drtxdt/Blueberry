# 16 项评审实施记录

2026-10-24 第三轮已完成三组合独立分配诊断，随后移除菜单布局每次构造／复制整份配置的成本，改为借用配置并传入行数与边框限制。36 个诊断布局逐帧一致，每例 100 次渲染，每帧减少 29–30 次分配请求；七项菜单回归、默认全目标及独立诊断 Clippy 通过。该变更仍是待测候选，分配次数不替代两批延迟／资源 A/B，也未作保留判定。[实际宿主诊断](benchmarks/v0.6/review-20261018/finalization/r3-baseline-diagnostics-20261024.json)、[布局归因与逐帧对照](benchmarks/v0.6/review-20261018/finalization/r3-menu-layout-allocation-20261024.json)。

2026-10-24 20:21（UTC+8）串行队列已停止：R2 首组 path 第 29 对的命中缓存被重建。前后 1,765 条命令完全一致，仅 C:\Windows 目录时间戳变化；旧夹具仅冻结 System32，现扩展根目录命令快照，三个针对性测试通过，不放宽命中断言。R1 资源及包等待控制器均因前置未完成而停止，后文“等待中”是历史状态。全部进程退出后，同轮 f8 原包已单独完成隔离核心安装、真实 0.5.2 升级／回滚／再升级和组件重装逐文件校验，设置、配置及用户 PATH 保留；GUI 和三轮性能仍未完成。[失败原因](benchmarks/v0.6/review-20261018/finalization/r2-windows-directory-churn-20261024.json)、[本机隔离验收](benchmarks/v0.6/review-20261018/finalization/f8-isolated-acceptance-20261024.json)。

2026-10-24 当前完整 CI 40090066092 的 10 项必需任务全部通过；同轮核心／增强 ZIP 已封存，分别核验 28／84 个文件。安装证据绑定相同包摘要，覆盖 0/1/5 历史、真实 0.5.2 升级、PS7／PS5.1 组件生命周期、旧捆绑迁移及两组真实磁盘满／权限恢复。新本机隔离验收控制器已启动等待，冻结 124 个文件，只有 R2 及 R1 资源采样成功结束才运行；尚未执行这一转换，不计最终验收通过。[同轮包](benchmarks/v0.6/review-20261018/finalization/ci-f8-sealed-packages.json)、[安装审计](benchmarks/v0.6/review-20261018/finalization/ci-f8-installation-audit.json)、[后续队列](benchmarks/v0.6/review-20261018/finalization/package-continuation-20261024.json)。

2026-10-20 最新完整 CI 39416003087 全部必需任务成功；同轮 180 对冷/暖实际 profile、36 项打包工作流及安装/恢复回执已核验。构建提交是 477e5af4a44f4cace4c99c17f8b64cb8154531af；profile 性能未达目标，不能标记可发布。交付校验器的固定分支手动 CI 与条件跳过任务规则经隔离定点 CI 39421329871 验证（110 项、9 项平台跳过），未触发另一轮完整终端测试。[完整包审计](benchmarks/v0.6/review-20261018/finalization/ci-ec-packaged-audit.json)、[定点校验](benchmarks/v0.6/review-20261018/finalization/validator-789651e-ci.json)。

2026-10-20 打包验证审计：CI 39399839301 的原始记录确认三个编辑器组合各 12 项工作流在关闭追踪时通过，90 对真实自动 profile 样本及恢复记录完整；0/1/5 历史、公开 0.5.2 升级和两组物理存储故障回执已核验。该包构建自 PR 合并提交 20bcbb104e269f6af014e0013c082535c0472949，不能误标为分支头 4088419。其 profile 性能未达 50 ms，最终包、性能及人工门槛保持未完成。[审计](benchmarks/v0.6/review-20261018/finalization/ci408-packaged-audit.json)。

2026-10-20 云端复验：ec70d59 的定点 CI [39414992059](https://github.com/Drtxdt/Blueberry/actions/runs/39414992059) 三组合全部通过。三个工件 SHA-256 和六份原始报告已在本机核验：冷缓存各两对均为 rebuild，暖缓存各两对均为 hit，暖缓存准备另计。这 12 对是 debug 功能验证，不用于正式性能或版本判定。[下载与报告审计](benchmarks/v0.6/review-20261018/finalization/cache-ec70d59-ci.json)。完整 CI 39416003087 正在执行；R2 新批次于 19:21（UTC+8）启动，仍在首个 before 矩阵中，尚无完整轮次结论。

2026-10-20 长路径缓存修复：冷/暖定点 CI 39401529774 未通过；已核验的两个 PS5.1 工件均为扫描成功但缓存 write_failed。新增超过 MAX_PATH 的用例在首次发布时复现 Win32 错误 3。共享原子替换现只规范化父目录为 Windows 扩展路径，保留最终文件名的替换语义。缓存 5 项、三个真实编辑器组合的冷/暖各 2 对、配置 10 项、收藏 6 项及 Clippy 通过；本地原始 ZIP 已封存，尚待云端复验及上传。[证据](benchmarks/v0.6/review-20261018/finalization/cache-long-path.json)。

R2 的 r2-stability-20261020 协调器与 worker 在会话中断后均不存在；只有一份启动报告完整，不能计作完整轮次。原回执未修改，另存[只读审计](benchmarks/v0.6/review-20261018/finalization/r2-stability-readonly-audit.json)。新 r2-breakaway-20261020 计划摘要 4B1A18ECE72E574C615E5C287961607A7DA30FFD528236F07CC8C43CC6C12638 已准备；显式 Windows breakaway 的 30 秒短任务在启动调用结束后完成，长批次尚待实际验证。

2026-10-20 稳定性门槛通过：固定提交 312f418 的单轮 CI 39393219193 和 30 轮 CI 39394720887 均通过。重复包含追踪开/关各 450 次 Direct、180 次 Nested；540 份窄屏/宽屏/取消后的原生快照均保留提示符且无历史候选残留，重复期间归属诊断关闭。三个原始工件摘要已核验。本地回归原始证据已上传至 run 39394098975 / artifact 12251427556，回下载及内部九份 ZIP 摘要全部匹配，一次性执行器已注销。现可继续完整 CI 和正式性能工作；这不是最终包或可发布判定。[门槛记录](benchmarks/v0.6/review-20261018/finalization/stability-gate-312f418.json)、[原始证据交付](benchmarks/v0.6/review-20261018/finalization/raw-stability-delivery-20261020.json)。

2026-10-20 最新重复结果：8f98815 的 CI 39385912604 完成 900 次 Direct 和 150 次 Nested，PS5.1/2.4.5 Nested/OSC 首次变宽检查失败。原生屏幕保留提示符，但 VT 观察器丢失其显示；原生历史另有一行旧候选残留。可控 ConPTY 用例随后复现了缩放事件到达前使用旧尺寸绘制候选的缺陷：越界行引发滚动，并遗忘旧菜单清理身份。现遇到实际视口与模型不符时先退掉旧菜单，等待新尺寸后再绘制。修复后的竞态 30 次、四项隔离回归、六组交互/原生历史共 12 项及 Clippy 通过，六份归属日志未发现未匹配候选行。缩放验收改用原生宽屏/取消快照，并严格拒绝提示符丢失和历史候选残留，等待期限不变。新提交的单轮及重复 CI 仍须通过；完整 CI、性能和最终包尚未放行。[重复结果](benchmarks/v0.6/review-20261018/finalization/stability-8f98815-repeat30.json)、[可控复现与修复](benchmarks/v0.6/review-20261018/finalization/overlay-stale-geometry.json)。

2026-10-20 软换行根因：fff465a 单轮 CI 39383272284 全部通过，但六份归属日志都显示变宽后相邻菜单行被合并。真实 ConPTY 往返缩放夹具已复现：清字符保留的软换行标记会被窄菜单继承。现仅在自有菜单行清理/绘制后显式结束行，并跳过物理底行以避免滚动；同尺寸缩放也完整重画。本机严格检查提示符、历史及全部菜单边框，30 次往返回归及六组交互/原生历史共 12 项通过；六份新归属日志的未匹配候选行均为零。云端新提交和关闭诊断的重复验证仍待完成。[复现与验证](benchmarks/v0.6/review-20261018/finalization/overlay-soft-wrap-retirement.json)。

追加清理顺序验证：真实 ConPTY 夹具确认待写出的新提示符可能被依据旧屏幕生成的清理指令擦除；现将前序输出同步到终端后再读取菜单归属，两个隔离回归及 PS5.1/2.0.0 pipe/OSC 单轮通过。单轮 Nested CI 启用明确标记的归属诊断，30 轮保持关闭；失败时补充最终原生屏幕。尚未将此修复视为云端残留或提示符问题的完整结论。[复现与边界](benchmarks/v0.6/review-20261018/finalization/overlay-pending-output.json)。

2026-10-20 后续结果：14a6467 单轮 CI 39381067743 三组合通过，本机六组 Unicode/RGB 历史各 30 次共 180 次通过。其云端原生快照仍有一个残留样本，清理问题未关闭。4a6ce7a 的 30 轮 CI 39379818074 中 PS5.1/2.4.5 全部通过；PS5.1/2.0.0 底部菜单在未收完输入时读取了部分 VT 帧，改为完整帧确认后，本机追踪开关六组各 30 次通过；PS7 Nested OSC 第 21 次缩放后 VT 观察器未见提示符，保持严格断言并继续诊断，不宣称修复。完整 CI、性能和最终包继续等待稳定性门槛。[重复 CI](benchmarks/v0.6/review-20261018/finalization/stability-4a6ce7a-repeat30.json)、[底部菜单复验](benchmarks/v0.6/review-20261018/finalization/stability-bottom-complete-frame-repeat30.json)、[原生历史](benchmarks/v0.6/review-20261018/finalization/stability-native-history-14a6467-repeat30.json)。

2026-10-20 菜单清理修复：4a6ce7a 单轮 CI 39378922073 三组合通过，30 轮 CI 39379818074 独立进行；原生快照审计仍在六个样本之一发现历史菜单残留。已用独立 ConPTY 回归复现终端先缩放、清理先于缩放事件的竞态：旧代码按旧坐标检查并遗忘已移动的菜单。最小修复读取真实视口尺寸判断重排，清理仍仅匹配自有内容；本机竞态回归 30 次及六组 Unicode/RGB 历史与后续补全通过，修复后的云端复验待完成。超时保持 20 秒，完整 CI、性能与最终包仍未放行。[单轮原始证据](benchmarks/v0.6/review-20261018/finalization/stability-4a6ce7a-single.json)、[竞态复现与修复证据](benchmarks/v0.6/review-20261018/finalization/overlay-unobserved-resize.json)。

2026-10-20 最新定点结果：`e47e4d0` 的 Direct 开关追踪在三个编辑器全部通过；Nested 五组通过，PS5.1/2.0.0 OSC 因整屏仍含旧菜单的 `gi` 别名而超时。原始 buffer 事件确认当前输入为 `git`，当前提示符下方菜单也已更新；调整当前菜单断言后，本机该组合 30 次通过。上方历史菜单碎片单独保留为未解决的清理问题，不因断言调整而标记修复。完整 CI、性能和最终包仍未放行。[原始证据与未解决项](benchmarks/v0.6/review-20261018/finalization/stability-e47e4d0-single-failure.json)。

2026-10-20 稳定性复验：冻结基线 `c0aa65e`，隔离分支 `codex/stability-diagnostics` 的 `ad34ce3` 已通过三组合单轮 CI `39252218595` 及 30 次重复 CI `39253381002`（450 次 Direct、180 次 Nested，共 630 次）。Direct 重复为追踪开启；诊断注入另行覆盖开关两种模式。当前补齐关闭追踪的实际交互，本机三组合共 15 项定点测试及六组 Nested 原生历史测试全部通过，未修改产品渲染、未延长超时。下一固定提交将复验追踪开关两种模式；PR #4 保持 Draft，完整 CI、性能与最终包仍待完成。证据：[重复 CI](benchmarks/v0.6/review-20261018/finalization/stability-ad34ce3-repeat30.json)、[本机复验](benchmarks/v0.6/review-20261018/finalization/stability-local-trace-off-native-history.json)。

2026-10-20 追加复验：`adf8cdd` 单轮 CI `39375377019` 的三个编辑器均在关闭追踪的工作台首次滚动附近未打开表单，不能进入新一轮 CI 重复或最终冻结。开启追踪的工作流和关闭追踪的缩放、底部菜单、Vi 用例均通过。原始夹具校验后发现测试以分段 VT 中的选中行变化判断完整画面；现验证仅在功能测试中启用的完整帧观察器，复用已有输出结束标记，不修改渲染或延长超时。分段标记及同次读取含下一帧片段两项回放测试通过，云端修复复验仍待执行。[失败证据](benchmarks/v0.6/review-20261018/finalization/stability-adf8cdd-single-failure.json)。另六组 Nested 原生历史各 30 次、共 180 次通过；本机原始 ZIP 已封存，尚未上传 Actions，不代替 GUI 验收。[重复证据](benchmarks/v0.6/review-20261018/finalization/stability-native-history-adf8cdd-repeat30.json)。

R2 durable 的原协调器/工作进程已不在运行，冻结状态文件保持原样；不完整数据不参与版本判定，稳定性完成前不重启测量。此前 `6bb6cc2` 全绿工件继续仅作历史证据。真实 PS5.1 profile 夹具及最终 GUI 验收仍须独立核实。

基线：`65332cf3622a5a15547c02094d2bd2e0682b68f6`，分支 `codex/v053-reliability-performance`，PR #4。
本轮不发布、合并或替换本机安装。以下是实施状态，不是发布验收结论。

| 项目 | 状态 | 已实施及剩余门槛 |
|---|---|---|
| 01 启动入口 | 实施中 | 已增加 fragment 的 Shell/安装/状态/移除；安装器已推荐官方入口；真实启动四路径和配对测量待完成 |
| 02 工具页 | 回归通过 | 功能键替代字符操作、成功后更新配置、完整自有区域清理；2 项真实 ConPTY 回归通过，最终包仍须复验 |
| 03 配置保存 | 回归通过 | 共享加锁、原子替换、字段差异合并；并发、冲突、错误文件、句柄占用及进程中断测试通过 |
| 04 参数与身份 | 回归通过 | 稳定项目动作 ID、共享结构化调用和实际 Shell 参数模式；普通动态候选、字面路径续查及 exe/cmd/bat × PS5.1/PS7 Legacy/Standard/Windows 原生 argv 回显通过；最终包复验待完成 |
| 05 表单上下文 | 回归通过 | 模板/前置值/目录/环境和请求身份校验、返回上一字段、可恢复错误；三个支持组合的 Direct/Nested 损坏 Cargo 表单取消恢复及后续补全/原生历史通过，最终包复验待完成 |
| 06 维护事务 | 回归通过 | 不可变备份清单与校验链 WAL、旧 schema 恢复、0/1/5 历史矩阵、真实公开 0.5.2 升级及 300 秒异步生命周期通过；WAL 写入失败不修改目标通过；最终包故障边界复验待完成 |
| 07 包拆分 | 实施中 | 默认核心包和按包类型校验已实现；增强组件独立生命周期、设置保留与卸载重试在 PS5.1/PS7 通过，CI 分离工件；旧捆绑迁移、核心回滚/再升级和 PS5.1/PS7 真实磁盘满/ACL 恢复重试通过；最终冻结验收待完成 |
| 08 能力与渲染 | 实施中 | 实际握手/安装/编译能力独立报告；Direct/Nested 普通模式使用下方自有区域，共享状态与普通终端功能 CI 通过；原生历史属性专项通过；真实 GUI 宿主及最终包人工验收待完成 |
| 09 系统缓存 | 功能回归通过 | 持久身份、加锁重建、原子发布、损坏拒绝与显式刷新已实现；修复长路径发布失败后，ec70d59 三组合冷/暖各两对云端报告及工件摘要已核验；正式性能与最终包身份仍待验收 |
| 10 Legacy | 回归通过 | 未匹配编辑器只保留手动入口，服务连接与自动菜单独立；手动补全/工作台/取消/原生编辑回归及 38e0a43 完整 CI 通过；最终包复验待完成 |
| 11 三轮性能 | 正在独立重采 | 稳定性门槛已通过；中断批次保留且排除。新 r2-breakaway-20261020 已于 2026-10-20 19:21 启动，首个热态分组完成，完整 AB/BA 尚未完成；第一轮新入口、第三轮及最终矩阵仍待完成 |
| 12 共享契约 | 回归通过 | Direct/Nested 共用 Workbench 状态、事件转换、表单/导航/取消/错误；可见帧和实际缓冲区修订校验保留；工作流刷新与完整功能 CI 通过，最终包复验待完成 |
| 13 Git | 自动回归通过，待最终包 | 分支、worktree、log/diff 表单与中文描述已实现；完整 CI 39399839301 通过，三组合关闭追踪的打包工作流回执已核验；最终冻结包及真实 GUI 验收待完成 |
| 14 Python | 自动回归通过，待最终包 | uv venv/sync/run/pytest、依赖组与静态测试发现已实现；配置与错误诊断、完整 CI 及三组合关闭追踪的打包工作流通过；最终冻结包及真实 GUI 验收待完成 |
| 15 Rust | 自动回归通过，待最终包 | build/test、package→features/test target、修改前置参数清理下游值及错误恢复已实现；完整 CI 及三组合关闭追踪的打包工作流通过；最终冻结包及真实 GUI 验收待完成 |
| 16 最终验收 | 验证中 | 6bb6cc2 push CI（39228676557）全部功能及打包任务通过，已生成同源核心/增强工件；尚未完成三轮性能、版本判定、最终冻结或人工验收，不可标记可发布 |

## 验证记录

命令在仓库根运行，将 `TEMP`、`TMP` 指向仓库 `target`，保证测试临时文件允许原子替换。

- `cargo test --locked --lib hub::template_tests -- --test-threads=1`：5 通过。
- `cargo test --locked --lib --test project_actions --test tools_terminal -- --test-threads=1`：147 项单元测试、4 项集成测试通过，2 项需要冻结 CI DLL 的单元测试按既有规则跳过。该运行使用正常 Windows 权限及系统临时目录；早期沙箱管道访问拒绝、仓库临时目录内的通知失败不计作通过。
- `cargo test --locked --test engine --test providers --test specs --test cli --test hub_terminal -- --test-threads=1`：74 项通过。
- `cargo clippy --locked --all-targets -- -D warnings`：通过；沙箱下增量缓存硬链接不可用，编译器自动复制。
- 工具页测试使用原始输入 API 注入功能键，`send_text` 注入普通文字和独立 Esc；已验证搜索不改配置、开关两次恢复、后台任务期间搜索/缩放/退出。
- 尚无本轮有效性能结果、最终包或可发布结论。

## 提交及原始证据

- `41c593e`：工具页、共享配置保存、表单上下文、项目动作身份及 Terminal fragment 管理；已推送 PR #4。
- [本地测试原始日志](benchmarks/v0.6/review-20261018/local-tests.log)：147 项单元测试、78 项集成测试通过，2 项固定工件测试跳过。
- [Clippy 原始日志](benchmarks/v0.6/review-20261018/clippy.log)。
- [PS5.1 事务日志测试](benchmarks/v0.6/review-20261018/journal-ps51.log)：备份清单不重写、重复路径去重、历史快照保护、截断尾修复、损坏完整记录拒绝、恢复原文件及结果回放。该测试不代表完整安装或断电硬件保证。
- `e27cf46`：新版事务、历史快照保护、维护阶段计时和 CI 独立失败证据上传；已推送 PR #4。
- [本地同步安装回归回执](benchmarks/v0.6/review-20261018/install-receipt.json)：隔离安装、占用失败恢复、升级、回滚、设置保留、卸载及重试通过。回执记录包/EXE/维护脚本摘要和调试构建身份；不是完整原始控制台日志，也不是最终包验收。
- [后续修复测试日志](benchmarks/v0.6/review-20261018/followup-tests.log)：149 项单元测试和 3 项工具页/工作台 ConPTY 测试通过，2 项固定工件测试跳过；覆盖新表的不同字段合并、修改字段保留行内注释、回退后清理后续动态候选。Clippy 再次通过。

## 继续实施（未冻结）

- 新增 Legacy 真实终端测试首次失败是测试直接等待一次可选空闲输出而超时；改为等待实际工作台标题后，补全接受、取消与后续原生编辑通过。没有放宽产品成功条件。
- Python 发布证据测试正常权限重跑：共 92 项，91 通过、1 项既有跳过；首次沙箱临时目录权限错误不计为产品失败或通过。
- `1b70b86` CI run `39107569415` 的 PS7/pipe Direct 任务失败于 `direct_above_menu_preserves_wrapped_unicode_input_and_right_text`，错误为导航/缩放修改编辑缓冲区。其余该组 30 项通过、7 项跳过；第 08 项尚未完成，不能冻结发布。

- 核心包隔离安装回归已通过，完整原始日志 `target/core-install-regression.log`，安装根 `C:/Users/dell/AppData/Local/Temp/blueberry-release-test-3f0552a0f3294972818efc05f3fef0a2`；为本地未冻结调试包，后续增加组件入口后仍须复验。
- 增强组件新增独立包脚本与 CLI、共享安装互斥锁、不可变版本目录和原子活动清单；升级→回滚→再升级、最新设置保留、占用卸载失败后重试已通过。原始日志 [原始日志](benchmarks/v0.6/review-20261018/continuation/terminal-component-lifecycle.log)；测试第二版本明确标为 `test2`，不是最终候选。
- PS5.1 元数据 6 项及编译桥接 1 项通过；Clippy 全目标通过。普通终端预留区域、旧捆绑完整迁移、正式性能和其余工作流仍未完成。

## 本轮新增验证（仍未冻结）

- [原始日志](benchmarks/v0.6/review-20261018/continuation/review-combined-regression.log)：156 项单元测试、87 项 CLI/provider/规格/终端测试通过；2 项单元测试和 7 项 Direct 辅助/固定工件测试跳过。
- [原始日志](benchmarks/v0.6/review-20261018/continuation/render-context-regression.log)：重建私有编辑器后 Direct 32 项、工作台 1 项和真实 argv 1 项通过。
- [原始日志](benchmarks/v0.6/review-20261018/continuation/native-argv-regression2.log)：空值、空格、单双引号、反斜杠、分号、@、逗号、中文及变量样式字面量在 PS5.1 和 PS7 三种原生参数模式中往返一致；不代表批处理脚本解析已验证。
- [原始日志](benchmarks/v0.6/review-20261018/continuation/core-install-regression3.log)：固定输入 EXE 后核心同步安装全流程通过；第二次运行期间构建改动了输入 EXE，该失败批次保留且不计验收。
- [原始日志](benchmarks/v0.6/review-20261018/continuation/maintenance-regression-review.log)：异步升级、回滚、取消及中断恢复通过，保留 300 秒期限；安装根为 `target/maintenance-tests-31e015cbf4fd42e28c19f1a2110414a6`。
- [原始日志](benchmarks/v0.6/review-20261018/continuation/terminal-component-ps51.log) 和 [原始日志](benchmarks/v0.6/review-20261018/continuation/terminal-component-lifecycle.log)：增强组件的独立安装、升级、回滚、最新设置保留和占用卸载重试通过。
- [原始日志](benchmarks/v0.6/review-20261018/continuation/release-evidence-regression2.log)：36 项发布证据测试中 35 通过、1 项既有跳过；首次新增核心包测试未同步夹具的人工记录摘要，修复夹具后通过，产品身份匹配要求未放宽。
- 历史快照首次测试使用了同一个包，回滚正确拒绝无内容变化的快照；改用两个实际内容不同的包继续验证。

- 最新单元/CLI/argv 回归：158 + 4 + 1 项通过，2 项既有单元测试跳过；12 个内置工作流在 PS7 与 PS5.1 分别通过原生编辑行填回验证。
- 0/1/5 历史矩阵和旧捆绑迁移全部通过，阶段耗时只作诊断，不属于正式性能 A/B。
- 显式历史 RGB/Unicode 缩放用例仍失败，保留日志；VT 观察器缩小时裁剪行，尚不能据此区分观察缺失与产品历史损坏。不得标记为通过。
- [本轮证据回执](benchmarks/v0.6/review-20261018/continuation/receipt.json)绑定诊断 EXE 与私有编辑器清单；不代表最终包冻结。


## 2026-10-18 最终化续作（尚未冻结）

- 真实公开 0.5.2 原始 ZIP 与原始安装器在隔离目录完成安装→升级→回滚→再升级；回滚 EXE 与公开版摘要一致，用户配置、注册记录和用户 PATH 保持一致。证据见 `benchmarks/v0.6/review-20261018/finalization/public-upgrade-receipt.json`。CI 已增加同一路径验证，待新提交执行。
- NativeModel 已增加完整逻辑历史与逐单元颜色读取，CI run 39113421729 的模型、原生宿主及三编辑器组合通过。Direct 普通终端历史 RGB/Unicode 缩放两项使用该观察器通过本地诊断；不是 GUI 人工验收，也不是最终包冻结结果。
- Nested 普通终端使用输入行下方预留区域，缩放不再重放 VT 历史快照；按当前 Console 中匹配的自有菜单行清理。重复缩放与大输出专项已通过，最终矩阵仍在运行。
- Direct 与 Nested 共用 Workbench 表单、错误恢复、刷新选择、导航与翻页策略，宿主仍独立校验实际可见帧。160 项单元、33 项 Direct 回归通过；Nested 初次 16/18、绘制确认修复后 17/18，剩余清行测试旧事件问题修复后专项通过。失败批次保留，不计完整矩阵通过。
- CI run 39113425657 的 PS7/pipe Direct 工作流用例过早读取编辑行；增加真实 editor_edit_applied 确认后，本地 12 个工作流通过。下一 CI 需要复验。
- 项目动作开始携带结构化 Invocation，实际 Shell 参数模式决定展示与填回；ID 不随渲染模式改变。全目标 Clippy 通过。普通补全及批处理参数边界仍需补齐。
- 三轮正式性能、新入口进程树及最终同源核心/增强工件尚未完成；当前版本数字仍为开发占位 0.5.3，不能据缺失数据判定版本或标记可发布。


- 后续 PS7/OSC 全组通过：Nested Beta 18 项、terminal modes 3 项（3 个辅助/特殊宿主用例按原规则跳过）。此前失败来自测试将旧查询菜单误认为当前查询完成，现等待目标候选并确认导航绘制；完整失败日志保留在 target。
- 普通动态 provider 候选已使用实际 Shell 参数模式，原生 argv 回显覆盖 PS5.1 与 PS7 Legacy/Standard/Windows 并通过；从命令索引解析应用/别名，避免热态再次扫描 PATH。现有路径表达式与批处理边界仍需继续验证。
- 增强组件新增解压总大小和条目数边界；过量条目与损坏状态均拒绝，活动状态字节保持不变，生命周期复验通过。证据见 finalization/component-fault-regression.log。


## 缓冲区身份回归修正

PS5.1/2.0.0 完整矩阵进一步暴露真实产品问题：同一输入批次后的多个 editor buffer 回调可能共享修订号，旧查询的替换范围被重新绑定到新行，导致 `gi|XYZ` 接受为 `gitgiXYZ`。接受前的实际行/光标断言为 `giXYZ`/2，排除了单纯清行或导航测试问题。现在实际行或光标变化会失效旧 completion、可见候选并分配新修订号；PS5.1/pipe 失败专项已通过，全目标 Clippy 通过，完整矩阵复验中。此前归因于测试时序的记录保留为诊断历史，不能据此宣称问题早已解决。

启动 hook 回归不再篡改生成区块的脚本保护条件：在真实交互父 Shell 中执行隔离 profile，验证子 Shell、递归保护与返回父 Shell。两项 PS5.1 启动回归通过；这是功能回归，不能冒充自动 profile 加载的性能测量。


- 缓冲区身份修复后，PS5.1/2.0.0/pipe 全部通过：host 4、startup 2、Beta 18、terminal modes 3；辅助/特定系统用例仍按显式规则跳过。PS5.1/2.4.5/OSC 的 Direct 33、host 4、startup 2、Beta 18 通过，缩放用例在整组出现一次 CPR 回复残片超时，单独复测通过，10 次固定二进制复验进行中，尚不计该组合全部通过。
- 原生路径诊断确认 PS5.1 将普通引号包围的带空格尾反斜杠目录改写为尾引号。字面路径现在使用共享 Legacy argv 编码，并能解码已接受目录继续查询子文件；已有环境变量表达式保持原语义。目录继续补全和真实 argv 两项通过，既有 7 项路径回归通过，全目标 Clippy 通过。
- Python 脚本回归 93 项完成（92 通过、1 项既有跳过）；尚未完成正式性能数据、最终冻结与人工 GUI 验收。


- 连续缩放超时的观察器问题已复现：console snapshot 辅助程序使用阻塞 output()，期间 ConPTY 光标查询没有及时回应，迟到的 CPR 成为 ReadLine 输入。采集现在以子进程 try_wait 配合 Harness 持续 pump，保留 5 秒上限及真实 Console 尺寸/候选断言。PS5.1/2.4.5/OSC 固定二进制连续 10 次通过；CI e9fad3b 的 PS7 OSC/pipe 失败为同类 CPR 残片，待新 CI 长重复验证。

- 5c3652c 本地 PS7/pipe 完整回归中工作台历史停留“正在加载”：history_pending 同时充当请求排队和后台合并状态，回复清除 native_request 后可能连续重发并使合并结果过期。现独立 history_queued，仅每次打开工作台发送一次。12 个工作流连续 10 轮全部通过；完整矩阵继续验证。证据 finalization/workflow-history-repeat.log。
- 真实 argv 扩展至 exe/cmd/bat × PS5.1/PS7 Legacy/Standard/Windows，含空值、尾反斜杠、双引号、分号和中文，全部通过。证据 finalization/batch-invocation-regression.log。
- CI 39117885633 的 PS5.1/2.4.5 在 Save-Module 的 PSGallery 注册/源参数失败，尚未开始产品测试。所有矩阵改用固定 SHA-256 的官方原版包，保留原版与私有编辑器分离。损坏缓存不覆盖、路径越界不落盘两项测试通过；全目标 Clippy 通过（本机沙箱硬链接缓存降级复制警告不影响结果）。

## 真实自动 profile 入口观察器

`product-probe --profile-entry` 仅接受 `scripts/prepare-profile-entry.ps1` 创建的隔离 PowerShell 7 副本，启动不带 `-Command` / `-File`，由 PowerShell 自动加载 AllUsersCurrentHost profile 并执行生成的受管区块；实际产品递归保护保持有效。夹具只增加隔离 data-dir 参数及外层进程退出收尾。报告绑定 Shell、EXE、profile 摘要，原生基线在同一副本内抑制自启动。现有直接启动验收器拒绝以 profile 数据替代 Direct 数据。

首可见静态/动态候选与完整候选分别记录，旧字段继续表示完整候选。计时结束后记录父子进程树；这是当时存活进程的快照，不是覆盖瞬时启动进程的完整生命周期跟踪。PS7 自动 profile 与直接 EXE 各两对交替诊断均通过，原始 JSON 在 finalization；自动 profile 的首版退出夹具失败保留在 target/profile-entry-diagnostic.log。这些 debug 样本不用于正式性能或版本判断。37 项发布证据测试完成（36 通过、1 既有跳过），观察器 2 项单元和全目标 Clippy 通过。

PS7/pipe 工作台历史修复后完整组通过：Beta 18、终端模式 3、工具页 2，原有辅助/特定环境用例按规则跳过。证据 finalization/ps7-history-finalization.log。增强 Terminal 入口、瞬时进程生命周期、正式三轮性能与最终包验收仍在推进。


## 2026-10-18 维护故障边界与 CI 入口修正

- 隔离维护全生命周期通过，预提交恢复分别验证 schema 1 兼容读取与 schema 2 WAL；保留原有 300 秒期限。日志：`benchmarks/v0.6/review-20261018/finalization/maintenance-legacy-recovery.log`。
- 将 WAL 文件独占锁定后调用实际 Copy-BytesAtomic，写前记录失败时目标文件、touched 集合和序号均不变；PS5.1、PS7 两次通过，日志同目录 `journal-failure-boundary-ps51.log` / `journal-failure-boundary-ps7.log`。
- 自动 profile 夹具现在解析 PATH 中的 Shell 名称，并明确选择第一个应用命令；本机存在两个 pwsh.exe 时已验证成功。CI 39119200177 的 PS7/pipe 因旧 Resolve-Path 不能处理裸 pwsh.exe 失败，不计作产品或性能通过。
- 同次 CI 的 PS5.1/2.0.0 搜索退出用例发现 Ctrl+G 被当作字面控制字符。检查上游 2.0.0 KeyBindings.cs/History.cs：Windows 默认模式没有 Abort 绑定，只有 Emacs 默认模式提供 Ctrl+G。测试现在显式配置 Ctrl+G=Abort，同时保留默认 Esc 退出路径；产品不改用户键位。失败原始工件 12133831021（run 39119200177，ZIP SHA-256 4455c3d3fd2d679474d9260c97c09ec47f581cfad92f946740707bfda9d23135）保留在 artifacts/ci-d904-history。
- 38e0a43 的 run 39118585941 功能矩阵全部通过，打包尚在进行；此记录不等于最终冻结或发布批准。

三个受支持组合（PS5.1/2.0.0、PS5.1/2.4.5、PS7/2.4.5）新增 Direct/Nested 损坏 Cargo 表单恢复均通过：Esc 保留原行/光标，后续 git 补全和原生历史可用；显式 Abort 与默认 Esc 的突发输入测试均通过。原始日志 finalization/form-error-final-*.log、search-exit-final-*.log。全目标 Clippy、7 项性能矩阵脚本回归通过。启动器三类宿主环境测试是隔离环境模拟，不代替真实 GUI 启动验收。

增强组件补充了两个活动清单提交边界用例：版本目录已发布但活动指针被锁定时保留旧状态并可重试；卸载停用状态无法持久化时不得删除运行文件。已完成语法检查，实际执行留给 CI，避免与本机正式性能采集重叠；未运行前不计通过。

第二轮首批协调器 `artifacts/r2-formal-20261018/` 已标记 interrupted：虽然工作目录为中性目录，热态原始夹具仍生成在仓库内，可能继承项目上下文。原始数据全部保留，47.6335 ms 等首批数值不进入正式结论。正式脚本新增祖先 Git/Cargo/Node/Python 标记拒绝规则，8 项矩阵脚本回归通过；将在仓库外新目录从头重采，不续写此批次。

增强组件提交边界实测在 PS5.1、PS7 均通过（finalization/component-commit-boundaries-*.log）。生命周期观察器 3 项边界测试通过，包括瞬时子进程、PID 重用丢失退出及订阅失败不启动负载；这不代表本机 WMI 实时订阅已成功。发布证据 38 项完成，37 通过、1 既有跳过；单侧资源报告不能替代最终矩阵。上述测试在无效批次停止后、正式重采开始前执行。

`01568ab` / CI 39120974091 在 PS5.1/2.0.0 的 `builtin_workflows_fill_native_editor_without_executing` 超时于“运行 Python uv 项目脚本”；其余该组 33 项通过、7 项跳过。该次失败的临时 Direct 目录没有纳入上传，只有错误日志，不能确认产品根因。测试改为确认 Enter 之后的新输入修订与可见表单帧，不再只等待可重复的页码文字；Direct 夹具保持在仓库外并纳入 CI 上传，增加三个组合的工作流十轮重复。等待新 CI 验证，不将此修正视为产品问题已解决。

元数据缓存清单补入 PS7 实际使用的 `pwsh.runtimeconfig.json`，避免启动器未变化而
运行时选择策略变化时复用旧缓存。增加身份稳定性、运行时配置新增/修改和模块删除
回归；格式检查通过，执行验证交给 CI，避免本机编译与正式延迟采样重叠。

`541cff9` / CI 39124662325 的 Windows 核心任务已验证四项缓存测试全部通过，
核心单元 161 通过、2 项既有跳过，Clippy 通过。实际日志节选及完整任务链接见
`benchmarks/v0.6/review-20261018/finalization/system-cache-runtime-ci.log`。
完整 Shell 交互矩阵仍在运行，尚未最终冻结。

`21cfd5a` / CI 39123622450 的全部功能任务随后通过并进入打包：三个支持组合各有
Direct 34 项通过、7 项既有辅助/环境用例跳过，并各自连续十轮通过 12 个工作流。
日志节选绑定源提交与三个实际 job，见
`benchmarks/v0.6/review-20261018/finalization/workflow-ci-three-combinations.log`。
这证明增强后的表单帧验证通过重复回归，不补写早期缺失的失败现场，也不代替最终包验收。

2026-10-19：`f4b6b69` / CI 39125323714 的 PS5.1/2.0.0 工作流第八次重复失败，
保留原始工件 12136648331（ZIP SHA-256
6B74A23D788E639D07A711181339CE1B27EF13479B07E4B8A040CFB4BBE529BD）。
失败可见行含光标响应残片 `[2;56R`。本机诊断记录了 833–2216 ms 的查询排队延迟；
这支持观察器阻塞假设，但不能单凭此确定全部根因。观察器改为增量读取跟踪日志，
工作流夹具使用等价的相对 setup 路径，避免同步输入大量临时绝对路径字符；
保留完整输入修订、实际可见选中行、表单页与最终编辑行断言。
PS5.1/2.0.0 连续 30 次通过，全目标 Clippy 和格式检查通过；其他组合继续验证。

隔夜 R2 中性目录协调器已退出，2026-10-19 检查时标为 interrupted；该时间是发现时间，
不是准确退出时间。仅一组启动与一组热态报告完成，不能据此作轮次保留或版本判定。
资源续跑器明确标记 not_started，没有在不完整延迟批次之后运行。
新增显式独立启动模式，无权限时记录失败且不降级；9 项协调器测试通过。
10 秒真实短任务在调用进程退出后完成，回执位于 artifacts/detach-smoke-20261019。
正式重采使用新目录，旧报告保留，不拼接为完整批次。

2026-10-19 后续：PS5.1/2.4.5 第 19 次重复仍在 Cargo 首字段失败，该现场光标响应延迟低，
不能归为前述 CPR 阻塞。Direct 的 UI 等待在循环入口检查 500 ms 期限，会在编辑器线程
晚醒时丢弃已经入队的表单。现先处理队列再检查期限；新增强制晚醒 600 ms 的完整工作流
回归。相同测试在旧检查顺序下于首个 Git 字段失败，修正后 12 个工作流全部通过，
正反对照见 finalization/workflow-late-wake-{positive,negative}.log。普通输入仍保留
原生突发输入独立回归；工作流夹具按字符泵送终端，避免观察器阻塞光标响应。
正常三组合重复矩阵仍在执行，尚不能据此宣称最新完整 CI 已通过。

完整生命周期诊断新增 observe-debug-lifecycle.py：仅启动并观察自身 DEBUG_PROCESS
继承树，记录 PID、创建时间、父身份、映像及退出事件；不附加其他进程，不把诊断计时
作为性能结果。真实短时子进程、超时清理和启动失败 3 项回归通过。实际一次 PS7
原生/直接 Blueberry 启动配对已取得相配的创建/退出记录，保留在
artifacts/lifecycle-direct-20261019。服务/代理启动及自行选择调试器的后代不在该观察器
覆盖范围；不声称替代增强 Terminal GUI 入口的独立验收。WMI 主机权限复测仍拒绝访问。

2026-10-19 15:25（UTC+8）：独立矩阵 artifacts/direct-final3-20261019 完整完成。
三个支持组合各自 Direct 全套 36 通过、7 项既有跳过，随后各 30 次工作流全部通过。
冻结二进制摘要、计划、完整套件日志、90 份重复日志和协调器回执打包为
artifacts/finalization-raw-20261019/direct-three-combinations-20261019.zip；这是本机修复回归，不是最终 CI 包验收。
早两次运行保留：首次工作进程以 0xc000013a 退出；第二次新进程组的 Ctrl+C 继承语义
使中断测试失败。最终工作进程使用 CREATE_NO_WINDOW，不设置禁用 Ctrl+C 的新进程组。

自动 profile 生命周期首轮虽记录完整，但探针因多个 PSReadLine 模块记录形成版本数组而
退出 1，不能算启动配对成功。身份读取现改用实际 cmdlet 所属模块；Direct/Nested 报告
同样修正，并加入 profile 保留多个同名模块的回归。新隔离副本的自动 profile 诊断已成功：
artifacts/lifecycle-profile-identity-20261019，工作负载退出 0，32 个进程创建/退出相配。
观察器显式区分 workload_succeeded 与 lifecycle_reconciled，新增失败工作负载回归后
4 项测试通过，避免只凭事件齐全误报采样成功。正式配对样本与最终同源包仍待完成。

2026-10-19：profile 多个同名模块、晚醒补全接受、晚醒工作流的专项在三个支持组合均通过，
共 9 项真实 ConPTY 回归。记录见 artifacts/finalization-raw-20261019/direct-identity-three-combinations-20261019.zip。
普通单模块 Direct 启动沿用已有模块记录，仅出现数组时解析实际 cmdlet 所属模块。
补全接受和排队导航同样先处理已到达响应，再检查期限，仍保持原来的有界等待与修订保护。
生命周期原始事件、负载输出、成功配对与首轮失败记录见 artifacts/finalization-raw-20261019/startup-lifecycles-20261019.zip。

正式 R2 采样期间继续静态核对剩余门槛：Python 测试路径发现补上目录条目的入队预算，
避免单个大目录先积累全部子项再检查原有 4096 路径/250 ms 限制。新增空 PATH 下
静态建议、损坏环境元数据修复及取消后再查询回归；未调用 uv、项目脚本或 pytest 收集。
新增 Windows CI 实际存储故障矩阵：仅创建专有临时 VHD，在校验非系统空盘后格式化，
以真实 NTFS 空间耗尽和 ACL 拒绝验证核心/增强包旧状态及设置保留、恢复和重试。
PS7/PS5.1 分别执行；日志与事务证据在独立目录保存，最终卸载并删除专有磁盘映像。
目前只完成静态语法检查，执行结果以新 CI 为准，不计作已经通过；本机不运行这些诊断，
不干扰冻结 R2 采样，也不修改本机安装或现有磁盘。

2026-10-19 证据存放修正：三个本机回归 ZIP 已移至忽略的 artifacts/finalization-raw-20261019，Git 仅保留 finalization/raw-evidence-location.json 的文件摘要。Actions 上传身份尚未生成；须在正式采样完成后按既有独立上传流程补齐，当前不能将本机记录称为远端已交付工件。

2026-10-19：CI 39219393013 的 Windows core 在 12 个并发配置保存回归中暴露 500 ms 锁排队超时（原文件仍受保护）。配置锁现使用独立 5 秒有界排队预算，仅对 WouldBlock 重试，真实 I/O 错误立即返回；原子替换仍为 500 ms。新增持锁 650 ms 后保存成功及短预算超时不修改文件的回归。babe671 的 Windows core job 117954338072 已通过全部 10 项配置存储回归（库测试共 162 通过、2 项既有跳过）；后续 b747328 的三平台 core 也已通过。失败原始日志属于该 run 的 job 117950866662；未将失败批次计为通过。

PS7/pipe CI 39219397975 第 29 轮工作流在读取 F12 快照时得到空字符串；保留文件与实际屏幕均为正确的创建分支命令。WriteAllText 创建与内容写入之间存在读取窗口，已改为临时文件写完后重命名发布，并以单次读取严格比较。通过 HTTP Range 仅读取失败夹具约 383 KiB，未解压完整工件或运行本机诊断；源工件及成员摘要见 finalization/workflow-snapshot-race.json。修复提交 7414f1f 的 push CI 39222271557 已通过三个支持组合的完整交互任务，包含各自 30 次工作流重复；该 run 正在打包。旧失败批次仍保留为失败记录，后续缩放修复须另行验证。

PS5.1/2.4.5 CI 39220171555 的多行缩放回归失败，连续两份原生快照均缺失 HEAD/RIGHT。修复候选将已绘制几何与自有行读取结果分开，避免读取失败时 Clear 提前返回而遗漏编辑器逻辑重排；新增诊断模式强制读取缺失的真实 ConPTY 回归。此修复尚待 CI，不能标记缩放矩阵通过；工件与成员摘要见 finalization/resize-ownership-readback.json。


7414f1f push CI 39222271557 的安装/事务、核心 ZIP 内 EXE 三组合输入与独立 UI、增强组件生命周期均通过。真实 NTFS 故障在卷返回 Win32 112 后验证了核心自动恢复和组件旧状态不变，但显式 Recover 调用缺少事务目录与清单摘要而失败。测试现为故障升级保留 WAL 并绑定故障前清单，恢复复用同一身份，补充恢复后状态及最终设置复核；不放宽产品恢复校验。完整工件 12176375020 的 SHA-256 已核验，见 finalization/storage-recovery-invocation.json。PS5.1 中文脚本使用 UTF-8 BOM；等待修正后的实际矩阵，不将当前失败批次计作通过。


b747328 push CI 39225504824 的三个支持组合 Direct 均为 38 通过、7 项既有跳过；原始多行缩放与强制菜单读取失败用例均通过，各 30 次工作流重复也全部通过。job 117970680058、117970680062、117970680171 的日志已保存，摘要见 finalization/resize-ownership-readback.json。这验证了修正路径，但不据此断言所有历史偶发故障均来自同一根因。最新 6bb6cc2 的包故障验收仍在执行。


6bb6cc2 push CI 39228676557 的 package job 117983005894 完整通过：核心 ZIP 内 EXE 三组合输入与独立 UI、增强组件生命周期及迁移、PS7/PS5.1 真实 NTFS 磁盘满/ACL 故障恢复与重试、公开 0.5.2 隔离升级均通过。核心工件 12177100408、增强工件 12177005375 和探针工件 12175393656 来自同一 run。两份小型验收证据工件已下载并核验完整 Actions SHA-256；候选 ZIP 未在本机展开，以免干扰正式 R2。定位及摘要见 finalization/ci-6bb6cc2-artifacts.json、storage-physical-passed.json、packaged-interaction-6bb6cc2.json。完整 CI 的长重复任务、三轮性能、版本判定和最终人工验收仍待完成；这些工件尚不是最终可发布结论。

6bb6cc2 的完整 push CI 39228676557 现已全部通过（10 个必需任务成功，独立 evidence-upload 按设计跳过）。完整任务记录见 finalization/ci-6bb6cc2-functional.json，候选清单绑定其摘要。正式性能、版本选择和最终本机隔离/人工验收仍未完成，候选未标记可发布。

2026-10-20：稳定性修复已快进到 PR #4 的 3f676b7。完整 CI 39398913707 的 Linux/macOS Core 因 probe.rs 的测试模块位于普通实现之前触发 Clippy items_after_test_module；Windows Core 已通过。原始失败日志已保留，现将测试模块原样移到文件末尾，未改测试或产品逻辑。包验收补充三个编辑器组合的 12 个无追踪工作流，以及每组合 30 对实际自动 profile 启动样本。profile 仅修改一次性 GitHub-hosted 账户并检查恢复；这些新增验证尚待新 CI，不计作已通过。

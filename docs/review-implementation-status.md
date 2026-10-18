# 16 项评审实施记录

基线：`65332cf3622a5a15547c02094d2bd2e0682b68f6`，分支 `codex/v053-reliability-performance`，PR #4。
本轮不发布、合并或替换本机安装。以下是实施状态，不是发布验收结论。

| 项目 | 状态 | 已实施及剩余门槛 |
|---|---|---|
| 01 启动入口 | 实施中 | 已增加 fragment 的 Shell/安装/状态/移除；安装器已推荐官方入口；真实启动四路径和配对测量待完成 |
| 02 工具页 | 回归通过 | 功能键替代字符操作、成功后更新配置、完整自有区域清理；2 项真实 ConPTY 回归通过，最终包仍须复验 |
| 03 配置保存 | 回归通过 | 共享加锁、原子替换、字段差异合并；并发、冲突、错误文件、句柄占用及进程中断测试通过 |
| 04 参数与身份 | 实施中 | 项目动作独立稳定 ID、动态动作参数引用；新增结构化调用及实际会话参数模式；PS5.1、PS7 Legacy/Standard/Windows 的原生 EXE argv 回显通过；普通补全与所有项目动作统一、脚本程序类型矩阵待完成 |
| 05 表单上下文 | 实施中 | 模板上下文、前置值、目录/环境校验、Shift+Tab、Direct 可恢复错误；5 项表单测试通过，宿主回归待完成 |
| 06 维护事务 | 实施中 | 新版不可变备份清单、校验链写前日志、旧版读取、历史快照免重复备份已实现；PS5.1/PS7 日志测试和本地同步安装回归通过，300 秒异步生命周期已通过；0/1/5 历史矩阵通过，真实 0.5.2 升级待执行 |
| 07 包拆分 | 实施中 | 默认核心包和按包类型校验已实现；增强组件独立生命周期、设置保留与卸载重试在 PS5.1/PS7 通过，CI 分离工件；旧捆绑迁移及核心回滚/再升级通过；完整故障矩阵待完成 |
| 08 能力与渲染 | 实施中 | Direct 握手连接与降级原因独立报告，连接失败保留服务；Direct 已改为下方预留区域，32 项回归通过；Nested 普通模式、历史属性缩放与最终矩阵仍需验证 |
| 09 系统缓存 | 实施中 | 持久元数据身份、加锁重建、原子发布、损坏拒绝、显式刷新与命中跳过辅助进程已实现；3 项缓存测试及 PS5.1/PS7 各 6 项元数据测试通过，性能及完整身份验证待完成 |
| 10 Legacy | 实施中 | 已删除字符批量注册，只保留手动入口，服务连接与自动菜单独立；编译桥接和真实 ConPTY 手动补全/取消/原生编辑通过，完整编辑器矩阵待复验 |
| 11 三轮性能 | 未执行 | 不沿用旧入口数据作为新入口的验收证据 |
| 12 共享契约 | 实施中 | 两宿主共用表单提交与失败恢复转换；刷新、导航及可见帧契约仍待统一 |
| 13 Git | 实施中 | 分支、worktree、log/diff 表单与中文描述已增加；完整宿主交互与缓存失效矩阵待执行 |
| 14 Python | 实施中 | uv venv/sync/run/pytest 表单、依赖组与测试静态发现；pyproject/pytest.ini 多行配置及错误诊断回归通过，缺失工具及完整宿主矩阵待验证 |
| 15 Rust | 实施中 | build/test 表单及 package→features/test target 已增加，前置字段修改清理下游值；完整宿主矩阵待执行 |
| 16 最终验收 | 未执行 | CI 原始工件冻结、版本判定、隔离安装及最终人工验收 |

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

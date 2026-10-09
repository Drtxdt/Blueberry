# 发布 Blueberry

> 当前 0.5.3 候选政策：功能、安装、包身份与本机人工验收仍须完成；性能可以按用户选择延期，外部试用未执行。v0.6 仍保留性能门槛。下文 0.5.0–0.5.2 的人工豁免仅属历史，不适用于 0.5.3。见 [当前验收](v0.5.3-validation.md) 和 [原始证据保留](benchmark-artifacts.md)。


GitHub 仓库存放源码；Release 存放用户下载的程序。CI 为已冻结的源码提交生成 Windows x64 包；发布工作流只复用这份绑定验收记录的包，不在打标签后重新构建替换。

`0.5.0` 按用户 2026-10-06 的明确要求“如果没有bug先直接发版吧，性能以后再优化”，性能目标及正式性能矩阵延期，不再阻断本版发布，也不标记为通过。最新本地探索的 PS5.1／2.0.0 启动增量 P50 为 70.04770 ms，超过 50 ms 目标；其他历史探索不能代替最终包的正式验收。功能 CI、安装升级回滚和最终 main CI 包身份仍须通过。最终包交付后，用户进一步明确“本次也免除人工检查，按自动回归结果发布”；Windows Terminal 人工项目记录为豁免、未执行，不标记通过。目标用户试用保留此前明确豁免。后文完整性能流程保留给后续优化；本版使用下面的限版本延期记录。候选验证记录见 [0.5.0 验收](v0.5.0-validation.md)。

本版证据仍为 schema v2，新增 `performance_waiver`：`version: "0.5.0"`、`status: "deferred_by_user"`、`passed: false`、`authorization` 为上面用户原文，`limitations` 写明超标及缺少正式矩阵。四个正式性能报告映射留空；`functional_ci` 绑定包含 GitHub 原始 run/jobs 的 JSON 文件路径及 SHA-256，必须是最终源码的成功 main push CI，完整功能与 package 工作均成功。校验器保留全部包校验和安装事务项目，延期不能绕过功能失败。Windows Terminal 使用 `windows_terminal: {}` 和 `windows_terminal_waiver`（版本 0.5.0、`status: waived_by_user`、`executed: false`、`passed: false`、上述用户原文 `authorization`），不伪造操作结果。发布工作流仍独立查询实际 CI 状态。开发原始失败数据继续保留，不伪造性能通过。

## 0.5.1 命令补全修复与标签草稿

本轮 0.5.1 延续用户明确批准的性能延期、Windows Terminal 人工检查和目标用户试用豁免；均记录为未通过／未执行。自动功能和安装生命周期仍须通过。保留既有 v0.5.0，不移动旧标签。

`v*` 标签推送自动触发草稿工作流。工作流启动时固定 main 提交，从该提交的 `docs/benchmarks/v0.5/<标签>-release-evidence.json` 读取 CI run ID，再校验标签源码、main 成功 CI、版本和包身份。证据缺失或身份不符直接失败；不构建、不选择其他包。手动入口用于重试，显式 CI ID 必须与对应证据一致。

先提交产品和版本，等待 main CI 成功，验证该包，再提交绑定摘要的证据，最后为被测产品提交推送注释标签。草稿上传后重新下载逐字节检查。本轮仅生成未公开 draft，不自动公开；已公开 Release 禁止覆盖。

证据 JSON 使用 UTF-8、LF 换行。计算报告摘要前，确认工作区字节与即将提交的 Git blob 一致，避免 Windows 写入 CRLF 后被 `.gitattributes` 规范化而改变摘要。

## 0.5.2 菜单修复草稿

用户明确选择“沿用旧版安排，创建草稿”：0.5.2 延续性能延期、Windows Terminal 人工检查和用户试用豁免，记录为未通过／未执行。自动功能矩阵、安装生命周期及最终 CI 包身份仍须通过。草稿保留未公开状态，附件使用成功 main CI 的原始包。

## 第一次准备

1. 在仓库的 Actions 页面启用工作流。
2. 本机运行 `scripts/verify-local.ps1 -Shell pwsh.exe`，完成安装指南中的视觉检查。
3. 检查 `Cargo.toml` 和 `Cargo.lock` 的 Blueberry 版本一致，更新 `CHANGELOG.md`。

## 冻结候选与验收

每次修改 `Cargo.toml` 中的版本号后，先同步锁文件并编译：

```powershell
cargo update --offline --package blueberry
python scripts/prepare-editor.py
python scripts/prepare-conpty.py
cargo build --release --locked
```

将 `Cargo.toml`、`Cargo.lock` 和包含精确标题 `## 0.5.0` 的 `CHANGELOG.md` 一起提交。仅修改 `Cargo.toml` 会让 CI 的 `--locked` 构建失败。候选合并到 main 后，记下源码提交 SHA 和成功的 main CI run ID，下载该 run 的 `blueberry-windows-x64` 工件；只对其中的 EXE 和 ZIP 正式测量。任何代码改动均须重新冻结、构建与测量。

先对三个固定组合（PS 5.1／PSReadLine 2.0.0、PS 5.1／2.4.5、PS 7／2.4.5）各做 10 对启动与每热态组 30 次探索；已有明确失败时继续定位，不先采完整矩阵。正式启动使用 `product-probe --host-mode direct --iterations 30 --host-executable <CI EXE>`，它配对测量完整产品与 plain，旧 `probe` 仅作适配器诊断。正式热态使用 `beta-probe --host-mode direct --transport pipe --samples 300`，另跑 `--no-descriptions`；动态组必须等待完整动态候选。每组保留慢样本，profile 或电源策略变化使整批失效。时间来自接收线程和 VT 观察模型，不宣称物理屏幕像素时间。

证据 schema v2 用 `startup_reports`、`hot_reports` 保存正式单层矩阵，用 `nested_compatibility_reports` 保存三个嵌套变体的正确性与六场景实际性能；后者不应用单层的 20 ms 限制。所有报告绑定源码提交、干净 release 构建、EXE 摘要、`environments` 中的机器、电源策略、profile 文件摘要、30×120 终端与实际宿主／传输；`ci` 绑定成功的 main push run 和同一 ZIP 摘要。

`comparison_reports` 保存四方轮换顺序、各至少 30 个启动及六场景两种条件各至少 300 个样本。plain 只记录字符回显，`menu` 为 null、缓存能力为 `not_applicable`；其他产品记录真实菜单指标。plain／inshellisense 的 Blueberry 宿主与传输为 `not_applicable`；beta.6 为 nested／OSC，候选为 direct／pipe。入口与依赖均需文件路径及 SHA-256，不伪造不具备的缓存能力。运行：

```powershell
python scripts/verify-release-evidence.py .\docs\benchmarks\v0.5\v0.5.0-release-evidence.json .\dist\candidate\blueberry-v0.5.0-windows-x64.zip --commit <冻结源码提交SHA> --public-beta6-package <从公开 v0.5.0-beta.6 Release 下载的 ZIP>
```

人工记录仍绑定最终 `executable_sha256` 和 `source_commit`。`installation` 的 install／upgrade／rollback／uninstall／queued_maintenance／interrupted_maintenance、`windows_terminal` 的 ime／font_zoom／selection／paste／nested_program 均须实际通过。本版豁免记录为 `user_trials: []`，并提供 `user_trial_waiver: {"status":"waived_by_user","version":"0.5.0","executed":false,"reason":"用户明确要求本版跳过目标用户试用"}`；不能填成 passed，也不豁免其他人工项目。另记录经公开附件核验的 `public_beta6_sha256`、固定的 `inshellisense_sha256`。换 EXE 后相关记录失效。正式报告及原始数据作为后续提交保存；不能修改被测程序或包。本版冻结之后的用户豁免仅允许更新发布校验器、对应测试、发布工作流、发布说明文档及证据。工作流从已合入 main 的证据提交运行校验器及其测试；仍禁止产品源码、依赖及打包脚本变化。最终 EXE 与 ZIP 保持原 main CI 字节，不重新编译。

校验器核对公开 beta.6 ZIP 内 `blueberry.exe` 的摘要与四方对照记录。发布工作流会从本仓库的公开 beta.6 Release 重新下载该 ZIP；无法下载或摘要不符时不会创建草稿。

## 创建标签与草稿

以 0.5.0 为例，在所有门槛通过后，为**被测源码提交**创建注释标签并推送：

```powershell
git tag -a v0.5.0 <冻结源码提交SHA> -m "Blueberry 0.5.0"
git push origin v0.5.0
```

标签必须与 Cargo 版本一致，且提交属于 main 历史。标签推送自动生成草稿；也可在 Actions 手动启动 **Blueberry release draft**，传入 `release_tag=v0.5.0`、被测的 main `ci_run_id`、正式报告提交 `evidence_ref`。工作流再次确认 CI 源码、全部原始性能报告、包和 EXE 摘要完全一致，才创建草稿并上传同一 ZIP、校验文件、清单及证据包。每次版本使用新标签。

失败运行中的标签仍指向原源码，重试不会重建或换用之后的产品修复。标签流程每次启动会固定当时的 main 证据提交；证据或发布校验修正可通过保留的手动入口指定重试，但仍须满足冻结后文件白名单和原包身份校验。产品需要修复时，使用新的版本号和标签，保留原标签。

## 检查并公开

1. 打开 Actions，确认 **Blueberry release draft** 成功。
2. 打开 Releases，进入对应草稿。
3. 检查版本说明以及 Windows x64 ZIP、SHA-256、发布清单、安装脚本和原始性能证据包；下载 ZIP，核对与被测工件的 SHA-256 相同。
4. 下载该 ZIP 做本机体验验收。
5. 点击 **Publish release**。0.5.0 正式版不设置 pre-release 标记；带预发布后缀的版本保留该标记。

已公开版本的附件由工作流保护，重复运行只更新尚未公开的草稿。公开版本需要修复时，增加版本号后重新发布。

## 本地打包

```powershell
python scripts/prepare-editor.py
python scripts/prepare-conpty.py
cargo build --release --locked
.\scripts\licenses.ps1 -OutputPath .\THIRD-PARTY-NOTICES.txt
.\scripts\release.ps1 -ExePath .\target\release\blueberry.exe -Version 0.5.0 -OutputDirectory .\dist\release
.\scripts\test-release.ps1 -BlueberryExecutable .\target\release\blueberry.exe
.\scripts\test-install-51.ps1 -PackagePath .\dist\release\blueberry-v0.5.0-windows-x64.zip
```

完整依赖许可文本必须随包分发。历史性能报告保留在仓库，当前安装包仅携带用户所需文档。

## 如何理解测试结果

- 三平台 core 工作检查编译、规则引擎和非交互 CLI。
- PowerShell 5.1 和 PowerShell 7 工作固定 PSReadLine 版本，执行真实 ConPTY 回归；两种传输分别测试。
- Windows Server 2022 覆盖 VT 鼠标透传。原生 Win32 鼠标记录另由 `windows-2025` 任务验证，两种传输均需通过才允许打包。旧版系统的原生鼠标支持受系统 ConPTY 限制，参见 [微软跟踪记录](https://github.com/microsoft/terminal/issues/376)。本机验证脚本在 Windows 11 及更新系统上也会运行该项。
- package 工作验证 ZIP 与安装、升级、回滚、卸载。
- PowerShell 7 本机回归和 Windows Terminal 视觉验收由发布者记录。

本机通过和远程 Actions 通过分别记录；CI 徽章链接到真实工作流结果。

发布界面操作见 [GitHub 发布说明](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository)。运行器配置参照 [GitHub runner 列表](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)，`macos-15` 使用 ARM64。

## 私有编辑器候选矩阵

Windows 候选默认 direct，nested 通过 `--host-mode nested` 保留。最终默认配置必须重新验收，不能用此前探索结果代替发布门槛。
Windows 构建先执行 `scripts/prepare-editor.py` 和 `scripts/prepare-conpty.py`；上游提交、补丁、许可证和每个 DLL 摘要
由私有编辑器构建清单管理，`doctor --json` 的 `build.private_editors` 随发布清单一起封存。
固定 ConPTY 的包与文件摘要由 `build.conpty` 封存，探针报告还须包含实际 `probe_conpty`；系统回退或运行时身份不一致不能通过。探针准备耗时单独报告，不计入某一侧启动；nested 产品内部的运行时准备仍计入产品耗时。

`scripts/run-editor-matrix.ps1` 串行运行三个组合的启动与说明开／关热态矩阵；默认是
10 对启动、每子组 30 个样本的探索。正式运行须指定 `-Formal -Samples 300 -StartupPairs 30`，
提供 CI EXE、探针及两者期望 SHA-256，并指定原版 2.0.0 和 2.4.5 模块用于 plain 对照。
脚本拒绝覆盖既有目录，保存原始样本与失败日志，任何降级或子组超标即阻断。
原始数据目录由 `BLUEBERRY_BENCH_EVIDENCE` 控制；正式矩阵脚本自动设置并在结束后还原。
诊断可用 `--diagnostic-trace-directory`，再由 `scripts/analyze-editor-trace.py` 关联 QPC 阶段，
但启用 trace 的数据不能作为正式计时。

自动矩阵不替代 Windows Terminal 的真实输入法、剪贴板、视觉、升级和回滚检查。最终包交付用户后按 [0.5.0 人工验收清单](v0.5.0-windows-terminal-acceptance.md) 留证；清单本身不表示已经通过。

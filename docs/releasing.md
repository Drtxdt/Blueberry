# 发布 Blueberry

GitHub 仓库存放源码；Release 存放用户下载的程序。CI 为已冻结的源码提交生成 Windows x64 包；发布工作流只复用这份经过性能验收的包，不在打标签后重新构建替换。

`0.5.0-beta.7` 当前仅为源码候选。首次输入增量 P50 ≤50 ms、每组热态完整菜单 P95 ≤20 ms 是发布硬门槛。单层通过正确性及探索门槛后才成为默认；正式矩阵为单层 pipe 说明开／关。嵌套 OSC／pipe 保留兼容回归并单独报告性能。安装升级回滚和 Windows Terminal 人工验收仍须通过；本版目标用户试用按用户明确要求豁免、未执行。未满足其他门槛前不打标签或发布。候选验证记录见 [Beta.7 验收](beta-7-validation.md)。

## 第一次准备

1. 在仓库的 Actions 页面启用工作流。
2. 本机运行 `scripts/verify-local.ps1 -Shell pwsh.exe`，完成安装指南中的视觉检查。
3. 检查 `Cargo.toml` 和 `Cargo.lock` 的 Blueberry 版本一致，更新 `CHANGELOG.md`。

## 冻结候选与验收

每次修改 `Cargo.toml` 中的版本号后，先同步锁文件并编译：

```powershell
cargo update --offline --package blueberry
cargo build --release --locked
```

将 `Cargo.toml`、`Cargo.lock` 和包含精确标题 `## 0.5.0-beta.7` 的 `CHANGELOG.md` 一起提交。仅修改 `Cargo.toml` 会让 CI 的 `--locked` 构建失败。候选合并到 main 后，记下源码提交 SHA 和成功的 main CI run ID，下载该 run 的 `blueberry-windows-x64` 工件；只对其中的 EXE 和 ZIP 正式测量。任何代码改动均须重新冻结、构建与测量。

先对三个固定组合（PS 5.1／PSReadLine 2.0.0、PS 5.1／2.4.5、PS 7／2.4.5）各做 10 对启动与每热态组 30 次探索；已有明确失败时继续定位，不先采完整矩阵。正式启动使用 `product-probe --host-mode direct --iterations 30 --host-executable <CI EXE>`，它配对测量完整产品与 plain，旧 `probe` 仅作适配器诊断。正式热态使用 `beta-probe --host-mode direct --transport pipe --samples 300`，另跑 `--no-descriptions`；动态组必须等待完整动态候选。每组保留慢样本，profile 或电源策略变化使整批失效。时间来自接收线程和 VT 观察模型，不宣称物理屏幕像素时间。

证据 schema v2 用 `startup_reports`、`hot_reports` 保存正式单层矩阵，用 `nested_compatibility_reports` 保存三个嵌套变体的正确性与六场景实际性能；后者不应用单层的 20 ms 限制。所有报告绑定源码提交、干净 release 构建、EXE 摘要、`environments` 中的机器、电源策略、profile 文件摘要、30×120 终端与实际宿主／传输；`ci` 绑定成功的 main push run 和同一 ZIP 摘要。

`comparison_reports` 保存四方轮换顺序、各至少 30 个启动及六场景两种条件各至少 300 个样本。plain 只记录字符回显，`menu` 为 null、缓存能力为 `not_applicable`；其他产品记录真实菜单指标。plain／inshellisense 的 Blueberry 宿主与传输为 `not_applicable`；beta.6 为 nested／OSC，候选为 direct／pipe。入口与依赖均需文件路径及 SHA-256，不伪造不具备的缓存能力。运行：

```powershell
python scripts/verify-release-evidence.py .\docs\benchmarks\v0.5\beta7-release-evidence.json .\dist\candidate\blueberry-v0.5.0-beta.7-windows-x64.zip --commit <冻结源码提交SHA> --public-beta6-package <从公开 v0.5.0-beta.6 Release 下载的 ZIP>
```

人工记录仍绑定最终 `executable_sha256` 和 `source_commit`。`installation` 的 install／upgrade／rollback、`windows_terminal` 的 ime／font_zoom／selection／paste／nested_program 均须实际通过。本版豁免记录为 `user_trials: []`，并提供 `user_trial_waiver: {"status":"waived_by_user","version":"0.5.0-beta.7","executed":false,"reason":"用户明确要求本版跳过目标用户试用"}`；不能填成 passed，也不豁免其他人工项目。另记录经公开附件核验的 `public_beta6_sha256`、固定的 `inshellisense_sha256`。换 EXE 后相关记录失效。正式报告及原始数据作为只含证据的后续提交保存；不能修改被测程序或包。

校验器核对公开 beta.6 ZIP 内 `blueberry.exe` 的摘要与四方对照记录。发布工作流会从本仓库的公开 beta.6 Release 重新下载该 ZIP；无法下载或摘要不符时不会创建草稿。

## 创建标签与草稿

以 beta.7 为例，在所有门槛通过后，为**被测源码提交**创建注释标签并推送：

```powershell
git tag -a v0.5.0-beta.7 <冻结源码提交SHA> -m "Blueberry 0.5.0-beta.7"
git push origin v0.5.0-beta.7
```

标签必须与 Cargo 版本一致，且提交属于 main 历史。标签本身不会自动生成草稿；在 Actions 手动启动 **Blueberry release draft**，传入 `release_tag=v0.5.0-beta.7`、被测的 main `ci_run_id`、正式报告提交 `evidence_ref`。工作流再次确认 CI 源码、全部原始性能报告、包和 EXE 摘要完全一致，才创建草稿并上传同一 ZIP、校验文件、清单及证据包。每次版本使用新标签。

失败运行中的标签仍指向原提交；点击 Re-run 不会读取之后的 main 修复。准备重发时，先确认标签指向包含修复的提交。已公开版本使用新的版本号和标签，保留原标签。

## 检查并公开

1. 打开 Actions，确认 **Blueberry release draft** 成功。
2. 打开 Releases，进入对应草稿。
3. 检查版本说明以及 Windows x64 ZIP、SHA-256、发布清单、安装脚本和原始性能证据包；下载 ZIP，核对与被测工件的 SHA-256 相同。
4. 下载该 ZIP 做本机体验验收。
5. 点击 **Publish release**。Beta 保留 pre-release 标记。

已公开版本的附件由工作流保护，重复运行只更新尚未公开的草稿。公开版本需要修复时，增加版本号后重新发布。

## 本地打包

```powershell
cargo build --release --locked
.\scripts\licenses.ps1 -OutputPath .\THIRD-PARTY-NOTICES.txt
.\scripts\release.ps1 -ExePath .\target\release\blueberry.exe -Version 0.5.0-beta.7 -OutputDirectory .\dist\release
.\scripts\test-release.ps1 -BlueberryExecutable .\target\release\blueberry.exe
.\scripts\test-install-51.ps1 -PackagePath .\dist\release\blueberry-v0.5.0-beta.7-windows-x64.zip
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

# 发布 Blueberry

GitHub 仓库存放源码；Release 存放用户下载的程序。CI 为已冻结的源码提交生成 Windows x64 包；发布工作流只复用这份经过性能验收的包，不在打标签后重新构建替换。

`0.5.0-beta.7` 当前仅为源码候选。首次输入增量 P50 ≤50 ms、热态菜单 P95 ≤20 ms 是发布硬门槛；两项正式复测、完整自动回归、Windows Terminal 人工验收及 8–12 位目标用户任务完成前，不推送 `v0.5.0-beta.7` 标签，也不公开 Release。候选验证记录见 [Beta.7 验收](beta-7-validation.md)。

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

在固定测试机器上对每个受支持的 PowerShell／PSReadLine 组合保存 `probe --with-profile --iterations 30` 的原始 JSON；对默认 OSC、pipe 和 OSC 关闭说明分别保存 `beta-probe --samples 300` 的原始 JSON。各热态报告含六个场景、两种缓存状态。`beta7-release-evidence.json` 记录源码提交、最终 ZIP 和 EXE 的 SHA-256，并引用全部原始报告。运行：

```powershell
python scripts/verify-release-evidence.py .\docs\benchmarks\v0.5\beta7-release-evidence.json .\dist\candidate\blueberry-v0.5.0-beta.7-windows-x64.zip --commit <冻结源码提交SHA>
```

脚本检查每组原始样本、实测 Shell／PSReadLine 与传输方式、性能分位数、包内每个文件的清单摘要，以及人工验收记录；任何缺项或超标均失败。证据 JSON 的 `manual_acceptance` 必须写入被测 `executable_sha256` 和 `source_commit`；其 `installation` 的 `install/upgrade/rollback`、`windows_terminal` 的 `ime/font_zoom/selection/paste/nested_program` 各项须为 `true`。`user_trials` 为 8–12 个匿名 ID，每人写入同一 `executable_sha256`，且 `install/explain/project_parameters/template/exit_restore` 须全部为 `true`。另记录与公开发布附件核对后的 `public_beta6_sha256`、冻结的 `inshellisense_sha256`。这些是实际完成后的验收记录，不预填通过；换 EXE 后全部失效。随后将正式报告与原始数据作为**仅文档**提交推送到 main；这个证据提交必须是源码提交的后代，不能改动待发布的程序或包。

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

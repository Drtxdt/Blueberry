# 发布 Blueberry

GitHub 仓库存放源码；Release 存放用户下载的程序。CI 验证代码后，发布工作流自动创建草稿并上传附件。

## 第一次准备

1. 在仓库的 Actions 页面启用工作流。
2. 本机运行 `scripts/verify-local.ps1 -Shell pwsh.exe`，完成安装指南中的视觉检查。
3. 检查 `Cargo.toml` 和 `Cargo.lock` 的 Blueberry 版本一致，更新 `CHANGELOG.md`。

## 创建版本

每次修改 `Cargo.toml` 中的版本号后，先同步锁文件并编译：

```powershell
cargo update --offline --package blueberry
cargo build --release --locked
```

将 `Cargo.toml`、`Cargo.lock` 和包含新版本章节的 `CHANGELOG.md` 一起提交。仅修改 `Cargo.toml` 会让 CI 的 `--locked` 构建失败；更新日志缺少对应版本章节会阻止发布草稿生成。

以 `0.5.0-beta.2` 为例，在已提交且工作区干净的 main 分支运行：

```powershell
git push origin main
git tag -a v0.5.0-beta.2 -m "Blueberry 0.5.0-beta.2"
git push origin v0.5.0-beta.2
```

标签必须与 Cargo 版本一致，且提交属于 main 历史。每次版本使用新标签。

失败运行中的标签仍指向原提交；点击 Re-run 不会读取之后的 main 修复。准备重发时，先确认标签指向包含修复的提交。已公开版本使用新的版本号和标签，保留原标签。

## 检查并公开

1. 打开 Actions，等待 **Blueberry release draft** 成功。
2. 打开 Releases，进入对应草稿。
3. 检查版本说明以及 Windows x64 ZIP、SHA-256、发布清单、安装脚本。
4. 下载该 ZIP 做本机体验验收。
5. 点击 **Publish release**。Beta 保留 pre-release 标记。

已公开版本的附件由工作流保护，重复运行只更新尚未公开的草稿。公开版本需要修复时，增加版本号后重新发布。

## 本地打包

```powershell
cargo build --release --locked
.\scripts\licenses.ps1 -OutputPath .\THIRD-PARTY-NOTICES.txt
.\scripts\release.ps1 -ExePath .\target\release\blueberry.exe -Version 0.5.0-beta.2 -OutputDirectory .\dist\release
.\scripts\test-release.ps1 -BlueberryExecutable .\target\release\blueberry.exe
.\scripts\test-install-51.ps1 -PackagePath .\dist\release\blueberry-v0.5.0-beta.2-windows-x64.zip
```

完整依赖许可文本必须随包分发。历史性能报告保留在仓库，当前安装包仅携带用户所需文档。

## 如何理解测试结果

- 三平台 core 工作检查编译、规则引擎和非交互 CLI。
- PowerShell 5.1 工作固定 PSReadLine 版本，执行真实 ConPTY 回归。
- Windows Server 2022 覆盖 VT 鼠标透传。原生 Win32 鼠标记录另由 `windows-2025` 任务验证，两种传输均需通过才允许打包。旧版系统的原生鼠标支持受系统 ConPTY 限制，参见 [微软跟踪记录](https://github.com/microsoft/terminal/issues/376)。本机验证脚本在 Windows 11 及更新系统上也会运行该项。
- package 工作验证 ZIP 与安装、升级、回滚、卸载。
- PowerShell 7 本机回归和 Windows Terminal 视觉验收由发布者记录。

本机通过和远程 Actions 通过分别记录；CI 徽章链接到真实工作流结果。

发布界面操作见 [GitHub 发布说明](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository)。运行器配置参照 [GitHub runner 列表](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)，`macos-15` 使用 ARM64。

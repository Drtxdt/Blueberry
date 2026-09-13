# ShellSense Beta 发布准备清单

这份清单用于 `0.5.0-beta.1` 的本地 Beta 准备与冻结，不表示已经公开分发。当前工作副本和待迁移资产位于
`C:\Users\dell\.codex\visualizations\2026\09\10\01a08c3e-7202-7b72-bd9a-33b2fa60a094\shellsense-work-v2`；E 盘离线。本轮只在 C 盘保留唯一源码、发布包和回滚版，完成检查后清理临时缓存。脚本只打包已经存在的 Windows x64
exe；它不会调用 Cargo、创建 GitHub Release、推送提交或签名。

## 构建与验收

已有基线证据：0.4 的完整 123 个 Rust 测试通过；root 当前还记录了不含粘贴路径的 Rust/PowerShell adapter 回归和新增 specs 用例通过。0.5 新增粘贴等宿主交互仍在验证，这些记录不代表当前版本最新全量通过。

- [ ] 在固定 Windows 机器、release 配置和固定设置下完成 `cargo fmt --check`、
      `cargo clippy --all-targets --locked -- -D warnings` 和 `cargo test --locked`。
- [ ] 运行 `tests/adapter.tests.ps1`，并在真实 Windows Terminal 中检查 PowerShell 7、
      中文输入法、粘贴、缩放、外部全屏程序恢复和退出后的 profile 状态；当前真实 IME/视觉验收尚未完成，鼠标只有编码层测试。
- [ ] 确认首次输入增量 P50 与热态菜单 P95 的结果和动态数据就绪时间已记录；
      未达性能门槛时，版本说明必须明确标为性能验收未通过。
- [ ] 确认当前提交没有把测试输出、用户配置、profile 或真实安装目录带入包。
- [ ] 确认 `.github/workflows/windows.yml` 的 CI 配置已经审阅；远程 runner 尚未执行，不能写成 CI 已通过。

## 生成包

从仓库根目录运行以下命令。`-ExePath` 必须是已经完成构建的 exe，发布脚本不会
自行构建：

```powershell
pwsh -NoProfile -File .\scripts\release.ps1 `
  -ExePath .\target\release\shellsense.exe `
  -Version 0.5.0-beta.1 `
  -OutputDirectory .\dist\beta `
  -LicenseNoticesPath .\THIRD-PARTY-NOTICES.txt `
  -ReleaseNotesPath .\CHANGELOG.md
```

如果仓库尚未提供完整的依赖许可证清单，先运行
`pwsh -NoProfile -File .\scripts\licenses.ps1`。收集器只从离线 Cargo registry
读取实际解析依赖的许可证原文；缺少原文时会生成 `.missing` 缺口报告并失败。
`release.ps1` 没有许可证文件时也会失败，不会用 SPDX 名称冒充许可证文本。
公开分发前必须审核 `THIRD-PARTY-NOTICES.txt`，并把它通过
`-LicenseNoticesPath` 传入发布脚本。根 `LICENSE` 和该依赖说明都必须进入压缩包。

逐项确认：

- [ ] 文件名为 `shellsense-v<version>-windows-x64.zip`，旁边有同名 `.sha256` 和
      `shellsense-v<version>-release.json`。
- [ ] `release.json` 的 `platform`/`architecture` 为 `windows-x64`/`x64`，
      `signature_status` 为 `unsigned`，每个文件有 SHA-256 和长度。
- [ ] ZIP 含 `shellsense.exe`、`VERSION.txt`、`README.md`、`LICENSE`、
      `THIRD-PARTY-NOTICES.txt`、`RELEASE-NOTES.md`、配置示例、规格说明、
      `docs/beta-progress.md`、`docs/performance.md`、`docs/powershell-adapter.md`
      和 `manage-install.ps1`。
- [ ] 运行 `Get-FileHash`，并把 ZIP 内 exe 的 SHA-256 与 `release.json` 对照。
- [ ] 若有上一版 exe，使用 `-PreviousExePath` 保存到输出目录的 `previous`；确认
      该文件与上一版清单一致，且没有覆盖既有文件。
- [ ] 不打包 C# 对照接入层或 DLL；该路径已因无收益而拒绝。OSC 仍为默认 transport，
      named pipe 仅作显式实验。
- [ ] 确认整段粘贴即使使用 pipe 也走会话私有 FIFO 文件通道，ShellSense 不另行记录粘贴正文，不写入 trace 或学习统计；
      PSReadLine 原有历史策略保持不变；当前 Windows 原生 `Event::Paste` 输入路径仍在补齐，尚未验收。

## 本地安装矩阵

- [ ] 在明确生成的临时目录运行 `scripts/test-release.ps1`。
- [ ] 对新临时安装根依次执行 Install、Upgrade、Rollback、Uninstall；确认升级前
      的 exe 可回滚，SHA-256 与清单一致。
- [ ] 在安装根外创建用户配置和用户规格，重复上述操作后确认字节内容不变。
- [ ] 确认没有 profile 自动修改，没有网络下载，没有执行项目脚本。
- [ ] 对标准 Windows Terminal settings 先 Preview，再 Apply；确认原字节备份存在，
      其他 profile/设置保留，ShellSense profile 的 GUID、命令行和名称正确。
- [ ] 对含注释或尾逗号的 JSONC 运行 Preview/Apply；确认 Apply 明确拒绝、输出手动
      profile，且原 settings 未被写入。

## CI 与交付记录

- [ ] `.github/workflows/windows.yml` 在 Windows runner 上运行 Rust 检查、适配器检查、
      release 构建和本地打包检查。
- [ ] CI 的 PowerShell 7 与 PSReadLine 版本矩阵全部记录实际版本；不要用 CI 的
      `workflow_dispatch` 代替真实 Windows Terminal 中文输入法验收。
- [ ] 上传的 CI artifact 只作为检查结果，公开 GitHub Release 仍需人工审核和明确
      的发布操作；工作流不包含 `gh release create`、推送或签名步骤。
- [ ] 版本说明写明 unsigned 状态、依赖许可证来源、已知性能结果、安装路径和回滚
      方法。
- [ ] 保存本次 ZIP、`.sha256`、release manifest、依赖许可证说明和上一版二进制，
      并记录构建机器、Rust、PowerShell、PSReadLine 和 Git 版本。
- [ ] 本轮只保留 C 盘本地 Beta 资产，不执行公开发布；正式性能数字由 root 更新后再写入
      版本说明。

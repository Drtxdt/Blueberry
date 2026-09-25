> 历史记录：本文保留 0.5.0-beta.1 阶段的安装与验收资料，不表示 beta.7 的状态。当前工作区已经位于 `E:\OpenSource\shellsense-rs`；下文有关 E 盘离线、迁移待办及 C 盘唯一副本的叙述仅为当时记录。当前版本请使用 [安装指南](installation.md)、[发版指南](releasing.md) 和 [beta.7 验收记录](beta-7-validation.md)。

# ShellSense Beta 发布准备与验收

当前版本 `0.5.0-beta.1` 是未签名的本地 Beta 候选，尚未公开发布。功能自动回归通过，性能门槛未通过；真实 Windows Terminal 输入法/视觉及远程 CI 仍待完成。原始测量、构建环境与可执行文件 SHA-256 见 [性能报告](performance-v0.5.md)，功能范围见 [验收记录](beta-progress.md)。

## 已完成的本机检查

- [x] `cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`、release 构建。
- [x] 完整 Rust 回归：144 passed、0 failed；一个 ignored helper 由实际鼠标测试作为子进程运行。回归使用 debug 测试构建，正式性能使用 release。
- [x] pipe 的 `terminal_beta` 与 `terminal_modes`：8 passed；默认 OSC 的 terminal_modes 定向复跑通过。
- [x] `tests/adapter.tests.ps1`；release CLI 的 6 项 Cargo/Git 上下文、链接发现和中文说明回归。
- [x] 单行/多行粘贴、中文/emoji、撤销、历史、真实插入范围、全屏恢复、鼠标与缩放的实际 ConPTY 回归。
- [x] Windows x64 静态 CRT；导入表仅包含 Windows 系统 DLL。C# 对照实验不进入生产路径或发布包。
- [x] 正式计时关闭 trace、保留 profile；30 对启动与三个变体各 3,600 次热态查询，分组统计及原始数组已保存。
- [x] 准确记录失败指标：首次输入增量 P50 **143.40 ms**（要求 ≤50 ms）；默认 OSC + 中文说明热态 P95 **36.80 ms**（要求 ≤20 ms）。不降低验收标准。
- [x] 95 项实际离线 Rust 依赖许可证原文已收集；包包含项目 LICENSE 和 THIRD-PARTY-NOTICES.txt。

旧版 terminal_modes 的首次完整运行曾超时一次，原因尚未确定；后续完整回归通过，增加启动阶段和 ready/transport 诊断后，OSC 与 pipe 的定向回归也通过。自动回归不替代 Windows Terminal 的输入法和视觉验收。

## 本地发布生命周期自测

已运行 `scripts/test-release.ps1`，以真实 0.4 和 0.5 exe 生成临时包并验证：

- [x] Install → Upgrade → Rollback → Uninstall；升级中断后恢复全部受管文件与元数据。
- [x] 升级和回滚保留用户配置；卸载保留用户文件及被用户修改的受管文件，恢复原内容后可干净重试。
- [x] ZIP、manifest、exe 版本/架构/长度/SHA-256 校验，以及报告引用的原始数据文件完整性。
- [x] 打包覆盖失败时保留恢复备份；拒绝路径穿越、链接和重解析点，避免写到指定目录之外。
- [x] 标准 Windows Terminal JSON 先 Preview 再 Apply，保留其他设置并备份原始字节；JSONC 含注释或尾逗号时拒绝自动改写，只提供手动合并片段。
- [x] 不自动修改 profile、不下载外部包、不运行项目脚本；自测只使用隔离的临时目录，结束后清理。

同一安装根或发布输出目录的并发操作不受支持。详细命令见 [安装、升级与回滚](beta-installation.md)。

## 生成本地包

发布脚本只打包已构建的 exe，不调用 Cargo，不签名，不创建 GitHub Release 或推送提交：

```powershell
pwsh -NoProfile -File .\scripts\release.ps1 `
  -ExePath .\target\release\shellsense.exe `
  -Version 0.5.0-beta.1 `
  -OutputDirectory .\dist\beta `
  -LicenseNoticesPath .\THIRD-PARTY-NOTICES.txt `
  -ReleaseNotesPath .\CHANGELOG.md `
  -PreviousExePath .\artifacts\shellsense-v0.4-frozen.exe
```

输出为 `shellsense-v0.5.0-beta.1-windows-x64.zip`、同名 `.zip.sha256`、`shellsense-v0.5.0-beta.1-release.json` 和 `previous/` 下的上一版 exe。ZIP 内 manifest 标记 `windows-x64` / `x64` / `unsigned`；每个受管文件有长度及 SHA-256，性能报告引用的原始数据也进入清单。manifest 自身不包含递归的自校验条目，由外部 ZIP 摘要覆盖。

出包后需独立核对 ZIP 摘要、文件清单、exe SHA-256 与离线文档链接。`dist/shellsense.exe` 更新前另存原文件，禁止覆盖用户配置。源码 Git bundle、最终文件摘要和核对结果存放在本地交付目录；生成物不提交到 Git。

## 尚未完成的验收

- [ ] 两项正式性能门槛。
- [ ] 真实 Windows Terminal 中文输入法、颜色、视觉布局，以及用户终端环境下的完整交互验收。
- [ ] 远程 Windows CI：历史 beta.1 检查表曾写“已配置 PowerShell 7.4/7.5/7.6”，当时与工作流不符。当前工作流配置 PowerShell 5.1 与运行器提供的 PowerShell 7、固定 PSReadLine 版本及 OSC／pipe；远程运行结果仍需确认。
- [ ] 公开发布前的许可证与分发审核；本轮不执行公开发布，不能把未签名包描述为已签名。
- [ ] E 盘恢复后迁移至独立的 `E:\OpenSource\shellsense-rs`。原 inshellisense 仓库保持不变。

E 盘离线期间，唯一源码、发布包、原始测量和回滚版本保留在 C 盘工作副本。交付完成后只删除临时编译缓存；待迁移并校验完成后再删除 C 盘剩余副本，避免丢失唯一交付物。

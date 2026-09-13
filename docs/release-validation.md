# 0.5.0-beta.2 发布验证记录

日期：2026-09-13。结果分为本机验证、远程验证和人工验收。

## 本机验证

| 范围 | 结果 |
|---|---|
| Windows x64 Rust | 构建、核心测试、Clippy、格式检查通过 |
| PowerShell 7 | 适配器、真实 ConPTY、OSC/pipe、启动 profile 回归通过 |
| Windows PowerShell 5.1 | PSReadLine 2.0.0 与 2.4.5 的适配器及 OSC/pipe 回归通过 |
| 默认入口 | 隔离 PATH 中没有 pwsh 时，无参数启动选择系统 5.1；指定 Shell 与自动启动保留版本 |
| profile | 重复启用/关闭、原始字节保留、中文路径、using/param、递归防护及退出回到父会话通过 |
| Linux x64 | 本机 WSL Ubuntu 22.04 的 128 项核心测试及 Clippy 通过；CI 使用 Ubuntu 24.04 |

安装测试使用临时目录和隔离配置，不写入本机用户 PATH，不修改真实 profile。文件生命周期验证包含安装、升级、回滚、卸载、校验失败和文件占用后的恢复；下载测试以固定 Release 响应代替网络，覆盖稳定版、Beta、指定版本、草稿过滤和中断保护。公开 Release 尚未创建，真实在线安装留在首发公开后验证。

## 开销记录

使用 Windows x64 release 构建、真实 ConPTY、OSC 传输，保留 OS 文件缓存，关闭历史写入。运行时没有并行构建或其他回归任务。

| 测量项 | PowerShell 5.1 | PowerShell 7 |
|---|---:|---:|
| 直接启动 P50 | 791.0 ms | 894.2 ms |
| profile 启动 P50 | 956.2 ms | 1238.3 ms |
| JSON 层初始化 P50 | 215.0 ms | 4.0 ms |
| 热态菜单 P95 | 38.5 ms | 36.8 ms |

启动采用 10 组交替配对，从进程创建测到首条命令执行完成。profile 测量通过隔离外层会话加载受管区块，子会话加载当前版本的正常 profile；外层测试入口跳过真实用户 profile。两版本的 profile 和运行时不同，不能将两列差值解释为兼容层成本。JSON 层另取 20 个新会话的初始化计时，5.1 包含框架程序集加载与 C# 编译。

菜单包含六类固定场景，每类缓存命中/未命中各 30 个热态样本。首次查询和动态提供器首次等待单独处理，不计入热态统计。当前数值超过 20 ms 目标，且样本数低于正式验收要求的每模式 300 个；本轮性能结果用于识别开销，性能目标尚未验收。启动表记录总时间，50 ms 的启动增量目标需独立基线配对验证。

测量快照及二进制 SHA-256 见 [原始数值](validation/0.5.0-beta.2.json)。测量脚本位于 `tests/release_metrics.rs`，仅显式运行 release 忽略测试时启用；菜单由 `blueberry beta-probe` 测量。发布包在最后一次构建后独立执行完整性检查。

## 远程与人工验收

- GitHub Actions 尚未运行；Windows 2022、Ubuntu 24.04、macOS 15 ARM64 的远程结果待推送后确认。
- macOS 本机没有执行环境，交互支持列在 README 的未来展望中。
- 发布者仍需在真实 Windows Terminal 按安装指南检查中文输入法、字体图标、窄窗口、缩放、F1 翻页和自动启动体验。
- 首次公开 Release 后，在干净用户环境验证 README 一条命令安装及 PATH 生效。

复现自动回归：`scripts/verify-local.ps1 -Shell pwsh.exe`。发布步骤见 [发版指南](releasing.md)。

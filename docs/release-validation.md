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

- [首轮 GitHub Actions](https://github.com/Drtxdt/Blueberry/actions/runs/34767062275) 已于 2026-09-14 检查：格式通过，三平台构建通过；三平台 Clippy 和两项 PowerShell 5.1 交互任务失败，打包被跳过。
- 本轮修复处理 Rust 1.98.1 的 match/return 检查、PSReadLine 自动加载版本冲突，以及旧版 ConPTY 中测试快捷键修饰键丢失。修复后本机 Rust 1.98.1 Clippy 和 144 项核心测试、5.1 配合 PSReadLine 2.0/2.4.5 的完整 OSC/pipe 回归通过；多版本并存的 2.0 定向回归通过。
- [后续运行](https://github.com/Drtxdt/Blueberry/actions/runs/34768060171) 的 Clippy、Ubuntu 核心测试及命令刷新回归通过。剩余失败涉及用途搜索快捷键注入、Windows 大输出测试生成数据过慢、macOS 监听事件使用真实路径。对应修复统一终端测试的 CSI-u 输入、预生成大输出样本，并规范化监听路径；保留原有超时、输出预算和测试矩阵。新增 Unix 符号链接回归，本机 Linux 的三项监听测试通过。
- [鼠标兼容修复后的运行](https://github.com/Drtxdt/Blueberry/actions/runs/34769112387) 中，格式、三平台核心、两版 PSReadLine 和新增原生鼠标检查共七项全部通过。Server 2022 验证 VT 鼠标，Server 2025 验证原生鼠标，两者均为打包前置条件。随后打包步骤暴露版本号读取的位置参数错误，已改用明确的 `-Path` / `-Pattern`。最新完整结果见 [main 的 Actions](https://github.com/Drtxdt/Blueberry/actions/workflows/ci.yml?query=branch%3Amain)。
- macOS 本机没有执行环境，交互支持列在 README 的未来展望中。
- 发布者仍需在真实 Windows Terminal 按安装指南检查中文输入法、字体图标、窄窗口、缩放、F1 翻页和自动启动体验。
- 首次公开 Release 后，在干净用户环境验证 README 一条命令安装及 PATH 生效。

复现自动回归：`scripts/verify-local.ps1 -Shell pwsh.exe`。发布步骤见 [发版指南](releasing.md)。

## 本地增量：右键粘贴与中文设置页

本次增量包含按子程序需求切换鼠标捕获，以及 `blueberry config edit` 中文设置页。设置页覆盖常用选项、即时预览、保留注释的 TOML 保存和外部修改提示。

按本轮要求，仅执行 `cargo build --release --locked` 编译检查。上述历史自动回归结果对应之前的提交，本次交互行为尚未实测：包括 Windows Terminal 右键粘贴、外部程序鼠标模式切换、窄窗口设置页和返回 PowerShell。现有 CI 保持原样；本次增量本地交付，未主动推送。

# 0.2 回归验证

在本机 Windows + PowerShell 7 的真实 ConPTY 上执行；不修改 profile，不写入 PSReadLine 历史文件。测试创建的 Git fixture 只回显参数，不操作仓库；真实 Cargo 用 `--version` 验证接受后的命令。

最终源码通过 69 个 Rust 测试、PowerShell 适配测试、Clippy（警告视为错误）和格式检查。

| 范围 | 验证内容 |
| --- | --- |
| 发现 | 本机 Cargo 链接、普通 exe/cmd、相对链接链、断链、目录链接、重复 PATH、PATHEXT 顺序 |
| 分批与缓存 | 超过 8192 项仍继续扫描；大量缺失/空目录分批；旧版本和部分缓存失效；只保存完整索引；cache hit 不写盘 |
| 会话快照 | 创建 700 个 alias，完整接收并可执行；删除旧 alias、加入新 alias 后刷新正确替换 |
| 上下文 | car、cargo build --rel、git log --o、git -C "含 空格目录" log --o；status 不泄漏 oneline；-- 后不提供选项 |
| 实际插入 | PSReadLine 读取和校验替换；接受 Git 选项、--ignored=matching 内联值后执行并验证真实参数；中文文件、emoji 文件可正确接受和执行 |
| 终端交互 | Esc 关闭菜单、历史反向搜索、80/120 列缩放、外部程序读取键盘、协议按键不泄漏 |
| 中文说明 | 所有内置表和根说明非空、有中文、无占位文本；未知来源、alias 目标用途、精确上下文自定义 |
| 重载和 JSON | 两次更新说明即时重算；无效配置保留上次有效设置；JSON 原有字段和替换范围保留 |
| 渲染 | 中文按终端单元格截断、关闭说明列、控制字符处理、恢复底层屏幕和相同菜单不重复写出 |
| 性能工具 | 标准中位数、nearest-rank P95；30 对启动、每种应用缓存状态至少 300 次热态菜单；trace 默认关闭且不含正文 |

复现：

```powershell
cargo test --locked
pwsh -NoProfile -NonInteractive -File tests/adapter.tests.ps1
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

链接夹具在本机具备创建权限且实际通过；无此权限的其他机器可能跳过人工链接夹具，所以仍应保留本机 Cargo 的集成验证。ConPTY 测试覆盖上述行为，尚不代表所有 Windows Terminal 配色、键位、全屏程序和多行 PowerShell 语法已验收。

规格取值规则对照官方 [Git diff](https://git-scm.com/docs/git-diff)、[Git status](https://git-scm.com/docs/git-status)、[Git pull](https://git-scm.com/docs/git-pull)、[Git push](https://git-scm.com/docs/git-push) 与 [Cargo build](https://doc.rust-lang.org/cargo/commands/cargo-build.html)。可选值与必需值分别建模，未知选项不猜测后续 token 的含义。

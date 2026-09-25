# Blueberry 更新日志

## 0.5.0-beta.7

- 收藏和模板改为跨进程锁保护的原子写入；损坏文件、未知 schema 与外部修改会报错，后台重载保留上一份有效快照。
- 会话工作台和独立工作台共用参数校验与 PowerShell 引用；支持从 `git switch -c`、`cargo test`、`docker compose up/logs` 的当前输入继续填写。
- 增加经 SHA-256 审批的仓库命令包及 `blueberry packs list/review/approve/revoke`；文件变化会立即停用批准。
- 帮助入口识别和历史加载移到后台，历史读取有预算；`doctor --json` 报告构建、Shell、PSReadLine、传输方式和命令库状态。
- 性能与兼容性验收以同一发布包的原始证据为准；证据包随 Release 提供。

## 0.5.0-beta.6

- 修复本机帮助知识合并：残缺或无法确认完整性的帮助输出只补充内置规则，不再删除 Cargo、pnpm 等工具的可靠命令。
- Cargo 使用 `cargo --list` 获取本机命令和扩展命令，支持 `build, b` 等别名并保留中文说明、参数格式和示例。
- 统一工具知识登记，补齐现有开发工具，并新增 Java、构建、基础设施、Windows、云平台和数据库客户端补全。
- 云资源、数据库对象和远程集群继续通过 `Ctrl+Alt+D` 主动读取；本机项目、环境、profile 和连接配置自动更新。
- 修复自动学习入口信任判断，拒绝仅凭相同文件名伪装成已登记工具的程序。
- 稳定 PowerShell 5.1 与 PSReadLine 2.0.0 的补全菜单退出和历史导航回归场景。
- 修复发布前格式检查，并补齐发布脚本要求的版本更新日志。

## 0.5.0-beta.5

- 修复自动学习入口信任判断和历史导航回归测试。
- 修复 Rust 格式检查；该标签的发布包因缺少对应更新日志章节而未生成，由 `0.5.0-beta.6` 取代。

## 0.5.0-beta.3

- 普通命令行编辑期间解除鼠标捕获，恢复 Windows Terminal 的右键粘贴和文字选择。
- 按外部程序需求转交鼠标输入，返回提示符时清理残留鼠标模式。
- 新增 `blueberry config edit` 中文设置页，支持常用配置、鼠标操作和即时预览。
- 设置保存保留 TOML 注释与高级配置，提示外部修改并支持打开原文件。
- 同步 Cargo 锁文件版本，修复发布工作流的 `--locked` 构建失败。

## 0.5.0-beta.2

- 统一 Blueberry 名称、命令及配置目录。
- 支持 Windows PowerShell 5.1 和 PowerShell 7，兼容 PSReadLine 2.0。
- 增加安装引导和 `startup enable / disable / status`。
- 发布草稿自动打包，CI 覆盖三平台核心及 PowerShell 5.1 交互回归。
- 重写安装、使用和发版文档。

## 0.5.0-beta.1 — 2026-09-13

未签名的本地 Beta 候选版本，尚未公开发布。**性能验收未通过**，完整方法与原始数据见 [性能报告](docs/performance-v0.5.md)。

- 新增 Git/Cargo/npm/pnpm 项目感知、声明式 TOML 规格、中文用途/离线示例、三种主题和 F1 详情、手动原生补全、本地散列选择排序，以及 explain/doctor/specs/learning 管理入口。
- Windows 输入改为 Win32/VT 双层增量解码，修复中文与 emoji 粘贴、CRLF、快捷键和错误序列恢复；区分多行编辑与执行，插入校验实际缓冲区并保留后缀、引号和撤销。
- 修复缺失目录监听、workspace 新成员、Git 状态递归监测及旧 watcher 根清理；修复活动会话重新写回已清除选择统计的问题。
- 在原 runspace 分批枚举 shell 命令，复用绑定快照，后台索引/缓存/项目数据，集中输出并避免无变化重绘。说明开启热态 P95 为 36.80 ms，关闭为 37.24 ms；保留 profile 的启动增量 P50 为 143.40 ms，均未达到 20/50 ms 门槛。
- named pipe 对照 P95 为 37.09 ms，未显示明确收益，默认继续 OSC。早期 C# 桥接试验未满足收益标准，不随包提供 DLL。Windows release 使用静态 CRT，仅导入 Windows 系统 DLL。
- 本机完整 Rust 回归 144 项通过，pipe 下 8 项真实交互通过；PowerShell adapter、release CLI、fmt/clippy/build 通过。真实 Windows Terminal 输入法/颜色视觉及远程 CI 矩阵尚未完成。
- 提供本地 ZIP/哈希/离线许可证及安装、升级、卸载、回滚工具；不改用户配置。粘贴正文不进入 ShellSense trace 或学习记录，PSReadLine 原有历史策略不变。

## 0.2.0 Alpha — 2026-09-12

- 修复链接形式的 Cargo、rustc 等 PATH 命令漏项，保留 PATH/PATHEXT 及 alias 优先级。
- 后台分批扫描，旧缓存自动失效，不保存不完整结果；命令快照取消 512 项限制，刷新可移除已删除的 alias/function。
- Git/Cargo 等规格按子命令层级、选项作用域和取值规则补全，修复 `git log --oneline` 等漏项。
- 新增默认关闭的数字 trace；使用已有 System.Text.Json 与 PSReadLine 直接 API，缓存写入退出查询线程，减少重复重绘和输出。
- 内置规格加入中文用途，未知工具标注来源，支持完整上下文的自定义说明及热重载。
- 增加真实 ConPTY 回归、标准中位数/nearest-rank P95、缓存命中/未命中和说明开关对照。

性能目标仍未全部达到，首个会话命令快照仍有同步成本；最终结果及限制见 [性能报告](docs/performance-v0.5.md)，本版本不复制未冻结的性能数字。源码、发布包和回滚版当前保留在 C 盘，待 E 盘恢复后迁移，交付后清理临时构建缓存。保留上一版 exe，升级不覆盖用户配置。

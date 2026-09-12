# 更新记录

## 0.2.0 Alpha — 2026-09-12

- 修复链接形式的 Cargo、rustc 等 PATH 命令漏项，保留 PATH/PATHEXT 及 alias 优先级。
- 后台分批扫描，旧缓存自动失效，不保存不完整结果；命令快照取消 512 项限制，刷新可移除已删除的 alias/function。
- Git/Cargo 等规格按子命令层级、选项作用域和取值规则补全，修复 `git log --oneline` 等漏项。
- 新增默认关闭的数字 trace；使用已有 System.Text.Json 与 PSReadLine 直接 API，缓存写入退出查询线程，减少重复重绘和输出。
- 内置规格加入中文用途，未知工具标注来源，支持完整上下文的自定义说明及热重载。
- 增加真实 ConPTY 回归、标准中位数/nearest-rank P95、缓存命中/未命中和说明开关对照。

性能目标仍未全部达到，首个会话命令快照仍有同步成本；结果及限制见 [性能报告](docs/performance.md)。保留上一版 exe，升级不覆盖用户配置。

# 工件保留与性能采集

已有历史归档保留，不重写 Git 历史。新诊断 ZIP 和超过 5 MiB 的基准文件不得提交
`docs/benchmarks/`；原始文件使用已忽略的 `artifacts/`，Actions 工件默认保留 90 天。
发布时将经过验证的完整证据包作为 Release 附件长期保留，不能只依赖会到期的链接。

Git 中保留报告摘要、统计口径和 SHA-256。发布证据 schema 2 可以增加 `raw_artifacts`
数组，每项为 `run_id`、`artifact_id`、工件下载 ZIP 的 `sha256`，以及 `files` 映射
（证据目录下的相对路径 → 文件 SHA-256）。工件须属于同一仓库、该固定 run 和源码提交。
使用 `scripts/materialize-release-artifacts.py <evidence.json>` 下载并校验后，现有发布
检查器读取同样的相对路径并将原始文件打入最终证据包。历史内联文件仍然可用。
缺失、过期、摘要错误、重复／越界路径或覆盖既有不同证据都应阻止发布。

## 正式采集

先用独立目录试跑，再冻结成功 CI 原包、探针和编辑器摘要。三个支持组合每组至少
30 对启动样本，六场景 × 两种缓存状态 × 说明开关每组至少 300 个热态样本。
正式计时关闭 trace；profile、机器、电源策略、Shell／编辑器版本、尺寸和配置均需记录。
对照沿用 plain／beta.6／candidate direct／inshellisense，nested 兼容指标单独保存。

`scripts/run-editor-matrix.ps1 -Formal -ContinueOnGateFailure` 保留原有参数和身份要求，
但超出 50 ms／20 ms 门槛后继续采集剩余组。`measurement_complete` 表示数据收齐；
`automated_performance_passed` 只有正式批次全部达标才为 true。最终存在超标时仍返回
非零退出码。身份不符、采集失败、样本不足、统计错误立即中止，不能算作完整慢样本。
正式批次不跨中断续跑；保留失败批次并使用新目录重测，不删掉慢样本。

四方对照 `scripts/run-comparison.py` 的 `measurement_complete` 只表示采集有效且完整，
旧 `passed` 字段作为该含义的兼容别名保留。`performance_passed` 为 null，性能门槛由
独立 direct 正式矩阵验收，不能把四方对照采集成功解释为性能达标。采集器、计划文件、
程序依赖及 profile 均校验是否在批次中变化；缺组、短样本和非有限／负延迟均失败。
对照的 `menu` 是探针模型中首个预期候选出现的时间，不代表完整候选或实际屏幕像素；
这两个未测阶段在 `observed_stages` 中明确标注。对照能力缺失需保留三次探测记录。

分配／阶段 trace 与进程树资源采集使用独立诊断批次。ConPTY 探针观察到菜单不等于
屏幕像素呈现时间；没有可靠测量的视觉阶段记为未测。性能改动须用相同环境复测，
不得把不同版本的小样本连接为正式性能趋势。

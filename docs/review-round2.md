# 第二轮复审：补全与启动归因

基线为公开 v0.5.3 的包内源码 `8b97ebbc`，被复审的产品源码为 `9d88a944`，本轮起点为 `43407bb`。以下正式数字来自[已封存的跨版本回执](review-hardening-remote-validation.json)，不是新候选的验收成绩。

## 目录无法读取

`PathCache::complete_with_mode` 遇到文件冒充目录或拒绝网络路径时，返回终态诊断。原来的 `completion::merge` 只转发“未完成且未扫描”的来源诊断。修复后，`sources::collect_lane` 明确标出要展示的消息，错误行不产生可接受的编辑，也不会留下同次查询中已被删除的旧候选。普通项目来源说明仍留在来源状态里。

回归 `unavailable_path_renders_a_final_status_without_retaining_an_old_edit` 修复前得到零行，修复后能看到两类提示；`unmatched_editor_manual_completion_preserves_native_editing` 用真实 PowerShell 注册参数补全器，验证错误行出现后 Tab 仍能填入原生候选。

## 360 对原始样本究竟慢在哪里

从每个 Actions 工件读取经 SHA-256 校验的 `summary.json`，先对**同一样本**计算“完整静态菜单到达 − 首个静态候选到达”，再逐对比较。下表是这个区间的毫秒数；每格依次为**发布版 P50 / 受审版 P50 / 逐对差值 P50**。每组 30 对，独立 runner、缓存状态与批次均未混合。

| Shell / PSReadLine | 冷 1 | 冷 2 | 暖 1 | 暖 2 |
| --- | ---: | ---: | ---: | ---: |
| PS5.1 / 2.0.0 | 0.23 / 48.62 / +46.93 | 0.21 / 45.60 / +45.38 | 0.23 / 121.51 / +121.29 | 0.22 / 124.01 / +123.80 |
| PS5.1 / 2.4.5 | 0.26 / 49.33 / +48.96 | 0.32 / 48.84 / +48.49 | 0.31 / 117.13 / +116.73 | 0.31 / 116.61 / +116.19 |
| PS7 / 2.4.5 | 0.36 / 87.84 / +87.45 | 0.40 / 87.99 / +87.24 | 0.37 / 274.01 / +273.59 | 0.42 / 266.51 / +266.06 |

复算：`python scripts/analyze-static-readiness.py artifacts/review-hardening/remote-41648304986`。输入是三份原始 ZIP；测试防止错位样本与反向时间戳被纳入计算。

**能确认的事实：** 两侧均由同一探针观测带候选的完整状态栏。发布版在启动子进程后即用父环境扫描，菜单使用 `Completion.incomplete`；受审版等待权威子环境，状态栏使用 `Completion.pending`。探针的“完整”指屏幕上的状态栏不再显示加载，不等价于索引扫描刚刚结束。冷暖区分的是系统命令元数据，PATH 索引在每一对中都从新数据目录开始。因此既不能把这张表当作目录扫描耗时，也不能把它当作首个提示符增量。

**诊断边界：** 在 `direct::run` 记录子环境发布、接收与 PATH/PATHEXT 相对父进程的差异（仅两个标志位）；在 `Worker::new` 区分缓存验证命中、未命中和强制刷新，记录准备、每批枚举、快照、批间工作及最终完整或带错误的结束状态。批间时间包含查询、发布和调度，不能直接当成 CPU 调度等待。保留原有权威环境、代际取消和缓存验证路径。历史正式样本未开追踪，不能把这些历史差值全部归因给任何单一阶段。

## 连续按键下的帧确认

`Bridge.Refresh` 在编辑线程上测量发送 `presented` 的实际同步耗时（编码、锁、写入和刷新）；`direct::run` 记录读取至分派的等待和当时已产生的更新帧数。通过修订号和帧号关联两端，帧号差不等于管道字节数。未改变候选接受或确认规则。

本地 PS7 / PSReadLine 2.4.5 的 `direct_path_navigation_stays_selected_and_orders_burst_acceptance` 使用真实 ConPTY。下表仅纳入发出连按至看见 Tab 插入之间实际绘制的帧，两个场景未合并。这是打开追踪的诊断运行，样本不足以证明没有输入瓶颈。

| 场景 | 绘制 / 两端匹配 | 同步写 P50 / 最大 (ms) | 分派等待最大 (ms) | 确认时领先帧数最大 |
| --- | ---: | ---: | ---: | ---: |
| 10 次 Down + Tab，无注入延迟 | 11 / 11 | 0.063 / 0.187 | 0.067 | 0 |
| 3 次 Down + Tab，注入 40 ms 帧延迟 | 4 / 4 | 0.063 / 0.090 | 0.533 | 0 |

复现：`cargo test --test direct_terminal direct_path_navigation_stays_selected_and_orders_burst_acceptance -- --nocapture`，分别对输出的路径运行 `python scripts/analyze-presented.py <trace.jsonl>`。每个目录的 `burst-window.json` 标明输入、QPC 边界与注入延迟，失败另外保留原生屏幕快照。PS7/OSC 缩放和 PS5.1/2.4.5 底部导航的历史故障仍未定位。

## 真实 Windows Terminal 的人工验收（待执行）

在官方 Windows Terminal 的新标签页，从仓库运行 `./scripts/check-wt-menu.ps1`。它使用隔离临时目录、候选 EXE、无 Profile 的子 Shell 和 F6 手动触发，不安装 Profile。保留屏幕截图、WT 版本、行列数和脚本输出的追踪目录。

1. 输入 `Get-ChildItem .\many\absent`，按 F6。观察“正在加载…”到“没有匹配的候选”的过渡；如果扫描太快未看清加载，单独记录“未观察到”。按普通字符后输入应继续。
2. 输入 `Get-ChildItem .\blocked\中文`，按 F6。应只看到目录错误，不应出现可选行；Esc 后原生光标和行内容应还在。
3. 在窄窗口和缩放后的窗口重复。在子 Shell 内执行 `function prompt { ('长提示符' * 12) + '> ' }`，用中文输入法组词、选字，在窗口底部触发和关闭菜单；核对换行、叠盖以及关闭后原有文本是否恢复。

ConPTY 的 Tab 验证、渲染单测和本表的帧时序不能替代上述人工结果。

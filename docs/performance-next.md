# 三轮性能优化与发版验收

执行依据：用户 2026-10-12 确认的计划。功能、CLI、协议、配置与缓存格式保持兼容；
复用 `codex/v053-reliability-performance` 与 PR #4。公开版仍为 0.5.2。

## 版本判定与共同发布门槛

三组合启动额外开销 P50 均 ≤50 ms、72 个正式热态组 P95 均 ≤20 ms，其他指标
无明显回退时发布 0.6.0。三轮结束后完整有效数据仍未全部达标时发布 0.5.3，保留
有收益的优化并列明未达项。采集无效不能作为优化失败或选择补丁版的理由。

两版均要求缩放时历史提示符完整恢复，功能、安装与最终包本机验收通过。
外部试用未执行；旧版人工豁免与此前其他 ZIP 的结果不能替代最终包验收。
明显回退采用两批独立复测重复确认：延迟增加超过 max(1 ms, 10%)、内存增加超过
max(5 MiB, 10%) 或空闲 CPU 增加超过 0.1 个百分点；CPU 使用单核百分比。

## 基线与轮次

固定 `2fe57b9eada4a868f2d5cd9218b546342cd43c67` CI 原包为优化前基线，EXE SHA-256
`99E696A60DE1D96718A6B56C226B57375AD3C138B16FA04B20775902A516B8CC`。
2026-10-10 正式矩阵只完成两份报告，进程已退出；资源队列也未执行，原始记录保留。
`artifacts/perf-baseline-2fe57b9-20261012/` 预检完整，正式批次在 path 的缓存命中
检查时无效中止：命令数组未变化，但 System32 目录时间戳变化触发了缓存重建。
这是夹具环境不稳定，不能用作性能不达标的结论。全部原始记录保留。

`scripts/snapshot-benchmark-path.py` 固定 System32 和临时 Codex PATH 目录中的实际
命令文件字节，保留名称、数量及 PATH 优先级；其他目录仍使用原位置。快照保存在
`artifacts/perf-path-snapshot-20261012/`，只发布清单与 SHA-256，不发布复制的系统 EXE。
首次快照预检因 Git 对过长夹具路径报错而中止，记录亦保留。采用较短的新证据目录
`artifacts/pb2-fp-1012/` 后，九份预检报告完整；正式协调器随会话中断退出，已记为
interrupted。保留五份完整有效报告：两份 30 对启动、三份热态报告共 10,800 个样本。
两组合启动增量 P50 分别为 327.37615 ms、328.56455 ms；已完成热态组达标。
剩余组未执行，批次不完整，不续写或用于版本判定。

旧探针的静态 `first_candidate_menu` 要求目标候选及完成状态同时出现，无法单独
表示首候选。新版报告增量加入 `candidate_stage_schema=1` 和 `complete_candidate_menu`，
保留 `first_menu`（任意首候选）、`first_candidate_menu`（夹具目标首候选）和输入回显。
所有场景的门槛统一使用完整候选；原始数组必须满足时间顺序，探针观察与像素可见
仍分开。新正式批次须绑定新探针摘要，宿主仍使用原 `2fe57b9` 包。

- 基线和最终候选：PS5.1／2.0.0、PS5.1／2.4.5、PS7／2.4.5，每组合 30 对启动，
  六场景 × miss／hit × 说明开关每组 300 个热态样本。四方对照与 nested 单独留证。
- 资源：每组合至少五个独立会话，每次至少 30 秒，记录进程树工作集、private bytes
  和空闲 CPU。正式计时关闭 trace，记录实际 Shell／DLL、profile、电源策略及依赖摘要。
- 第一轮按诊断拆解启动，第二轮处理补全过程的主要阶段，第三轮处理分配、复制、
  缓存生命周期、空闲唤醒及剩余热点。每轮需诊断、可审查修改、同条件 A/B 和正确性结果。

已有独立三对启动 trace 是诊断数据，不计正式验收；首个资源准备明显较慢，后两次
约 2 ms。QPC 阶段时间和日志接收时间必须分开，不能将 queued trace 的接收延迟
误认为对应阶段的执行耗时。屏幕像素可见时间未测。

第一轮候选 `ef2e6b6` 将系统命令元数据 helper 的进程创建移出 direct 分派线程，
保留取消、超时和进程树清理语义。CI run `38264480753` 成功；原包已按 artifact
`11756420417` 下载并验证摘要，保存于 `artifacts/r1-ci/`。同条件 A/B 尚未执行，
不能据此宣称提速或完成第一轮。

2026-10-13 的 PS7 四方资源预检全部完成，各模式各一个独立会话，属于预检。
PS5.1／2.0.0 预检中 direct 的初始化辅助进程在第一次快照期间退出，正确判为
无效，原始记录保留于 `artifacts/rp1013/`。采集器新增最多 30 秒的进程树稳定等待，
保留全部初始化快照；正式 30 秒区间一旦开始，不删除样本或重启计时。
系统线程性能计数器每次查询约 11 秒，服务端过滤也未改善，现移入独立
`-IncludeThreadCounters` 诊断，不能计正式资源验收。正式批次记录进程树内存、
CPU、线程数量与句柄数量，线程上下文切换计数明确未测；真实子进程退出回归覆盖此路径。

后续独立批次 `artifacts/pb2-1013/` 的三组合正式资源采集全部完成：每模式五个
独立会话，共 60 个；已逐份校验摘要并从全部原始快照复算，摘要见
[资源基线](benchmarks/v0.6/baseline-resource-2fe57b9-20261013.json)。这只确认资源
采集完整，尚未完成候选的两次独立 A/B，也不表示整体性能通过。原始记录仍只在
忽略目录，尚未上传 Actions Artifact 或创建 Release 证据包。

同一队列的延迟预检在测量开始前因隐藏 PowerShell 的 GBK 解码破坏 UTF-8 doctor
JSON 而停止；已增加原生中文／引号／emoji JSON 与调用者编码恢复的回归。
延迟矩阵重新使用新目录 `artifacts/pl3-1013/`，不续写无效批次；固定的观察探针
来自干净 `f2f9e99` release 本机构建，SHA-256
`9E847A768DB1CB22A85134EB0605A39784BEC34070576F7C7B817C5F62EE71B2`。
它的来源单独记录，不能称为 CI 原包；宿主仍是原 `2fe57b9` CI 包。

## 缩放发布阻塞

普通 Esc 关闭菜单的历史 RGB／Unicode 恢复已获用户验证；缩放后的恢复仍走
`CHAR_INFO` 快照，不能保留完整 RGB 属性。新增独立矩形页缩放约束测试和明确标为
已知发布阻塞的产品回归，后者须对冻结 CI EXE 显式运行，不计入普通 CI 的通过数。

Windows Terminal 的 `PageManager::Get` 会将后台页调整到当前可见尺寸，使用
`ResizeTraditional`，不会自动重排；访问已经缩小的后台页会裁掉超出尺寸的内容。
不能把后台页看作跨缩放的完整富文本存储。来源：
[PageManager.cpp](https://github.com/microsoft/terminal/blob/main/src/terminal/adapter/PageManager.cpp)、
[TextBuffer::ResizeTraditional](https://github.com/microsoft/terminal/blob/main/src/buffer/out/textBuffer.cpp)。

2026-10-13 本机已安装 Microsoft.WindowsTerminal 1.24.12741.0；复核对应发布标签
[v1.24.12741.0](https://github.com/microsoft/terminal/releases/tag/v1.24.12741.0) 的
[PageManager](https://github.com/microsoft/terminal/blob/v1.24.12741.0/src/terminal/adapter/PageManager.cpp)
仍使用上述后台页缩放路径。该版本的
[UIA 文本属性实现](https://github.com/microsoft/terminal/blob/v1.24.12741.0/src/types/UiaTextRangeBase.cpp)
只提供部分格式属性；未知属性返回不支持。因此 RGB 可读不能单独证明全部终端属性
可以完整往返，也不能替代对自己的终端窗格及软／硬换行边界的验证。

用户本机 Windows Terminal 的只读 UIA 诊断确认前景色、背景色 RGB、中文、emoji、
组合字符及软折行文本可读。记录位于 `artifacts/console-attributes-capability-20261012/`。
初版 `supported=false` 源于 FindText 返回范围多一个空格；schema 2 分别报告 RGB、
标记前缀完整性与范围精确性，原记录不覆盖。实测宽字符矩形宽度多算一列，需使用
相邻字符起点核对；逐字符 181 个标记约 154 ms，不能放到每次按键的计时路径中。
目前只验证接口能力，尚未实现或验收跨缩放恢复，也未测屏幕像素显示时间。

独立 Unicode／折行阻塞回归已对固定 `2fe57b9` EXE 显式执行并失败：
`resized history lost Unicode`，原始记录保存在
`target/input-protocol-evidence/direct-resize-prompt-colors-7pUzCo/`。
它独立于 RGB 断言，避免颜色先失败掩盖 emoji／组合字符丢失；新测试仍明确标为
已知阻塞，忽略状态不表示通过。

发布证据检查新增最终 ZIP 身份、三组合 direct/nested 的逐项人工记录、连续缩放、
历史 RGB、pip/npm Ctrl+C 及 VS Code/PS7 direct 检查。0.5.3 性能延期也须保留完整
正式矩阵、三轮诊断和独立 A/B；旧的“修复先交付”空性能记录不再满足本轮计划。

恢复须保留原位置覆盖、完整字符与属性，并正确处理换行；滚动腾空白方案未获用户接受。
用户 2026-10-13 再次明确：所有历史属性都必须恢复，包括 UIA 尚未提供的属性；
不能将 Starship 常用样式或部分属性恢复作为验收通过，公开发布继续阻塞。
发布验收增量加入独立 `history_attributes` 项，要求三个组合的 direct 与 nested
均有最终包的实际通过记录；只有 `history_rgb` 或 `live_resize` 通过不能代替。
如果现有接口无法满足，提交复现及限制证据，发布继续阻塞，不以忽略回归表示通过。

最终选择版本后，使用成功 main CI 的原包重新绑定性能、自动回归、升级回滚与人工结果，
验证 Release 草稿下载字节一致后才公开。完整证据随 Release 长期保存，Actions 原始工件
默认保留 90 天，Git 只保存摘要、SHA-256 与定位信息。

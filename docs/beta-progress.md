# ShellSense Beta 开发与验收记录

目标平台是 Windows Terminal + PowerShell 7。开发顺序为交互可靠性、项目功能、性能优化、0.5 Beta 本地交付；本文件区分代码落地、测试资产和正式验收，不把其中一项自动视为 Beta 已通过。

当前源码版本为 `0.5.0-beta.1`，本轮只冻结本地 Beta 准备状态。已有证据显示 0.4 基线的完整 123 个 Rust 测试通过；0.5 新增的粘贴等宿主交互仍在验证，不能把这项基线记录写成最新全量通过。正式启动 A/B 和宿主性能数据仍待 root 更新，本文件不宣布性能达标。

## 0.3：已落地的基础能力

以下内容已经进入独立的 `shellsense-rs` 代码与协议：

- Rust 核心、PowerShell 7 接入层和 ConPTY/OSC 协议；保留旧的 `complete --json` 字段、UTF-8 替换范围，以及编辑状态和插入确认能力。
- 命令发现按 PATH/PATHEXT 保留顺序，处理普通可执行文件、链接、重复 PATH、alias/function/cmdlet 和分批 shell 快照；未完成的快照不会被当作完整缓存。
- Rust 规格目录由版本化 TOML 在构建时校验和生成；用户规格仅从配置目录加载，支持 `specs check`、`specs list`、错误诊断和热重载。
- 规格节点、选项作用域、重复/互斥/依赖、短选项附值、长选项内联值和 `--` 分隔符已经有统一表示。未知参数仍按普通输入处理。
- 中文用途说明、`[descriptions]` 覆盖、未知项降级、菜单详情和 `complete --explain`/`doctor` 接口已经接入。
- 查询使用当前可用快照；项目数据和路径数据在后台分批采集，按请求号和采集代次丢弃过期结果，并在目录或清单变化时失效相关缓存。
- 菜单接受请求会校验实际 PowerShell 缓冲区、UTF-16 光标和替换范围；纯协议事件不会无条件擦除整块菜单。PSReadLine 的行内预测仍由其自身负责。

## 0.4：已落地的项目感知与交互能力

以下内容已经有代码或协议实现，最终宿主验收仍见下文：

- 内置 Git 子命令节点使用通用 `provider = "git"`。provider 根据子命令、选项作用域、位置参数和 `--` 选择引用、远端、工作树、状态或路径数据；Git 查询固定为本地只读调用。
- 选项正在等待值，或使用 `--name=value` 内联值时，只使用选项自己的 `option.provider`，不会继承命令节点的 provider。没有显式 provider 的 `git log --format <值>` 不会误启动 Git 动态查询；`--source`、`--onto` 等显式引用选项仍可使用 `git.refs`。
- 已覆盖 Git 的分支、标签、远端、worktree、状态路径以及 `-C` 上下文；`checkout --` 后只提供路径。Git 非零退出或预算耗尽的结果标记为未完成，不写入完整缓存。
- Cargo workspace 包、features、bin/example/test/bench 目标，以及 npm/pnpm 的本地脚本、workspace、依赖和选择器上下文已经接入；不运行构建、项目脚本、安装或 fetch。递归项目扫描跳过生成目录，同时保留显式声明的路径。
- 菜单已提供深色、浅色和高对比主题、候选数量、来源、后台加载状态、F1 中文详情、可配置快捷键和内部前缀；原生 PowerShell 补全仅手动触发，并使用其真实替换范围单独展示。
- 本地选择统计只保存散列标识，可关闭和清除；排序不能越过精确匹配、作用域规则和 shell 命令优先级。配置、规格和说明覆盖支持热重载。

## 0.5.0-beta.1：本地 Beta 交付范围

### CLI 与离线诊断

当前 CLI 已提供以下入口，命令帮助和源码定义已核对：

- `complete --json --explain`：不启动 PowerShell，回显解析上下文、选中的规格、数据源和候选理由。
- `beta-probe`：完整宿主的 Beta 场景探针；它是诊断/采样入口，正式性能结果仍由独立 runner 和 root 的验收记录给出。
- `probe`：适配器回显探针；`--with-profile`、`--adapter-script` 和 `--host` 分别覆盖适配器、对照脚本和外层宿主诊断口径。
- `doctor`：检查平台、配置、缓存、规格、可用命令、当前会话能力和键位仲裁。
- `specs check/list`、`learning clear`、`config init/check`、`theme`、`terminal-profile`：规格校验/查看、清除本地学习统计、配置维护、主题和 Windows Terminal profile 辅助。

Git、Cargo、npm、pnpm 和 PowerShell 数据源只读取离线本机的 PATH、清单、规格和项目元数据；不会联网，也不会执行构建、项目脚本、安装或 fetch。缺少命令或数据时保留静态候选并报告诊断。

### 原生补全、粘贴与传输

PowerShell 原生补全只由用户手动触发，使用独立菜单和真实替换范围，可能运行用户已有的 completer；它不加入自动菜单的时间承诺，ShellSense 也不会强制终止任意脚本。整段粘贴的会话通道设计为当前会话私有的 FIFO 文件通道；即使显式选择 `run --transport pipe`，该通道仍保持会话私有，ShellSense 不另行记录粘贴正文，不将正文写入 trace 或学习统计；PSReadLine 原有历史策略保持不变，退出会话时清理临时文件。Windows 原生 `Event::Paste` 输入路径仍在补齐，整段粘贴尚未完成真实宿主验收。

OSC 仍是默认 transport，named pipe 只是显式实验路径并可回退到 OSC；顺序、回显、坐标和兼容性收益尚未完成完整宿主验收。C# 对照接入没有收益，已拒绝，不打包 DLL。

### Windows 范围与当前限制

本地 Beta 的目标范围是 Windows 10/11、Windows Terminal、PowerShell 7 和 ConPTY。Windows Terminal 的真实输入法、视觉布局和颜色验收尚未完成，鼠标目前只有编码层测试；粘贴及其他新增宿主交互仍在验证。`.github/workflows/windows.yml` 只有配置证据，尚未在远程 runner 执行；WSL 和其他 shell 暂不纳入本轮范围。

## 测试资产与当前证据边界

仓库已经包含规格、发现、provider、缓存失效、适配器、协议、CLI 和终端回归用例；这些用例的存在不等于本轮 release 构建、完整矩阵或真实宿主验收已经通过。0.4 基线的完整 123 个 Rust 测试已有通过记录；root 当前还记录了不含粘贴路径的 Rust/PowerShell adapter 回归通过，新增 specs 用例单独通过。原生 `Event::Paste` 输入路径仍在补齐，0.5 新增粘贴等验证仍在进行。最终结果应以 root 运行的 release build/test 日志和本文件后续更新为准。

`.github/workflows/windows.yml` 已配置 Windows CI，包含目标 PowerShell/PSReadLine 组合、格式检查、构建、测试、适配器和打包步骤；截至本记录尚未在远程 runner 执行，因此不能写成 CI 已通过。

## 尚未验收的项目

### 正式性能门槛

正式门槛仍为首次输入增量 P50 ≤ 50 ms、热态菜单 P95 ≤ 20 ms。当前没有满足全部口径的最终性能报告：

- 旧 30 对样本中的 `178.3705 ms` 是省略公共快捷键初始化的预冻结诊断值，不能作为正式指标。旧版报告中的历史基线也不能替代本轮结果。
- 正式计时必须在 release、保留 profile、关闭 trace 的条件下，交替采集至少 30 对启动样本；启动门槛由独立 probe 测量，不能用热态回显代替。
- root、Git、Cargo、JS、path、fuzzy 六个场景的每种 cache mode 都必须各自拥有至少 300 次真实热态查询；miss/hit 需要成对使用相互独立的空目录，不能把同一目录的重复 hit 计作 miss。
- 首次动态结果就绪时间必须单独记录，静态候选和完整动态结果要分开；每次实际 transport 必须由 `adapter.json` 证明，不能把 pipe fallback 当成 pipe。
- 报告还需包含输入回显、内存、空闲 CPU 和输出吞吐，使用标准中位数和 nearest-rank P95，并明确 OS 缓存没有清空。

在上述数据完成并逐场景验收前，不能宣称性能门槛通过。

### 真实宿主与交付

- Windows Terminal 中的中文输入法、视觉布局与颜色尚未完成真实验收；鼠标目前只有编码层测试；整段粘贴及其他新增宿主交互仍在验证。窗口缩放、全屏外部程序恢复、多行续行、光标中间插入、历史搜索、Esc 和实际 PSReadLine 缓冲区执行仍需最终人工验收。
- named pipe 仍是显式实验路径；默认 transport 仍为 OSC。只有完整宿主测试证明顺序、回显、坐标和兼容性收益后，才考虑切换默认值。
- 预编译 C# 接入层的 30 对测量收益为 `-37.88 ms`（桥接更慢），已拒绝，不进入生产默认路径。
- Windows x64 发布包、SHA-256、依赖许可证、版本说明、用户目录安装、升级、卸载、回滚、保留上一版 exe 和无签名标记仍需最终检查；这些步骤不得覆盖用户配置。

因此，0.3/0.4 的主要代码范围已落地，0.5 Beta 仍处于功能回归、真实 Windows Terminal 验收、正式性能测量、CI 执行和发布包检查阶段。

## 本地交付边界

当前冻结工作副本为 `C:\Users\dell\.codex\visualizations\2026\09\10\01a08c3e-7202-7b72-bd9a-33b2fa60a094\shellsense-work-v2`；E 盘离线，本轮只在 C 盘准备。源码、待迁移的发布包和回滚版 exe 都保留在 C 盘的唯一副本中，完成本地检查后只清理临时缓存，不覆盖上一版或用户配置。这里是本地 Beta 准备，不执行公开发布；迁移和公开发布另行处理。

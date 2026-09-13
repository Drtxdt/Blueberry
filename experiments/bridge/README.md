# PSReadLine bridge experiment

这是一个可移除的性能实验，不属于 ShellSense 的默认加载路径。它只包住
已加载 PSReadLine 的公共接口，供 root 在专用 adapter 副本中做 A/B 对照：

- `GetKeyHandlers(bool, bool)`：一次枚举并在 C# 中生成不可变的 handler 快照和按 chord 索引；
- `GetBufferState(ref string, ref int)`：返回当前行和 UTF-16 光标；
- `Replace(int, int, string, ..., ...)`：保留 PSReadLine 的实际范围检查和编辑行为；
- `SetKeyHandler(string[], ScriptBlock, string, string)`：缓存方法信息，供已确认的保留键注册路径调用；
- `System.Text.Json`：只缓存 ASCII 安全的序列化选项，协议形状仍由 adapter 负责。

桥接器不实现规格解析、候选排序、动态数据、缓存、渲染、事件生命周期或取消逻辑。它也不导入 PSReadLine、不创建 runspace、不执行脚本。类型查找只扫描当前 AppDomain 中已经加载的
`Microsoft.PowerShell.PSConsoleReadLine`，所以未加载模块时 `TryBind` 会返回失败，调用方应保留原有 PowerShell 路径。

本机检查的 PSReadLine 2.4.5 公共签名如下；Windows PowerShell 模块中的 2.0.0 也具有同样的四个目标接口，但每个 A/B 目标仍须在实际版本中重新验证：

```text
IEnumerable<KeyHandler> GetKeyHandlers(Boolean, Boolean)
Void GetBufferState(String ByRef, Int32 ByRef)
Void Replace(Int32, Int32, String, Action<Nullable<ConsoleKeyInfo>,Object>, Object)
Void SetKeyHandler(String[], ScriptBlock, String, String)
```

输出与输入都继续由 PSReadLine 处理，桥接器不改变 UTF-16 偏移，也不绕过实时 buffer 的一致性检查。

## 离线构建

使用与目标 adapter 相同的 PowerShell 7 安装执行：

```powershell
pwsh -NoProfile -File .\experiments\bridge\build.ps1
```

默认输出为 `experiments/bridge/bin/ShellSense.PSReadLineBridge.dll`。构建脚本使用目标
`$PSHOME` 随 PowerShell 7 提供的 Roslyn 文件，通过 `Add-Type` 只在构建时编译；它会先检查
`Microsoft.CodeAnalysis*.dll`、`System.Text.Json.dll` 和 `System.Text.Encodings.Web.dll`。
运行时 adapter 只用 `Assembly.LoadFrom` 加载已经编译好的 DLL，不会调用 `Add-Type` 编译。
脚本不联网、不还原包，也不需要 .NET SDK。`bin/` 和 DLL 已在本实验目录忽略。

如果目标机器的 `$PSHOME` 缺少上述 Roslyn 或运行时程序集，构建应失败并说明缺失文件，不能偷偷改用其他版本。
本机没有安装 .NET SDK，但 `D:\pwsh\7` 的 PowerShell 7.6.6 包含所需离线 Roslyn 文件；桥接 DLL
应在同一 PowerShell runtime 上构建和验证：

```powershell
pwsh -NoProfile -File .\experiments\bridge\verify-bridge.ps1 -Json
```

验证脚本只加载 DLL、绑定已加载的 PSReadLine、读取 buffer/handler 并检查中文和 emoji 的
ASCII JSON 往返，不注册键位，也不改 profile 或历史。

## 生成 A/B adapter 副本

`prepare-ab-adapter.ps1` 在临时目录生成一个可交给 probe 的 adapter 副本。`Baseline` 是原文件的
逐字复制；`Bridge` 复制 DLL，并只替换 buffer、Replace、handler 快照、键注册和序列化这几个调用点。
Bridge 副本在首次成功绑定时写入一次临时 `bridge-proof.json`，记录实际绑定的公共方法；缺少此文件
时，A/B 结果必须视为 fallback，不能当作桥接收益。该证明文件不在每次查询时写入。
它会检查生产 adapter 的调用形状，输出 `ab-manifest.json` 和源文件 SHA-256；不会修改源文件、用户
profile、配置或终端设置。生成目录由调用方负责在测试完成后清理。

```powershell
$result = & pwsh -NoProfile -File .\experiments\bridge\prepare-ab-adapter.ps1 `
    -Mode Bridge -Json | ConvertFrom-Json
pwsh -NoProfile -File .\experiments\bridge\verify-bridge.ps1 -Json
.\target\release\shellsense.exe probe --with-profile --iterations 30 `
    --adapter-script $result.AdapterPath
```

也可以让实验脚本负责生成副本并启动同一套 probe：

```powershell
pwsh -NoProfile -File .\experiments\bridge\run-ab-probe.ps1 `
    -ShellSenseExecutable .\target\release\shellsense.exe `
    -Mode Bridge -Iterations 30 -OutputPath .\artifacts\bridge-ab.json
```

它不会替代正式的 Beta 统计工具；输出仍需由 root 按交替 30 对样本、正确的缓存状态和
热态菜单样本规则验收。

需要把原 adapter 与 Bridge adapter 直接交替比较时，使用 paired runner。它为每一对分别启动
`probe --iterations 1 --shell <Shell>`，奇数对先跑 Baseline、偶数对先跑 Bridge；每对的完整 probe
JSON 和一次性 binding proof 都保存在最终报告中。报告的主差值严格按
`baseline_delta_ms - bridge_delta_ms` 计算，P95 使用 nearest-rank，计时循环结束后才写一次报告：

```powershell
pwsh -NoProfile -File .\experiments\bridge\run-ab-paired.ps1 `
    -ShellSenseExecutable .\target\release\shellsense.exe `
    -Shell D:\pwsh\7\pwsh.exe -PairCount 30 `
    -OutputPath .\artifacts\bridge-ab-paired.json
```

该 runner 不调用 Cargo、不构建程序，且不会在每个查询中写报告文件。若任一 Bridge 子进程没有
发布绑定证明，报告会标记 `verified=false` 并以失败退出，相关差值不能用于门槛判断。

对照组把 `-Mode Bridge` 改为 `-Mode Baseline`。`--adapter-script` 只接收生成的临时副本；正式
宿主仍使用仓库内原 adapter。桥接不可用时每个包装自动回退到原 PowerShell 公共调用。

## 可替换热点与验收

当前 trace 显示首轮 `readline_init` 约 120 ms，`adapter_bootstrap` 约 179 ms，而序列化约 1.6 ms。因而本实验把重点放在 PSReadLine 方法发现、PowerShell 反射调用和 handler/buffer 对象转换；序列化缓存只是配套项，不能单独作为收益依据。桥接器本身不宣称降低启动或热态延迟。

后续 A/B 应在专用 adapter 副本中用功能开关替换相同调用点，并保留旧路径：加载 PSReadLine 后调用
`[ShellSense.Bridge.PsReadLineBridge]::TryBind(...)`，成功才使用快照、buffer、Replace 和键注册包装。正式验收要在相同 release、机器、配置和缓存状态下交替测量至少 30 对启动样本，同时保持交互回归、原生补全、Unicode 范围、输入法、多行和外部程序测试通过。只有启动增量 P50 至少降低 20 ms、热态菜单 P95 不变差且兼容回归全部通过，才有理由保留；否则删除 A/B 接入，留下本目录作为未采用实验记录。

实验不得改动规格、排序、持久化统计、缓存或渲染，也不得因为桥接失败而阻塞 shell；失败应回退到原 adapter 调用路径。

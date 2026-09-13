> 历史记录：本文保留 0.5.0-beta.1 阶段的安装与验收资料。当前版本请使用 [安装指南](installation.md)、[发版指南](releasing.md) 和 [发布验证记录](release-validation.md)。

# ShellSense Beta 安装与回滚

Beta 首批交付是 Windows x64 ZIP。压缩包包含已构建的
`shellsense.exe`、`release.json`、`VERSION.txt`、许可证、依赖许可证说明、
配置示例、规格源文件和安装管理脚本。包内的 `signature_status` 固定为
`unsigned`；没有签名证书时不要把它当作已签名发行版运行。

本包的 Windows x64 `shellsense.exe` 使用 MSVC 静态 C 运行库（Rust
`+crt-static`，属于 `/MT` 静态链接类别）构建；包内不带
`vcruntime140*.dll`、`msvcp*.dll` 或 `vc_redist.x64.exe`，安装不需要单独安装
Visual C++ Runtime。`THIRD-PARTY-NOTICES.txt` 由 `scripts/licenses.ps1` 根据离线
Cargo metadata 生成，只覆盖当前解析出的 95 个 Rust crate 的许可证文本，不代替
MSVC/Visual Studio 工具链许可说明。MSVC/Visual Studio 构建工具组件的再分发
（包括静态链接产物涉及的组件）仍受实际 Visual Studio/Build Tools 许可证及相应
REDIST/EULA 约束；若以后改用动态 CRT 或附带 Microsoft 运行库文件，还应对所加入
的文件重新审核。本说明不判断发行者的许可资格。

## 校验本地包

把 ZIP 和同名 `.sha256` 文件放在同一目录，在 PowerShell 7 中执行：

```powershell
$package = (Resolve-Path .\shellsense-v0.5.0-beta.1-windows-x64.zip).Path
$expected = (Get-Content "$package.sha256" -Raw).Trim().Split()[0].ToUpperInvariant()
$actual = (Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash.ToUpperInvariant()
if ($actual -ne $expected) { throw "SHA-256 不匹配" }
"SHA-256 OK: $actual"
```

`manage-install.ps1` 还会在解包前检查 ZIP 内的相对路径、平台、版本、每个
文件的 SHA-256、非空必需资产、x64 PE 标头及 `unsigned` 标记。同目录存在 `.sha256` 时也会核对外部摘要。它只接受本地 `.zip` 路径，不下载包，不执行包内脚本或未知 exe。

## 安装、升级和回滚

首次安装使用包内的管理脚本。建议先把 ZIP 解到临时目录，再传入 ZIP 的完整
路径；安装根默认为 `%LOCALAPPDATA%\ShellSense`：

```powershell
pwsh -NoProfile -File .\manage-install.ps1 `
  -Action Install `
  -PackagePath .\shellsense-v0.5.0-beta.1-windows-x64.zip
```

从另一个本地包升级：

```powershell
pwsh -NoProfile -File .\manage-install.ps1 `
  -Action Upgrade `
  -PackagePath .\shellsense-v0.5.0-beta.2-windows-x64.zip
```

升级前会把当前版本的全部受管文件和安装元数据保存到 `previous`，记录 SHA-256。升级失败会恢复原版本；回滚会一并恢复程序、文档和版本元数据。需要回退时执行：

```powershell
pwsh -NoProfile -File .\manage-install.ps1 -Action Rollback
```

回滚同样保存当前版本，因此可以在两个已保存版本之间再次回滚。卸载只删除
安装清单中由 ShellSense 管理的文件：

```powershell
pwsh -NoProfile -File .\manage-install.ps1 -Action Uninstall
```

安装清单记录了产品名、canonical 安装根和全部受管文件。卸载必须先验证这些
字段；磁盘根、用户目录、符号链接和没有 ShellSense 清单的目录都会被拒绝，
不会进行任意递归删除。安装根中的其他用户文件会被保留；已被用户修改、与清单摘要不同的文件也会保留并报告。

用户配置、用户规格、选择统计和 PowerShell profile 位于安装根之外，Install、
Upgrade、Rollback、Uninstall 都不会覆盖或自动迁移它们。安装脚本也不会修改
PowerShell profile。

## Windows Terminal profile

Windows Terminal 的 settings 文件位置随安装方式而异，请显式传入实际路径。
Preview 只读取文件并显示差异：

```powershell
$settings = Join-Path $env:LOCALAPPDATA 'Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json'
pwsh -NoProfile -File .\manage-install.ps1 `
  -Action PreviewSettings `
  -SettingsPath $settings `
  -InstallRoot "$env:LOCALAPPDATA\ShellSense"
```

确认差异后再 Apply。Apply 会先在控制台输出同一份 Preview，然后按原始字节
创建时间戳 `.bak` 备份，最后原子替换 settings 文件：

```powershell
pwsh -NoProfile -File .\manage-install.ps1 `
  -Action ApplySettings `
  -SettingsPath $settings `
  -InstallRoot "$env:LOCALAPPDATA\ShellSense"
```

Apply 只增加或更新固定 GUID 的 ShellSense profile，保留其他 profile 和其他
顶层设置；它不会自动选择 Windows Terminal 默认 profile。含有 JSONC 注释或
尾逗号的标准 Windows Terminal settings 无法由通用 JSON 序列化器无损写回，
因此 Preview 仍会给出候选差异，但 Apply 会拒绝写入并输出需要手动合并的
profile，原文件和备份都不会改变。

## 源码树中的临时目录自测

自测使用真实 x64 release exe 构造本地 ZIP，只在系统临时目录创建安装根。覆盖安装、升级、回滚、升级失败恢复、完整版本元数据、摘要篡改、路径重解析点、配置预览/备份、JSONC 拒绝和卸载保护。自测结束删除它创建的临时目录；`-KeepTemp` 可保留故障现场。

```powershell
pwsh -NoProfile -File .\scripts\test-release.ps1 `
  -ShellSenseExecutable .\target\release\shellsense.exe `
  -PreviousExePath .\artifacts\shellsense-v0.4-frozen.exe
```

`-PreviousExePath` 可省略；CI 使用当前 exe 验证生命周期，本地交付另用保留的上一版验证版本切换。以上命令均不改真实用户配置或 Windows Terminal 设置。

安装、升级、回滚、卸载与打包命令应逐个执行；本版尚不支持对同一安装根或输出目录并发执行这些操作。

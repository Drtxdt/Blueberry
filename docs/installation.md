# 安装与使用

## 一条命令安装

在 Windows PowerShell 5.1 或 PowerShell 7 中运行：

```powershell
(irm 'https://raw.githubusercontent.com/Drtxdt/Blueberry/main/install.ps1').TrimStart([char]0xFEFF) | iex
```

请复制代码块中的完整命令，网址保留单引号，不要使用 `[网址](网址)` 链接格式。命令会移除下载文本开头的 UTF-8 BOM，兼容带 BOM 的旧安装入口。

安装到当前用户目录，无需 Rust。安装末尾选择是否随 PowerShell 启动。之后输入 `blueberry` 即可使用；其他已打开的终端需要重新打开以读取新 PATH。

发布者需要先公开 GitHub Release，安装命令才能下载对应版本。

`0.5.0` 尚在候选验收中；下面指定版本和本地包的示例供该版本发布后使用。当前进度见 [0.5.0 验收记录](v0.5.0-validation.md)。

若 PowerShell 提示“此系统上禁止运行脚本”，先运行 `Get-ExecutionPolicy -List` 查看策略。个人电脑可自行执行 `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`，允许本地安装脚本和 profile 加载；组织管理的电脑请遵循管理员策略。安装程序保留现有执行策略。

## 指定版本与无人值守安装

```powershell
$installer = (irm 'https://raw.githubusercontent.com/Drtxdt/Blueberry/main/install.ps1').TrimStart([char]0xFEFF)
& ([scriptblock]::Create($installer)) -Version 0.5.0 -NoPrompt
```

`-EnableStartup` 明确开启自动启动；`-NoPrompt` 跳过询问，默认关闭。`-InstallRoot` 可更改安装目录，`-ConfigRoot` 可更改首次配置写入目录；自定义配置目录启动时使用 `--config` 指定文件。

## ZIP 安装

从 Releases 下载 ZIP 及同名 `.sha256`，放入同一目录。解压 ZIP 可直接运行 `blueberry.exe`；纳入安装管理时执行：

```powershell
.\install.ps1 -PackagePath C:\Downloads\blueberry-v0.5.0-windows-x64.zip
```

## 升级、回滚与卸载

0.5.0 起，受管理安装直接使用以下命令。升级先下载并校验包，等待相关 Blueberry 会话自然退出后执行；不会强制关闭终端。

```powershell
blueberry upgrade --check
blueberry upgrade
blueberry upgrade --version 0.5.0
blueberry upgrade --package C:\Downloads\blueberry-v0.5.0-windows-x64.zip
blueberry rollback
blueberry uninstall
blueberry maintenance status
blueberry maintenance cancel
```

回滚恢复上一套受管文件。卸载移除受管文件、自身添加的 PATH 项及属于该安装的启动区块，保留用户配置和用户修改过的文件。

`blueberry update` 是 `upgrade` 的别名；默认升级只选择正式版，指定版本可选择预发布版。`--package` 不能与 `--check` 或 `--version` 同时使用。排队成功不代表已经完成，使用 `maintenance status` 查看结果；提交开始后不能取消。命令会显示安装目录外的持久维护日志位置，卸载或回滚到不支持新命令的旧版本后仍可读取该日志。

系统限制后台助手独立运行时，操作保持排队，并在后续启动时重试。升级或回滚中断后使用持久事务备份恢复；无法核验的现场保留并阻止继续启动，不覆盖身份已变化的安装。

从 beta.6 等旧版本首次升级到 0.5.0，先关闭旧 Blueberry 会话并重新运行在线安装入口。便携 ZIP 副本需先通过安装入口纳入管理，才可使用这些维护命令。

## 随 PowerShell 启动

```powershell
blueberry startup enable
blueberry startup status
blueberry startup disable
```

这组命令管理 PowerShell 的当前用户 ConsoleHost profile（[Microsoft profile 说明](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_profiles)）。5.1 与 7 分别使用各自的 profile；修改前创建 `.blueberry-*.bak` 备份，重复启用保持一个区块。

自动启动沿用当前 PowerShell 版本。`-NoProfile` 会跳过 profile；脚本、非交互和重定向调用也会跳过启动。关闭自动启动后，仍可手动运行 `blueberry`。

## 本机验收

先运行自动回归：

```powershell
.\scripts\verify-local.ps1 -Shell pwsh.exe
```

再在真实 Windows Terminal 检查：

- `git log --`、`codex exec --`、目录和枚举值的菜单选择及接受。
- 中文输入法、emoji、光标中间编辑，确认右侧文本和撤销正常。
- F1 长详情翻页、关闭后的选中项恢复。
- 输入“查看分支”后按 Ctrl+Alt+F，用 Tab 接受 `git branch`。
- 窄窗口、缩放和主题颜色；已配置 Nerd Font 时检查对应图标。
- 自动启动启用、关闭，以及退出 Blueberry 后继续使用 PowerShell。

出现问题时附上 `blueberry doctor`、PowerShell/PSReadLine 版本及复现步骤。

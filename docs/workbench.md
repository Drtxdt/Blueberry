# 命令工作台

按 `Ctrl+Alt+P` 可以在当前 Blueberry 会话中打开工作台。搜索框与 PowerShell 编辑缓冲区分开，输入中文、英文或粘贴文本都不会改变原命令行。按 Enter 将选中的完整命令填回，随后由用户决定是否执行；按 Esc 返回时原命令行和光标保持不变。

独立运行 `blueberry hub` 会打开全屏选择器。确认普通命令后复制到 Windows 剪贴板。模板含有参数时，页面逐项收集参数并显示最终命令预览。

## commands.toml

收藏和模板保存在 `%APPDATA%\Blueberry\commands.toml`。旧版 `command = "..."` 模板仍可读取。新版模板将命令 token 与参数定义分开：

```toml
schema_version = 2

[[favorites]]
name = "启动开发服务"
command = "npm run dev"
description = "启动当前前端项目"
tags = ["前端", "开发"]

[[templates]]
id = "git.create-branch"
name = "创建 Git 分支"
tokens = ["git", "switch", "-c", "{branch}"]
description = "创建并切换到新分支"

[[templates.parameters]]
name = "branch"
label = "分支名称"
kind = "text"
required = true
```

参数类型包括 `text`、`file`、`directory`、`enum` 和 `dynamic`。占位符需要占据完整 token。含空格或单引号的值会按 PowerShell 规则引用。

## 普通菜单建议

`workbench.suggestions` 开启后，Blueberry 会在光标位于行末、且已经输入内容时加入收藏、当前项目操作和最近历史。整行建议拥有独立替换范围，不参与光标中间的 token 补全。`suggestion_limit` 控制每次最多加入的数量。

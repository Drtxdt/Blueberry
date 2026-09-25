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

收藏文件仅在不存在时创建。读取或解析失败时，工作台报告路径和原因，并保留运行中的上一份有效内容。添加和删除收藏会在文件锁内重新读取，保留无关字段和注释；检测到外部修改时终止写入。参数表单的 Esc 会保留原行；Enter 只填回，不执行。

## 从当前命令继续填写

在 PowerShell 输入 `git switch -c`、`cargo test -p`、`cargo test --features`、`cargo test --test`、`docker compose up` 或 `docker compose logs` 后按 `Ctrl+Alt+P`。选择“继续填写”会打开参数表单。本机项目参数可以用方向键选值，也可以直接输入。含有不明确引号、管道或重定向的行仍使用普通补全。

## 仓库命令包

只读取 Git 根目录的 `.blueberry/commands.toml`，schema 为 1。操作使用结构化 token 和参数，不支持脚本钩子或远程读取。例如：

```toml
schema_version = 1

[pack]
id = "team-workflows"
name = "团队工作流"
version = "1.0.0"

[[operations]]
id = "build"
name = "构建项目"
tokens = ["cargo", "build", "-p", "{package}"]

[[operations.parameters]]
name = "package"
label = "包名"
kind = "dynamic"
source = "cargo.packages"
required = true
```

在仓库中运行 `blueberry packs review` 查看规范路径、SHA-256 和与上次批准内容的差异。检查后执行 `blueberry packs approve <review 中的 SHA-256>`；`blueberry packs list` 查看状态，`blueberry packs revoke` 撤销批准。批准同时绑定规范路径和精确文件哈希，改动后须重新审查。已批准操作只填回命令，由用户决定是否执行。

## 普通菜单建议

`workbench.suggestions` 开启后，Blueberry 会在光标位于行末、且已经输入内容时加入收藏、当前项目操作和最近历史。整行建议拥有独立替换范围，不参与光标中间的 token 补全。`suggestion_limit` 控制每次最多加入的数量。

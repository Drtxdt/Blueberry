# 下一版人工体验清单

本轮按发布者要求只执行 `cargo build --release --locked`。以下交互项目留给 Windows Terminal 实测，检查结果应在下一次发版前记录。

- PowerShell 5.1 和 7 中切换 Conda、Mamba、uv 与虚拟环境，确认环境名和依赖候选更新。
- 在 Python、Node、Rust、Go、.NET、CMake 项目中检查脚本、工作区、项目文件和预设。
- 检查 Compose 服务、kubectl 上下文、Helm Chart 与 SSH Host；远程资源应在 Ctrl+Alt+D 后读取。
- 在设置页检查中文搜索、仅修改项、三套预设、差异预览和快捷键冲突信息。
- 运行 `blueberry tools`、`blueberry doctor --json` 和 `blueberry setup`。
- 用 `blueberry hub --add` 添加收藏；在会话中按 Ctrl+Alt+P，检查 Tab 填回与 Esc 恢复原编辑行。
- 检查窄窗口、中文、多行历史、右键粘贴，以及设置页和工作台退出后的鼠标状态。

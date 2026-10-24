pub const CASES: &[(&str, &[&str], &str)] = &[
    ("切换 Git 分支", &["main"], "git switch main"),
    (
        "创建 Git 分支",
        &["feature-中文", ""],
        "git switch -c 'feature-中文'",
    ),
    (
        "添加 Git worktree 工作目录",
        &["../work tree", "feature", "main"],
        "git worktree add '../work tree' -b feature main",
    ),
    (
        "进入 Git worktree 工作目录",
        &["../work tree"],
        "Set-Location '../work tree'",
    ),
    (
        "查看 Git 提交日志",
        &["main", ""],
        "git log --oneline main --",
    ),
    ("查看 Git 差异", &["main", ""], "git diff main --"),
    (
        "创建 Python uv 虚拟环境",
        &["", "3.12"],
        "uv venv --python 3.12",
    ),
    ("同步 Python uv 项目依赖", &["test"], "uv sync --group test"),
    ("运行 Python uv 项目脚本", &["hello"], "uv run hello"),
    (
        "运行 Python uv pytest 测试",
        &["tests", "smoke"],
        "uv run pytest tests -k smoke",
    ),
    (
        "构建 Rust Cargo 工作区",
        &["fixture", ""],
        "cargo build -p fixture",
    ),
    (
        "运行 Rust Cargo 测试",
        &["fixture", "", "integration"],
        "cargo test -p fixture --test integration",
    ),
];

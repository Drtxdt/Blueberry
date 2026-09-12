//! Static, context-aware command specifications.
//!
//! The completion engine deliberately keeps command knowledge in Rust.  This
//! module contains the small declarative grammar used by the host: command
//! names, nested subcommands, option scope, and the value shape of options.
//! It never invokes a command or evaluates a script while completing.

use crate::model::CandidateKind;

/// A completion item produced by a command specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecCandidate {
    pub name: String,
    pub description: &'static str,
    pub key: String,
    pub kind: CandidateKind,
}

/// The result of applying a command specification to an invocation prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecResult {
    pub candidates: Vec<SpecCandidate>,
    pub context: String,
    pub path_values: bool,
    pub options_ended: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ValueKind {
    None,
    Text,
    Path,
}

#[derive(Debug, Copy, Clone)]
struct ValueDef {
    name: &'static str,
    description: &'static str,
}

#[derive(Debug, Copy, Clone)]
struct OptionDef {
    name: &'static str,
    description: &'static str,
    value: ValueKind,
    values: &'static [ValueDef],
    optional_value: bool,
}

#[derive(Debug, Copy, Clone)]
struct CommandDef {
    name: &'static str,
    description: &'static str,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum PositionalKind {
    None,
    Value,
    Path,
}

#[derive(Debug, Copy, Clone)]
struct ContextDef {
    subcommands: &'static [CommandDef],
    options: &'static [OptionDef],
    positional: PositionalKind,
}

const EMPTY_VALUES: &[ValueDef] = &[];

const fn option(name: &'static str, description: &'static str) -> OptionDef {
    OptionDef {
        name,
        description,
        value: ValueKind::None,
        values: EMPTY_VALUES,
        optional_value: false,
    }
}

const fn text_option(name: &'static str, description: &'static str) -> OptionDef {
    OptionDef {
        name,
        description,
        value: ValueKind::Text,
        values: EMPTY_VALUES,
        optional_value: false,
    }
}

const fn path_option(name: &'static str, description: &'static str) -> OptionDef {
    OptionDef {
        name,
        description,
        value: ValueKind::Path,
        values: EMPTY_VALUES,
        optional_value: false,
    }
}

const fn choice_option(
    name: &'static str,
    description: &'static str,
    values: &'static [ValueDef],
) -> OptionDef {
    OptionDef {
        name,
        description,
        value: ValueKind::Text,
        values,
        optional_value: false,
    }
}

const fn optional_text_option(name: &'static str, description: &'static str) -> OptionDef {
    OptionDef {
        name,
        description,
        value: ValueKind::Text,
        values: EMPTY_VALUES,
        optional_value: true,
    }
}

const fn optional_path_option(name: &'static str, description: &'static str) -> OptionDef {
    OptionDef {
        name,
        description,
        value: ValueKind::Path,
        values: EMPTY_VALUES,
        optional_value: true,
    }
}

const fn optional_choice_option(
    name: &'static str,
    description: &'static str,
    values: &'static [ValueDef],
) -> OptionDef {
    OptionDef {
        name,
        description,
        value: ValueKind::Text,
        values,
        optional_value: true,
    }
}

const fn command(name: &'static str, description: &'static str) -> CommandDef {
    CommandDef { name, description }
}

const fn context(
    subcommands: &'static [CommandDef],
    options: &'static [OptionDef],
    positional: PositionalKind,
) -> ContextDef {
    ContextDef {
        subcommands,
        options,
        positional,
    }
}

const COLOR_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "always",
        description: "始终使用颜色",
    },
    ValueDef {
        name: "auto",
        description: "在适当时使用颜色",
    },
    ValueDef {
        name: "never",
        description: "禁用颜色",
    },
];

const GIT_DATE_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "relative",
        description: "显示相对日期",
    },
    ValueDef {
        name: "local",
        description: "显示本地日期",
    },
    ValueDef {
        name: "iso",
        description: "显示 ISO 日期",
    },
    ValueDef {
        name: "iso-strict",
        description: "显示严格 ISO 日期",
    },
    ValueDef {
        name: "rfc",
        description: "显示 RFC 日期",
    },
    ValueDef {
        name: "short",
        description: "显示短日期",
    },
    ValueDef {
        name: "raw",
        description: "显示原始时间戳",
    },
];

const GIT_DIFF_FILTER_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "A",
        description: "新增路径",
    },
    ValueDef {
        name: "C",
        description: "已复制路径",
    },
    ValueDef {
        name: "D",
        description: "已删除路径",
    },
    ValueDef {
        name: "M",
        description: "已修改路径",
    },
    ValueDef {
        name: "R",
        description: "已重命名路径",
    },
    ValueDef {
        name: "T",
        description: "已更改的文件类型",
    },
    ValueDef {
        name: "U",
        description: "未合并路径",
    },
    ValueDef {
        name: "X",
        description: "未知路径",
    },
    ValueDef {
        name: "B",
        description: "配对损坏",
    },
];

const GIT_IGNORED_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "traditional",
        description: "显示传统的已忽略文件和目录",
    },
    ValueDef {
        name: "no",
        description: "不显示已忽略文件",
    },
    ValueDef {
        name: "matching",
        description: "显示匹配忽略模式的文件和目录",
    },
];

const GIT_UNTRACKED_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "no",
        description: "隐藏未跟踪文件",
    },
    ValueDef {
        name: "normal",
        description: "显示未跟踪文件",
    },
    ValueDef {
        name: "all",
        description: "显示所有未跟踪文件",
    },
];

const GIT_IGNORE_SUBMODULES_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "none",
        description: "显示所有子模块更改",
    },
    ValueDef {
        name: "untracked",
        description: "忽略未跟踪的子模块文件",
    },
    ValueDef {
        name: "dirty",
        description: "忽略有未提交更改的子模块工作树",
    },
    ValueDef {
        name: "all",
        description: "忽略所有子模块更改",
    },
];

const GIT_PORCELAIN_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "v1",
        description: "使用 porcelain 格式版本 1",
    },
    ValueDef {
        name: "v2",
        description: "使用 porcelain 格式版本 2",
    },
];

const GIT_COLUMN_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "always",
        description: "始终使用列",
    },
    ValueDef {
        name: "never",
        description: "从不使用列",
    },
    ValueDef {
        name: "auto",
        description: "在适当时使用列",
    },
];

const GIT_RECURSE_SUBMODULE_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "yes",
        description: "递归获取所有子模块",
    },
    ValueDef {
        name: "on-demand",
        description: "仅按需获取子模块",
    },
    ValueDef {
        name: "no",
        description: "不获取子模块",
    },
];

const GIT_TRACK_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "direct",
        description: "直接跟踪起始点",
    },
    ValueDef {
        name: "inherit",
        description: "继承起始点的跟踪配置",
    },
];

const GIT_DECORATE_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "short",
        description: "显示不带完整前缀的引用名",
    },
    ValueDef {
        name: "full",
        description: "显示带完整前缀的引用名",
    },
    ValueDef {
        name: "auto",
        description: "在终端中显示简短引用名",
    },
    ValueDef {
        name: "no",
        description: "不显示引用名",
    },
];

const CARGO_MESSAGE_FORMAT_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "human",
        description: "人类可读的编译器消息",
    },
    ValueDef {
        name: "json",
        description: "JSON 编译器消息",
    },
    ValueDef {
        name: "short",
        description: "简短的编译器消息",
    },
    ValueDef {
        name: "json-diagnostic-short",
        description: "紧凑的 JSON 诊断信息",
    },
    ValueDef {
        name: "json-diagnostic-rendered-ansi",
        description: "ANSI 渲染的 JSON 诊断信息",
    },
];

const CARGO_TIMING_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "html",
        description: "写入 HTML 耗时报告",
    },
    ValueDef {
        name: "json",
        description: "写入 JSON 耗时报告",
    },
];

const NPM_LOGLEVEL_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "silent",
        description: "不显示输出",
    },
    ValueDef {
        name: "error",
        description: "仅显示错误",
    },
    ValueDef {
        name: "warn",
        description: "显示警告和错误",
    },
    ValueDef {
        name: "notice",
        description: "显示提示和警告",
    },
    ValueDef {
        name: "http",
        description: "显示 HTTP 活动",
    },
    ValueDef {
        name: "info",
        description: "显示信息输出",
    },
    ValueDef {
        name: "verbose",
        description: "显示详细输出",
    },
    ValueDef {
        name: "silly",
        description: "显示全部调试输出",
    },
];

const DOCKER_PULL_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "always",
        description: "始终拉取镜像",
    },
    ValueDef {
        name: "missing",
        description: "仅在镜像缺失时拉取",
    },
    ValueDef {
        name: "never",
        description: "从不拉取镜像",
    },
];

const PWSH_INPUT_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "Text",
        description: "读取文本输入",
    },
    ValueDef {
        name: "XML",
        description: "读取 XML 输入",
    },
];

const PWSH_OUTPUT_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "Text",
        description: "写入文本输出",
    },
    ValueDef {
        name: "XML",
        description: "写入 XML 输出",
    },
];

const PWSH_WINDOW_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "Hidden",
        description: "以隐藏窗口启动",
    },
    ValueDef {
        name: "Minimized",
        description: "以最小化窗口启动",
    },
    ValueDef {
        name: "Maximized",
        description: "以最大化窗口启动",
    },
    ValueDef {
        name: "Normal",
        description: "以普通窗口启动",
    },
];

const GIT_COMMANDS: &[CommandDef] = &[
    command("add", "将文件添加到索引"),
    command("am", "从邮箱应用补丁"),
    command("archive", "创建文件归档"),
    command("bisect", "找出引入错误的提交"),
    command("branch", "列出、创建或删除分支"),
    command("checkout", "切换分支或恢复路径"),
    command("cherry-pick", "应用现有提交中的更改"),
    command("clean", "删除未跟踪文件"),
    command("clone", "克隆仓库"),
    command("commit", "记录仓库中的更改"),
    command("config", "读写仓库选项"),
    command("diff", "显示提交或文件之间的更改"),
    command("fetch", "从另一个仓库下载对象和引用"),
    command("format-patch", "准备用于电子邮件提交的补丁"),
    command("grep", "打印匹配模式的行"),
    command("init", "创建空仓库"),
    command("log", "显示提交历史"),
    command("merge", "合并开发历史"),
    command("mv", "移动或重命名文件、目录或符号链接"),
    command("pull", "获取并集成另一个仓库"),
    command("push", "更新远程引用和对象"),
    command("rebase", "将提交重新应用到另一个基准之上"),
    command("reflog", "管理引用日志"),
    command("remote", "管理已跟踪的仓库"),
    command("rename", "重命名仓库或引用"),
    command("reset", "重置当前 HEAD"),
    command("restore", "恢复工作树文件"),
    command("revert", "创建反转更改的提交"),
    command("rm", "从工作树和索引中删除文件"),
    command("show", "显示一个或多个对象"),
    command("sparse-checkout", "管理稀疏检出文件"),
    command("stash", "暂存未完成的更改"),
    command("status", "显示工作树状态"),
    command("switch", "切换分支"),
    command("tag", "创建、列出或删除标签"),
    command("worktree", "管理多个工作树"),
];

const GIT_GLOBAL_OPTIONS: &[OptionDef] = &[
    option("--help", "显示 Git 帮助"),
    option("--version", "显示 Git 版本"),
    optional_path_option("--exec-path", "设置 Git 可执行文件路径"),
    path_option("--git-dir", "设置仓库目录"),
    path_option("--work-tree", "设置工作树目录"),
    option("--bare", "将仓库视为裸仓库"),
    text_option("--config-env", "从环境变量读取配置值"),
    path_option("-C", "像 Git 从此目录启动一样运行"),
    text_option("-c", "为本次调用设置配置变量"),
    text_option("--namespace", "使用独立的 Git 命名空间"),
    path_option("--super-prefix", "为递归 Git 命令设置路径前缀"),
    option("-p", "通过分页器输出"),
    optional_choice_option(
        "--paginate",
        "通过分页器输出",
        &[
            ValueDef {
                name: "auto",
                description: "在适当时使用分页器",
            },
            ValueDef {
                name: "always",
                description: "始终使用分页器",
            },
            ValueDef {
                name: "never",
                description: "禁用分页器",
            },
        ],
    ),
    option("--no-pager", "不通过分页器输出"),
    option("--no-replace-objects", "不替换 Git 对象"),
    option("--no-lazy-fetch", "不延迟获取缺失对象"),
    option("--no-optional-locks", "避免可选锁"),
    option("--literal-pathspecs", "按字面解释路径规格"),
    option("--glob-pathspecs", "对路径规格使用 glob 通配"),
    option("--noglob-pathspecs", "禁用路径规格的 glob 通配"),
    option("--icase-pathspecs", "使路径规格匹配不区分大小写"),
    text_option("--list-cmds", "列出命令组"),
];

const GIT_LOG_OPTIONS: &[OptionDef] = &[
    option("--follow", "重命名后继续跟踪历史"),
    option("--no-decorate", "隐藏引用装饰"),
    optional_choice_option("--decorate", "显示引用装饰", GIT_DECORATE_VALUES),
    text_option("--decorate-refs", "为匹配的引用添加装饰"),
    text_option("--decorate-refs-exclude", "从装饰中排除匹配引用"),
    option("--source", "显示指向每个提交的引用"),
    option("--use-mailmap", "使用 mailmap 文件处理名称"),
    option("--mailmap", "使用 mailmap 文件处理名称"),
    option("--full-diff", "显示每个提交的完整差异"),
    option("--log-size", "显示每条提交消息的大小"),
    optional_text_option("--notes", "显示附加到提交的说明"),
    option("--no-notes", "不显示提交说明"),
    optional_text_option("--show-notes", "显示来自引用的说明"),
    option("--standard-notes", "显示标准说明"),
    option("--show-signature", "显示已签名的提交签名"),
    option("--relative-date", "显示相对于当前时间的日期"),
    choice_option("--date", "选择日期格式", GIT_DATE_VALUES),
    optional_text_option("--pretty", "格式化提交消息"),
    text_option("--format", "格式化提交消息"),
    option("--abbrev-commit", "显示缩写的提交 ID"),
    option("--oneline", "每条提交显示为一行"),
    option("--no-abbrev-commit", "显示完整提交 ID"),
    option("--full-history", "不简化历史"),
    option("--dense", "仅显示选中的提交和有意义的合并"),
    option("--sparse", "显示简化历史中的所有提交"),
    option("--simplify-merges", "通过重写合并来简化历史"),
    option("--simplify-by-decoration", "仅显示带装饰的提交"),
    option("--all", "显示所有引用"),
    optional_text_option("--branches", "显示匹配模式的分支"),
    optional_text_option("--tags", "显示匹配模式的标签"),
    optional_text_option("--remotes", "显示远程跟踪引用"),
    text_option("--glob", "显示匹配 glob 的引用"),
    text_option("--exclude", "排除匹配模式的引用"),
    option("--reflog", "显示引用日志条目"),
    option("--alternate-refs", "使用备用引用进行装饰"),
    option("--single-worktree", "仅检查一个工作树"),
    option("--ignore-missing", "忽略缺失对象"),
    option("--bisect", "显示二分查找历史"),
    option("--stdin", "从标准输入读取修订"),
    text_option("--max-count", "限制提交数量"),
    text_option("-n", "限制提交数量"),
    text_option("--skip", "在显示结果前跳过提交"),
    text_option("--since", "显示日期之后的提交"),
    text_option("--after", "显示日期之后的提交"),
    text_option("--until", "显示日期之前的提交"),
    text_option("--before", "显示日期之前的提交"),
    text_option("--author", "按作者限制提交"),
    text_option("--committer", "按提交者限制提交"),
    text_option("--grep", "按消息模式限制提交"),
    option("--invert-grep", "排除匹配的提交消息"),
    option("--all-match", "要求所有消息模式都匹配"),
    option("--basic-regexp", "使用基本正则表达式"),
    option("--extended-regexp", "使用扩展正则表达式"),
    option("--fixed-strings", "将模式视为固定字符串"),
    option("-F", "将模式视为固定字符串"),
    option("-E", "使用扩展正则表达式"),
    option("-G", "使用基本正则表达式"),
    option("-i", "忽略模式中的大小写"),
    option("--regexp-ignore-case", "忽略模式中的大小写"),
    option("--textconv", "允许文本转换过滤器"),
    optional_choice_option(
        "--ignore-submodules",
        "选择子模块更改处理方式",
        GIT_IGNORE_SUBMODULES_VALUES,
    ),
    optional_text_option("--submodule", "选择子模块差异格式"),
    option("--pickaxe-all", "显示 pickaxe 匹配的所有更改集"),
    text_option("-S", "查找添加或删除字符串的更改"),
    text_option("--pickaxe-regex", "将 pickaxe 字符串视为正则表达式"),
    text_option("-O", "使用文件控制差异文件顺序"),
    text_option("--diff-merges", "选择合并差异展示方式"),
    option("--no-diff-merges", "隐藏合并差异"),
    option("--cc", "显示合并后的组合差异"),
    option("--combined-all-paths", "显示所有父级的路径"),
    option("--first-parent", "仅跟随第一个父级"),
    option("--exclude-first-parent-only", "排除第一父级历史"),
    option("--merges", "显示合并提交"),
    option("--no-merges", "隐藏合并提交"),
    text_option("--min-parents", "要求至少包含指定数量的父级"),
    text_option("--max-parents", "限制父级数量"),
    option("--remove-empty", "路径消失时停止"),
    option("--mergetag", "显示嵌入的已签名合并标签"),
    option("--boundary", "显示被排除的边界提交"),
    option("--graph", "绘制提交图"),
    option("--show-linear-break", "显示线性历史之间的分隔"),
    text_option("--diff-algorithm", "选择差异算法"),
    choice_option("--diff-filter", "选择更改路径状态", GIT_DIFF_FILTER_VALUES),
    text_option("--anchored", "使用锚定差异匹配"),
    optional_text_option("--word-diff", "显示单词级差异"),
    optional_text_option("--color-words", "用颜色显示更改单词"),
    option("--no-renames", "禁用重命名检测"),
    option("--rename-empty", "允许空文件作为重命名源"),
    option("--parents", "显示父级提交"),
    option("--children", "显示子级提交"),
    option("--timestamp", "显示原始提交时间戳"),
    option("--left-right", "标记对称差异两侧的提交"),
    option("--cherry-mark", "标记等价提交"),
    option("--cherry-pick", "省略等价提交"),
    option("--left-only", "仅显示左侧可达的提交"),
    option("--right-only", "仅显示右侧可达的提交"),
    option("--merge", "显示涉及已合并文件的提交"),
    path_option("--output", "将输出写入文件"),
    path_option("-o", "将输出写入文件"),
];

const GIT_SHOW_OPTIONS: &[OptionDef] = &[
    optional_text_option("--pretty", "格式化提交消息"),
    text_option("--format", "格式化提交消息"),
    option("--oneline", "每条提交显示为一行"),
    option("--abbrev-commit", "显示缩写的提交 ID"),
    option("--no-abbrev-commit", "显示完整提交 ID"),
    optional_text_option("--abbrev", "设置对象名称缩写长度"),
    option("--full-index", "在差异标头中显示完整对象名称"),
    option("--binary", "输出二进制更改"),
    option("--patch", "显示补丁文本"),
    option("-p", "显示补丁文本"),
    option("--no-patch", "隐藏补丁输出"),
    option("--raw", "显示原始差异格式"),
    option("--patch-with-raw", "显示带原始标头的补丁"),
    optional_text_option("--stat", "显示差异统计"),
    option("--numstat", "显示数值差异统计"),
    option("--shortstat", "仅显示最后一行差异统计"),
    optional_text_option("--dirstat", "显示目录级差异统计"),
    option("--dirstat-by-file", "按文件显示目录统计"),
    option("--summary", "显示精简的更改摘要"),
    option("--name-only", "仅显示更改的名称"),
    option("--name-status", "显示更改的名称和状态"),
    optional_choice_option("--color", "选择彩色输出", COLOR_VALUES),
    text_option("--ws-error-highlight", "突出显示空白错误"),
    option("--full-diff", "显示每个提交的完整差异"),
    text_option("--diff-merges", "选择合并差异展示方式"),
    option("--no-diff-merges", "隐藏合并差异"),
    option("--cc", "显示合并后的组合差异"),
    option("--combined-all-paths", "显示所有父级的路径"),
    option("--no-renames", "禁用重命名检测"),
    optional_text_option("--find-renames", "检测重命名"),
    optional_text_option("-M", "使用相似度阈值检测重命名"),
    optional_text_option("--find-copies", "检测复制"),
    optional_text_option("-C", "使用相似度阈值检测复制"),
    option("--find-copies-harder", "更努力地搜索复制"),
    text_option("--diff-algorithm", "选择差异算法"),
    optional_text_option("--word-diff", "显示单词级差异"),
    optional_text_option("--color-words", "用颜色显示更改单词"),
    option("--ignore-space-change", "忽略空白数量更改"),
    option("-w", "忽略所有空白更改"),
    option("--ignore-all-space", "忽略所有空白更改"),
    option("--ignore-blank-lines", "忽略空行更改"),
    option("--indent-heuristic", "使用缩进启发式"),
    option("--no-indent-heuristic", "禁用缩进启发式"),
    text_option("--inter-hunk-context", "显示差异区块之间的上下文"),
    path_option("--output", "将输出写入文件"),
    path_option("-o", "将输出写入文件"),
];

const GIT_DIFF_OPTIONS: &[OptionDef] = &[
    option("--cached", "比较索引与 HEAD"),
    option("--staged", "比较索引与 HEAD"),
    option("--merge-base", "与合并基准比较"),
    option("--no-index", "比较仓库外的两个路径"),
    option("--exit-code", "使用状态报告差异"),
    option("--quiet", "隐藏所有输出"),
    option("--no-ext-diff", "禁止外部差异辅助程序"),
    option("--text", "将所有文件视为文本"),
    optional_choice_option(
        "--ignore-submodules",
        "选择子模块更改处理方式",
        GIT_IGNORE_SUBMODULES_VALUES,
    ),
    optional_text_option("--submodule", "选择子模块差异格式"),
    option("--no-renames", "禁用重命名检测"),
    optional_text_option("--find-renames", "检测重命名"),
    optional_text_option("-M", "使用相似度阈值检测重命名"),
    optional_text_option("--find-copies", "检测复制"),
    optional_text_option("-C", "使用相似度阈值检测复制"),
    option("--find-copies-harder", "更努力地搜索复制"),
    option("--irreversible-delete", "省略已删除文件的内容"),
    optional_path_option("--relative", "显示相对于当前目录的路径"),
    text_option("--src-prefix", "使用自定义源前缀"),
    text_option("--dst-prefix", "使用自定义目标前缀"),
    text_option("--line-prefix", "为每行输出添加前缀"),
    choice_option("--diff-filter", "选择更改路径状态", GIT_DIFF_FILTER_VALUES),
    option("--no-prefix", "省略源和目标前缀"),
    option("--default-prefix", "使用默认差异前缀"),
    text_option("--inter-hunk-context", "显示差异区块之间的上下文"),
    text_option("--output-indicator-new", "选择新增行指示符"),
    text_option("--output-indicator-old", "选择删除行指示符"),
    text_option("--output-indicator-context", "选择上下文行指示符"),
    option("--full-index", "显示完整对象名称"),
    option("--binary", "输出二进制更改"),
    optional_text_option("--abbrev", "使用缩写对象名称"),
    option("--patch", "显示补丁文本"),
    option("-p", "显示补丁文本"),
    option("--no-patch", "隐藏补丁输出"),
    option("--raw", "显示原始差异格式"),
    option("--patch-with-raw", "显示带原始标头的补丁"),
    option("--patch-with-stat", "显示带差异统计的补丁"),
    optional_text_option("--stat", "显示差异统计"),
    option("--compact-summary", "显示紧凑的差异摘要"),
    option("--numstat", "显示数值差异统计"),
    option("--shortstat", "仅显示最后一行差异统计"),
    optional_text_option("--dirstat", "显示目录级差异统计"),
    option("--cumulative", "累积目录统计"),
    option("--dirstat-by-file", "按文件显示目录统计"),
    option("--summary", "显示精简的更改摘要"),
    option("--name-only", "仅显示更改的名称"),
    option("--name-status", "显示更改的名称和状态"),
    optional_choice_option("--color", "选择彩色输出", COLOR_VALUES),
    option("--no-color", "禁用彩色输出"),
    optional_text_option("--color-moved", "选择移动行颜色"),
    optional_text_option("--color-moved-ws", "选择移动行的空白处理方式"),
    option("--no-color-moved", "禁用移动行颜色"),
    optional_text_option("--word-diff", "显示单词级差异"),
    text_option("--word-diff-regex", "选择单词边界表达式"),
    optional_text_option("--color-words", "用颜色显示更改单词"),
    option("--minimal", "投入更多计算来查找更小的差异"),
    option("--patience", "使用 patience 差异算法"),
    option("--histogram", "使用 histogram 差异算法"),
    text_option("--diff-algorithm", "选择差异算法"),
    option("--indent-heuristic", "使用缩进启发式"),
    option("--no-indent-heuristic", "禁用缩进启发式"),
    text_option("--anchored", "使用锚定差异匹配"),
    option("--ignore-space-at-eol", "忽略行尾空白"),
    option("--ignore-space-change", "忽略空白数量更改"),
    option("-b", "忽略空白数量的更改"),
    option("--ignore-all-space", "忽略所有空白更改"),
    option("-w", "忽略所有空白更改"),
    option("--ignore-blank-lines", "忽略空行更改"),
    option("-I", "忽略匹配的更改行"),
    text_option("-O", "使用文件控制差异文件顺序"),
    path_option("--skip-to", "从指定路径开始输出"),
    path_option("--rotate-to", "优先显示路径"),
    path_option("--output", "将输出写入文件"),
    path_option("-o", "将输出写入文件"),
];

const GIT_STATUS_OPTIONS: &[OptionDef] = &[
    option("--short", "使用简短状态格式"),
    option("-s", "使用简短状态格式"),
    option("--branch", "显示分支信息"),
    option("--show-stash", "显示暂存条目数量"),
    optional_choice_option(
        "--porcelain",
        "使用稳定的机器可读格式",
        GIT_PORCELAIN_VALUES,
    ),
    option("--long", "使用默认长格式"),
    option("--null", "使用 NUL 终止条目"),
    option("-z", "使用 NUL 终止条目"),
    option("--ahead-behind", "计算领先和落后计数"),
    option("--no-ahead-behind", "跳过领先和落后计数"),
    option("--renames", "检测重命名"),
    option("--no-renames", "不检测重命名"),
    optional_text_option("--find-renames", "检测重命名"),
    optional_choice_option(
        "--untracked-files",
        "选择未跟踪文件报告方式",
        GIT_UNTRACKED_VALUES,
    ),
    optional_choice_option("-u", "选择未跟踪文件报告方式", GIT_UNTRACKED_VALUES),
    optional_choice_option(
        "--ignore-submodules",
        "选择子模块更改处理方式",
        GIT_IGNORE_SUBMODULES_VALUES,
    ),
    optional_choice_option("--ignored", "选择已忽略文件报告方式", GIT_IGNORED_VALUES),
    optional_choice_option("--column", "选择列显示方式", GIT_COLUMN_VALUES),
    option("--no-column", "禁用列"),
    option("--verbose", "显示附加信息"),
    option("--optional-locks", "使用可选锁"),
    option("--no-optional-locks", "避免可选锁"),
];

const GIT_ADD_OPTIONS: &[OptionDef] = &[
    option("--verbose", "显示文件添加过程"),
    option("-n", "试运行且不更改索引"),
    option("--dry-run", "试运行且不更改索引"),
    option("-p", "交互式选择区块"),
    option("--patch", "交互式选择区块"),
    option("-e", "编辑生成的补丁"),
    option("--edit", "编辑生成的补丁"),
    option("-f", "允许添加已忽略文件"),
    option("--force", "允许添加已忽略文件"),
    option("-u", "更新已跟踪文件"),
    option("--update", "更新已跟踪文件"),
    option("-N", "记录添加未跟踪文件的意图"),
    option("--intent-to-add", "记录添加未跟踪文件的意图"),
    option("-A", "添加所有文件"),
    option("--all", "添加所有文件"),
    option("--no-all", "不添加工作树外的文件"),
    option("--ignore-removal", "忽略工作树中已删除的路径"),
    option("--refresh", "刷新索引"),
    option("--ignore-errors", "添加出错后继续"),
    option("--ignore-missing", "忽略缺失文件"),
    option("--renormalize", "对已跟踪文件应用 clean 过滤器"),
    option("--sparse", "允许更新稀疏锥外的路径"),
    text_option("--chmod", "在索引中设置可执行位"),
    path_option("--pathspec-from-file", "从文件读取路径规格"),
    option("--pathspec-file-nul", "使用 NUL 分隔路径规格"),
];

const GIT_COMMIT_OPTIONS: &[OptionDef] = &[
    option("-v", "在提交消息模板中显示差异"),
    option("--verbose", "在提交消息模板中显示差异"),
    option("-q", "隐藏摘要输出"),
    option("--quiet", "隐藏摘要输出"),
    path_option("-F", "从文件读取提交消息"),
    path_option("--file", "从文件读取提交消息"),
    text_option("--author", "覆盖作者身份"),
    text_option("--date", "覆盖作者日期"),
    text_option("-m", "使用给定的提交消息"),
    text_option("--message", "使用给定的提交消息"),
    text_option("-c", "复用并编辑提交消息"),
    text_option("--reedit-message", "复用并编辑提交消息"),
    text_option("-C", "复用提交消息但不编辑"),
    text_option("--reuse-message", "复用提交消息但不编辑"),
    text_option("--fixup", "创建 fixup 提交"),
    text_option("--squash", "创建 squash 提交"),
    option("--reset-author", "使用提交者作为作者"),
    option("--allow-empty", "允许空提交"),
    option("--allow-empty-message", "允许空提交消息"),
    option("--no-verify", "跳过提交钩子"),
    option("--verify", "运行提交钩子"),
    option("-s", "添加 Signed-off-by 尾部字段"),
    option("--signoff", "添加 Signed-off-by 尾部字段"),
    option("--no-post-rewrite", "跳过 post-rewrite 钩子"),
    option("--amend", "替换顶端提交"),
    option("--no-edit", "使用选中的提交消息"),
    option("--status", "在提交模板中包含状态"),
    option("--no-status", "不在模板中包含状态"),
    optional_text_option("-S", "使用密钥签名提交"),
    optional_text_option("--gpg-sign", "使用密钥签名提交"),
    option("--no-gpg-sign", "不为提交签名"),
    path_option("--template", "使用提交消息模板"),
    text_option("--trailer", "为提交消息添加尾部字段"),
    option("--dry-run", "显示将要提交的内容"),
    option("--porcelain", "使用机器可读输出"),
    option("--long", "使用长状态格式"),
    option("--short", "使用短状态格式"),
    option("--branch", "显示分支信息"),
    option("--ahead-behind", "计算领先和落后计数"),
    option("--no-ahead-behind", "跳过领先和落后计数"),
];

const GIT_BRANCH_OPTIONS: &[OptionDef] = &[
    option("-v", "显示分支提交信息"),
    option("--verbose", "显示分支提交信息"),
    option("-q", "隐藏非错误输出"),
    option("--quiet", "隐藏非错误输出"),
    optional_choice_option("-t", "设置上游跟踪", GIT_TRACK_VALUES),
    optional_choice_option("--track", "设置上游跟踪", GIT_TRACK_VALUES),
    text_option("-u", "设置上游分支"),
    text_option("--set-upstream-to", "设置上游分支"),
    option("--unset-upstream", "移除上游跟踪"),
    optional_choice_option("--color", "选择分支颜色", COLOR_VALUES),
    option("-r", "列出远程跟踪分支"),
    option("--remotes", "列出远程跟踪分支"),
    option("-a", "列出本地和远程分支"),
    option("--all", "列出本地和远程分支"),
    option("-l", "列出分支"),
    option("--list", "列出分支"),
    option("--show-current", "显示当前分支"),
    option("--create-reflog", "为分支创建引用日志"),
    option("--edit-description", "编辑分支描述"),
    option("-d", "删除已合并分支"),
    option("--delete", "删除已合并分支"),
    option("-D", "强制删除分支"),
    option("-m", "重命名分支"),
    option("--move", "重命名分支"),
    option("-M", "强制重命名分支"),
    option("-c", "复制分支"),
    option("--copy", "复制分支"),
    option("-C", "强制复制分支"),
    text_option("--contains", "列出包含某提交的分支"),
    text_option("--no-contains", "列出不包含某提交的分支"),
    text_option("--merged", "列出已合并到某提交的分支"),
    text_option("--no-merged", "列出未合并到某提交的分支"),
    text_option("--sort", "按字段排序分支"),
    text_option("--points-at", "列出指向某对象的分支"),
    text_option("--format", "格式化分支输出"),
    optional_choice_option("--column", "选择列显示方式", GIT_COLUMN_VALUES),
    option("--no-column", "禁用列"),
    option("--omit-empty", "省略空的格式化行"),
    option("--recurse-submodules", "在子模块中创建分支"),
];

const GIT_CHECKOUT_OPTIONS: &[OptionDef] = &[
    option("-q", "隐藏进度报告"),
    option("--quiet", "隐藏进度报告"),
    option("--progress", "强制显示进度报告"),
    option("--no-progress", "禁用进度报告"),
    option("-f", "丢弃本地更改"),
    option("--force", "丢弃本地更改"),
    option("-m", "尝试三方合并"),
    option("--merge", "尝试三方合并"),
    option("--detach", "在指定提交处分离 HEAD"),
    text_option("--orphan", "创建新的孤立分支"),
    text_option("-b", "创建并切换到分支"),
    text_option("-B", "创建或重置并切换到分支"),
    optional_choice_option("--track", "设置上游跟踪", GIT_TRACK_VALUES),
    option("--guess", "猜测远程分支"),
    option("--no-guess", "不猜测远程分支"),
    option("--overlay", "覆盖已恢复文件"),
    option("--no-overlay", "删除目标中不存在的文件"),
    option("--ignore-other-worktrees", "允许检出到其他位置的分支"),
    option("--recurse-submodules", "更新子模块"),
    option("--no-recurse-submodules", "不更新子模块"),
    path_option("--pathspec-from-file", "从文件读取路径规格"),
    option("--pathspec-file-nul", "使用 NUL 分隔路径规格"),
];

const GIT_SWITCH_OPTIONS: &[OptionDef] = &[
    option("-q", "隐藏进度报告"),
    option("--quiet", "隐藏进度报告"),
    option("--progress", "强制显示进度报告"),
    option("--no-progress", "禁用进度报告"),
    option("-f", "丢弃本地更改"),
    option("--force", "丢弃本地更改"),
    option("-d", "在指定提交处分离 HEAD"),
    option("--detach", "在指定提交处分离 HEAD"),
    text_option("-c", "创建并切换到分支"),
    text_option("--create", "创建并切换到分支"),
    text_option("-C", "强制创建并切换到分支"),
    text_option("--force-create", "强制创建并切换到分支"),
    option("--guess", "猜测远程分支"),
    option("--no-guess", "不猜测远程分支"),
    text_option("--orphan", "创建新的孤立分支"),
    option("--discard-changes", "丢弃本地更改"),
    option("--merge", "执行三方合并"),
    text_option("--conflict", "选择冲突样式"),
    option("--recurse-submodules", "更新子模块"),
    option("--no-recurse-submodules", "不更新子模块"),
];

const GIT_FETCH_OPTIONS: &[OptionDef] = &[
    option("--all", "获取所有远程仓库"),
    option("--append", "将引用名称追加到 FETCH_HEAD"),
    option("--atomic", "使用原子更新"),
    text_option("--depth", "限制获取深度"),
    text_option("--deepen", "加深浅层仓库"),
    text_option("--shallow-since", "加深指定日期后的历史"),
    text_option("--shallow-exclude", "加深排除指定引用的历史"),
    option("--unshallow", "将浅层仓库转换为完整仓库"),
    option("--update-shallow", "接受浅层边界更新"),
    text_option("--negotiation-tip", "使用提交作为协商提示"),
    option("--negotiate-only", "报告共同祖先但不获取"),
    option("--dry-run", "显示将要获取的内容"),
    option("--porcelain", "使用机器可读输出"),
    option("-f", "强制更新本地引用"),
    option("--force", "强制更新本地引用"),
    option("--keep", "保留下载的 pack"),
    option("-m", "从多个远程仓库获取"),
    option("--multiple", "从多个远程仓库获取"),
    option("--auto-maintenance", "获取后运行维护"),
    option("--auto-gc", "运行自动垃圾回收"),
    option("--write-fetch-head", "写入 FETCH_HEAD"),
    option("--no-write-fetch-head", "不写入 FETCH_HEAD"),
    option("--prefetch", "获取到 prefetch 命名空间"),
    option("-p", "清理已删除的远程跟踪引用"),
    option("--prune", "清理已删除的远程跟踪引用"),
    option("--prune-tags", "清理远程标签"),
    option("--no-tags", "不获取标签"),
    option("--tags", "获取所有标签"),
    optional_choice_option(
        "--recurse-submodules",
        "选择获取子模块方式",
        GIT_RECURSE_SUBMODULE_VALUES,
    ),
    text_option("--jobs", "设置并行子模块任务数"),
    text_option("--submodule-prefix", "设置子模块路径前缀"),
    text_option("--recurse-submodules-default", "设置默认子模块策略"),
    option("-u", "为获取的分支设置上游"),
    option("--set-upstream", "为获取的分支设置上游"),
    text_option("--upload-pack", "选择 upload-pack 程序"),
    option("-q", "隐藏获取输出"),
    option("--quiet", "隐藏获取输出"),
    option("-v", "显示详细获取输出"),
    option("--verbose", "显示详细获取输出"),
    option("--progress", "强制显示进度报告"),
    text_option("--server-option", "向服务器发送选项"),
    option("--show-forced-updates", "报告强制更新"),
    option("--no-show-forced-updates", "不报告强制更新"),
    option("--ipv4", "仅使用 IPv4"),
    option("--ipv6", "仅使用 IPv6"),
];

const GIT_PULL_OPTIONS: &[OptionDef] = &[
    option("--all", "获取所有远程仓库"),
    option("--append", "将引用名称追加到 FETCH_HEAD"),
    option("--commit", "合并后提交"),
    option("--no-commit", "合并后不提交"),
    option("-e", "编辑合并消息"),
    option("--edit", "编辑合并消息"),
    option("--no-edit", "使用默认合并消息"),
    text_option("--cleanup", "清理合并消息"),
    option("--ff", "允许快进更新"),
    option("--ff-only", "仅允许快进更新"),
    option("--no-ff", "始终创建合并提交"),
    option("--verify-signatures", "验证顶端提交的签名"),
    option("-v", "显示详细输出"),
    option("--verbose", "显示详细输出"),
    option("-q", "隐藏输出"),
    option("--quiet", "隐藏输出"),
    option("--progress", "强制显示进度报告"),
    option("--no-progress", "禁用进度报告"),
    optional_choice_option(
        "--recurse-submodules",
        "选择获取子模块方式",
        GIT_RECURSE_SUBMODULE_VALUES,
    ),
    option("--no-recurse-submodules", "不获取子模块"),
    option("--tags", "获取所有标签"),
    option("--no-tags", "不获取标签"),
    option("-p", "清理已删除的远程跟踪引用"),
    option("--prune", "清理已删除的远程跟踪引用"),
    option("-u", "为获取的分支设置上游"),
    option("--set-upstream", "为获取的分支设置上游"),
    optional_choice_option(
        "--rebase",
        "选择变基方式",
        &[
            ValueDef {
                name: "false",
                description: "合并已获取的更改",
            },
            ValueDef {
                name: "true",
                description: "对本地提交执行变基",
            },
            ValueDef {
                name: "merges",
                description: "保留合并执行变基",
            },
            ValueDef {
                name: "interactive",
                description: "运行交互式变基",
            },
        ],
    ),
    option("--no-rebase", "合并已获取的更改"),
    option("--autostash", "更新前暂存本地更改"),
    option("--no-autostash", "不暂存本地更改"),
    option("--allow-unrelated-histories", "允许不相关历史"),
    option("--stat", "显示差异统计"),
    option("--no-stat", "隐藏差异统计"),
    optional_text_option("--log", "包含 shortlog 条目"),
    option("--no-log", "不包含 shortlog 条目"),
    option("--squash", "不创建合并提交"),
    text_option("--strategy", "选择合并策略"),
    text_option("-s", "选择合并策略"),
    text_option("-X", "向合并策略传递选项"),
    text_option("--strategy-option", "向合并策略传递选项"),
    optional_text_option("-S", "为合并提交签名"),
    optional_text_option("--gpg-sign", "为合并提交签名"),
    option("--signoff", "添加 Signed-off-by 尾部字段"),
    option("--no-signoff", "不添加 Signed-off-by 尾部字段"),
    option("--verify", "运行钩子"),
    option("--no-verify", "跳过钩子"),
    text_option("--depth", "限制获取深度"),
    text_option("--shallow-since", "加深指定日期后的历史"),
    text_option("--shallow-exclude", "加深排除指定引用的历史"),
    option("--unshallow", "将浅层仓库转换为完整仓库"),
    option("--update-shallow", "接受浅层边界更新"),
    text_option("--negotiation-tip", "使用提交作为协商提示"),
];

const GIT_PUSH_OPTIONS: &[OptionDef] = &[
    option("-v", "显示详细推送输出"),
    option("--verbose", "显示详细推送输出"),
    option("-q", "隐藏推送输出"),
    option("--quiet", "隐藏推送输出"),
    option("--progress", "强制显示进度报告"),
    option("--no-progress", "禁用进度报告"),
    option("-n", "显示将要推送的内容"),
    option("--dry-run", "显示将要推送的内容"),
    option("--porcelain", "使用机器可读输出"),
    option("--delete", "删除远程引用"),
    option("--all", "推送所有分支"),
    option("--prune", "删除本地不存在的远程引用"),
    option("--mirror", "镜像所有引用"),
    option("--tags", "推送所有标签"),
    option("--follow-tags", "推送带注释标签"),
    optional_choice_option(
        "--signed",
        "选择推送签名方式",
        &[
            ValueDef {
                name: "true",
                description: "要求推送签名",
            },
            ValueDef {
                name: "false",
                description: "不为推送签名",
            },
            ValueDef {
                name: "if-asked",
                description: "服务器要求时签名",
            },
        ],
    ),
    option("--atomic", "使用原子更新"),
    text_option("-o", "向服务器发送推送选项"),
    text_option("--push-option", "向服务器发送推送选项"),
    text_option("--receive-pack", "选择 receive-pack 程序"),
    text_option("--exec", "选择 receive-pack 程序"),
    option("-u", "为推送的分支设置上游"),
    option("--set-upstream", "为推送的分支设置上游"),
    option("-f", "强制更新远程引用"),
    option("--force", "强制更新远程引用"),
    optional_text_option("--force-with-lease", "使用租约强制更新"),
    option("--force-if-includes", "强制更新前要求已获取历史"),
    choice_option(
        "--recurse-submodules",
        "选择子模块推送方式",
        &[
            ValueDef {
                name: "check",
                description: "拒绝缺失的子模块提交",
            },
            ValueDef {
                name: "on-demand",
                description: "推送所需的子模块提交",
            },
            ValueDef {
                name: "no",
                description: "不递归进入子模块",
            },
        ],
    ),
    option("--thin", "使用精简 pack"),
    option("--no-thin", "使用完整 pack"),
    text_option("--repo", "选择要推送的仓库"),
    option("--ipv4", "仅使用 IPv4"),
    option("--ipv6", "仅使用 IPv6"),
    option("--no-verify", "跳过 pre-push 钩子"),
];

const GIT_STASH_COMMANDS: &[CommandDef] = &[
    command("list", "列出暂存条目"),
    command("show", "显示暂存中记录的更改"),
    command("drop", "删除一个暂存条目"),
    command("pop", "应用并删除暂存条目"),
    command("apply", "应用暂存条目"),
    command("branch", "从暂存条目创建分支"),
    command("push", "将本地更改保存到新暂存"),
    command("save", "将本地更改保存到新暂存"),
    command("clear", "删除所有暂存条目"),
    command("create", "创建但不保存暂存对象"),
    command("store", "保存暂存对象"),
];

const GIT_STASH_OPTIONS: &[OptionDef] = &[
    option("-q", "隐藏暂存输出"),
    option("--quiet", "隐藏暂存输出"),
    option("--no-quiet", "显示暂存输出"),
    option("-p", "交互式选择区块"),
    option("--patch", "交互式选择区块"),
    option("-u", "包含未跟踪文件"),
    option("--include-untracked", "包含未跟踪文件"),
    option("-a", "包含已忽略文件"),
    option("--all", "包含已忽略文件"),
    text_option("-m", "设置暂存消息"),
    text_option("--message", "设置暂存消息"),
    option("-k", "保留已暂存更改"),
    option("--keep-index", "保留已暂存更改"),
    option("--no-keep-index", "取消暂存中的更改"),
    option("--index", "尝试恢复索引状态"),
    option("--no-apply", "不应用暂存"),
    option("--only-untracked", "仅暂存未跟踪文件"),
    option("--staged", "仅暂存已暂存更改"),
    path_option("--pathspec-from-file", "从文件读取路径规格"),
    option("--pathspec-file-nul", "使用 NUL 分隔路径规格"),
];

const GIT_STASH_LIST_OPTIONS: &[OptionDef] = &[
    option("--stat", "显示差异统计"),
    option("--patch", "显示补丁文本"),
    option("-p", "显示补丁文本"),
    option("--oneline", "每条暂存显示为一行"),
    text_option("--format", "格式化暂存输出"),
    choice_option("--date", "选择日期格式", GIT_DATE_VALUES),
];

const GIT_STASH_SHOW_OPTIONS: &[OptionDef] = &[
    option("--patch", "显示补丁文本"),
    option("-p", "显示补丁文本"),
    option("--stat", "显示差异统计"),
    option("--name-only", "仅显示更改的名称"),
    option("--name-status", "显示更改的名称和状态"),
    optional_choice_option("--color", "选择彩色输出", COLOR_VALUES),
    option("--no-color", "禁用彩色输出"),
    option("--include-untracked", "包含未跟踪文件"),
    option("--only-untracked", "仅显示未跟踪文件"),
];

const GIT_STASH_APPLY_OPTIONS: &[OptionDef] = &[
    option("--index", "尝试恢复索引状态"),
    option("--quiet", "隐藏暂存输出"),
    option("--reindex", "应用前重建索引"),
];

const GIT_STASH_DROP_OPTIONS: &[OptionDef] = &[option("--quiet", "隐藏暂存输出")];

const GIT_STASH_PUSH_OPTIONS: &[OptionDef] = &[
    option("-q", "隐藏暂存输出"),
    option("--quiet", "隐藏暂存输出"),
    option("-p", "交互式选择区块"),
    option("--patch", "交互式选择区块"),
    option("-u", "包含未跟踪文件"),
    option("--include-untracked", "包含未跟踪文件"),
    option("-a", "包含已忽略文件"),
    option("--all", "包含已忽略文件"),
    text_option("-m", "设置暂存消息"),
    text_option("--message", "设置暂存消息"),
    option("-k", "保留已暂存更改"),
    option("--keep-index", "保留已暂存更改"),
    option("--no-keep-index", "取消暂存中的更改"),
    option("--no-apply", "不应用暂存"),
    option("--only-untracked", "仅暂存未跟踪文件"),
    option("--staged", "仅暂存已暂存更改"),
    path_option("--pathspec-from-file", "从文件读取路径规格"),
    option("--pathspec-file-nul", "使用 NUL 分隔路径规格"),
];

const GIT_REMOTE_COMMANDS: &[CommandDef] = &[
    command("add", "添加远程仓库"),
    command("rename", "重命名远程仓库"),
    command("remove", "删除远程仓库"),
    command("rm", "删除远程仓库"),
    command("set-head", "设置或删除默认分支"),
    command("set-branches", "更改跟踪的分支"),
    command("get-url", "获取远程 URL"),
    command("set-url", "更改远程 URL"),
    command("show", "显示远程仓库信息"),
    command("prune", "删除过时的远程跟踪分支"),
    command("update", "更新远程仓库"),
];

const GIT_REMOTE_OPTIONS: &[OptionDef] = &[
    option("-v", "显示远程 URL"),
    option("--verbose", "显示远程 URL"),
    option("--no-verbose", "隐藏远程 URL"),
];

const GIT_REMOTE_ADD_OPTIONS: &[OptionDef] = &[
    option("-f", "添加远程仓库后获取"),
    option("--fetch", "添加远程仓库后获取"),
    option("--tags", "从远程仓库导入标签"),
    option("--no-tags", "不导入标签"),
    text_option("-t", "跟踪选中的分支"),
    text_option("--track", "跟踪选中的分支"),
    text_option("-m", "设置远程默认分支"),
    choice_option(
        "--mirror",
        "配置镜像方式",
        &[
            ValueDef {
                name: "fetch",
                description: "获取时镜像引用",
            },
            ValueDef {
                name: "push",
                description: "推送时镜像引用",
            },
        ],
    ),
];

const GIT_REMOTE_SET_HEAD_OPTIONS: &[OptionDef] = &[
    option("-a", "自动选择默认分支"),
    option("--auto", "自动选择默认分支"),
    option("-d", "删除默认分支"),
    option("--delete", "删除默认分支"),
];

const GIT_REMOTE_SET_BRANCHES_OPTIONS: &[OptionDef] = &[
    option("-a", "将分支添加到跟踪列表"),
    option("--add", "将分支添加到跟踪列表"),
];

const GIT_REMOTE_GET_URL_OPTIONS: &[OptionDef] = &[
    option("--push", "显示推送 URL"),
    option("--all", "显示所有 URL"),
];

const GIT_REMOTE_SET_URL_OPTIONS: &[OptionDef] = &[
    option("--push", "更改推送 URL"),
    option("--add", "添加另一个 URL"),
    option("--delete", "删除 URL"),
];

const GIT_REMOTE_SHOW_OPTIONS: &[OptionDef] = &[
    option("-n", "跳过远程 HEAD 查询"),
    option("--no-query", "跳过远程 HEAD 查询"),
    option("-v", "显示详细远程信息"),
    option("--verbose", "显示详细远程信息"),
];

const GIT_REMOTE_PRUNE_OPTIONS: &[OptionDef] = &[option("--dry-run", "显示将要删除的内容")];

const GIT_REMOTE_UPDATE_OPTIONS: &[OptionDef] = &[
    option("--prune", "清理过时引用"),
    option("--prune-tags", "清理过时标签"),
    text_option("--upload-pack", "选择 upload-pack 程序"),
];

const CARGO_COMMANDS: &[CommandDef] = &[
    command("add", "将依赖添加到清单"),
    command("bench", "运行基准测试"),
    command("build", "编译包"),
    command("check", "检查包但不生成二进制文件"),
    command("clean", "删除生成的构建产物"),
    command("clippy", "运行 Clippy 代码检查"),
    command("doc", "构建包文档"),
    command("fetch", "下载依赖"),
    command("fix", "自动修复编译器警告"),
    command("fmt", "格式化 Rust 代码"),
    command("generate-lockfile", "生成 Cargo.lock 文件"),
    command("install", "安装 Rust 二进制"),
    command("metadata", "输出包元数据"),
    command("new", "创建新包"),
    command("publish", "将包发布到包仓库"),
    command("remove", "从清单移除依赖"),
    command("report", "显示 Cargo 报告"),
    command("run", "运行二进制或示例"),
    command("rustc", "使用 rustc 选项编译包"),
    command("rustdoc", "使用 rustdoc 选项构建文档"),
    command("search", "在包仓库中搜索包"),
    command("test", "运行测试"),
    command("tree", "显示依赖树"),
    command("uninstall", "卸载 Rust 二进制"),
    command("update", "更新依赖"),
    command("vendor", "将所有依赖放入本地目录"),
    command("version", "显示 Cargo 版本"),
    command("locate-project", "定位 Cargo 项目清单"),
];

const CARGO_GLOBAL_OPTIONS: &[OptionDef] = &[
    option("--help", "显示 Cargo 帮助"),
    option("--version", "显示 Cargo 版本"),
    option("-v", "使用详细输出"),
    option("--verbose", "使用详细输出"),
    option("-q", "隐藏 Cargo 输出"),
    option("--quiet", "隐藏 Cargo 输出"),
    choice_option("--color", "选择彩色输出", COLOR_VALUES),
    option("--locked", "要求 Cargo.lock 保持不变"),
    option("--offline", "在不访问网络的情况下运行"),
    option("--frozen", "使用锁定和离线模式"),
    path_option("--manifest-path", "使用指定 Cargo.toml 文件"),
    text_option("--package", "选择包"),
    option("--workspace", "对整个工作区操作"),
    text_option("--exclude", "排除工作区包"),
    text_option("--features", "启用包特性"),
    option("--all-features", "启用所有包特性"),
    option("--no-default-features", "禁用默认特性"),
    text_option("--target", "选择编译目标"),
    text_option("--jobs", "设置并行任务数"),
    text_option("-j", "设置并行任务数"),
    text_option("--config", "覆盖 Cargo 配置值"),
    option("--future-incompat-report", "显示未来不兼容报告"),
    text_option("-Z", "使用不稳定的 Cargo 选项"),
];

const CARGO_BUILD_OPTIONS: &[OptionDef] = &[
    option("--lib", "构建库目标"),
    text_option("--bin", "构建二进制目标"),
    text_option("--example", "构建示例目标"),
    text_option("--test", "构建集成测试目标"),
    text_option("--bench", "构建基准测试目标"),
    option("--all-targets", "构建所有目标"),
    option("--release", "使用发布配置构建，默认启用优化"),
    text_option("--profile", "选择 Cargo 配置"),
    path_option("--target-dir", "选择构建输出目录"),
    path_option("--artifact-dir", "选择构建产物输出目录"),
    optional_choice_option("--timings", "写入编译耗时报告", CARGO_TIMING_VALUES),
    option("--unit-graph", "输出单元依赖图"),
    option("--build-plan", "输出实验性构建计划"),
    option("--keep-going", "继续构建独立包"),
];

const CARGO_CHECK_OPTIONS: &[OptionDef] = &[
    option("--lib", "检查库目标"),
    text_option("--bin", "检查二进制目标"),
    text_option("--example", "检查示例目标"),
    text_option("--test", "检查集成测试目标"),
    text_option("--bench", "检查基准测试目标"),
    option("--all-targets", "检查所有目标"),
    option("--release", "使用发布配置检查"),
    text_option("--profile", "选择 Cargo 配置"),
    path_option("--target-dir", "选择构建输出目录"),
    optional_choice_option("--timings", "写入编译耗时报告", CARGO_TIMING_VALUES),
    option("--unit-graph", "输出单元依赖图"),
    option("--keep-going", "继续检查独立包"),
];

const CARGO_RUN_OPTIONS: &[OptionDef] = &[
    option("--lib", "运行库目标"),
    option("--release", "使用发布配置运行"),
    text_option("--profile", "选择 Cargo 配置"),
    text_option("--bin", "选择二进制目标"),
    text_option("--example", "选择示例目标"),
    path_option("--target-dir", "选择构建输出目录"),
    choice_option(
        "--message-format",
        "选择编译器消息格式",
        CARGO_MESSAGE_FORMAT_VALUES,
    ),
    option("--unit-graph", "输出单元依赖图"),
    option("--keep-going", "继续运行独立包"),
];

const CARGO_TEST_OPTIONS: &[OptionDef] = &[
    option("--no-run", "编译测试但不运行"),
    option("--no-fail-fast", "失败后运行所有测试"),
    option("--doc", "测试文档"),
    option("--lib", "测试库目标"),
    text_option("--bin", "测试二进制目标"),
    text_option("--example", "测试示例目标"),
    text_option("--test", "测试集成目标"),
    text_option("--bench", "测试基准目标"),
    option("--all-targets", "测试所有目标"),
    option("--release", "使用发布配置测试"),
    text_option("--profile", "选择 Cargo 配置"),
    path_option("--target-dir", "选择构建输出目录"),
    choice_option(
        "--message-format",
        "选择编译器消息格式",
        CARGO_MESSAGE_FORMAT_VALUES,
    ),
    option("--keep-going", "继续运行独立包"),
];

const CARGO_BENCH_OPTIONS: &[OptionDef] = &[
    option("--lib", "对库目标执行基准测试"),
    text_option("--bin", "对二进制目标执行基准测试"),
    text_option("--example", "对示例目标执行基准测试"),
    text_option("--bench", "选择基准测试目标"),
    option("--no-run", "编译基准测试但不运行"),
    option("--no-fail-fast", "失败后运行所有基准测试"),
    option("--all-targets", "对所有目标执行基准测试"),
    option("--release", "使用发布配置执行基准测试"),
    text_option("--profile", "选择 Cargo 配置"),
    path_option("--target-dir", "选择构建输出目录"),
    choice_option(
        "--message-format",
        "选择编译器消息格式",
        CARGO_MESSAGE_FORMAT_VALUES,
    ),
    option("--keep-going", "继续运行独立包"),
];

const CARGO_CLIPPY_OPTIONS: &[OptionDef] = &[
    option("--all-targets", "检查所有目标代码"),
    option("--lib", "检查库目标代码"),
    text_option("--bin", "检查二进制目标代码"),
    text_option("--example", "检查示例目标代码"),
    text_option("--test", "检查集成测试目标代码"),
    text_option("--bench", "检查基准测试目标代码"),
    option("--fix", "自动应用 Clippy 建议"),
    option("--allow-dirty", "允许在有未提交更改的工作树中修复"),
    option("--allow-staged", "允许在有暂存更改时修复"),
    option("--no-deps", "跳过依赖包"),
    option("--release", "使用发布配置检查代码"),
    text_option("--profile", "选择 Cargo 配置"),
    path_option("--target-dir", "选择构建输出目录"),
    choice_option(
        "--message-format",
        "选择编译器消息格式",
        CARGO_MESSAGE_FORMAT_VALUES,
    ),
    option("--keep-going", "继续检查独立包代码"),
];

const CARGO_FMT_OPTIONS: &[OptionDef] = &[
    option("--check", "检查格式但不更改文件"),
    option("--all", "格式化所有工作区包"),
    option("-v", "使用详细输出"),
    option("--verbose", "使用详细输出"),
    option("-q", "隐藏输出"),
    option("--quiet", "隐藏输出"),
    choice_option(
        "--message-format",
        "选择格式化器消息格式",
        &[
            ValueDef {
                name: "short",
                description: "使用简短的格式化器消息",
            },
            ValueDef {
                name: "json",
                description: "使用 JSON 格式化器消息",
            },
        ],
    ),
    text_option("--emit", "选择格式化器输出模式"),
    text_option("--edition", "选择 Rust 版本"),
    path_option("--manifest-path", "使用指定 Cargo.toml 文件"),
];

const NPM_COMMANDS: &[CommandDef] = &[
    command("access", "管理包访问设置"),
    command("audit", "运行安全审计"),
    command("bugs", "打开包错误跟踪器"),
    command("cache", "管理 npm 缓存"),
    command("ci", "从锁定文件安装"),
    command("completion", "启用 Shell 补全"),
    command("config", "管理 npm 配置"),
    command("dedupe", "减少重复依赖"),
    command("deprecate", "弃用软件包版本"),
    command("diff", "比较软件包版本"),
    command("dist-tag", "管理发布标签"),
    command("doctor", "检查 npm 环境"),
    command("docs", "打开软件包文档"),
    command("exec", "从软件包运行命令"),
    command("explain", "解释已安装的软件包"),
    command("explore", "打开软件包目录"),
    command("fund", "显示资助信息"),
    command("help", "显示 npm 帮助"),
    command("hook", "管理软件包仓库钩子"),
    command("init", "创建 package.json 文件"),
    command("install", "安装软件包依赖"),
    command("i", "安装软件包依赖"),
    command("install-ci-test", "安装依赖并运行 CI 测试"),
    command("install-test", "安装依赖并运行测试"),
    command("link", "为软件包创建符号链接"),
    command("ll", "列出已安装的软件包"),
    command("login", "登录软件包仓库"),
    command("logout", "退出软件包仓库"),
    command("ls", "列出已安装的软件包"),
    command("org", "管理组织"),
    command("outdated", "检查过时的软件包"),
    command("owner", "管理软件包所有者"),
    command("pack", "创建软件包归档"),
    command("ping", "Ping 软件包仓库"),
    command("pkg", "管理软件包元数据"),
    command("prefix", "显示 npm 前缀"),
    command("profile", "管理 npm 个人资料"),
    command("prune", "移除多余的软件包"),
    command("publish", "发布软件包"),
    command("query", "查询已安装的软件包"),
    command("rebuild", "重新构建软件包"),
    command("repo", "打开软件包仓库"),
    command("restart", "运行 restart 脚本"),
    command("root", "显示 npm 根目录"),
    command("run", "运行软件包脚本"),
    command("run-script", "运行软件包脚本"),
    command("search", "搜索软件包仓库"),
    command("set", "设置配置值"),
    command("shrinkwrap", "创建或更新 shrinkwrap"),
    command("start", "运行 start 脚本"),
    command("stop", "运行 stop 脚本"),
    command("team", "管理团队"),
    command("test", "运行 test 脚本"),
    command("token", "管理身份验证令牌"),
    command("uninstall", "移除软件包依赖"),
    command("remove", "移除软件包依赖"),
    command("unpublish", "从软件包仓库移除软件包"),
    command("update", "更新软件包依赖"),
    command("version", "提升软件包版本"),
    command("view", "查看软件包元数据"),
];

const NPM_GLOBAL_OPTIONS: &[OptionDef] = &[
    option("--help", "显示 npm 帮助"),
    option("--version", "显示 npm 版本"),
    option("-g", "执行全局操作"),
    option("--global", "执行全局操作"),
    option("--save", "保存依赖"),
    option("--save-dev", "保存开发依赖"),
    option("--save-exact", "保存精确版本"),
    path_option("--prefix", "使用其他 npm 前缀"),
    text_option("--workspace", "指定工作区"),
    option("--workspaces", "指定所有工作区"),
    option("--include-workspace-root", "包含工作区根目录"),
    option("--ignore-scripts", "跳过软件包脚本"),
    option("--production", "省略开发依赖"),
    option("--omit", "省略依赖类型"),
    option("--include", "包含依赖类型"),
    option("--json", "使用 JSON 输出"),
    option("--silent", "隐藏输出"),
    choice_option("--loglevel", "选择日志详细级别", NPM_LOGLEVEL_VALUES),
    choice_option("--level", "选择日志详细级别", NPM_LOGLEVEL_VALUES),
    text_option("--registry", "使用软件包仓库"),
    path_option("--userconfig", "使用其他用户配置文件"),
    path_option("--cache", "使用其他 npm 缓存"),
    option("--yes", "对提示自动回答 yes"),
    option("--force", "强制执行 npm 操作"),
    option("--offline", "仅使用缓存的软件包"),
    option("--prefer-offline", "优先使用缓存的软件包"),
    option("--prefer-online", "优先使用最新软件包元数据"),
    option("--no-audit", "跳过安全审计"),
    option("--no-fund", "跳过资助信息"),
    option("--no-update-notifier", "禁用更新通知"),
    option("--foreground-scripts", "在前台运行脚本"),
    option("--no-bin-links", "不创建二进制链接"),
    option("--ignore-optional", "跳过可选依赖"),
    choice_option("--color", "选择彩色输出", COLOR_VALUES),
];

const NPM_INSTALL_OPTIONS: &[OptionDef] = &[
    option("--save", "保存依赖"),
    option("--no-save", "不保存依赖"),
    option("--save-dev", "保存开发依赖"),
    option("--save-optional", "保存可选依赖"),
    option("--save-peer", "保存对等依赖"),
    option("--save-exact", "保存精确版本"),
    option("--global", "全局安装"),
    option("-g", "全局安装"),
    option("--dry-run", "显示将要安装的内容"),
    option("--package-lock", "更新软件包锁定文件"),
    option("--no-package-lock", "跳过软件包锁定文件"),
    option("--ignore-scripts", "跳过软件包脚本"),
    option("--foreground-scripts", "在前台运行脚本"),
    option("--audit", "运行安全审计"),
    option("--no-audit", "跳过安全审计"),
    option("--fund", "显示资助信息"),
    option("--no-fund", "跳过资助信息"),
    option("--legacy-peer-deps", "忽略对等依赖冲突"),
    option("--strict-peer-deps", "遇到对等依赖冲突时失败"),
    option("--engine-strict", "拒绝不兼容的运行时"),
    option("--force", "强制解析依赖"),
    option("--install-links", "将文件依赖安装为链接"),
    text_option("--install-strategy", "选择依赖安装策略"),
    text_option("--omit", "省略依赖类型"),
    text_option("--include", "包含依赖类型"),
    text_option("--workspace", "指定工作区"),
    option("--workspaces", "指定所有工作区"),
    option("--include-workspace-root", "包含工作区根目录"),
    path_option("--prefix", "使用其他 npm 前缀"),
];

const NPM_RUN_OPTIONS: &[OptionDef] = &[
    text_option("--workspace", "指定工作区"),
    option("--workspaces", "指定所有工作区"),
    option("--include-workspace-root", "包含工作区根目录"),
    option("--if-present", "忽略缺失的脚本"),
    option("--ignore-scripts", "跳过软件包脚本"),
    option("--foreground-scripts", "在前台运行脚本"),
    path_option("--script-shell", "选择脚本 Shell"),
];

const NPM_INIT_OPTIONS: &[OptionDef] = &[
    option("--yes", "接受所有默认值"),
    option("--force", "覆盖现有软件包文件"),
    text_option("--scope", "设置软件包作用域"),
    option("--private", "将软件包标记为私有"),
    text_option("--workspace", "创建工作区软件包"),
];

const NPM_CONFIG_COMMANDS: &[CommandDef] = &[
    command("get", "获取配置值"),
    command("set", "设置配置值"),
    command("delete", "删除配置值"),
    command("list", "列出配置值"),
    command("fix", "修复配置问题"),
];

const NPM_CONFIG_OPTIONS: &[OptionDef] = &[
    option("--global", "使用全局配置"),
    option("--location", "选择配置位置"),
    option("--json", "使用 JSON 输出"),
    option("--long", "显示详细配置"),
    option("--parseable", "使用可解析输出"),
];

const DOCKER_COMMANDS: &[CommandDef] = &[
    command("build", "从 Dockerfile 构建镜像"),
    command("builder", "管理构建"),
    command("buildx", "使用 BuildKit 构建"),
    command("checkpoint", "管理检查点"),
    command("commit", "从容器创建镜像"),
    command("compose", "定义并运行多容器应用"),
    command("config", "管理 Swarm 配置"),
    command("container", "管理容器"),
    command("context", "管理 Docker 上下文"),
    command("cp", "在容器与主机之间复制文件"),
    command("create", "创建新容器"),
    command("diff", "检查容器文件系统的更改"),
    command("events", "获取服务器实时事件"),
    command("exec", "在运行中的容器内执行命令"),
    command("export", "导出容器文件系统"),
    command("history", "显示镜像历史记录"),
    command("image", "管理镜像"),
    command("images", "列出镜像"),
    command("info", "显示系统信息"),
    command("init", "为容器化项目创建文件"),
    command("inspect", "返回底层信息"),
    command("kill", "强制终止运行中的容器"),
    command("load", "从归档加载镜像"),
    command("login", "登录软件包仓库"),
    command("logout", "退出软件包仓库"),
    command("logs", "获取容器日志"),
    command("manifest", "管理镜像清单"),
    command("network", "管理网络"),
    command("node", "管理 Swarm 节点"),
    command("pause", "暂停容器中的所有进程"),
    command("plugin", "管理插件"),
    command("port", "列出端口映射"),
    command("ps", "列出容器"),
    command("pull", "拉取镜像或仓库"),
    command("push", "推送镜像或仓库"),
    command("rename", "重命名容器"),
    command("restart", "重启容器"),
    command("rm", "移除容器"),
    command("rmi", "移除镜像"),
    command("run", "在新容器中执行命令"),
    command("save", "将镜像保存到归档"),
    command("search", "搜索 Docker Hub"),
    command("secret", "管理 Swarm 密钥"),
    command("service", "管理 Swarm 服务"),
    command("stack", "管理 Swarm 堆栈"),
    command("start", "启动已停止的容器"),
    command("stats", "显示容器资源使用情况"),
    command("stop", "停止运行中的容器"),
    command("swarm", "管理 Swarm"),
    command("system", "管理 Docker 数据"),
    command("tag", "为镜像创建标签"),
    command("top", "显示运行中的进程"),
    command("trust", "管理镜像信任"),
    command("unpause", "恢复容器中进程运行"),
    command("update", "更新容器配置"),
    command("version", "显示 Docker 版本信息"),
    command("volume", "管理卷"),
    command("wait", "等待容器停止"),
];

const DOCKER_GLOBAL_OPTIONS: &[OptionDef] = &[
    option("--help", "显示 Docker 帮助"),
    option("--version", "显示 Docker 版本"),
    path_option("--config", "使用客户端配置目录"),
    text_option("--context", "使用 Docker 上下文"),
    option("-D", "启用调试模式"),
    option("--debug", "启用调试模式"),
    text_option("-H", "连接 Docker 守护进程"),
    text_option("--host", "连接 Docker 守护进程"),
    choice_option(
        "--log-level",
        "选择日志详细级别",
        &[
            ValueDef {
                name: "debug",
                description: "显示调试日志",
            },
            ValueDef {
                name: "info",
                description: "显示信息日志",
            },
            ValueDef {
                name: "warn",
                description: "显示警告",
            },
            ValueDef {
                name: "error",
                description: "显示错误",
            },
            ValueDef {
                name: "fatal",
                description: "显示致命错误",
            },
        ],
    ),
    option("--tls", "使用 TLS"),
    path_option("--tlscacert", "使用 TLS CA 证书"),
    path_option("--tlscert", "使用 TLS 客户端证书"),
    path_option("--tlskey", "使用 TLS 客户端密钥"),
    option("--tlsverify", "验证 TLS 守护进程"),
];

const DOCKER_BUILD_OPTIONS: &[OptionDef] = &[
    path_option("-f", "使用 Dockerfile"),
    path_option("--file", "使用 Dockerfile"),
    text_option("-t", "为构建的镜像添加标签"),
    text_option("--tag", "为构建的镜像添加标签"),
    text_option("--build-arg", "设置构建时变量"),
    text_option("--label", "设置镜像标签"),
    option("--no-cache", "不使用构建缓存"),
    choice_option("--pull", "选择基础镜像拉取策略", DOCKER_PULL_VALUES),
    option("-q", "隐藏构建输出"),
    option("--quiet", "隐藏构建输出"),
    option("--rm", "移除中间容器"),
    option("--force-rm", "始终移除中间容器"),
    text_option("--memory", "设置构建内存上限"),
    text_option("--shm-size", "设置共享内存大小"),
    text_option("--network", "选择构建网络"),
    text_option("--platform", "选择目标平台"),
    text_option("--progress", "选择构建进度输出"),
    text_option("--secret", "暴露构建密钥"),
    text_option("--ssh", "暴露 SSH 代理"),
    text_option("--target", "选择构建阶段"),
    text_option("--cache-from", "使用外部构建缓存"),
    text_option("--cache-to", "导出构建缓存"),
    text_option("--build-context", "添加命名构建上下文"),
    text_option("-o", "导出构建输出"),
    text_option("--output", "导出构建输出"),
    option("--provenance", "设置来源证明模式"),
    option("--sbom", "设置 SBOM 证明模式"),
    path_option("--metadata-file", "将构建元数据写入文件"),
];

const DOCKER_RUN_OPTIONS: &[OptionDef] = &[
    option("-d", "以分离模式运行"),
    option("--detach", "以分离模式运行"),
    option("-i", "保持标准输入打开"),
    option("--interactive", "保持标准输入打开"),
    option("-t", "分配伪终端"),
    option("--tty", "分配伪终端"),
    option("--rm", "退出后移除容器"),
    text_option("--name", "指定容器名称"),
    text_option("-h", "设置容器主机名"),
    text_option("--hostname", "设置容器主机名"),
    text_option("-e", "设置环境变量"),
    text_option("--env", "设置环境变量"),
    path_option("--env-file", "从文件读取环境变量"),
    text_option("-p", "发布容器端口"),
    option("-P", "发布所有已暴露端口"),
    option("--publish-all", "发布所有已暴露端口"),
    text_option("-v", "绑定挂载卷"),
    text_option("--volume", "绑定挂载卷"),
    text_option("--mount", "附加文件系统挂载"),
    text_option("-w", "设置工作目录"),
    text_option("--workdir", "设置工作目录"),
    text_option("-u", "设置用户或 UID"),
    text_option("--user", "设置用户或 UID"),
    text_option("--network", "连接到网络"),
    text_option("--network-alias", "添加网络别名"),
    text_option("--restart", "选择重启策略"),
    text_option("--entrypoint", "覆盖镜像入口点"),
    text_option("-l", "设置容器标签"),
    text_option("--label", "设置容器标签"),
    text_option("--add-host", "添加主机到 IP 的映射"),
    text_option("--cap-add", "添加 Linux 能力"),
    text_option("--cap-drop", "丢弃 Linux 能力"),
    text_option("--device", "添加主机设备"),
    text_option("--group-add", "添加附加组"),
    option("--init", "使用 init 进程"),
    text_option("--ip", "设置 IPv4 地址"),
    text_option("--ip6", "设置 IPv6 地址"),
    text_option("--mac-address", "设置 MAC 地址"),
    text_option("-m", "设置内存上限"),
    text_option("--memory", "设置内存上限"),
    text_option("--cpus", "设置 CPU 配额"),
    text_option("--cpu-shares", "设置 CPU 份额权重"),
    text_option("--cpuset-cpus", "设置使用的 CPU 数量"),
    text_option("--pids-limit", "限制进程数量"),
    option("--privileged", "授予扩展权限"),
    option("--read-only", "以只读方式挂载根文件系统"),
    text_option("--security-opt", "设置安全选项"),
    text_option("--shm-size", "设置共享内存大小"),
    text_option("--sysctl", "设置内核参数"),
    text_option("--ulimit", "设置 ulimit"),
    text_option("--runtime", "选择容器运行时"),
    text_option("--platform", "选择目标平台"),
    choice_option("--pull", "选择镜像拉取策略", DOCKER_PULL_VALUES),
    option("-q", "隐藏拉取输出"),
    option("--quiet", "隐藏拉取输出"),
    option("--sig-proxy", "将信号转发给进程"),
    text_option("--stop-signal", "设置停止信号"),
    text_option("--stop-timeout", "设置停止超时"),
    text_option("--tmpfs", "挂载临时文件系统"),
    text_option("--userns", "选择用户命名空间"),
    text_option("--uts", "选择 UTS 命名空间"),
    text_option("--pid", "选择 PID 命名空间"),
    text_option("--ipc", "选择 IPC 命名空间"),
    text_option("--health-cmd", "设置健康检查命令"),
    text_option("--health-interval", "设置健康检查间隔"),
    text_option("--health-timeout", "设置健康检查超时"),
    text_option("--health-retries", "设置健康检查重试次数"),
    text_option("--health-start-period", "设置健康检查启动等待时间"),
    option("--no-healthcheck", "禁用镜像健康检查"),
];

const DOCKER_PS_OPTIONS: &[OptionDef] = &[
    option("-a", "显示所有容器"),
    option("--all", "显示所有容器"),
    text_option("-f", "筛选容器列表"),
    text_option("--filter", "筛选容器列表"),
    text_option("--format", "设置容器输出格式"),
    text_option("-n", "显示最新的 n 个容器"),
    text_option("--last", "显示最新的 n 个容器"),
    option("-l", "显示最新容器"),
    option("--latest", "显示最新容器"),
    option("--no-trunc", "不截断输出"),
    option("-q", "仅显示 ID"),
    option("--quiet", "仅显示 ID"),
    option("-s", "显示文件总大小"),
    option("--size", "显示文件总大小"),
];

const DOCKER_EXEC_OPTIONS: &[OptionDef] = &[
    option("-d", "以分离模式运行"),
    option("--detach", "以分离模式运行"),
    text_option("--detach-keys", "设置分离键序列"),
    text_option("-e", "设置环境变量"),
    text_option("--env", "设置环境变量"),
    path_option("--env-file", "从文件读取环境变量"),
    option("-i", "保持标准输入打开"),
    option("--interactive", "保持标准输入打开"),
    option("--privileged", "授予扩展权限"),
    option("-t", "分配伪终端"),
    option("--tty", "分配伪终端"),
    text_option("-u", "设置用户或 UID"),
    text_option("--user", "设置用户或 UID"),
    text_option("-w", "设置工作目录"),
    text_option("--workdir", "设置工作目录"),
];

const DOCKER_IMAGES_OPTIONS: &[OptionDef] = &[
    option("-a", "显示中间镜像"),
    option("--all", "显示中间镜像"),
    option("--digests", "显示镜像摘要"),
    text_option("-f", "筛选镜像列表"),
    text_option("--filter", "筛选镜像列表"),
    text_option("--format", "设置镜像输出格式"),
    option("--no-trunc", "不截断输出"),
    option("-q", "仅显示 ID"),
    option("--quiet", "仅显示 ID"),
];

const DOCKER_PULL_OPTIONS: &[OptionDef] = &[
    option("-a", "下载所有带标签的镜像"),
    option("--all-tags", "下载所有带标签的镜像"),
    text_option("--platform", "选择目标平台"),
    option("-q", "隐藏拉取输出"),
    option("--quiet", "隐藏拉取输出"),
    option("--disable-content-trust", "跳过镜像签名"),
];

const DOCKER_PUSH_OPTIONS: &[OptionDef] = &[
    option("-a", "推送所有带标签的镜像"),
    option("--all-tags", "推送所有带标签的镜像"),
    option("-q", "隐藏推送输出"),
    option("--quiet", "隐藏推送输出"),
    option("--disable-content-trust", "跳过镜像签名"),
];

const DOCKER_COMMON_OPTIONS: &[OptionDef] = &[option("--help", "显示命令帮助")];

const DOCKER_COMPOSE_COMMANDS: &[CommandDef] = &[
    command("build", "构建或重新构建服务"),
    command("config", "验证并查看 Compose 文件"),
    command("cp", "在服务与主机之间复制文件"),
    command("create", "创建服务"),
    command("down", "停止并移除资源"),
    command("events", "接收服务实时事件"),
    command("exec", "在服务中运行命令"),
    command("images", "列出服务使用的镜像"),
    command("kill", "强制停止服务"),
    command("logs", "查看服务输出"),
    command("pause", "暂停服务"),
    command("port", "输出公开端口绑定"),
    command("ps", "列出服务"),
    command("pull", "拉取服务镜像"),
    command("push", "推送服务镜像"),
    command("restart", "重启服务"),
    command("rm", "移除已停止的服务容器"),
    command("run", "运行一次性命令"),
    command("start", "启动服务"),
    command("stop", "停止服务"),
    command("top", "显示服务进程"),
    command("unpause", "恢复服务运行"),
    command("up", "创建并启动服务"),
    command("version", "显示 Compose 版本"),
    command("watch", "监视源文件并重新构建服务"),
];

const DOCKER_COMPOSE_OPTIONS: &[OptionDef] = &[
    path_option("-f", "使用 Compose 文件"),
    path_option("--file", "使用 Compose 文件"),
    text_option("-p", "设置项目名称"),
    text_option("--project-name", "设置项目名称"),
    path_option("--project-directory", "设置项目目录"),
    path_option("--env-file", "使用环境文件"),
    text_option("--profile", "启用 Compose profile"),
    option("--verbose", "启用详细输出"),
    option("--parallel", "并行构建"),
    option("--progress", "选择进度输出"),
];

const DOCKER_COMPOSE_UP_OPTIONS: &[OptionDef] = &[
    option("-d", "以分离模式运行"),
    option("--detach", "以分离模式运行"),
    option("--build", "启动前构建镜像"),
    option("--no-build", "不构建镜像"),
    option("--force-recreate", "重新创建容器"),
    option("--no-recreate", "不重新创建现有容器"),
    option("--no-deps", "不启动关联服务"),
    option("--remove-orphans", "移除孤立容器"),
    option("--always-recreate-deps", "重新创建依赖容器"),
    option("--renew-anon-volumes", "重新创建匿名卷"),
    option("--wait", "等待服务运行"),
    text_option("--wait-timeout", "设置等待超时"),
    option("--quiet-pull", "拉取时不显示进度输出"),
    text_option("--scale", "设置服务副本数"),
];

const DOCKER_CONTAINER_COMMANDS: &[CommandDef] = &[
    command("attach", "将本地输入输出连接到容器"),
    command("commit", "从容器创建镜像"),
    command("cp", "在容器与主机之间复制文件"),
    command("create", "创建容器"),
    command("diff", "检查文件系统更改"),
    command("exec", "在运行中的容器内执行命令"),
    command("export", "导出容器文件系统"),
    command("inspect", "显示容器详情"),
    command("kill", "强制终止容器"),
    command("logs", "获取容器日志"),
    command("ls", "列出容器"),
    command("pause", "暂停容器"),
    command("port", "列出端口映射"),
    command("prune", "移除未使用的容器"),
    command("rename", "重命名容器"),
    command("restart", "重启容器"),
    command("rm", "移除容器"),
    command("run", "在新容器中执行命令"),
    command("start", "启动容器"),
    command("stats", "显示容器资源使用情况"),
    command("stop", "停止容器"),
    command("top", "显示容器中的进程"),
    command("unpause", "恢复容器运行"),
    command("update", "更新容器配置"),
    command("wait", "等待容器停止"),
];

const DOCKER_IMAGE_COMMANDS: &[CommandDef] = &[
    command("build", "构建镜像"),
    command("history", "显示镜像历史记录"),
    command("import", "从文件系统归档创建镜像"),
    command("inspect", "显示镜像详情"),
    command("load", "从归档加载镜像"),
    command("ls", "列出镜像"),
    command("prune", "移除未使用的镜像"),
    command("pull", "拉取镜像"),
    command("push", "推送镜像"),
    command("rm", "移除镜像"),
    command("save", "将镜像保存到归档"),
    command("tag", "为镜像添加标签"),
];

const DOCKER_NETWORK_COMMANDS: &[CommandDef] = &[
    command("connect", "将容器连接到网络"),
    command("create", "创建网络"),
    command("disconnect", "断开容器与网络的连接"),
    command("inspect", "显示网络详情"),
    command("ls", "列出网络"),
    command("prune", "移除未使用的网络"),
    command("rm", "移除网络"),
];

const DOCKER_VOLUME_COMMANDS: &[CommandDef] = &[
    command("create", "创建卷"),
    command("inspect", "显示卷详情"),
    command("ls", "列出卷"),
    command("prune", "移除未使用的卷"),
    command("rm", "移除卷"),
];

const DOCKER_SYSTEM_COMMANDS: &[CommandDef] = &[
    command("df", "显示 Docker 磁盘使用情况"),
    command("events", "显示 Docker 事件"),
    command("info", "显示 Docker 系统信息"),
    command("prune", "移除未使用的 Docker 数据"),
];

const PWSH_OPTIONS: &[OptionDef] = &[
    text_option("-Command", "运行指定的 PowerShell 命令"),
    text_option("-EncodedCommand", "运行 Base64 编码的命令"),
    text_option("-EncodedArguments", "传递 Base64 编码的参数"),
    choice_option(
        "-ExecutionPolicy",
        "设置执行策略",
        &[
            ValueDef {
                name: "Bypass",
                description: "不阻止此进程中的脚本",
            },
            ValueDef {
                name: "Unrestricted",
                description: "允许不受限制地执行脚本",
            },
            ValueDef {
                name: "RemoteSigned",
                description: "要求下载脚本具有签名",
            },
            ValueDef {
                name: "AllSigned",
                description: "要求所有脚本具有签名",
            },
            ValueDef {
                name: "Restricted",
                description: "禁止脚本",
            },
        ],
    ),
    path_option("-File", "运行 PowerShell 脚本文件"),
    choice_option("-InputFormat", "选择输入格式", PWSH_INPUT_VALUES),
    option("-Login", "以登录 Shell 启动"),
    option("-Mta", "使用多线程单元状态"),
    option("-NoExit", "启动后保持 Shell 打开"),
    option("-NoLogo", "隐藏启动徽标"),
    option("-NonInteractive", "禁用交互式提示"),
    option("-NoProfile", "跳过加载 PowerShell 配置文件"),
    choice_option("-OutputFormat", "选择输出格式", PWSH_OUTPUT_VALUES),
    option("-Sta", "使用单线程单元状态"),
    text_option("-Version", "选择 PowerShell 版本"),
    choice_option("-WindowStyle", "选择窗口样式", PWSH_WINDOW_VALUES),
    path_option("-WorkingDirectory", "设置初始工作目录"),
    option("--help", "显示 PowerShell 帮助"),
    option("--version", "显示 PowerShell 版本"),
];

const GH_COMMANDS: &[CommandDef] = &[
    command("alias", "管理 GitHub CLI 别名"),
    command("api", "发起经过身份验证的 GitHub API 请求"),
    command("attestation", "管理构件证明"),
    command("auth", "进行 GitHub 身份验证"),
    command("browse", "在浏览器中打开 GitHub 页面"),
    command("codespace", "连接并管理 Codespaces"),
    command("config", "管理 GitHub CLI 配置"),
    command("copilot", "使用 GitHub Copilot"),
    command("extension", "管理 GitHub CLI 扩展"),
    command("gist", "管理 Gist"),
    command("issue", "管理 Issue"),
    command("label", "管理 Issue 标签"),
    command("org", "管理组织"),
    command("pr", "管理 Pull Request"),
    command("project", "管理项目"),
    command("release", "管理发布"),
    command("repo", "管理代码仓库"),
    command("ruleset", "管理代码仓库规则集"),
    command("run", "查看和管理 GitHub Actions 运行记录"),
    command("search", "搜索 GitHub"),
    command("secret", "管理代码仓库密钥"),
    command("ssh-key", "管理 SSH 密钥"),
    command("status", "显示通知和状态"),
    command("variable", "管理代码仓库变量"),
    command("workflow", "管理 GitHub Actions 工作流"),
];

const GH_GLOBAL_OPTIONS: &[OptionDef] = &[
    option("--help", "显示 GitHub CLI 帮助"),
    option("--version", "显示 GitHub CLI 版本"),
    text_option("--hostname", "使用 GitHub Enterprise 主机"),
    text_option("--repo", "选择代码仓库"),
    text_option("--json", "输出 JSON 字段"),
    text_option("--jq", "使用 jq 筛选 JSON"),
    text_option("--template", "使用模板设置输出格式"),
    option("--web", "在浏览器中打开结果"),
    option("--paginate", "请求所有分页"),
    option("--slurp", "将分页 JSON 包装在数组中"),
];

const GH_API_OPTIONS: &[OptionDef] = &[
    text_option("--method", "选择 HTTP 方法"),
    text_option("-F", "添加类型化参数"),
    text_option("--field", "添加类型化参数"),
    text_option("-f", "添加字符串参数"),
    text_option("--raw-field", "添加字符串参数"),
    text_option("-H", "添加 HTTP 请求头"),
    text_option("--header", "添加 HTTP 请求头"),
    path_option("--input", "从文件读取请求正文"),
    path_option("--input-file", "从文件读取请求正文"),
    text_option("--jq", "使用 jq 筛选 JSON"),
    text_option("--template", "使用模板设置输出格式"),
    option("--include", "包含响应头"),
    option("--silent", "不输出响应正文"),
    option("--cache", "缓存响应"),
    option("--paginate", "请求所有分页"),
    option("--slurp", "将分页 JSON 包装在数组中"),
    option("--verbose", "显示请求详情"),
];

const GH_PR_COMMANDS: &[CommandDef] = &[
    command("checkout", "在本地检出 Pull Request"),
    command("checks", "显示 Pull Request 检查"),
    command("close", "关闭 Pull Request"),
    command("comment", "向 Pull Request 添加评论"),
    command("create", "创建 Pull Request"),
    command("diff", "查看 Pull Request 更改"),
    command("edit", "编辑 Pull Request"),
    command("list", "列出 Pull Request"),
    command("lock", "锁定 Pull Request 讨论"),
    command("merge", "合并 Pull Request"),
    command("ready", "将 Pull Request 标记为可供审查"),
    command("reopen", "重新打开 Pull Request"),
    command("review", "审查 Pull Request"),
    command("status", "显示 Pull Request 状态"),
    command("unlock", "解锁 Pull Request 讨论"),
    command("view", "查看 Pull Request"),
];

const GH_ISSUE_COMMANDS: &[CommandDef] = &[
    command("close", "关闭 Issue"),
    command("comment", "向 Issue 添加评论"),
    command("create", "创建 Issue"),
    command("delete", "删除 Issue"),
    command("develop", "管理 Issue 开发分支"),
    command("edit", "编辑 Issue"),
    command("list", "列出 Issue"),
    command("lock", "锁定 Issue 讨论"),
    command("pin", "固定 Issue"),
    command("reopen", "重新打开 Issue"),
    command("status", "显示 Issue 状态"),
    command("transfer", "转移 Issue"),
    command("unlock", "解锁 Issue 讨论"),
    command("unpin", "取消固定 Issue"),
    command("view", "查看 Issue"),
];

const GH_REPO_COMMANDS: &[CommandDef] = &[
    command("archive", "归档代码仓库"),
    command("clone", "克隆仓库"),
    command("create", "创建代码仓库"),
    command("delete", "删除代码仓库"),
    command("deploy-key", "管理部署密钥"),
    command("edit", "编辑代码仓库设置"),
    command("fork", "派生代码仓库"),
    command("list", "列出代码仓库"),
    command("rename", "重命名代码仓库"),
    command("set-default", "设置默认代码仓库"),
    command("sync", "同步派生仓库"),
    command("unarchive", "取消归档代码仓库"),
    command("view", "查看代码仓库"),
];

const GH_RUN_COMMANDS: &[CommandDef] = &[
    command("cancel", "取消工作流运行"),
    command("delete", "删除工作流运行"),
    command("download", "下载运行构件"),
    command("list", "列出工作流运行记录"),
    command("rerun", "重新运行工作流"),
    command("view", "查看工作流运行记录"),
    command("watch", "监视工作流运行"),
];

const GH_WORKFLOW_COMMANDS: &[CommandDef] = &[
    command("disable", "禁用工作流"),
    command("enable", "启用工作流"),
    command("list", "列出工作流"),
    command("run", "运行工作流"),
    command("view", "查看工作流"),
];

const GH_AUTH_COMMANDS: &[CommandDef] = &[
    command("login", "登录 GitHub"),
    command("logout", "退出 GitHub"),
    command("refresh", "刷新身份验证"),
    command("setup-git", "配置 Git 凭据存储"),
    command("status", "显示身份验证状态"),
    command("switch", "切换身份验证账户"),
    command("token", "输出身份验证令牌"),
];

const GH_SEARCH_COMMANDS: &[CommandDef] = &[
    command("code", "搜索代码"),
    command("commits", "搜索提交"),
    command("issues", "搜索 Issue"),
    command("prs", "搜索 Pull Request"),
    command("repos", "搜索代码仓库"),
    command("topics", "搜索主题"),
    command("users", "搜索用户"),
];

const GH_COMMON_OPTIONS: &[OptionDef] = &[
    text_option("--repo", "选择代码仓库"),
    text_option("--json", "输出 JSON 字段"),
    text_option("--jq", "使用 jq 筛选 JSON"),
    text_option("--template", "使用模板设置输出格式"),
    option("--web", "在浏览器中打开结果"),
    option("--help", "显示命令帮助"),
];

const GH_LIST_OPTIONS: &[OptionDef] = &[
    text_option("--limit", "限制结果数量"),
    text_option("--state", "按状态筛选"),
    text_option("--author", "按作者筛选"),
    text_option("--assignee", "按受指派人筛选"),
    text_option("--label", "按标签筛选"),
    text_option("--search", "使用搜索表达式筛选"),
    text_option("--sort", "对结果排序"),
    text_option("--order", "选择排序顺序"),
];

const SET_LOCATION_OPTIONS: &[OptionDef] = &[
    path_option("-Path", "设置工作目录"),
    path_option("-LiteralPath", "设置工作目录且不展开通配符"),
    option("-Force", "包含隐藏位置"),
    option("-PassThru", "返回选定位置"),
    text_option("-StackName", "使用位置堆栈"),
    option("-UseTransaction", "使用当前事务"),
    option("-Verbose", "显示详细输出"),
    text_option("-ErrorAction", "选择错误处理方式"),
    text_option("-ErrorVariable", "将错误存入变量"),
];

const GET_CHILD_ITEM_OPTIONS: &[OptionDef] = &[
    path_option("-Path", "选择路径"),
    path_option("-LiteralPath", "选择路径且不展开通配符"),
    text_option("-Filter", "使用提供程序模式筛选项目"),
    text_option("-Include", "包含匹配项目"),
    text_option("-Exclude", "排除匹配项目"),
    option("-Recurse", "递归搜索子项目"),
    option("-Force", "包含隐藏和系统项目"),
    option("-Name", "仅返回项目名称"),
    option("-Directory", "仅返回目录"),
    option("-File", "仅返回文件"),
    text_option("-Depth", "限制递归深度"),
    text_option("-Attributes", "按项目属性筛选"),
    option("-FollowSymlink", "跟随符号链接"),
    option("-Hidden", "返回隐藏项目"),
    option("-ReadOnly", "返回只读项目"),
    option("-System", "返回系统项目"),
    text_option("-ErrorAction", "选择错误处理方式"),
    text_option("-ErrorVariable", "将错误存入变量"),
    option("-Verbose", "显示详细输出"),
];

fn git_context(parts: &[&str]) -> ContextDef {
    match parts {
        ["git"] => context(GIT_COMMANDS, GIT_GLOBAL_OPTIONS, PositionalKind::None),
        ["git", "log"] => context(&[], GIT_LOG_OPTIONS, PositionalKind::Path),
        ["git", "show"] => context(&[], GIT_SHOW_OPTIONS, PositionalKind::Path),
        ["git", "diff"] => context(&[], GIT_DIFF_OPTIONS, PositionalKind::Path),
        ["git", "status"] => context(&[], GIT_STATUS_OPTIONS, PositionalKind::Path),
        ["git", "add"] => context(&[], GIT_ADD_OPTIONS, PositionalKind::Path),
        ["git", "commit"] => context(&[], GIT_COMMIT_OPTIONS, PositionalKind::Path),
        ["git", "branch"] => context(&[], GIT_BRANCH_OPTIONS, PositionalKind::Value),
        ["git", "checkout"] => context(&[], GIT_CHECKOUT_OPTIONS, PositionalKind::Path),
        ["git", "switch"] => context(&[], GIT_SWITCH_OPTIONS, PositionalKind::Value),
        ["git", "fetch"] => context(&[], GIT_FETCH_OPTIONS, PositionalKind::Value),
        ["git", "pull"] => context(&[], GIT_PULL_OPTIONS, PositionalKind::Value),
        ["git", "push"] => context(&[], GIT_PUSH_OPTIONS, PositionalKind::Value),
        ["git", "stash"] => context(GIT_STASH_COMMANDS, GIT_STASH_OPTIONS, PositionalKind::Path),
        ["git", "stash", "list"] => context(&[], GIT_STASH_LIST_OPTIONS, PositionalKind::Value),
        ["git", "stash", "show"] => context(&[], GIT_STASH_SHOW_OPTIONS, PositionalKind::Value),
        ["git", "stash", "drop"] => context(&[], GIT_STASH_DROP_OPTIONS, PositionalKind::Value),
        ["git", "stash", "pop"] | ["git", "stash", "apply"] => {
            context(&[], GIT_STASH_APPLY_OPTIONS, PositionalKind::Value)
        }
        ["git", "stash", "branch"] => context(&[], GIT_STASH_APPLY_OPTIONS, PositionalKind::Value),
        ["git", "stash", "push"] | ["git", "stash", "save"] => {
            context(&[], GIT_STASH_PUSH_OPTIONS, PositionalKind::Path)
        }
        ["git", "stash", "clear"] => context(&[], &[], PositionalKind::None),
        ["git", "stash", "create"] => context(&[], &[], PositionalKind::Value),
        ["git", "stash", "store"] => context(&[], GIT_STASH_APPLY_OPTIONS, PositionalKind::Value),
        ["git", "remote"] => context(
            GIT_REMOTE_COMMANDS,
            GIT_REMOTE_OPTIONS,
            PositionalKind::Value,
        ),
        ["git", "remote", "add"] => context(&[], GIT_REMOTE_ADD_OPTIONS, PositionalKind::Value),
        ["git", "remote", "rename"] => context(&[], &[], PositionalKind::Value),
        ["git", "remote", "remove"] | ["git", "remote", "rm"] => {
            context(&[], &[], PositionalKind::Value)
        }
        ["git", "remote", "set-head"] => {
            context(&[], GIT_REMOTE_SET_HEAD_OPTIONS, PositionalKind::Value)
        }
        ["git", "remote", "set-branches"] => {
            context(&[], GIT_REMOTE_SET_BRANCHES_OPTIONS, PositionalKind::Value)
        }
        ["git", "remote", "get-url"] => {
            context(&[], GIT_REMOTE_GET_URL_OPTIONS, PositionalKind::Value)
        }
        ["git", "remote", "set-url"] => {
            context(&[], GIT_REMOTE_SET_URL_OPTIONS, PositionalKind::Value)
        }
        ["git", "remote", "show"] => context(&[], GIT_REMOTE_SHOW_OPTIONS, PositionalKind::Value),
        ["git", "remote", "prune"] => context(&[], GIT_REMOTE_PRUNE_OPTIONS, PositionalKind::Value),
        ["git", "remote", "update"] => {
            context(&[], GIT_REMOTE_UPDATE_OPTIONS, PositionalKind::Value)
        }
        ["git", "config"] => context(&[], &[], PositionalKind::Value),
        ["git", "init"] => context(&[], &[], PositionalKind::Path),
        ["git", "clone"] => context(&[], &[], PositionalKind::Path),
        ["git", "clean"] => context(&[], &[], PositionalKind::Path),
        ["git", "restore"] => context(&[], &[], PositionalKind::Path),
        ["git", "rm"] | ["git", "mv"] => context(&[], &[], PositionalKind::Path),
        ["git", "tag"] => context(&[], &[], PositionalKind::Value),
        ["git", "reset"] | ["git", "revert"] | ["git", "merge"] => {
            context(&[], &[], PositionalKind::Value)
        }
        _ => context(&[], &[], PositionalKind::Value),
    }
}

fn cargo_context(parts: &[&str]) -> ContextDef {
    match parts {
        ["cargo"] => context(CARGO_COMMANDS, CARGO_GLOBAL_OPTIONS, PositionalKind::None),
        ["cargo", "build"] => context(&[], CARGO_BUILD_OPTIONS, PositionalKind::Value),
        ["cargo", "check"] => context(&[], CARGO_CHECK_OPTIONS, PositionalKind::Value),
        ["cargo", "run"] => context(&[], CARGO_RUN_OPTIONS, PositionalKind::Value),
        ["cargo", "test"] => context(&[], CARGO_TEST_OPTIONS, PositionalKind::Value),
        ["cargo", "bench"] => context(&[], CARGO_BENCH_OPTIONS, PositionalKind::Value),
        ["cargo", "clippy"] => context(&[], CARGO_CLIPPY_OPTIONS, PositionalKind::Value),
        ["cargo", "fmt"] => context(&[], CARGO_FMT_OPTIONS, PositionalKind::Path),
        ["cargo", "new"] | ["cargo", "init"] => context(&[], &[], PositionalKind::Path),
        ["cargo", "locate-project"] => context(&[], &[], PositionalKind::Path),
        ["cargo", "metadata"] | ["cargo", "tree"] | ["cargo", "search"] => {
            context(&[], &[], PositionalKind::Value)
        }
        ["cargo", "clean"] | ["cargo", "vendor"] => context(&[], &[], PositionalKind::Path),
        _ => context(&[], &[], PositionalKind::Value),
    }
}

fn npm_context(parts: &[&str]) -> ContextDef {
    match parts {
        ["npm"] => context(NPM_COMMANDS, NPM_GLOBAL_OPTIONS, PositionalKind::Value),
        ["npm", "install"] | ["npm", "i"] | ["npm", "add"] => {
            context(&[], NPM_INSTALL_OPTIONS, PositionalKind::Value)
        }
        ["npm", "ci"] => context(&[], NPM_INSTALL_OPTIONS, PositionalKind::Value),
        ["npm", "run"] | ["npm", "run-script"] => {
            context(&[], NPM_RUN_OPTIONS, PositionalKind::Value)
        }
        ["npm", "start"] | ["npm", "stop"] | ["npm", "restart"] | ["npm", "test"] => {
            context(&[], NPM_RUN_OPTIONS, PositionalKind::Value)
        }
        ["npm", "init"] => context(&[], NPM_INIT_OPTIONS, PositionalKind::Path),
        ["npm", "config"] => context(
            NPM_CONFIG_COMMANDS,
            NPM_CONFIG_OPTIONS,
            PositionalKind::Value,
        ),
        ["npm", "uninstall"] | ["npm", "remove"] => {
            context(&[], NPM_INSTALL_OPTIONS, PositionalKind::Value)
        }
        _ => context(&[], NPM_GLOBAL_OPTIONS, PositionalKind::Value),
    }
}

fn docker_context(parts: &[&str]) -> ContextDef {
    match parts {
        ["docker"] => context(
            DOCKER_COMMANDS,
            DOCKER_GLOBAL_OPTIONS,
            PositionalKind::Value,
        ),
        ["docker", "build"] => context(&[], DOCKER_BUILD_OPTIONS, PositionalKind::Path),
        ["docker", "run"] | ["docker", "create"] => {
            context(&[], DOCKER_RUN_OPTIONS, PositionalKind::Value)
        }
        ["docker", "ps"] => context(&[], DOCKER_PS_OPTIONS, PositionalKind::Value),
        ["docker", "images"] => context(&[], DOCKER_IMAGES_OPTIONS, PositionalKind::Value),
        ["docker", "exec"] => context(&[], DOCKER_EXEC_OPTIONS, PositionalKind::Value),
        ["docker", "pull"] => context(&[], DOCKER_PULL_OPTIONS, PositionalKind::Value),
        ["docker", "push"] => context(&[], DOCKER_PUSH_OPTIONS, PositionalKind::Value),
        ["docker", "compose"] => context(
            DOCKER_COMPOSE_COMMANDS,
            DOCKER_COMPOSE_OPTIONS,
            PositionalKind::Value,
        ),
        ["docker", "compose", "up"] => {
            context(&[], DOCKER_COMPOSE_UP_OPTIONS, PositionalKind::Value)
        }
        ["docker", "container"] => context(
            DOCKER_CONTAINER_COMMANDS,
            DOCKER_COMMON_OPTIONS,
            PositionalKind::Value,
        ),
        ["docker", "image"] => context(
            DOCKER_IMAGE_COMMANDS,
            DOCKER_COMMON_OPTIONS,
            PositionalKind::Value,
        ),
        ["docker", "network"] => context(
            DOCKER_NETWORK_COMMANDS,
            DOCKER_COMMON_OPTIONS,
            PositionalKind::Value,
        ),
        ["docker", "volume"] => context(
            DOCKER_VOLUME_COMMANDS,
            DOCKER_COMMON_OPTIONS,
            PositionalKind::Value,
        ),
        ["docker", "system"] => context(
            DOCKER_SYSTEM_COMMANDS,
            DOCKER_COMMON_OPTIONS,
            PositionalKind::Value,
        ),
        _ => context(&[], DOCKER_COMMON_OPTIONS, PositionalKind::Value),
    }
}

fn gh_context(parts: &[&str]) -> ContextDef {
    match parts {
        ["gh"] => context(GH_COMMANDS, GH_GLOBAL_OPTIONS, PositionalKind::Value),
        ["gh", "api"] => context(&[], GH_API_OPTIONS, PositionalKind::Value),
        ["gh", "pr"] => context(GH_PR_COMMANDS, GH_COMMON_OPTIONS, PositionalKind::Value),
        ["gh", "issue"] => context(GH_ISSUE_COMMANDS, GH_COMMON_OPTIONS, PositionalKind::Value),
        ["gh", "repo"] => context(GH_REPO_COMMANDS, GH_COMMON_OPTIONS, PositionalKind::Value),
        ["gh", "run"] => context(GH_RUN_COMMANDS, GH_COMMON_OPTIONS, PositionalKind::Value),
        ["gh", "workflow"] => context(
            GH_WORKFLOW_COMMANDS,
            GH_COMMON_OPTIONS,
            PositionalKind::Value,
        ),
        ["gh", "auth"] => context(GH_AUTH_COMMANDS, GH_COMMON_OPTIONS, PositionalKind::Value),
        ["gh", "search"] => context(GH_SEARCH_COMMANDS, GH_COMMON_OPTIONS, PositionalKind::Value),
        ["gh", "pr", "list"] | ["gh", "issue", "list"] => {
            context(&[], GH_LIST_OPTIONS, PositionalKind::Value)
        }
        ["gh", "browse"] => context(&[], GH_COMMON_OPTIONS, PositionalKind::Value),
        _ => context(&[], GH_COMMON_OPTIONS, PositionalKind::Value),
    }
}

fn context_for(root: &'static str, parts: &[&str]) -> ContextDef {
    match root {
        "git" => git_context(parts),
        "cargo" => cargo_context(parts),
        "npm" => npm_context(parts),
        "docker" => docker_context(parts),
        "gh" => gh_context(parts),
        "pwsh" => context(&[], PWSH_OPTIONS, PositionalKind::Value),
        "Set-Location" => context(&[], SET_LOCATION_OPTIONS, PositionalKind::Path),
        "Get-ChildItem" => context(&[], GET_CHILD_ITEM_OPTIONS, PositionalKind::Path),
        _ => context(&[], &[], PositionalKind::Value),
    }
}

/// Return the canonical root command name for a command or one of its
/// executable/PowerShell aliases.
pub fn canonical_command(command: &str) -> Option<&'static str> {
    let trimmed = command
        .trim()
        .trim_matches(|character| character == '\'' || character == '"');
    let basename = trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed);
    let lower = basename.to_ascii_lowercase();
    let stem = [".exe", ".cmd", ".bat", ".com"]
        .iter()
        .find_map(|suffix| lower.strip_suffix(suffix))
        .unwrap_or(&lower);
    match stem {
        "git" => Some("git"),
        "cargo" => Some("cargo"),
        "npm" => Some("npm"),
        "docker" => Some("docker"),
        "pwsh" | "powershell" => Some("pwsh"),
        "gh" => Some("gh"),
        "set-location" | "cd" | "chdir" | "sl" => Some("Set-Location"),
        "get-childitem" | "gci" | "dir" | "ls" => Some("Get-ChildItem"),
        _ => None,
    }
}

/// Return a short, human-readable description for a recognized root command.
pub fn describe_command(command: &str) -> Option<&'static str> {
    match canonical_command(command)? {
        "git" => Some("管理代码版本并协作开发"),
        "cargo" => Some("构建项目并管理 Rust 依赖"),
        "npm" => Some("安装和管理 JavaScript 依赖"),
        "docker" => Some("构建镜像并管理容器"),
        "pwsh" => Some("运行 PowerShell 命令和脚本"),
        "gh" => Some("在终端管理 GitHub 项目"),
        "Set-Location" => Some("切换当前目录或位置"),
        "Get-ChildItem" => Some("列出文件、目录和其他子项"),
        _ => None,
    }
}

/// Return every built-in description, including entries in tables that are
/// not currently reachable from the completion parser.
///
/// This is intentionally a small inspection API for localization coverage.
/// Keeping the table inventory here makes the Rust coverage test exercise the
/// same static catalog that stage-3 translation updates.
#[doc(hidden)]
pub fn all_builtin_descriptions() -> Vec<&'static str> {
    let mut descriptions = Vec::new();

    for values in [
        COLOR_VALUES,
        GIT_DATE_VALUES,
        GIT_DIFF_FILTER_VALUES,
        GIT_IGNORED_VALUES,
        GIT_UNTRACKED_VALUES,
        GIT_IGNORE_SUBMODULES_VALUES,
        GIT_PORCELAIN_VALUES,
        GIT_COLUMN_VALUES,
        GIT_RECURSE_SUBMODULE_VALUES,
        GIT_TRACK_VALUES,
        GIT_DECORATE_VALUES,
        CARGO_MESSAGE_FORMAT_VALUES,
        CARGO_TIMING_VALUES,
        NPM_LOGLEVEL_VALUES,
        DOCKER_PULL_VALUES,
        PWSH_INPUT_VALUES,
        PWSH_OUTPUT_VALUES,
        PWSH_WINDOW_VALUES,
    ] {
        append_value_descriptions(&mut descriptions, values);
    }

    for commands in [
        GIT_COMMANDS,
        GIT_STASH_COMMANDS,
        GIT_REMOTE_COMMANDS,
        CARGO_COMMANDS,
        NPM_COMMANDS,
        NPM_CONFIG_COMMANDS,
        DOCKER_COMMANDS,
        DOCKER_COMPOSE_COMMANDS,
        DOCKER_CONTAINER_COMMANDS,
        DOCKER_IMAGE_COMMANDS,
        DOCKER_NETWORK_COMMANDS,
        DOCKER_VOLUME_COMMANDS,
        DOCKER_SYSTEM_COMMANDS,
        GH_COMMANDS,
        GH_PR_COMMANDS,
        GH_ISSUE_COMMANDS,
        GH_REPO_COMMANDS,
        GH_RUN_COMMANDS,
        GH_WORKFLOW_COMMANDS,
        GH_AUTH_COMMANDS,
        GH_SEARCH_COMMANDS,
    ] {
        append_command_descriptions(&mut descriptions, commands);
    }

    for options in [
        GIT_GLOBAL_OPTIONS,
        GIT_LOG_OPTIONS,
        GIT_SHOW_OPTIONS,
        GIT_DIFF_OPTIONS,
        GIT_STATUS_OPTIONS,
        GIT_ADD_OPTIONS,
        GIT_COMMIT_OPTIONS,
        GIT_BRANCH_OPTIONS,
        GIT_CHECKOUT_OPTIONS,
        GIT_SWITCH_OPTIONS,
        GIT_FETCH_OPTIONS,
        GIT_PULL_OPTIONS,
        GIT_PUSH_OPTIONS,
        GIT_STASH_OPTIONS,
        GIT_STASH_LIST_OPTIONS,
        GIT_STASH_SHOW_OPTIONS,
        GIT_STASH_APPLY_OPTIONS,
        GIT_STASH_DROP_OPTIONS,
        GIT_STASH_PUSH_OPTIONS,
        GIT_REMOTE_OPTIONS,
        GIT_REMOTE_ADD_OPTIONS,
        GIT_REMOTE_SET_HEAD_OPTIONS,
        GIT_REMOTE_SET_BRANCHES_OPTIONS,
        GIT_REMOTE_GET_URL_OPTIONS,
        GIT_REMOTE_SET_URL_OPTIONS,
        GIT_REMOTE_SHOW_OPTIONS,
        GIT_REMOTE_PRUNE_OPTIONS,
        GIT_REMOTE_UPDATE_OPTIONS,
        CARGO_GLOBAL_OPTIONS,
        CARGO_BUILD_OPTIONS,
        CARGO_CHECK_OPTIONS,
        CARGO_RUN_OPTIONS,
        CARGO_TEST_OPTIONS,
        CARGO_BENCH_OPTIONS,
        CARGO_CLIPPY_OPTIONS,
        CARGO_FMT_OPTIONS,
        NPM_GLOBAL_OPTIONS,
        NPM_INSTALL_OPTIONS,
        NPM_RUN_OPTIONS,
        NPM_INIT_OPTIONS,
        NPM_CONFIG_OPTIONS,
        DOCKER_GLOBAL_OPTIONS,
        DOCKER_BUILD_OPTIONS,
        DOCKER_RUN_OPTIONS,
        DOCKER_PS_OPTIONS,
        DOCKER_EXEC_OPTIONS,
        DOCKER_IMAGES_OPTIONS,
        DOCKER_PULL_OPTIONS,
        DOCKER_PUSH_OPTIONS,
        DOCKER_COMMON_OPTIONS,
        DOCKER_COMPOSE_OPTIONS,
        DOCKER_COMPOSE_UP_OPTIONS,
        PWSH_OPTIONS,
        GH_GLOBAL_OPTIONS,
        GH_API_OPTIONS,
        GH_COMMON_OPTIONS,
        GH_LIST_OPTIONS,
        SET_LOCATION_OPTIONS,
        GET_CHILD_ITEM_OPTIONS,
    ] {
        append_option_descriptions(&mut descriptions, options);
    }

    descriptions.extend(
        [
            "git",
            "cargo",
            "npm",
            "docker",
            "pwsh",
            "gh",
            "Set-Location",
            "Get-ChildItem",
        ]
        .into_iter()
        .filter_map(describe_command),
    );
    descriptions
}

fn append_value_descriptions(descriptions: &mut Vec<&'static str>, values: &'static [ValueDef]) {
    descriptions.extend(values.iter().map(|value| value.description));
}

fn append_command_descriptions(
    descriptions: &mut Vec<&'static str>,
    commands: &'static [CommandDef],
) {
    descriptions.extend(commands.iter().map(|command| command.description));
}

fn append_option_descriptions(descriptions: &mut Vec<&'static str>, options: &'static [OptionDef]) {
    for option in options {
        descriptions.push(option.description);
        append_value_descriptions(descriptions, option.values);
    }
}

#[derive(Debug)]
struct Parsed<'a> {
    parts: Vec<&'static str>,
    spec: ContextDef,
    pending: Option<&'static OptionDef>,
    options_ended: bool,
    allow_subcommands: bool,
    _input: std::marker::PhantomData<&'a [String]>,
}

/// Complete a decoded argument prefix for a known command.
///
/// `preceding_args` contains complete tokens after the executable and before
/// the token currently being completed.  Unknown options are consumed as one
/// token only; their arity is deliberately unknown and is never guessed.
pub fn complete(command: &str, preceding_args: &[String], prefix: &str) -> Option<SpecResult> {
    let root = canonical_command(command)?;
    let parsed = parse(root, preceding_args);
    let context = parsed.parts.join(" ");
    let case_insensitive = option_names_case_insensitive(root);
    let mut candidates = Vec::new();

    if let Some(option) = parsed.pending {
        for value in option.values {
            if starts_with(value.name, prefix, case_insensitive)
                && !candidates
                    .iter()
                    .any(|candidate: &SpecCandidate| candidate.name.as_str() == value.name)
            {
                candidates.push(SpecCandidate {
                    name: value.name.to_owned(),
                    description: value.description,
                    key: format!("{context} {} {}", option.name, value.name),
                    kind: CandidateKind::Value,
                });
            }
        }
        return Some(SpecResult {
            candidates,
            context,
            path_values: option.value == ValueKind::Path,
            options_ended: parsed.options_ended,
        });
    }

    if !parsed.options_ended {
        if let Some(option) =
            find_inline_option_for(root, &parsed.parts, parsed.spec.options, prefix)
        {
            for value in option.values {
                let spelling = inline_value_spelling(option.name, value.name);
                if starts_with(&spelling, prefix, case_insensitive)
                    && !candidates.iter().any(|candidate: &SpecCandidate| {
                        candidate.name.as_str() == spelling.as_str()
                    })
                {
                    candidates.push(SpecCandidate {
                        name: spelling,
                        description: value.description,
                        key: format!("{context} {} {}", option.name, value.name),
                        kind: CandidateKind::Value,
                    });
                }
            }
            return Some(SpecResult {
                candidates,
                context,
                path_values: option.value == ValueKind::Path,
                options_ended: parsed.options_ended,
            });
        }
    }

    let options_requested =
        prefix.starts_with('-') || (prefix.is_empty() && root == "pwsh" && parsed.parts.len() == 1);
    if !parsed.options_ended && options_requested {
        for option in parsed.spec.options {
            if starts_with(option.name, prefix, case_insensitive)
                && !candidates
                    .iter()
                    .any(|candidate: &SpecCandidate| candidate.name.as_str() == option.name)
            {
                candidates.push(SpecCandidate {
                    name: option.name.to_owned(),
                    description: option.description,
                    key: format!("{context} {}", option.name),
                    kind: CandidateKind::Option,
                });
            }
        }
        if root == "cargo" && parsed.parts.len() > 1 {
            for option in CARGO_GLOBAL_OPTIONS {
                if starts_with(option.name, prefix, case_insensitive)
                    && !candidates
                        .iter()
                        .any(|candidate| candidate.name.as_str() == option.name)
                {
                    candidates.push(SpecCandidate {
                        name: option.name.to_owned(),
                        description: option.description,
                        key: format!("{context} {}", option.name),
                        kind: CandidateKind::Option,
                    });
                }
            }
        }
    } else if !parsed.options_ended && parsed.allow_subcommands && !prefix.starts_with('-') {
        for subcommand in parsed.spec.subcommands {
            if starts_with(subcommand.name, prefix, case_insensitive) {
                candidates.push(SpecCandidate {
                    name: subcommand.name.to_owned(),
                    description: subcommand.description,
                    key: format!("{context} {}", subcommand.name),
                    kind: CandidateKind::Subcommand,
                });
            }
        }
    }

    Some(SpecResult {
        candidates,
        context,
        path_values: parsed.spec.positional == PositionalKind::Path,
        options_ended: parsed.options_ended,
    })
}

fn parse<'a>(root: &'static str, args: &'a [String]) -> Parsed<'a> {
    let mut parts = vec![root];
    let mut spec = context_for(root, &parts);
    let mut pending = None;
    let mut options_ended = false;
    let mut allow_subcommands = !spec.subcommands.is_empty();
    let mut index = 0;

    while index < args.len() {
        let token = args[index].as_str();

        if let Some(option) = pending.take() {
            // This complete token is the value for a known option.  It can be
            // `--` legitimately (for example, a path named --), so consume it
            // before interpreting option termination.
            let _ = option;
            index += 1;
            continue;
        }

        if !options_ended && token == "--" {
            options_ended = true;
            allow_subcommands = false;
            index += 1;
            continue;
        }

        // Cargo accepts a toolchain selector immediately after `cargo` and
        // before the subcommand (for example, `cargo +nightly test`).  It is
        // a selector, not a positional argument or a command name.
        if root == "cargo"
            && parts.len() == 1
            && allow_subcommands
            && token.starts_with('+')
            && token.len() > 1
        {
            index += 1;
            continue;
        }

        if !options_ended && token.starts_with('-') && token != "-" {
            if let Some(option) = find_option_for(root, &parts, spec.options, token) {
                let case_insensitive = option_names_case_insensitive(root);
                let attached = has_inline_value(token, option.name, case_insensitive)
                    || short_option_with_attached_value(token, option.name, case_insensitive);
                if option.value != ValueKind::None && !option.optional_value && !attached {
                    pending = Some(option);
                }
            }
            // Unknown options consume no following token.  This is important
            // for forward compatibility and avoids inventing option arity.
            index += 1;
            continue;
        }

        if allow_subcommands {
            if let Some(subcommand) = find_subcommand(spec.subcommands, token) {
                parts.push(subcommand.name);
                spec = context_for(root, &parts);
                allow_subcommands = !spec.subcommands.is_empty();
                index += 1;
                continue;
            }
            // A positional token fixes the current command path.  Later
            // tokens cannot start a nested command path by accident.
            allow_subcommands = false;
        }
        index += 1;
    }

    Parsed {
        parts,
        spec,
        pending,
        options_ended,
        allow_subcommands,
        _input: std::marker::PhantomData,
    }
}

fn find_subcommand(commands: &'static [CommandDef], token: &str) -> Option<&'static CommandDef> {
    commands.iter().find(|candidate| candidate.name == token)
}

fn find_option_for(
    root: &'static str,
    parts: &[&'static str],
    options: &'static [OptionDef],
    token: &str,
) -> Option<&'static OptionDef> {
    find_option(options, token, option_names_case_insensitive(root)).or_else(|| {
        // Cargo accepts its common package/workspace flags after the
        // subcommand as well as before it.  Keep this fallback Cargo-only so
        // command-specific npm/docker/gh options do not leak across scopes.
        if root == "cargo" && parts.len() > 1 {
            find_option(CARGO_GLOBAL_OPTIONS, token, false)
        } else {
            None
        }
    })
}

fn find_inline_option_for(
    root: &'static str,
    parts: &[&'static str],
    options: &'static [OptionDef],
    prefix: &str,
) -> Option<&'static OptionDef> {
    let case_insensitive = option_names_case_insensitive(root);
    find_inline_option(options, prefix, case_insensitive).or_else(|| {
        if root == "cargo" && parts.len() > 1 {
            find_inline_option(CARGO_GLOBAL_OPTIONS, prefix, false)
        } else {
            None
        }
    })
}

fn find_inline_option(
    options: &'static [OptionDef],
    prefix: &str,
    case_insensitive: bool,
) -> Option<&'static OptionDef> {
    options.iter().find(|option| {
        option.value != ValueKind::None
            && (has_inline_value(prefix, option.name, case_insensitive)
                || short_option_with_attached_value(prefix, option.name, case_insensitive))
    })
}

fn inline_value_spelling(option_name: &str, value_name: &str) -> String {
    if option_name.starts_with("--") {
        format!("{option_name}={value_name}")
    } else {
        format!("{option_name}{value_name}")
    }
}

fn find_option(
    options: &'static [OptionDef],
    token: &str,
    case_insensitive: bool,
) -> Option<&'static OptionDef> {
    options.iter().find(|option| {
        token_matches_exact(token, option.name, case_insensitive)
            || (option.value != ValueKind::None
                && token_matches_option(token, option.name, case_insensitive))
    })
}

fn option_names_case_insensitive(root: &str) -> bool {
    matches!(root, "pwsh" | "Set-Location" | "Get-ChildItem")
}

fn token_matches_option(token: &str, name: &str, case_insensitive: bool) -> bool {
    token_matches_exact(token, name, case_insensitive)
        || has_inline_value(token, name, case_insensitive)
        || short_option_with_attached_value(token, name, case_insensitive)
}

fn token_matches_exact(token: &str, name: &str, case_insensitive: bool) -> bool {
    if case_insensitive {
        token.eq_ignore_ascii_case(name)
    } else {
        token == name
    }
}

fn has_inline_value(token: &str, name: &str, case_insensitive: bool) -> bool {
    token.get(..name.len()).is_some_and(|head| {
        if case_insensitive {
            head.eq_ignore_ascii_case(name)
        } else {
            head == name
        }
    }) && token
        .get(name.len()..)
        .is_some_and(|suffix| suffix.starts_with('='))
}

fn short_option_with_attached_value(token: &str, name: &str, case_insensitive: bool) -> bool {
    name.len() == 2
        && name.starts_with('-')
        && !name.starts_with("--")
        && token.len() > name.len()
        && token.get(..name.len()).is_some_and(|head| {
            if case_insensitive {
                head.eq_ignore_ascii_case(name)
            } else {
                head == name
            }
        })
}

fn starts_with(candidate: &str, prefix: &str, case_insensitive: bool) -> bool {
    candidate.get(..prefix.len()).is_some_and(|head| {
        if case_insensitive {
            head.eq_ignore_ascii_case(prefix)
        } else {
            head == prefix
        }
    })
}

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
    pub name: &'static str,
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
    }
}

const fn text_option(name: &'static str, description: &'static str) -> OptionDef {
    OptionDef {
        name,
        description,
        value: ValueKind::Text,
        values: EMPTY_VALUES,
    }
}

const fn path_option(name: &'static str, description: &'static str) -> OptionDef {
    OptionDef {
        name,
        description,
        value: ValueKind::Path,
        values: EMPTY_VALUES,
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
        description: "Always use color",
    },
    ValueDef {
        name: "auto",
        description: "Use color when appropriate",
    },
    ValueDef {
        name: "never",
        description: "Disable color",
    },
];

const GIT_DATE_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "relative",
        description: "Show relative dates",
    },
    ValueDef {
        name: "local",
        description: "Show local dates",
    },
    ValueDef {
        name: "iso",
        description: "Show ISO dates",
    },
    ValueDef {
        name: "iso-strict",
        description: "Show strict ISO dates",
    },
    ValueDef {
        name: "rfc",
        description: "Show RFC dates",
    },
    ValueDef {
        name: "short",
        description: "Show short dates",
    },
    ValueDef {
        name: "raw",
        description: "Show raw timestamps",
    },
];

const GIT_DIFF_FILTER_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "A",
        description: "Added paths",
    },
    ValueDef {
        name: "C",
        description: "Copied paths",
    },
    ValueDef {
        name: "D",
        description: "Deleted paths",
    },
    ValueDef {
        name: "M",
        description: "Modified paths",
    },
    ValueDef {
        name: "R",
        description: "Renamed paths",
    },
    ValueDef {
        name: "T",
        description: "Changed file types",
    },
    ValueDef {
        name: "U",
        description: "Unmerged paths",
    },
    ValueDef {
        name: "X",
        description: "Unknown paths",
    },
    ValueDef {
        name: "B",
        description: "Broken pairing",
    },
];

const CARGO_MESSAGE_FORMAT_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "human",
        description: "Human-readable compiler messages",
    },
    ValueDef {
        name: "json",
        description: "JSON compiler messages",
    },
    ValueDef {
        name: "short",
        description: "Short compiler messages",
    },
    ValueDef {
        name: "json-diagnostic-short",
        description: "Compact JSON diagnostics",
    },
    ValueDef {
        name: "json-diagnostic-rendered-ansi",
        description: "ANSI-rendered JSON diagnostics",
    },
];

const CARGO_TIMING_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "html",
        description: "Write an HTML timing report",
    },
    ValueDef {
        name: "json",
        description: "Write a JSON timing report",
    },
];

const NPM_LOGLEVEL_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "silent",
        description: "Show no output",
    },
    ValueDef {
        name: "error",
        description: "Show errors only",
    },
    ValueDef {
        name: "warn",
        description: "Show warnings and errors",
    },
    ValueDef {
        name: "notice",
        description: "Show notices and warnings",
    },
    ValueDef {
        name: "http",
        description: "Show HTTP activity",
    },
    ValueDef {
        name: "info",
        description: "Show informational output",
    },
    ValueDef {
        name: "verbose",
        description: "Show detailed output",
    },
    ValueDef {
        name: "silly",
        description: "Show all debug output",
    },
];

const DOCKER_PULL_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "always",
        description: "Always pull the image",
    },
    ValueDef {
        name: "missing",
        description: "Pull only when the image is missing",
    },
    ValueDef {
        name: "never",
        description: "Never pull the image",
    },
];

const PWSH_INPUT_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "Text",
        description: "Read text input",
    },
    ValueDef {
        name: "XML",
        description: "Read XML input",
    },
];

const PWSH_OUTPUT_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "Text",
        description: "Write text output",
    },
    ValueDef {
        name: "XML",
        description: "Write XML output",
    },
];

const PWSH_WINDOW_VALUES: &[ValueDef] = &[
    ValueDef {
        name: "Hidden",
        description: "Start with a hidden window",
    },
    ValueDef {
        name: "Minimized",
        description: "Start with a minimized window",
    },
    ValueDef {
        name: "Maximized",
        description: "Start with a maximized window",
    },
    ValueDef {
        name: "Normal",
        description: "Start with a normal window",
    },
];

const GIT_COMMANDS: &[CommandDef] = &[
    command("add", "Add files to the index"),
    command("am", "Apply patches from a mailbox"),
    command("archive", "Create an archive of files"),
    command("bisect", "Find the commit that introduced a bug"),
    command("branch", "List, create, or delete branches"),
    command("checkout", "Switch branches or restore paths"),
    command("cherry-pick", "Apply changes from existing commits"),
    command("clean", "Remove untracked files"),
    command("clone", "Clone a repository"),
    command("commit", "Record changes in the repository"),
    command("config", "Read and write repository options"),
    command("diff", "Show changes between commits or files"),
    command("fetch", "Download objects and refs from another repository"),
    command("format-patch", "Prepare patches for email submission"),
    command("grep", "Print lines matching a pattern"),
    command("init", "Create an empty repository"),
    command("log", "Show commit history"),
    command("merge", "Join development histories"),
    command("mv", "Move or rename a file, directory, or symlink"),
    command("pull", "Fetch and integrate another repository"),
    command("push", "Update remote refs and objects"),
    command("rebase", "Reapply commits on top of another base"),
    command("reflog", "Manage reference logs"),
    command("remote", "Manage tracked repositories"),
    command("rename", "Rename a repository or ref"),
    command("reset", "Reset the current HEAD"),
    command("restore", "Restore working tree files"),
    command("revert", "Create a commit that reverses changes"),
    command("rm", "Remove files from the working tree and index"),
    command("show", "Show one or more objects"),
    command("sparse-checkout", "Manage the sparse-checkout file"),
    command("stash", "Temporarily store unfinished changes"),
    command("status", "Show the working tree status"),
    command("switch", "Switch branches"),
    command("tag", "Create, list, or delete tags"),
    command("worktree", "Manage multiple working trees"),
];

const GIT_GLOBAL_OPTIONS: &[OptionDef] = &[
    option("--help", "Show Git help"),
    option("--version", "Show the Git version"),
    path_option("--exec-path", "Set the Git executable path"),
    path_option("--git-dir", "Set the repository directory"),
    path_option("--work-tree", "Set the working tree directory"),
    option("--bare", "Treat the repository as bare"),
    text_option(
        "--config-env",
        "Read a configuration value from an environment variable",
    ),
    path_option("-C", "Run as if Git started in this directory"),
    text_option("-c", "Set a configuration variable for this invocation"),
    text_option("--namespace", "Use a separate Git namespace"),
    path_option("--super-prefix", "Prefix paths for recursive Git commands"),
    option("-p", "Pipe output through a pager"),
    choice_option(
        "--paginate",
        "Pipe output through a pager",
        &[
            ValueDef {
                name: "auto",
                description: "Use a pager when appropriate",
            },
            ValueDef {
                name: "always",
                description: "Always use a pager",
            },
            ValueDef {
                name: "never",
                description: "Disable the pager",
            },
        ],
    ),
    option("--no-pager", "Do not pipe output through a pager"),
    option("--no-replace-objects", "Do not replace Git objects"),
    option("--no-lazy-fetch", "Do not lazily fetch missing objects"),
    option("--no-optional-locks", "Avoid optional locks"),
    option("--literal-pathspecs", "Treat pathspecs literally"),
    option("--glob-pathspecs", "Use glob magic for pathspecs"),
    option("--noglob-pathspecs", "Disable glob magic for pathspecs"),
    option(
        "--icase-pathspecs",
        "Make pathspec matching case-insensitive",
    ),
    text_option("--list-cmds", "List command groups"),
];

const GIT_LOG_OPTIONS: &[OptionDef] = &[
    option("--follow", "Follow history beyond renames"),
    option("--no-decorate", "Hide ref decorations"),
    option("--decorate", "Show ref decorations"),
    text_option("--decorate-refs", "Decorate matching refs"),
    text_option(
        "--decorate-refs-exclude",
        "Exclude matching refs from decoration",
    ),
    option("--source", "Show the ref that led to each commit"),
    option("--use-mailmap", "Use the mailmap file for names"),
    path_option("--mailmap", "Use a specific mailmap file"),
    option("--full-diff", "Show full diffs for each commit"),
    option("--log-size", "Show the size of each commit message"),
    option("--notes", "Show notes attached to commits"),
    option("--no-notes", "Do not show commit notes"),
    text_option("--show-notes", "Show notes from a ref"),
    option("--standard-notes", "Show standard notes"),
    option("--show-signature", "Show signed commit signatures"),
    option("--relative-date", "Show dates relative to the current time"),
    choice_option("--date", "Choose the date format", GIT_DATE_VALUES),
    text_option("--pretty", "Format commit messages"),
    text_option("--format", "Format commit messages"),
    option("--abbrev-commit", "Show abbreviated commit IDs"),
    option("--oneline", "Show each commit on one line"),
    option("--no-abbrev-commit", "Show full commit IDs"),
    option("--full-history", "Do not simplify history"),
    option(
        "--dense",
        "Show only selected commits and meaningful merges",
    ),
    option("--sparse", "Show all commits in the simplified history"),
    option("--simplify-merges", "Simplify history by rewriting merges"),
    option("--simplify-by-decoration", "Show only decorated commits"),
    option("--all", "Show all refs"),
    text_option("--branches", "Show branches matching a pattern"),
    text_option("--tags", "Show tags matching a pattern"),
    text_option("--remotes", "Show remote-tracking refs"),
    text_option("--glob", "Show refs matching a glob"),
    text_option("--exclude", "Exclude refs matching a pattern"),
    option("--reflog", "Show reflog entries"),
    option("--alternate-refs", "Use alternate refs for decoration"),
    option("--single-worktree", "Inspect only one worktree"),
    option("--ignore-missing", "Ignore missing objects"),
    option("--bisect", "Show the bisection history"),
    option("--stdin", "Read revisions from standard input"),
    text_option("--max-count", "Limit the number of commits"),
    text_option("-n", "Limit the number of commits"),
    text_option("--skip", "Skip commits before showing results"),
    text_option("--since", "Show commits after a date"),
    text_option("--after", "Show commits after a date"),
    text_option("--until", "Show commits before a date"),
    text_option("--before", "Show commits before a date"),
    text_option("--author", "Limit commits by author"),
    text_option("--committer", "Limit commits by committer"),
    text_option("--grep", "Limit commits by message pattern"),
    option("--invert-grep", "Exclude matching commit messages"),
    option("--all-match", "Require all message patterns to match"),
    option("--basic-regexp", "Use basic regular expressions"),
    option("--extended-regexp", "Use extended regular expressions"),
    option("--fixed-strings", "Treat patterns as fixed strings"),
    option("-F", "Treat patterns as fixed strings"),
    option("-E", "Use extended regular expressions"),
    option("-G", "Use basic regular expressions"),
    option("-i", "Ignore case in patterns"),
    option("--regexp-ignore-case", "Ignore case in patterns"),
    option("--textconv", "Allow text conversion filters"),
    choice_option(
        "--ignore-submodules",
        "Choose submodule change handling",
        &[
            ValueDef {
                name: "none",
                description: "Show all submodule changes",
            },
            ValueDef {
                name: "untracked",
                description: "Ignore untracked submodule files",
            },
            ValueDef {
                name: "dirty",
                description: "Ignore dirty submodule worktrees",
            },
            ValueDef {
                name: "all",
                description: "Ignore all submodule changes",
            },
        ],
    ),
    text_option("--submodule", "Choose submodule diff format"),
    option("--pickaxe-all", "Show all changesets for a pickaxe match"),
    text_option("-S", "Find changes that add or remove a string"),
    text_option(
        "--pickaxe-regex",
        "Treat pickaxe strings as regular expressions",
    ),
    text_option("-O", "Control diff file order with a file"),
    text_option("--diff-merges", "Choose merge diff presentation"),
    option("--no-diff-merges", "Hide merge diffs"),
    option("--cc", "Show combined merge diffs"),
    option("--combined-all-paths", "Show paths from all parents"),
    option("--first-parent", "Follow only the first parent"),
    option(
        "--exclude-first-parent-only",
        "Exclude first-parent history",
    ),
    option("--merges", "Show merge commits"),
    option("--no-merges", "Hide merge commits"),
    text_option("--min-parents", "Require a minimum number of parents"),
    text_option("--max-parents", "Limit the number of parents"),
    option("--remove-empty", "Stop when a path disappears"),
    option("--mergetag", "Show embedded signed merge tags"),
    option("--boundary", "Show excluded boundary commits"),
    option("--graph", "Draw the commit graph"),
    option(
        "--show-linear-break",
        "Show breaks between linear histories",
    ),
    text_option("--diff-algorithm", "Choose the diff algorithm"),
    choice_option(
        "--diff-filter",
        "Choose changed path statuses",
        GIT_DIFF_FILTER_VALUES,
    ),
    text_option("--anchored", "Use anchored diff matching"),
    text_option("--word-diff", "Show word-level differences"),
    text_option("--color-words", "Show changed words with color"),
    option("--no-renames", "Disable rename detection"),
    option("--rename-empty", "Allow empty files as rename sources"),
    option("--parents", "Show parent commits"),
    option("--children", "Show child commits"),
    option("--timestamp", "Show raw commit timestamps"),
    option(
        "--left-right",
        "Mark commits on each side of a symmetric difference",
    ),
    option("--cherry-mark", "Mark equivalent commits"),
    option("--cherry-pick", "Omit equivalent commits"),
    option("--left-only", "Show commits reachable only from the left"),
    option("--right-only", "Show commits reachable only from the right"),
    option("--merge", "Show commits touching merged files"),
    path_option("--output", "Write output to a file"),
    path_option("-o", "Write output to a file"),
];

const GIT_SHOW_OPTIONS: &[OptionDef] = &[
    option("--pretty", "Format commit messages"),
    option("--format", "Format commit messages"),
    option("--oneline", "Show each commit on one line"),
    option("--abbrev-commit", "Show abbreviated commit IDs"),
    option("--no-abbrev-commit", "Show full commit IDs"),
    text_option("--abbrev", "Set the object name abbreviation length"),
    option("--full-index", "Show full object names in diff headers"),
    option("--binary", "Output binary changes"),
    option("--patch", "Show patch text"),
    option("-p", "Show patch text"),
    option("--no-patch", "Suppress patch output"),
    option("--raw", "Show raw diff format"),
    option("--patch-with-raw", "Show patches with raw headers"),
    option("--stat", "Show a diffstat"),
    option("--numstat", "Show numeric diffstat"),
    option("--shortstat", "Show only the final diffstat line"),
    text_option("--dirstat", "Show directory-level diff statistics"),
    option("--dirstat-by-file", "Show directory statistics by file"),
    option("--summary", "Show condensed change summaries"),
    option("--name-only", "Show changed names only"),
    option("--name-status", "Show changed names and statuses"),
    choice_option("--color", "Choose colored output", COLOR_VALUES),
    text_option("--ws-error-highlight", "Highlight whitespace errors"),
    text_option("--full-diff", "Show full diffs for each commit"),
    text_option("--diff-merges", "Choose merge diff presentation"),
    option("--no-diff-merges", "Hide merge diffs"),
    option("--cc", "Show combined merge diffs"),
    option("--combined-all-paths", "Show paths from all parents"),
    option("--no-renames", "Disable rename detection"),
    option("--find-renames", "Detect renames"),
    text_option("-M", "Detect renames with a similarity threshold"),
    option("--find-copies", "Detect copies"),
    text_option("-C", "Detect copies with a similarity threshold"),
    option("--find-copies-harder", "Search harder for copies"),
    text_option("--diff-algorithm", "Choose the diff algorithm"),
    text_option("--word-diff", "Show word-level differences"),
    text_option("--color-words", "Show changed words with color"),
    text_option(
        "--ignore-space-change",
        "Ignore changes in whitespace amount",
    ),
    option("-w", "Ignore all whitespace changes"),
    option("--ignore-all-space", "Ignore all whitespace changes"),
    option("--ignore-blank-lines", "Ignore blank-line changes"),
    option("--indent-heuristic", "Use the indent heuristic"),
    option("--no-indent-heuristic", "Disable the indent heuristic"),
    text_option("--inter-hunk-context", "Show context between diff hunks"),
    path_option("--output", "Write output to a file"),
    path_option("-o", "Write output to a file"),
];

const GIT_DIFF_OPTIONS: &[OptionDef] = &[
    option("--cached", "Compare the index with HEAD"),
    option("--staged", "Compare the index with HEAD"),
    option("--merge-base", "Compare with the merge base"),
    option("--no-index", "Compare two paths outside a repository"),
    option("--exit-code", "Use status to report differences"),
    option("--quiet", "Suppress all output"),
    option("--no-ext-diff", "Disallow external diff helpers"),
    option("--text", "Treat all files as text"),
    option("--ignore-submodules", "Ignore submodule changes"),
    text_option("--submodule", "Choose submodule diff format"),
    option("--no-renames", "Disable rename detection"),
    option("--find-renames", "Detect renames"),
    text_option("-M", "Detect renames with a similarity threshold"),
    option("--find-copies", "Detect copies"),
    text_option("-C", "Detect copies with a similarity threshold"),
    option("--find-copies-harder", "Search harder for copies"),
    option("--irreversible-delete", "Omit deleted file contents"),
    option("--relative", "Show paths relative to the current directory"),
    text_option("--src-prefix", "Use a custom source prefix"),
    text_option("--dst-prefix", "Use a custom destination prefix"),
    text_option("--line-prefix", "Prefix every output line"),
    choice_option(
        "--diff-filter",
        "Choose changed path statuses",
        GIT_DIFF_FILTER_VALUES,
    ),
    option("--no-prefix", "Omit source and destination prefixes"),
    option("--default-prefix", "Use the default diff prefixes"),
    text_option("--inter-hunk-context", "Show context between diff hunks"),
    option("--output-indicator-new", "Choose the added-line indicator"),
    option(
        "--output-indicator-old",
        "Choose the removed-line indicator",
    ),
    option(
        "--output-indicator-context",
        "Choose the context-line indicator",
    ),
    option("--full-index", "Show full object names"),
    option("--binary", "Output binary changes"),
    option("--abbrev", "Use abbreviated object names"),
    option("--patch", "Show patch text"),
    option("-p", "Show patch text"),
    option("--no-patch", "Suppress patch output"),
    option("--raw", "Show raw diff format"),
    option("--patch-with-raw", "Show patches with raw headers"),
    option("--patch-with-stat", "Show patches with a diffstat"),
    option("--stat", "Show a diffstat"),
    option("--compact-summary", "Show a compact diff summary"),
    option("--numstat", "Show numeric diffstat"),
    option("--shortstat", "Show only the final diffstat line"),
    text_option("--dirstat", "Show directory-level diff statistics"),
    option("--dirstat-by-file", "Show directory statistics by file"),
    option("--cumulative", "Accumulate directory statistics"),
    option("--dirstat-by-file", "Show directory statistics by file"),
    option("--summary", "Show condensed change summaries"),
    option("--name-only", "Show changed names only"),
    option("--name-status", "Show changed names and statuses"),
    choice_option("--color", "Choose colored output", COLOR_VALUES),
    option("--no-color", "Disable colored output"),
    text_option("--color-moved", "Choose moved-line coloring"),
    text_option(
        "--color-moved-ws",
        "Choose whitespace handling for moved lines",
    ),
    option("--no-color-moved", "Disable moved-line coloring"),
    text_option("--word-diff", "Show word-level differences"),
    text_option("--word-diff-regex", "Choose the word boundary expression"),
    option("--color-words", "Show changed words with color"),
    text_option("--minimal", "Spend extra effort to find a smaller diff"),
    option("--patience", "Use the patience diff algorithm"),
    option("--histogram", "Use the histogram diff algorithm"),
    option("--anchored", "Use anchored diff matching"),
    text_option("--diff-algorithm", "Choose the diff algorithm"),
    option("--indent-heuristic", "Use the indent heuristic"),
    option("--no-indent-heuristic", "Disable the indent heuristic"),
    text_option("--anchored", "Use anchored diff matching"),
    option("--ignore-space-at-eol", "Ignore end-of-line whitespace"),
    option(
        "--ignore-space-change",
        "Ignore changes in whitespace amount",
    ),
    option("-b", "Ignore changes in the amount of whitespace"),
    option("--ignore-all-space", "Ignore all whitespace changes"),
    option("-w", "Ignore all whitespace changes"),
    option("--ignore-blank-lines", "Ignore blank-line changes"),
    option("-I", "Ignore matching changed lines"),
    text_option("-O", "Control diff file order with a file"),
    text_option("--skip-to", "Start output at a path"),
    text_option("--rotate-to", "Show a path first"),
    path_option("--output", "Write output to a file"),
    path_option("-o", "Write output to a file"),
];

const GIT_STATUS_OPTIONS: &[OptionDef] = &[
    option("--short", "Use short status format"),
    option("-s", "Use short status format"),
    option("--branch", "Show branch information"),
    option("--show-stash", "Show the number of stashed entries"),
    choice_option(
        "--porcelain",
        "Use a stable machine-readable format",
        &[
            ValueDef {
                name: "v1",
                description: "Use porcelain version 1",
            },
            ValueDef {
                name: "v2",
                description: "Use porcelain version 2",
            },
        ],
    ),
    option("--long", "Use the default long format"),
    option("--null", "Terminate entries with NUL"),
    option("-z", "Terminate entries with NUL"),
    option("--ahead-behind", "Compute ahead and behind counts"),
    option("--no-ahead-behind", "Skip ahead and behind counts"),
    option("--renames", "Detect renames"),
    option("--no-renames", "Do not detect renames"),
    option("--find-renames", "Detect renames"),
    choice_option(
        "--untracked-files",
        "Choose untracked file reporting",
        &[
            ValueDef {
                name: "no",
                description: "Hide untracked files",
            },
            ValueDef {
                name: "normal",
                description: "Show untracked files",
            },
            ValueDef {
                name: "all",
                description: "Show every untracked file",
            },
        ],
    ),
    option("-u", "Show untracked files"),
    choice_option(
        "--ignore-submodules",
        "Choose submodule change handling",
        &[
            ValueDef {
                name: "none",
                description: "Show all submodule changes",
            },
            ValueDef {
                name: "untracked",
                description: "Ignore untracked submodule files",
            },
            ValueDef {
                name: "dirty",
                description: "Ignore dirty submodule worktrees",
            },
            ValueDef {
                name: "all",
                description: "Ignore all submodule changes",
            },
        ],
    ),
    option("--ignored", "Show ignored files"),
    choice_option(
        "--column",
        "Choose column display",
        &[
            ValueDef {
                name: "always",
                description: "Always use columns",
            },
            ValueDef {
                name: "never",
                description: "Never use columns",
            },
            ValueDef {
                name: "auto",
                description: "Use columns when appropriate",
            },
        ],
    ),
    option("--no-column", "Disable columns"),
    option("--verbose", "Show additional information"),
    option("--optional-locks", "Take optional locks"),
    option("--no-optional-locks", "Avoid optional locks"),
    text_option("--ignored", "Choose ignored-file reporting"),
];

const GIT_ADD_OPTIONS: &[OptionDef] = &[
    option("--verbose", "Show files as they are added"),
    option("-n", "Dry-run without changing the index"),
    option("--dry-run", "Dry-run without changing the index"),
    option("-p", "Interactively choose hunks"),
    option("--patch", "Interactively choose hunks"),
    option("-e", "Edit the generated patch"),
    option("--edit", "Edit the generated patch"),
    option("-f", "Allow adding ignored files"),
    option("--force", "Allow adding ignored files"),
    option("-u", "Update tracked files"),
    option("--update", "Update tracked files"),
    option("-N", "Record intent to add untracked files"),
    option("--intent-to-add", "Record intent to add untracked files"),
    option("-A", "Add all files"),
    option("--all", "Add all files"),
    option("--no-all", "Do not add files outside the working tree"),
    option("--ignore-removal", "Keep removed files out of the index"),
    option("--refresh", "Refresh the index"),
    option("--ignore-errors", "Continue after add errors"),
    option("--ignore-missing", "Ignore missing files"),
    option("--renormalize", "Apply clean filters to tracked files"),
    option("--sparse", "Allow updating paths outside the sparse cone"),
    text_option("--chmod", "Set the executable bit in the index"),
    path_option("--pathspec-from-file", "Read pathspecs from a file"),
    option("--pathspec-file-nul", "Separate pathspecs with NUL"),
];

const GIT_COMMIT_OPTIONS: &[OptionDef] = &[
    option("-v", "Show the diff in the commit message template"),
    option("--verbose", "Show the diff in the commit message template"),
    option("-q", "Suppress summary output"),
    option("--quiet", "Suppress summary output"),
    path_option("-F", "Read the commit message from a file"),
    path_option("--file", "Read the commit message from a file"),
    text_option("--author", "Override the author identity"),
    text_option("--date", "Override the author date"),
    text_option("-m", "Use the given commit message"),
    text_option("--message", "Use the given commit message"),
    text_option("-c", "Reuse and edit a commit message"),
    text_option("--reedit-message", "Reuse and edit a commit message"),
    text_option("-C", "Reuse a commit message without editing"),
    text_option("--reuse-message", "Reuse a commit message without editing"),
    text_option("--fixup", "Create a fixup commit"),
    text_option("--squash", "Create a squash commit"),
    option("--reset-author", "Use the committer as the author"),
    option("--allow-empty", "Allow an empty commit"),
    option("--allow-empty-message", "Allow an empty commit message"),
    option("--no-verify", "Skip commit hooks"),
    option("--verify", "Run commit hooks"),
    option("-s", "Add a Signed-off-by trailer"),
    option("--signoff", "Add a Signed-off-by trailer"),
    option("--no-post-rewrite", "Skip the post-rewrite hook"),
    option("--amend", "Replace the tip commit"),
    option("--no-edit", "Use the selected commit message"),
    option("--status", "Include status in the commit template"),
    option("--no-status", "Do not include status in the template"),
    text_option("-S", "Sign the commit with a key"),
    text_option("--gpg-sign", "Sign the commit with a key"),
    option("--no-gpg-sign", "Do not sign the commit"),
    path_option("--template", "Use a commit message template"),
    text_option("--trailer", "Add a trailer to the commit message"),
    option("--dry-run", "Show what would be committed"),
    option("--porcelain", "Use a machine-readable output"),
    option("--long", "Use the long status format"),
    option("--short", "Use the short status format"),
    option("--branch", "Show branch information"),
    option("--ahead-behind", "Compute ahead and behind counts"),
    option("--no-ahead-behind", "Skip ahead and behind counts"),
];

const GIT_BRANCH_OPTIONS: &[OptionDef] = &[
    option("-v", "Show branch commit information"),
    option("--verbose", "Show branch commit information"),
    option("-q", "Suppress non-error output"),
    option("--quiet", "Suppress non-error output"),
    option("-t", "Set upstream tracking"),
    option("--track", "Set upstream tracking"),
    text_option("-u", "Set the upstream branch"),
    text_option("--set-upstream-to", "Set the upstream branch"),
    option("--unset-upstream", "Remove upstream tracking"),
    choice_option("--color", "Choose branch colors", COLOR_VALUES),
    option("-r", "List remote-tracking branches"),
    option("--remotes", "List remote-tracking branches"),
    option("-a", "List local and remote branches"),
    option("--all", "List local and remote branches"),
    option("-l", "List branches"),
    option("--list", "List branches"),
    option("--show-current", "Show the current branch"),
    option("--create-reflog", "Create a reflog for a branch"),
    option("--edit-description", "Edit a branch description"),
    option("-d", "Delete a merged branch"),
    option("--delete", "Delete a merged branch"),
    option("-D", "Force-delete a branch"),
    option("-m", "Rename a branch"),
    option("--move", "Rename a branch"),
    option("-M", "Force-rename a branch"),
    option("-c", "Copy a branch"),
    option("--copy", "Copy a branch"),
    option("-C", "Force-copy a branch"),
    text_option("--contains", "List branches containing a commit"),
    text_option("--no-contains", "List branches without a commit"),
    text_option("--merged", "List branches merged into a commit"),
    text_option("--no-merged", "List branches not merged into a commit"),
    text_option("--sort", "Sort branches by a field"),
    text_option("--points-at", "List branches pointing at an object"),
    text_option("--format", "Format branch output"),
    choice_option(
        "--column",
        "Choose column display",
        &[
            ValueDef {
                name: "always",
                description: "Always use columns",
            },
            ValueDef {
                name: "never",
                description: "Never use columns",
            },
            ValueDef {
                name: "auto",
                description: "Use columns when appropriate",
            },
        ],
    ),
    option("--no-column", "Disable columns"),
    option("--omit-empty", "Omit empty formatted lines"),
    option("--recurse-submodules", "Create branches in submodules"),
];

const GIT_CHECKOUT_OPTIONS: &[OptionDef] = &[
    option("-q", "Suppress progress reporting"),
    option("--quiet", "Suppress progress reporting"),
    option("--progress", "Force progress reporting"),
    option("--no-progress", "Disable progress reporting"),
    option("-f", "Discard local changes"),
    option("--force", "Discard local changes"),
    option("-m", "Attempt a three-way merge"),
    option("--merge", "Attempt a three-way merge"),
    option("--detach", "Detach HEAD at a commit"),
    text_option("--orphan", "Create a new orphan branch"),
    text_option("-b", "Create and switch to a branch"),
    text_option("-B", "Create or reset and switch to a branch"),
    option("--track", "Set upstream tracking"),
    option("--guess", "Guess a remote branch"),
    option("--no-guess", "Do not guess a remote branch"),
    option("--overlay", "Overlay restored files"),
    option("--no-overlay", "Remove files absent from the target"),
    option(
        "--ignore-other-worktrees",
        "Allow a branch checked out elsewhere",
    ),
    option("--recurse-submodules", "Update submodules"),
    option("--no-recurse-submodules", "Do not update submodules"),
    path_option("--pathspec-from-file", "Read pathspecs from a file"),
    option("--pathspec-file-nul", "Separate pathspecs with NUL"),
];

const GIT_SWITCH_OPTIONS: &[OptionDef] = &[
    option("-q", "Suppress progress reporting"),
    option("--quiet", "Suppress progress reporting"),
    option("--progress", "Force progress reporting"),
    option("--no-progress", "Disable progress reporting"),
    option("-f", "Discard local changes"),
    option("--force", "Discard local changes"),
    option("-d", "Detach HEAD at a commit"),
    option("--detach", "Detach HEAD at a commit"),
    text_option("-c", "Create and switch to a branch"),
    text_option("--create", "Create and switch to a branch"),
    text_option("-C", "Force-create and switch to a branch"),
    text_option("--force-create", "Force-create and switch to a branch"),
    option("--guess", "Guess a remote branch"),
    option("--no-guess", "Do not guess a remote branch"),
    text_option("--orphan", "Create a new orphan branch"),
    option("--discard-changes", "Discard local changes"),
    option("--merge", "Perform a three-way merge"),
    text_option("--conflict", "Choose conflict style"),
    option("--recurse-submodules", "Update submodules"),
    option("--no-recurse-submodules", "Do not update submodules"),
];

const GIT_FETCH_OPTIONS: &[OptionDef] = &[
    option("--all", "Fetch all remotes"),
    option("--append", "Append ref names to FETCH_HEAD"),
    option("--atomic", "Use an atomic update"),
    text_option("--depth", "Limit the fetch depth"),
    text_option("--deepen", "Deepen a shallow repository"),
    text_option("--shallow-since", "Deepen history after a date"),
    text_option("--shallow-exclude", "Deepen history excluding a ref"),
    option("--unshallow", "Convert a shallow repository to complete"),
    option("--update-shallow", "Accept updates to shallow boundaries"),
    text_option("--negotiation-tip", "Use a commit as a negotiation tip"),
    option(
        "--negotiate-only",
        "Report common ancestors without fetching",
    ),
    option("--dry-run", "Show what would be fetched"),
    option("--porcelain", "Use machine-readable output"),
    option("-f", "Force-update local refs"),
    option("--force", "Force-update local refs"),
    option("--keep", "Keep downloaded pack"),
    option("-m", "Fetch from multiple remotes"),
    option("--multiple", "Fetch from multiple remotes"),
    option("--auto-maintenance", "Run maintenance after fetching"),
    option("--auto-gc", "Run automatic garbage collection"),
    option("--write-fetch-head", "Write FETCH_HEAD"),
    option("--no-write-fetch-head", "Do not write FETCH_HEAD"),
    option("--prefetch", "Fetch into the prefetch namespace"),
    option("-p", "Prune deleted remote-tracking refs"),
    option("--prune", "Prune deleted remote-tracking refs"),
    option("--prune-tags", "Prune remote tags"),
    option("--no-tags", "Do not fetch tags"),
    option("--tags", "Fetch all tags"),
    option("--recurse-submodules", "Fetch submodules"),
    text_option("--jobs", "Set parallel submodule jobs"),
    text_option("--submodule-prefix", "Prefix submodule paths"),
    text_option(
        "--recurse-submodules-default",
        "Set the default submodule policy",
    ),
    option("-u", "Set upstream for fetched branches"),
    option("--set-upstream", "Set upstream for fetched branches"),
    text_option("--upload-pack", "Choose the upload-pack program"),
    option("-q", "Suppress fetch output"),
    option("--quiet", "Suppress fetch output"),
    option("-v", "Show detailed fetch output"),
    option("--verbose", "Show detailed fetch output"),
    option("--progress", "Force progress reporting"),
    text_option("--server-option", "Send an option to the server"),
    option("--show-forced-updates", "Report forced updates"),
    option("--no-show-forced-updates", "Do not report forced updates"),
    option("--ipv4", "Use IPv4 only"),
    option("--ipv6", "Use IPv6 only"),
];

const GIT_PULL_OPTIONS: &[OptionDef] = &[
    option("--all", "Fetch all remotes"),
    option("--append", "Append ref names to FETCH_HEAD"),
    option("--commit", "Commit after merging"),
    option("--no-commit", "Do not commit after merging"),
    option("-e", "Edit the merge message"),
    option("--edit", "Edit the merge message"),
    option("--no-edit", "Use the default merge message"),
    text_option("--cleanup", "Clean up the merge message"),
    choice_option(
        "--ff",
        "Choose fast-forward behavior",
        &[
            ValueDef {
                name: "only",
                description: "Allow only fast-forward updates",
            },
            ValueDef {
                name: "false",
                description: "Always create a merge commit",
            },
        ],
    ),
    option("--ff-only", "Allow only fast-forward updates"),
    option("--no-ff", "Always create a merge commit"),
    option("--verify-signatures", "Verify the tip commit signature"),
    option("-v", "Show detailed output"),
    option("--verbose", "Show detailed output"),
    option("-q", "Suppress output"),
    option("--quiet", "Suppress output"),
    option("--progress", "Force progress reporting"),
    option("--no-progress", "Disable progress reporting"),
    option("--recurse-submodules", "Fetch submodules"),
    option("--no-recurse-submodules", "Do not fetch submodules"),
    option("--tags", "Fetch all tags"),
    option("--no-tags", "Do not fetch tags"),
    option("-p", "Prune deleted remote-tracking refs"),
    option("--prune", "Prune deleted remote-tracking refs"),
    option("-u", "Set upstream for fetched branches"),
    option("--set-upstream", "Set upstream for fetched branches"),
    choice_option(
        "--rebase",
        "Choose rebase behavior",
        &[
            ValueDef {
                name: "false",
                description: "Merge fetched changes",
            },
            ValueDef {
                name: "true",
                description: "Rebase local commits",
            },
            ValueDef {
                name: "merges",
                description: "Rebase while preserving merges",
            },
            ValueDef {
                name: "interactive",
                description: "Run an interactive rebase",
            },
        ],
    ),
    option("--no-rebase", "Merge fetched changes"),
    option("--autostash", "Stash local changes before updating"),
    option("--no-autostash", "Do not stash local changes"),
    option("--allow-unrelated-histories", "Allow unrelated histories"),
    option("--stat", "Show a diffstat"),
    option("--no-stat", "Suppress the diffstat"),
    option("--log", "Include shortlog entries"),
    option("--no-log", "Do not include shortlog entries"),
    option("--squash", "Do not make a merge commit"),
    text_option("--strategy", "Choose the merge strategy"),
    text_option("-s", "Choose the merge strategy"),
    text_option("-X", "Pass an option to the merge strategy"),
    text_option("--strategy-option", "Pass an option to the merge strategy"),
    text_option("-S", "Sign the merge commit"),
    text_option("--gpg-sign", "Sign the merge commit"),
    option("--signoff", "Add a Signed-off-by trailer"),
    option("--no-signoff", "Do not add a Signed-off-by trailer"),
    option("--verify", "Run hooks"),
    option("--no-verify", "Skip hooks"),
    text_option("--depth", "Limit the fetch depth"),
    text_option("--shallow-since", "Deepen history after a date"),
    text_option("--shallow-exclude", "Deepen history excluding a ref"),
    option("--unshallow", "Convert a shallow repository to complete"),
    option("--update-shallow", "Accept updates to shallow boundaries"),
    text_option("--negotiation-tip", "Use a commit as a negotiation tip"),
];

const GIT_PUSH_OPTIONS: &[OptionDef] = &[
    option("-v", "Show detailed push output"),
    option("--verbose", "Show detailed push output"),
    option("-q", "Suppress push output"),
    option("--quiet", "Suppress push output"),
    option("--progress", "Force progress reporting"),
    option("--no-progress", "Disable progress reporting"),
    option("-n", "Show what would be pushed"),
    option("--dry-run", "Show what would be pushed"),
    option("--porcelain", "Use machine-readable output"),
    option("--delete", "Delete remote refs"),
    option("--all", "Push all branches"),
    option("--prune", "Delete remote refs absent locally"),
    option("--mirror", "Mirror all refs"),
    option("--tags", "Push all tags"),
    option("--follow-tags", "Push annotated tags"),
    choice_option(
        "--signed",
        "Choose push signing behavior",
        &[
            ValueDef {
                name: "true",
                description: "Require a signed push",
            },
            ValueDef {
                name: "false",
                description: "Do not sign the push",
            },
            ValueDef {
                name: "if-asked",
                description: "Sign when the server asks",
            },
        ],
    ),
    option("--atomic", "Use an atomic update"),
    text_option("-o", "Send a push option to the server"),
    text_option("--push-option", "Send a push option to the server"),
    text_option("--receive-pack", "Choose the receive-pack program"),
    text_option("--exec", "Choose the receive-pack program"),
    option("-u", "Set upstream for pushed branches"),
    option("--set-upstream", "Set upstream for pushed branches"),
    option("-f", "Force-update remote refs"),
    option("--force", "Force-update remote refs"),
    text_option("--force-with-lease", "Force-update with a lease"),
    option(
        "--force-if-includes",
        "Require fetched history before forcing",
    ),
    choice_option(
        "--recurse-submodules",
        "Choose submodule push behavior",
        &[
            ValueDef {
                name: "check",
                description: "Reject missing submodule commits",
            },
            ValueDef {
                name: "on-demand",
                description: "Push needed submodule commits",
            },
            ValueDef {
                name: "no",
                description: "Do not recurse into submodules",
            },
        ],
    ),
    option("--thin", "Use thin packs"),
    option("--no-thin", "Use complete packs"),
    text_option("--repo", "Choose the repository to push"),
    option("--ipv4", "Use IPv4 only"),
    option("--ipv6", "Use IPv6 only"),
    option("--no-verify", "Skip pre-push hooks"),
];

const GIT_STASH_COMMANDS: &[CommandDef] = &[
    command("list", "List stash entries"),
    command("show", "Show changes recorded in a stash"),
    command("drop", "Remove one stash entry"),
    command("pop", "Apply and remove a stash entry"),
    command("apply", "Apply a stash entry"),
    command("branch", "Create a branch from a stash entry"),
    command("push", "Save local changes in a new stash"),
    command("save", "Save local changes in a new stash"),
    command("clear", "Remove all stash entries"),
    command("create", "Create a stash object without storing it"),
    command("store", "Store a stash object"),
];

const GIT_STASH_OPTIONS: &[OptionDef] = &[
    option("-q", "Suppress stash output"),
    option("--quiet", "Suppress stash output"),
    option("--no-quiet", "Show stash output"),
    option("-p", "Interactively select hunks"),
    option("--patch", "Interactively select hunks"),
    option("-u", "Include untracked files"),
    option("--include-untracked", "Include untracked files"),
    option("-a", "Include ignored files"),
    option("--all", "Include ignored files"),
    text_option("-m", "Set the stash message"),
    text_option("--message", "Set the stash message"),
    option("-k", "Keep changes staged"),
    option("--keep-index", "Keep changes staged"),
    option("--no-keep-index", "Unstage changes in the stash"),
    option("--index", "Try to reinstate the index state"),
    option("--no-apply", "Do not apply the stash"),
    option("--only-untracked", "Stash only untracked files"),
    option("--staged", "Stash only staged changes"),
    path_option("--pathspec-from-file", "Read pathspecs from a file"),
    option("--pathspec-file-nul", "Separate pathspecs with NUL"),
];

const GIT_STASH_LIST_OPTIONS: &[OptionDef] = &[
    option("--stat", "Show a diffstat"),
    option("--patch", "Show patch text"),
    option("-p", "Show patch text"),
    option("--oneline", "Show each stash on one line"),
    text_option("--format", "Format stash output"),
    choice_option("--date", "Choose the date format", GIT_DATE_VALUES),
];

const GIT_STASH_SHOW_OPTIONS: &[OptionDef] = &[
    option("--patch", "Show patch text"),
    option("-p", "Show patch text"),
    option("--stat", "Show a diffstat"),
    option("--name-only", "Show changed names only"),
    option("--name-status", "Show changed names and statuses"),
    choice_option("--color", "Choose colored output", COLOR_VALUES),
    option("--no-color", "Disable colored output"),
    option("--include-untracked", "Include untracked files"),
    option("--only-untracked", "Show only untracked files"),
];

const GIT_STASH_APPLY_OPTIONS: &[OptionDef] = &[
    option("--index", "Try to reinstate the index state"),
    option("--quiet", "Suppress stash output"),
    option("--reindex", "Rebuild the index before applying"),
];

const GIT_STASH_DROP_OPTIONS: &[OptionDef] = &[option("--quiet", "Suppress stash output")];

const GIT_STASH_PUSH_OPTIONS: &[OptionDef] = &[
    option("-q", "Suppress stash output"),
    option("--quiet", "Suppress stash output"),
    option("-p", "Interactively select hunks"),
    option("--patch", "Interactively select hunks"),
    option("-u", "Include untracked files"),
    option("--include-untracked", "Include untracked files"),
    option("-a", "Include ignored files"),
    option("--all", "Include ignored files"),
    text_option("-m", "Set the stash message"),
    text_option("--message", "Set the stash message"),
    option("-k", "Keep changes staged"),
    option("--keep-index", "Keep changes staged"),
    option("--no-keep-index", "Unstage changes in the stash"),
    option("--no-apply", "Do not apply the stash"),
    option("--only-untracked", "Stash only untracked files"),
    option("--staged", "Stash only staged changes"),
    path_option("--pathspec-from-file", "Read pathspecs from a file"),
    option("--pathspec-file-nul", "Separate pathspecs with NUL"),
];

const GIT_REMOTE_COMMANDS: &[CommandDef] = &[
    command("add", "Add a remote repository"),
    command("rename", "Rename a remote"),
    command("remove", "Remove a remote"),
    command("rm", "Remove a remote"),
    command("set-head", "Set or delete the default branch"),
    command("set-branches", "Change tracked branches"),
    command("get-url", "Retrieve remote URLs"),
    command("set-url", "Change remote URLs"),
    command("show", "Show information about remotes"),
    command("prune", "Delete stale remote-tracking branches"),
    command("update", "Update remotes"),
];

const GIT_REMOTE_OPTIONS: &[OptionDef] = &[
    option("-v", "Show remote URLs"),
    option("--verbose", "Show remote URLs"),
    option("--no-verbose", "Hide remote URLs"),
];

const GIT_REMOTE_ADD_OPTIONS: &[OptionDef] = &[
    option("-f", "Fetch after adding the remote"),
    option("--fetch", "Fetch after adding the remote"),
    option("--tags", "Import tags from the remote"),
    option("--no-tags", "Do not import tags"),
    text_option("-t", "Track selected branches"),
    text_option("--track", "Track selected branches"),
    text_option("-m", "Set the remote default branch"),
    choice_option(
        "--mirror",
        "Configure mirror behavior",
        &[
            ValueDef {
                name: "fetch",
                description: "Mirror refs on fetch",
            },
            ValueDef {
                name: "push",
                description: "Mirror refs on push",
            },
        ],
    ),
];

const GIT_REMOTE_SET_HEAD_OPTIONS: &[OptionDef] = &[
    option("-a", "Choose the default branch automatically"),
    option("--auto", "Choose the default branch automatically"),
    option("-d", "Delete the default branch"),
    option("--delete", "Delete the default branch"),
];

const GIT_REMOTE_SET_BRANCHES_OPTIONS: &[OptionDef] = &[
    option("-a", "Add branches to the tracked list"),
    option("--add", "Add branches to the tracked list"),
];

const GIT_REMOTE_GET_URL_OPTIONS: &[OptionDef] = &[
    option("--push", "Show the push URL"),
    option("--all", "Show all URLs"),
];

const GIT_REMOTE_SET_URL_OPTIONS: &[OptionDef] = &[
    option("--push", "Change the push URL"),
    option("--add", "Add another URL"),
    option("--delete", "Delete a URL"),
];

const GIT_REMOTE_SHOW_OPTIONS: &[OptionDef] = &[
    option("-n", "Skip remote-head queries"),
    option("--no-query", "Skip remote-head queries"),
    option("-v", "Show detailed remote information"),
    option("--verbose", "Show detailed remote information"),
];

const GIT_REMOTE_PRUNE_OPTIONS: &[OptionDef] = &[option("--dry-run", "Show what would be deleted")];

const GIT_REMOTE_UPDATE_OPTIONS: &[OptionDef] = &[
    option("--prune", "Prune stale refs"),
    option("--prune-tags", "Prune stale tags"),
    text_option("--upload-pack", "Choose the upload-pack program"),
];

const CARGO_COMMANDS: &[CommandDef] = &[
    command("add", "Add dependencies to a manifest"),
    command("bench", "Run benchmarks"),
    command("build", "Compile a package"),
    command("check", "Check a package without producing binaries"),
    command("clean", "Remove generated artifacts"),
    command("clippy", "Run Clippy lints"),
    command("doc", "Build package documentation"),
    command("fetch", "Download dependencies"),
    command("fix", "Automatically fix compiler warnings"),
    command("fmt", "Format Rust code"),
    command("generate-lockfile", "Generate a Cargo.lock file"),
    command("install", "Install a Rust binary"),
    command("metadata", "Output package metadata"),
    command("new", "Create a new package"),
    command("publish", "Publish a package to a registry"),
    command("remove", "Remove dependencies from a manifest"),
    command("report", "Display Cargo reports"),
    command("run", "Run a binary or example"),
    command("rustc", "Compile a package with rustc options"),
    command("rustdoc", "Build documentation with rustdoc options"),
    command("search", "Search a registry for packages"),
    command("test", "Run tests"),
    command("tree", "Display a dependency tree"),
    command("uninstall", "Uninstall a Rust binary"),
    command("update", "Update dependencies"),
    command("vendor", "Vendor all dependencies locally"),
    command("version", "Show the Cargo version"),
    command("locate-project", "Locate a Cargo project manifest"),
];

const CARGO_GLOBAL_OPTIONS: &[OptionDef] = &[
    option("--help", "Show Cargo help"),
    option("--version", "Show the Cargo version"),
    option("-v", "Use verbose output"),
    option("--verbose", "Use verbose output"),
    option("-q", "Suppress Cargo output"),
    option("--quiet", "Suppress Cargo output"),
    choice_option("--color", "Choose colored output", COLOR_VALUES),
    option("--locked", "Require Cargo.lock to be unchanged"),
    option("--offline", "Run without accessing the network"),
    option("--frozen", "Use locked and offline modes"),
    path_option("--manifest-path", "Use a specific Cargo.toml file"),
    text_option("--package", "Select a package"),
    option("--workspace", "Operate on the entire workspace"),
    text_option("--exclude", "Exclude a workspace package"),
    text_option("--features", "Enable package features"),
    option("--all-features", "Enable all package features"),
    option("--no-default-features", "Disable default features"),
    text_option("--target", "Choose a compilation target"),
    text_option("--jobs", "Set the number of parallel jobs"),
    text_option("-j", "Set the number of parallel jobs"),
    text_option("--config", "Override a Cargo configuration value"),
    option(
        "--future-incompat-report",
        "Display a future-incompatibility report",
    ),
    text_option("-Z", "Use an unstable Cargo option"),
];

const CARGO_BUILD_OPTIONS: &[OptionDef] = &[
    option("--lib", "Build the library target"),
    text_option("--bin", "Build a binary target"),
    text_option("--example", "Build an example target"),
    text_option("--test", "Build an integration test target"),
    text_option("--bench", "Build a benchmark target"),
    option("--all-targets", "Build all targets"),
    option("--release", "Build with the release profile"),
    text_option("--profile", "Choose a Cargo profile"),
    path_option("--target-dir", "Choose the build output directory"),
    text_option("--artifact-dir", "Choose the artifact output directory"),
    choice_option(
        "--timings",
        "Write compilation timing reports",
        CARGO_TIMING_VALUES,
    ),
    option("--unit-graph", "Output the unit dependency graph"),
    option("--build-plan", "Output an experimental build plan"),
    option("--keep-going", "Continue building independent packages"),
];

const CARGO_CHECK_OPTIONS: &[OptionDef] = &[
    option("--lib", "Check the library target"),
    text_option("--bin", "Check a binary target"),
    text_option("--example", "Check an example target"),
    text_option("--test", "Check an integration test target"),
    text_option("--bench", "Check a benchmark target"),
    option("--all-targets", "Check all targets"),
    option("--release", "Check with the release profile"),
    text_option("--profile", "Choose a Cargo profile"),
    path_option("--target-dir", "Choose the build output directory"),
    choice_option(
        "--timings",
        "Write compilation timing reports",
        CARGO_TIMING_VALUES,
    ),
    option("--unit-graph", "Output the unit dependency graph"),
    option("--keep-going", "Continue checking independent packages"),
];

const CARGO_RUN_OPTIONS: &[OptionDef] = &[
    option("--lib", "Run the library target"),
    option("--release", "Run with the release profile"),
    text_option("--profile", "Choose a Cargo profile"),
    text_option("--bin", "Choose a binary target"),
    text_option("--example", "Choose an example target"),
    path_option("--target-dir", "Choose the build output directory"),
    choice_option(
        "--message-format",
        "Choose compiler message format",
        CARGO_MESSAGE_FORMAT_VALUES,
    ),
    option("--unit-graph", "Output the unit dependency graph"),
    option("--keep-going", "Continue running independent packages"),
];

const CARGO_TEST_OPTIONS: &[OptionDef] = &[
    option("--no-run", "Compile tests without running them"),
    option("--no-fail-fast", "Run all tests after a failure"),
    option("--doc", "Test documentation"),
    option("--lib", "Test the library target"),
    text_option("--bin", "Test a binary target"),
    text_option("--example", "Test an example target"),
    text_option("--test", "Test an integration target"),
    text_option("--bench", "Test a benchmark target"),
    option("--all-targets", "Test all targets"),
    option("--release", "Test with the release profile"),
    text_option("--profile", "Choose a Cargo profile"),
    path_option("--target-dir", "Choose the build output directory"),
    choice_option(
        "--message-format",
        "Choose compiler message format",
        CARGO_MESSAGE_FORMAT_VALUES,
    ),
    option("--keep-going", "Continue running independent packages"),
];

const CARGO_BENCH_OPTIONS: &[OptionDef] = &[
    option("--lib", "Benchmark the library target"),
    text_option("--bin", "Benchmark a binary target"),
    text_option("--example", "Benchmark an example target"),
    text_option("--bench", "Choose a benchmark target"),
    option("--no-run", "Compile benchmarks without running them"),
    option("--no-fail-fast", "Run all benchmarks after a failure"),
    option("--all-targets", "Benchmark all targets"),
    option("--release", "Benchmark with the release profile"),
    text_option("--profile", "Choose a Cargo profile"),
    path_option("--target-dir", "Choose the build output directory"),
    choice_option(
        "--message-format",
        "Choose compiler message format",
        CARGO_MESSAGE_FORMAT_VALUES,
    ),
    option("--keep-going", "Continue running independent packages"),
];

const CARGO_CLIPPY_OPTIONS: &[OptionDef] = &[
    option("--all-targets", "Lint all targets"),
    option("--lib", "Lint the library target"),
    text_option("--bin", "Lint a binary target"),
    text_option("--example", "Lint an example target"),
    text_option("--test", "Lint an integration test target"),
    text_option("--bench", "Lint a benchmark target"),
    option("--fix", "Automatically apply Clippy suggestions"),
    option("--allow-dirty", "Allow fixes in a dirty working tree"),
    option("--allow-staged", "Allow fixes with staged changes"),
    option("--no-deps", "Skip dependency packages"),
    option("--release", "Lint with the release profile"),
    text_option("--profile", "Choose a Cargo profile"),
    path_option("--target-dir", "Choose the build output directory"),
    choice_option(
        "--message-format",
        "Choose compiler message format",
        CARGO_MESSAGE_FORMAT_VALUES,
    ),
    option("--keep-going", "Continue linting independent packages"),
];

const CARGO_FMT_OPTIONS: &[OptionDef] = &[
    option("--check", "Check formatting without changing files"),
    option("--all", "Format all workspace packages"),
    option("-v", "Use verbose output"),
    option("--verbose", "Use verbose output"),
    option("-q", "Suppress output"),
    option("--quiet", "Suppress output"),
    choice_option(
        "--message-format",
        "Choose formatter message format",
        &[
            ValueDef {
                name: "short",
                description: "Use short formatter messages",
            },
            ValueDef {
                name: "json",
                description: "Use JSON formatter messages",
            },
        ],
    ),
    text_option("--emit", "Choose formatter output mode"),
    text_option("--edition", "Choose the Rust edition"),
    path_option("--manifest-path", "Use a specific Cargo.toml file"),
];

const NPM_COMMANDS: &[CommandDef] = &[
    command("access", "Manage package access settings"),
    command("audit", "Run a security audit"),
    command("bugs", "Open a package bug tracker"),
    command("cache", "Manage the npm cache"),
    command("ci", "Install from a lockfile"),
    command("completion", "Enable shell completion"),
    command("config", "Manage npm configuration"),
    command("dedupe", "Reduce duplicate dependencies"),
    command("deprecate", "Deprecate a package version"),
    command("diff", "Compare package versions"),
    command("dist-tag", "Manage distribution tags"),
    command("doctor", "Check the npm environment"),
    command("docs", "Open package documentation"),
    command("exec", "Run a command from a package"),
    command("explain", "Explain an installed package"),
    command("explore", "Open a package directory"),
    command("fund", "Show funding information"),
    command("help", "Show npm help"),
    command("hook", "Manage registry hooks"),
    command("init", "Create a package.json file"),
    command("install", "Install package dependencies"),
    command("i", "Install package dependencies"),
    command("install-ci-test", "Install dependencies and run CI tests"),
    command("install-test", "Install dependencies and run tests"),
    command("link", "Symlink a package"),
    command("ll", "List installed packages"),
    command("login", "Log in to a registry"),
    command("logout", "Log out of a registry"),
    command("ls", "List installed packages"),
    command("org", "Manage organizations"),
    command("outdated", "Check for outdated packages"),
    command("owner", "Manage package owners"),
    command("pack", "Create a package archive"),
    command("ping", "Ping a registry"),
    command("pkg", "Manage package metadata"),
    command("prefix", "Display the npm prefix"),
    command("profile", "Manage the npm profile"),
    command("prune", "Remove extraneous packages"),
    command("publish", "Publish a package"),
    command("query", "Query installed packages"),
    command("rebuild", "Rebuild packages"),
    command("repo", "Open a package repository"),
    command("restart", "Run the restart script"),
    command("root", "Display the npm root"),
    command("run", "Run a package script"),
    command("run-script", "Run a package script"),
    command("search", "Search the registry"),
    command("set", "Set a configuration value"),
    command("shrinkwrap", "Create or update a shrinkwrap"),
    command("start", "Run the start script"),
    command("stop", "Run the stop script"),
    command("team", "Manage teams"),
    command("test", "Run the test script"),
    command("token", "Manage authentication tokens"),
    command("uninstall", "Remove package dependencies"),
    command("remove", "Remove package dependencies"),
    command("unpublish", "Remove a package from the registry"),
    command("update", "Update package dependencies"),
    command("version", "Bump the package version"),
    command("view", "View package metadata"),
];

const NPM_GLOBAL_OPTIONS: &[OptionDef] = &[
    option("--help", "Show npm help"),
    option("--version", "Show the npm version"),
    option("-g", "Operate globally"),
    option("--global", "Operate globally"),
    option("--save", "Save a dependency"),
    option("--save-dev", "Save a development dependency"),
    option("--save-exact", "Save an exact version"),
    path_option("--prefix", "Use a different npm prefix"),
    text_option("--workspace", "Target a workspace"),
    option("--workspaces", "Target all workspaces"),
    option("--include-workspace-root", "Include the workspace root"),
    option("--ignore-scripts", "Skip package scripts"),
    option("--production", "Omit development dependencies"),
    option("--omit", "Omit dependency types"),
    option("--include", "Include dependency types"),
    option("--json", "Use JSON output"),
    option("--silent", "Suppress output"),
    choice_option(
        "--loglevel",
        "Choose logging verbosity",
        NPM_LOGLEVEL_VALUES,
    ),
    choice_option("--level", "Choose logging verbosity", NPM_LOGLEVEL_VALUES),
    text_option("--registry", "Use a package registry"),
    path_option("--userconfig", "Use a different user configuration file"),
    path_option("--cache", "Use a different npm cache"),
    option("--yes", "Answer yes to prompts"),
    option("--force", "Force npm operations"),
    option("--offline", "Use only cached packages"),
    option("--prefer-offline", "Prefer cached packages"),
    option("--prefer-online", "Prefer fresh package metadata"),
    option("--no-audit", "Skip the security audit"),
    option("--no-fund", "Skip funding information"),
    option("--no-update-notifier", "Disable update notices"),
    option("--foreground-scripts", "Run scripts in the foreground"),
    option("--no-bin-links", "Do not create binary links"),
    option("--ignore-optional", "Skip optional dependencies"),
    choice_option("--color", "Choose colored output", COLOR_VALUES),
];

const NPM_INSTALL_OPTIONS: &[OptionDef] = &[
    option("--save", "Save a dependency"),
    option("--no-save", "Do not save a dependency"),
    option("--save-dev", "Save a development dependency"),
    option("--save-optional", "Save an optional dependency"),
    option("--save-peer", "Save a peer dependency"),
    option("--save-exact", "Save an exact version"),
    option("--global", "Install globally"),
    option("-g", "Install globally"),
    option("--dry-run", "Show what would be installed"),
    option("--package-lock", "Update the package lock"),
    option("--no-package-lock", "Skip the package lock"),
    option("--ignore-scripts", "Skip package scripts"),
    option("--foreground-scripts", "Run scripts in the foreground"),
    option("--audit", "Run a security audit"),
    option("--no-audit", "Skip the security audit"),
    option("--fund", "Show funding information"),
    option("--no-fund", "Skip funding information"),
    option("--legacy-peer-deps", "Ignore peer dependency conflicts"),
    option("--strict-peer-deps", "Fail on peer dependency conflicts"),
    option("--engine-strict", "Reject incompatible engines"),
    option("--force", "Force dependency resolution"),
    option("--install-links", "Install file dependencies as links"),
    text_option(
        "--install-strategy",
        "Choose dependency installation strategy",
    ),
    text_option("--omit", "Omit dependency types"),
    text_option("--include", "Include dependency types"),
    text_option("--workspace", "Target a workspace"),
    option("--workspaces", "Target all workspaces"),
    option("--include-workspace-root", "Include the workspace root"),
    path_option("--prefix", "Use a different npm prefix"),
];

const NPM_RUN_OPTIONS: &[OptionDef] = &[
    text_option("--workspace", "Target a workspace"),
    option("--workspaces", "Target all workspaces"),
    option("--include-workspace-root", "Include the workspace root"),
    option("--if-present", "Ignore missing scripts"),
    option("--ignore-scripts", "Skip package scripts"),
    option("--foreground-scripts", "Run scripts in the foreground"),
    path_option("--script-shell", "Choose the script shell"),
];

const NPM_INIT_OPTIONS: &[OptionDef] = &[
    option("--yes", "Accept all defaults"),
    option("--force", "Overwrite an existing package file"),
    text_option("--scope", "Set the package scope"),
    option("--private", "Mark the package private"),
    text_option("--workspace", "Create a workspace package"),
];

const NPM_CONFIG_COMMANDS: &[CommandDef] = &[
    command("get", "Get a configuration value"),
    command("set", "Set a configuration value"),
    command("delete", "Delete a configuration value"),
    command("list", "List configuration values"),
    command("fix", "Fix configuration problems"),
];

const NPM_CONFIG_OPTIONS: &[OptionDef] = &[
    option("--global", "Use the global configuration"),
    option("--location", "Choose the configuration location"),
    option("--json", "Use JSON output"),
    option("--long", "Show long configuration details"),
    option("--parseable", "Use parseable output"),
];

const DOCKER_COMMANDS: &[CommandDef] = &[
    command("build", "Build an image from a Dockerfile"),
    command("builder", "Manage builds"),
    command("buildx", "Build with BuildKit"),
    command("checkpoint", "Manage checkpoints"),
    command("commit", "Create an image from a container"),
    command("compose", "Define and run multi-container applications"),
    command("config", "Manage Swarm configs"),
    command("container", "Manage containers"),
    command("context", "Manage Docker contexts"),
    command("cp", "Copy files between a container and the host"),
    command("create", "Create a new container"),
    command("diff", "Inspect changes to a container filesystem"),
    command("events", "Get real-time events from the server"),
    command("exec", "Run a command in a running container"),
    command("export", "Export a container filesystem"),
    command("history", "Show an image history"),
    command("image", "Manage images"),
    command("images", "List images"),
    command("info", "Display system information"),
    command("init", "Create files for a containerized project"),
    command("inspect", "Return low-level information"),
    command("kill", "Kill running containers"),
    command("load", "Load an image from an archive"),
    command("login", "Log in to a registry"),
    command("logout", "Log out of a registry"),
    command("logs", "Fetch container logs"),
    command("manifest", "Manage image manifests"),
    command("network", "Manage networks"),
    command("node", "Manage Swarm nodes"),
    command("pause", "Pause all processes in containers"),
    command("plugin", "Manage plugins"),
    command("port", "List port mappings"),
    command("ps", "List containers"),
    command("pull", "Pull an image or repository"),
    command("push", "Push an image or repository"),
    command("rename", "Rename a container"),
    command("restart", "Restart containers"),
    command("rm", "Remove containers"),
    command("rmi", "Remove images"),
    command("run", "Run a command in a new container"),
    command("save", "Save images to an archive"),
    command("search", "Search Docker Hub"),
    command("secret", "Manage Swarm secrets"),
    command("service", "Manage Swarm services"),
    command("stack", "Manage Swarm stacks"),
    command("start", "Start stopped containers"),
    command("stats", "Display container resource usage"),
    command("stop", "Stop running containers"),
    command("swarm", "Manage Swarm"),
    command("system", "Manage Docker data"),
    command("tag", "Create a tag for an image"),
    command("top", "Display running processes"),
    command("trust", "Manage image trust"),
    command("unpause", "Unpause processes in containers"),
    command("update", "Update container configuration"),
    command("version", "Show Docker version information"),
    command("volume", "Manage volumes"),
    command("wait", "Wait for containers to stop"),
];

const DOCKER_GLOBAL_OPTIONS: &[OptionDef] = &[
    option("--help", "Show Docker help"),
    option("--version", "Show the Docker version"),
    path_option("--config", "Use a client configuration directory"),
    text_option("--context", "Use a Docker context"),
    option("-D", "Enable debug mode"),
    option("--debug", "Enable debug mode"),
    text_option("-H", "Connect to a Docker daemon"),
    text_option("--host", "Connect to a Docker daemon"),
    choice_option(
        "--log-level",
        "Choose log verbosity",
        &[
            ValueDef {
                name: "debug",
                description: "Show debug logs",
            },
            ValueDef {
                name: "info",
                description: "Show informational logs",
            },
            ValueDef {
                name: "warn",
                description: "Show warnings",
            },
            ValueDef {
                name: "error",
                description: "Show errors",
            },
            ValueDef {
                name: "fatal",
                description: "Show fatal errors",
            },
        ],
    ),
    option("--tls", "Use TLS"),
    path_option("--tlscacert", "Use a TLS CA certificate"),
    path_option("--tlscert", "Use a TLS client certificate"),
    path_option("--tlskey", "Use a TLS client key"),
    option("--tlsverify", "Verify the TLS daemon"),
];

const DOCKER_BUILD_OPTIONS: &[OptionDef] = &[
    path_option("-f", "Use a Dockerfile"),
    path_option("--file", "Use a Dockerfile"),
    text_option("-t", "Tag the built image"),
    text_option("--tag", "Tag the built image"),
    text_option("--build-arg", "Set a build-time variable"),
    text_option("--label", "Set an image label"),
    option("--no-cache", "Do not use the build cache"),
    choice_option("--pull", "Choose base image pulling", DOCKER_PULL_VALUES),
    option("-q", "Suppress build output"),
    option("--quiet", "Suppress build output"),
    option("--rm", "Remove intermediate containers"),
    option("--force-rm", "Always remove intermediate containers"),
    text_option("--memory", "Set build memory limit"),
    text_option("--shm-size", "Set shared memory size"),
    text_option("--network", "Choose the build network"),
    text_option("--platform", "Choose the target platform"),
    text_option("--progress", "Choose build progress output"),
    text_option("--secret", "Expose a build secret"),
    text_option("--ssh", "Expose an SSH agent"),
    text_option("--target", "Choose a build stage"),
    text_option("--cache-from", "Use external build cache"),
    text_option("--cache-to", "Export build cache"),
    text_option("--build-context", "Add a named build context"),
    path_option("-o", "Export build output"),
    text_option("--output", "Export build output"),
    option("--provenance", "Set provenance attestation mode"),
    option("--sbom", "Set SBOM attestation mode"),
    path_option("--metadata-file", "Write build metadata to a file"),
];

const DOCKER_RUN_OPTIONS: &[OptionDef] = &[
    option("-d", "Run in detached mode"),
    option("--detach", "Run in detached mode"),
    option("-i", "Keep standard input open"),
    option("--interactive", "Keep standard input open"),
    option("-t", "Allocate a pseudo-terminal"),
    option("--tty", "Allocate a pseudo-terminal"),
    option("--rm", "Remove the container after exit"),
    text_option("--name", "Assign a container name"),
    text_option("-h", "Set the container hostname"),
    text_option("--hostname", "Set the container hostname"),
    text_option("-e", "Set an environment variable"),
    text_option("--env", "Set an environment variable"),
    path_option("--env-file", "Read environment variables from a file"),
    text_option("-p", "Publish a container port"),
    option("-P", "Publish all exposed ports"),
    option("--publish-all", "Publish all exposed ports"),
    text_option("-v", "Bind-mount a volume"),
    text_option("--volume", "Bind-mount a volume"),
    text_option("--mount", "Attach a filesystem mount"),
    text_option("-w", "Set the working directory"),
    text_option("--workdir", "Set the working directory"),
    text_option("-u", "Set the user or UID"),
    text_option("--user", "Set the user or UID"),
    text_option("--network", "Connect to a network"),
    text_option("--network-alias", "Add a network alias"),
    text_option("--restart", "Choose a restart policy"),
    text_option("--entrypoint", "Override the image entrypoint"),
    text_option("-l", "Set a container label"),
    text_option("--label", "Set a container label"),
    text_option("--add-host", "Add a host-to-IP mapping"),
    text_option("--cap-add", "Add a Linux capability"),
    text_option("--cap-drop", "Drop a Linux capability"),
    text_option("--device", "Add a host device"),
    text_option("--group-add", "Add a supplementary group"),
    option("--init", "Use an init process"),
    text_option("--ip", "Set an IPv4 address"),
    text_option("--ip6", "Set an IPv6 address"),
    text_option("--mac-address", "Set a MAC address"),
    text_option("-m", "Set a memory limit"),
    text_option("--memory", "Set a memory limit"),
    text_option("--cpus", "Set CPU quota"),
    text_option("--cpu-shares", "Set CPU share weight"),
    text_option("--cpuset-cpus", "Set CPUs to use"),
    text_option("--pids-limit", "Limit process count"),
    option("--privileged", "Give extended privileges"),
    option("--read-only", "Mount the root filesystem read-only"),
    text_option("--security-opt", "Set a security option"),
    text_option("--shm-size", "Set shared memory size"),
    text_option("--sysctl", "Set a kernel parameter"),
    text_option("--ulimit", "Set a ulimit"),
    text_option("--runtime", "Choose the container runtime"),
    text_option("--platform", "Choose the target platform"),
    choice_option("--pull", "Choose image pulling", DOCKER_PULL_VALUES),
    option("-q", "Suppress pull output"),
    option("--quiet", "Suppress pull output"),
    option("--sig-proxy", "Proxy signals to the process"),
    text_option("--stop-signal", "Set the stop signal"),
    text_option("--stop-timeout", "Set the stop timeout"),
    text_option("--tmpfs", "Mount a temporary filesystem"),
    text_option("--userns", "Choose the user namespace"),
    text_option("--uts", "Choose the UTS namespace"),
    text_option("--pid", "Choose the PID namespace"),
    text_option("--ipc", "Choose the IPC namespace"),
    text_option("--health-cmd", "Set a healthcheck command"),
    text_option("--health-interval", "Set a healthcheck interval"),
    text_option("--health-timeout", "Set a healthcheck timeout"),
    text_option("--health-retries", "Set healthcheck retries"),
    text_option("--health-start-period", "Set healthcheck start period"),
    option("--no-healthcheck", "Disable the image healthcheck"),
];

const DOCKER_PS_OPTIONS: &[OptionDef] = &[
    option("-a", "Show all containers"),
    option("--all", "Show all containers"),
    text_option("-f", "Filter the container list"),
    text_option("--filter", "Filter the container list"),
    text_option("--format", "Format container output"),
    text_option("-n", "Show the latest n containers"),
    text_option("--last", "Show the latest n containers"),
    option("-l", "Show the latest container"),
    option("--latest", "Show the latest container"),
    option("--no-trunc", "Do not truncate output"),
    option("-q", "Show IDs only"),
    option("--quiet", "Show IDs only"),
    option("-s", "Show total file sizes"),
    option("--size", "Show total file sizes"),
];

const DOCKER_EXEC_OPTIONS: &[OptionDef] = &[
    option("-d", "Run detached"),
    option("--detach", "Run detached"),
    text_option("--detach-keys", "Set the detach key sequence"),
    text_option("-e", "Set an environment variable"),
    text_option("--env", "Set an environment variable"),
    path_option("--env-file", "Read environment variables from a file"),
    option("-i", "Keep standard input open"),
    option("--interactive", "Keep standard input open"),
    option("--privileged", "Give extended privileges"),
    option("-t", "Allocate a pseudo-terminal"),
    option("--tty", "Allocate a pseudo-terminal"),
    text_option("-u", "Set the user or UID"),
    text_option("--user", "Set the user or UID"),
    text_option("-w", "Set the working directory"),
    text_option("--workdir", "Set the working directory"),
];

const DOCKER_IMAGES_OPTIONS: &[OptionDef] = &[
    option("-a", "Show intermediate images"),
    option("--all", "Show intermediate images"),
    option("--digests", "Show image digests"),
    text_option("-f", "Filter the image list"),
    text_option("--filter", "Filter the image list"),
    text_option("--format", "Format image output"),
    option("--no-trunc", "Do not truncate output"),
    option("-q", "Show IDs only"),
    option("--quiet", "Show IDs only"),
];

const DOCKER_PULL_OPTIONS: &[OptionDef] = &[
    option("-a", "Download all tagged images"),
    option("--all-tags", "Download all tagged images"),
    text_option("--platform", "Choose a target platform"),
    option("-q", "Suppress pull output"),
    option("--quiet", "Suppress pull output"),
    option("--disable-content-trust", "Skip image signing"),
];

const DOCKER_PUSH_OPTIONS: &[OptionDef] = &[
    option("-a", "Push all tagged images"),
    option("--all-tags", "Push all tagged images"),
    option("-q", "Suppress push output"),
    option("--quiet", "Suppress push output"),
    option("--disable-content-trust", "Skip image signing"),
];

const DOCKER_COMMON_OPTIONS: &[OptionDef] = &[option("--help", "Show command help")];

const DOCKER_COMPOSE_COMMANDS: &[CommandDef] = &[
    command("build", "Build or rebuild services"),
    command("config", "Validate and view the Compose file"),
    command("cp", "Copy files between services and the host"),
    command("create", "Create services"),
    command("down", "Stop and remove resources"),
    command("events", "Receive real-time service events"),
    command("exec", "Run a command in a service"),
    command("images", "List images used by services"),
    command("kill", "Force-stop services"),
    command("logs", "View service output"),
    command("pause", "Pause services"),
    command("port", "Print a public port binding"),
    command("ps", "List services"),
    command("pull", "Pull service images"),
    command("push", "Push service images"),
    command("restart", "Restart services"),
    command("rm", "Remove stopped service containers"),
    command("run", "Run a one-off command"),
    command("start", "Start services"),
    command("stop", "Stop services"),
    command("top", "Display service processes"),
    command("unpause", "Unpause services"),
    command("up", "Create and start services"),
    command("version", "Show Compose version"),
    command("watch", "Watch source files and rebuild services"),
];

const DOCKER_COMPOSE_OPTIONS: &[OptionDef] = &[
    path_option("-f", "Use a Compose file"),
    path_option("--file", "Use a Compose file"),
    text_option("-p", "Set the project name"),
    text_option("--project-name", "Set the project name"),
    path_option("--project-directory", "Set the project directory"),
    path_option("--env-file", "Use an environment file"),
    text_option("--profile", "Enable a Compose profile"),
    option("--verbose", "Enable verbose output"),
    option("--parallel", "Build in parallel"),
    option("--progress", "Choose progress output"),
];

const DOCKER_COMPOSE_UP_OPTIONS: &[OptionDef] = &[
    option("-d", "Run in detached mode"),
    option("--detach", "Run in detached mode"),
    option("--build", "Build images before starting"),
    option("--no-build", "Do not build images"),
    option("--force-recreate", "Recreate containers"),
    option("--no-recreate", "Do not recreate existing containers"),
    option("--no-deps", "Do not start linked services"),
    option("--remove-orphans", "Remove orphan containers"),
    option("--always-recreate-deps", "Recreate dependent containers"),
    option("--renew-anon-volumes", "Recreate anonymous volumes"),
    option("--wait", "Wait for services to be running"),
    text_option("--wait-timeout", "Set the wait timeout"),
    option("--quiet-pull", "Pull without progress output"),
    text_option("--scale", "Set a service scale"),
];

const DOCKER_CONTAINER_COMMANDS: &[CommandDef] = &[
    command("attach", "Attach local input and output to a container"),
    command("commit", "Create an image from a container"),
    command("cp", "Copy files between a container and the host"),
    command("create", "Create a container"),
    command("diff", "Inspect filesystem changes"),
    command("exec", "Run a command in a running container"),
    command("export", "Export a container filesystem"),
    command("inspect", "Display container details"),
    command("kill", "Kill containers"),
    command("logs", "Fetch container logs"),
    command("ls", "List containers"),
    command("pause", "Pause containers"),
    command("port", "List port mappings"),
    command("prune", "Remove unused containers"),
    command("rename", "Rename a container"),
    command("restart", "Restart containers"),
    command("rm", "Remove containers"),
    command("run", "Run a command in a new container"),
    command("start", "Start containers"),
    command("stats", "Display container resource usage"),
    command("stop", "Stop containers"),
    command("top", "Display processes in a container"),
    command("unpause", "Unpause containers"),
    command("update", "Update container configuration"),
    command("wait", "Wait for containers to stop"),
];

const DOCKER_IMAGE_COMMANDS: &[CommandDef] = &[
    command("build", "Build an image"),
    command("history", "Show image history"),
    command("import", "Create an image from a filesystem archive"),
    command("inspect", "Display image details"),
    command("load", "Load images from an archive"),
    command("ls", "List images"),
    command("prune", "Remove unused images"),
    command("pull", "Pull an image"),
    command("push", "Push an image"),
    command("rm", "Remove images"),
    command("save", "Save images to an archive"),
    command("tag", "Tag an image"),
];

const DOCKER_NETWORK_COMMANDS: &[CommandDef] = &[
    command("connect", "Connect a container to a network"),
    command("create", "Create a network"),
    command("disconnect", "Disconnect a container from a network"),
    command("inspect", "Display network details"),
    command("ls", "List networks"),
    command("prune", "Remove unused networks"),
    command("rm", "Remove networks"),
];

const DOCKER_VOLUME_COMMANDS: &[CommandDef] = &[
    command("create", "Create a volume"),
    command("inspect", "Display volume details"),
    command("ls", "List volumes"),
    command("prune", "Remove unused volumes"),
    command("rm", "Remove volumes"),
];

const DOCKER_SYSTEM_COMMANDS: &[CommandDef] = &[
    command("df", "Show Docker disk usage"),
    command("events", "Show Docker events"),
    command("info", "Show Docker system information"),
    command("prune", "Remove unused Docker data"),
];

const PWSH_OPTIONS: &[OptionDef] = &[
    text_option("-Command", "Run the specified PowerShell command"),
    text_option("-EncodedCommand", "Run a Base64-encoded command"),
    text_option("-EncodedArguments", "Pass Base64-encoded arguments"),
    choice_option(
        "-ExecutionPolicy",
        "Set the execution policy",
        &[
            ValueDef {
                name: "Bypass",
                description: "Do not block scripts for this process",
            },
            ValueDef {
                name: "Unrestricted",
                description: "Allow unrestricted script execution",
            },
            ValueDef {
                name: "RemoteSigned",
                description: "Require signatures for downloaded scripts",
            },
            ValueDef {
                name: "AllSigned",
                description: "Require signatures for all scripts",
            },
            ValueDef {
                name: "Restricted",
                description: "Disallow scripts",
            },
        ],
    ),
    path_option("-File", "Run a PowerShell script file"),
    choice_option("-InputFormat", "Choose input format", PWSH_INPUT_VALUES),
    option("-Login", "Start as a login shell"),
    option("-Mta", "Use the multithreaded apartment state"),
    option("-NoExit", "Keep the shell open after startup"),
    option("-NoLogo", "Hide the startup logo"),
    option("-NonInteractive", "Disable interactive prompts"),
    option("-NoProfile", "Skip loading the PowerShell profile"),
    choice_option("-OutputFormat", "Choose output format", PWSH_OUTPUT_VALUES),
    option("-Sta", "Use the single-threaded apartment state"),
    text_option("-Version", "Choose the PowerShell version"),
    choice_option(
        "-WindowStyle",
        "Choose the window style",
        PWSH_WINDOW_VALUES,
    ),
    path_option("-WorkingDirectory", "Set the initial working directory"),
    option("--help", "Show PowerShell help"),
    option("--version", "Show the PowerShell version"),
];

const GH_COMMANDS: &[CommandDef] = &[
    command("alias", "Manage GitHub CLI aliases"),
    command("api", "Make an authenticated GitHub API request"),
    command("attestation", "Manage artifact attestations"),
    command("auth", "Authenticate with GitHub"),
    command("browse", "Open a GitHub page in a browser"),
    command("codespace", "Connect to and manage codespaces"),
    command("config", "Manage GitHub CLI configuration"),
    command("copilot", "Use GitHub Copilot"),
    command("extension", "Manage GitHub CLI extensions"),
    command("gist", "Manage gists"),
    command("issue", "Manage issues"),
    command("label", "Manage issue labels"),
    command("org", "Manage organizations"),
    command("pr", "Manage pull requests"),
    command("project", "Manage projects"),
    command("release", "Manage releases"),
    command("repo", "Manage repositories"),
    command("ruleset", "Manage repository rulesets"),
    command("run", "View and manage GitHub Actions runs"),
    command("search", "Search GitHub"),
    command("secret", "Manage repository secrets"),
    command("ssh-key", "Manage SSH keys"),
    command("status", "Show notifications and status"),
    command("variable", "Manage repository variables"),
    command("workflow", "Manage GitHub Actions workflows"),
];

const GH_GLOBAL_OPTIONS: &[OptionDef] = &[
    option("--help", "Show GitHub CLI help"),
    option("--version", "Show the GitHub CLI version"),
    text_option("--hostname", "Use a GitHub Enterprise host"),
    text_option("--repo", "Select a repository"),
    text_option("--json", "Print JSON fields"),
    text_option("--jq", "Filter JSON with jq"),
    text_option("--template", "Format output with a template"),
    option("--web", "Open the result in a browser"),
    option("--paginate", "Request all pages"),
    option("--slurp", "Wrap paginated JSON in an array"),
];

const GH_API_OPTIONS: &[OptionDef] = &[
    text_option("--method", "Choose the HTTP method"),
    text_option("-F", "Add a typed parameter"),
    text_option("--field", "Add a typed parameter"),
    text_option("-f", "Add a string parameter"),
    text_option("--raw-field", "Add a string parameter"),
    text_option("-H", "Add an HTTP header"),
    text_option("--header", "Add an HTTP header"),
    path_option("--input", "Read a request body from a file"),
    path_option("--input-file", "Read a request body from a file"),
    text_option("--jq", "Filter JSON with jq"),
    text_option("--template", "Format output with a template"),
    option("--include", "Include response headers"),
    option("--silent", "Suppress response body"),
    option("--cache", "Cache a response"),
    option("--paginate", "Request all pages"),
    option("--slurp", "Wrap paginated JSON in an array"),
    option("--verbose", "Show request details"),
];

const GH_PR_COMMANDS: &[CommandDef] = &[
    command("checkout", "Check out a pull request locally"),
    command("checks", "Show pull request checks"),
    command("close", "Close a pull request"),
    command("comment", "Add a comment to a pull request"),
    command("create", "Create a pull request"),
    command("diff", "View pull request changes"),
    command("edit", "Edit a pull request"),
    command("list", "List pull requests"),
    command("lock", "Lock a pull request conversation"),
    command("merge", "Merge a pull request"),
    command("ready", "Mark a pull request ready for review"),
    command("reopen", "Reopen a pull request"),
    command("review", "Review a pull request"),
    command("status", "Show pull request status"),
    command("unlock", "Unlock a pull request conversation"),
    command("view", "View a pull request"),
];

const GH_ISSUE_COMMANDS: &[CommandDef] = &[
    command("close", "Close an issue"),
    command("comment", "Add a comment to an issue"),
    command("create", "Create an issue"),
    command("delete", "Delete an issue"),
    command("develop", "Manage issue development branches"),
    command("edit", "Edit an issue"),
    command("list", "List issues"),
    command("lock", "Lock an issue conversation"),
    command("pin", "Pin an issue"),
    command("reopen", "Reopen an issue"),
    command("status", "Show issue status"),
    command("transfer", "Transfer an issue"),
    command("unlock", "Unlock an issue conversation"),
    command("unpin", "Unpin an issue"),
    command("view", "View an issue"),
];

const GH_REPO_COMMANDS: &[CommandDef] = &[
    command("archive", "Archive a repository"),
    command("clone", "Clone a repository"),
    command("create", "Create a repository"),
    command("delete", "Delete a repository"),
    command("deploy-key", "Manage deploy keys"),
    command("edit", "Edit repository settings"),
    command("fork", "Fork a repository"),
    command("list", "List repositories"),
    command("rename", "Rename a repository"),
    command("set-default", "Set the default repository"),
    command("sync", "Sync a fork"),
    command("unarchive", "Unarchive a repository"),
    command("view", "View a repository"),
];

const GH_RUN_COMMANDS: &[CommandDef] = &[
    command("cancel", "Cancel a workflow run"),
    command("delete", "Delete a workflow run"),
    command("download", "Download run artifacts"),
    command("list", "List workflow runs"),
    command("rerun", "Rerun a workflow"),
    command("view", "View a workflow run"),
    command("watch", "Watch a workflow run"),
];

const GH_WORKFLOW_COMMANDS: &[CommandDef] = &[
    command("disable", "Disable a workflow"),
    command("enable", "Enable a workflow"),
    command("list", "List workflows"),
    command("run", "Run a workflow"),
    command("view", "View a workflow"),
];

const GH_AUTH_COMMANDS: &[CommandDef] = &[
    command("login", "Log in to GitHub"),
    command("logout", "Log out of GitHub"),
    command("refresh", "Refresh authentication"),
    command("setup-git", "Configure Git credential storage"),
    command("status", "Show authentication status"),
    command("switch", "Switch authentication accounts"),
    command("token", "Print an authentication token"),
];

const GH_SEARCH_COMMANDS: &[CommandDef] = &[
    command("code", "Search code"),
    command("commits", "Search commits"),
    command("issues", "Search issues"),
    command("prs", "Search pull requests"),
    command("repos", "Search repositories"),
    command("topics", "Search topics"),
    command("users", "Search users"),
];

const GH_COMMON_OPTIONS: &[OptionDef] = &[
    text_option("--repo", "Select a repository"),
    text_option("--json", "Print JSON fields"),
    text_option("--jq", "Filter JSON with jq"),
    text_option("--template", "Format output with a template"),
    option("--web", "Open the result in a browser"),
    option("--help", "Show command help"),
];

const GH_LIST_OPTIONS: &[OptionDef] = &[
    text_option("--limit", "Limit the number of results"),
    text_option("--state", "Filter by state"),
    text_option("--author", "Filter by author"),
    text_option("--assignee", "Filter by assignee"),
    text_option("--label", "Filter by label"),
    text_option("--search", "Filter with a search expression"),
    text_option("--sort", "Sort results"),
    text_option("--order", "Choose sort order"),
];

const SET_LOCATION_OPTIONS: &[OptionDef] = &[
    path_option("-Path", "Set the working directory"),
    path_option(
        "-LiteralPath",
        "Set the working directory without wildcard expansion",
    ),
    option("-Force", "Include hidden locations"),
    option("-PassThru", "Return the selected location"),
    text_option("-StackName", "Use a location stack"),
    option("-UseTransaction", "Use the current transaction"),
    option("-Verbose", "Show detailed output"),
    text_option("-ErrorAction", "Choose error handling"),
    text_option("-ErrorVariable", "Store errors in a variable"),
];

const GET_CHILD_ITEM_OPTIONS: &[OptionDef] = &[
    path_option("-Path", "Select a path"),
    path_option("-LiteralPath", "Select a path without wildcard expansion"),
    text_option("-Filter", "Filter items with a provider pattern"),
    text_option("-Include", "Include matching items"),
    text_option("-Exclude", "Exclude matching items"),
    option("-Recurse", "Search child items recursively"),
    option("-Force", "Include hidden and system items"),
    option("-Name", "Return item names only"),
    option("-Directory", "Return directories only"),
    option("-File", "Return files only"),
    text_option("-Depth", "Limit recursive depth"),
    text_option("-Attributes", "Filter by item attributes"),
    option("-FollowSymlink", "Follow symbolic links"),
    option("-Hidden", "Return hidden items"),
    option("-ReadOnly", "Return read-only items"),
    option("-System", "Return system items"),
    text_option("-ErrorAction", "Choose error handling"),
    text_option("-ErrorVariable", "Store errors in a variable"),
    option("-Verbose", "Show detailed output"),
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
        "git" => Some("Distributed version control"),
        "cargo" => Some("Rust package manager and build tool"),
        "npm" => Some("JavaScript package manager"),
        "docker" => Some("Container image and runtime manager"),
        "pwsh" => Some("PowerShell command-line shell"),
        "gh" => Some("GitHub command-line interface"),
        "Set-Location" => Some("Change the current PowerShell location"),
        "Get-ChildItem" => Some("List files and child items"),
        _ => None,
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
                    .any(|candidate: &SpecCandidate| candidate.name == value.name)
            {
                candidates.push(SpecCandidate {
                    name: value.name,
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

    let options_requested =
        prefix.starts_with('-') || (prefix.is_empty() && root == "pwsh" && parsed.parts.len() == 1);
    if !parsed.options_ended && options_requested {
        for option in parsed.spec.options {
            if starts_with(option.name, prefix, case_insensitive)
                && !candidates
                    .iter()
                    .any(|candidate: &SpecCandidate| candidate.name == option.name)
            {
                candidates.push(SpecCandidate {
                    name: option.name,
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
                        .any(|candidate| candidate.name == option.name)
                {
                    candidates.push(SpecCandidate {
                        name: option.name,
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
                    name: subcommand.name,
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
                if option.value != ValueKind::None && !attached {
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

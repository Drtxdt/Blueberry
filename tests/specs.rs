use shellsense::model::CandidateKind;
use shellsense::spec_catalog::{Catalog, DiagnosticSeverity};
use shellsense::specs::{all_builtin_descriptions, canonical_command, complete, describe_command};
use std::fs;
use tempfile::tempdir;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn names(result: &shellsense::specs::SpecResult) -> Vec<&str> {
    result
        .candidates
        .iter()
        .map(|candidate| candidate.name.as_str())
        .collect()
}

fn has_chinese(value: &str) -> bool {
    value
        .chars()
        .any(|character| ('\u{3400}'..='\u{9fff}').contains(&character))
}

#[test]
fn canonical_commands_include_executable_stems_and_powershell_aliases() {
    assert_eq!(canonical_command("git"), Some("git"));
    assert_eq!(canonical_command("git.exe"), Some("git"));
    assert_eq!(
        canonical_command(r"C:\Program Files\Git\bin\git.exe"),
        Some("git")
    );
    assert_eq!(canonical_command("PowerShell"), Some("pwsh"));
    assert_eq!(canonical_command("set-location"), Some("Set-Location"));
    assert_eq!(canonical_command("cd"), Some("Set-Location"));
    assert_eq!(canonical_command("ls"), Some("Get-ChildItem"));
    assert_eq!(canonical_command("unknown-command"), None);
}

#[test]
fn root_descriptions_are_human_readable() {
    for command in [
        "git",
        "cargo",
        "npm",
        "docker",
        "pwsh",
        "gh",
        "set-location",
        "get-childitem",
    ] {
        let description = describe_command(command).expect("recognized command");
        assert!(!description.trim().is_empty());
        assert!(has_chinese(description));
    }
}

#[test]
fn git_log_scope_contains_oneline_and_status_does_not() {
    let log = complete("git", &args(&["log"]), "--o").expect("git spec");
    assert_eq!(log.context, "git log");
    assert!(names(&log).contains(&"--oneline"));
    assert!(
        log.candidates
            .iter()
            .all(|candidate| candidate.kind == CandidateKind::Option)
    );
    assert!(
        log.candidates
            .iter()
            .all(|candidate| has_chinese(&candidate.description))
    );
    assert_eq!(
        log.candidates
            .iter()
            .find(|candidate| candidate.name == "--oneline")
            .map(|candidate| candidate.description.as_str()),
        Some("每条提交显示为一行")
    );

    let status = complete("git", &args(&["status"]), "--o").expect("git spec");
    assert_eq!(status.context, "git status");
    assert!(!names(&status).contains(&"--oneline"));
}

#[test]
fn git_global_directory_and_config_options_are_consumed_before_subcommand() {
    let result = complete(
        "git",
        &args(&["-C", "repo", "-c", "core.pager=cat", "log"]),
        "--o",
    )
    .expect("git spec");
    assert_eq!(result.context, "git log");
    assert!(names(&result).contains(&"--oneline"));

    let path = complete("git", &args(&["-C"]), "rep").expect("git spec");
    assert!(path.path_values);
    assert!(path.candidates.is_empty());

    let attached = complete("git", &args(&["-Crepo", "log"]), "--o").expect("git spec");
    assert_eq!(attached.context, "git log");
    assert!(names(&attached).contains(&"--oneline"));

    let config_value = complete("git", &args(&["-c"]), "core.").expect("git spec");
    assert!(!config_value.path_values);
    assert!(config_value.candidates.is_empty());
}

#[test]
fn git_short_option_case_is_significant_but_powershell_parameters_are_not() {
    let upper = complete("git", &[], "-C").expect("git spec");
    assert_eq!(names(&upper), vec!["-C"]);
    assert!(upper.candidates[0].description.contains("目录"));

    let lower = complete("git", &[], "-c").expect("git spec");
    assert_eq!(names(&lower), vec!["-c"]);
    assert!(lower.candidates[0].description.contains("配置"));

    let powershell = complete("pwsh", &[], "-command").expect("PowerShell spec");
    assert_eq!(names(&powershell), vec!["-Command"]);
}

#[test]
fn unknown_options_do_not_consume_their_following_token() {
    let result = complete("git", &args(&["--future-flag", "log"]), "--o").expect("git spec");
    assert_eq!(result.context, "git log");
    assert!(names(&result).contains(&"--oneline"));

    let positional =
        complete("git", &args(&["--future-flag", "value", "log"]), "--o").expect("git spec");
    assert_eq!(positional.context, "git");
    assert!(!names(&positional).contains(&"--oneline"));
}

#[test]
fn options_after_double_dash_are_never_offered() {
    let result = complete("git", &args(&["log", "--"]), "--o").expect("git spec");
    assert_eq!(result.context, "git log");
    assert!(result.options_ended);
    assert!(result.candidates.is_empty());
    assert!(result.path_values);

    let cargo = complete("cargo", &args(&["run", "--"]), "--rel").expect("cargo spec");
    assert!(cargo.options_ended);
    assert!(cargo.candidates.is_empty());
}

#[test]
fn cargo_toolchain_and_command_options_are_context_aware() {
    let result = complete("cargo", &args(&["+nightly", "build"]), "--rel").expect("cargo spec");
    assert_eq!(result.context, "cargo build");
    assert!(names(&result).contains(&"--release"));

    let root = complete("cargo", &args(&["+nightly"]), "bu").expect("cargo spec");
    assert_eq!(root.context, "cargo");
    assert!(names(&root).contains(&"build"));
}

#[test]
fn value_options_report_path_or_value_completion_without_options() {
    let path = complete("cargo", &args(&["build", "--manifest-path"]), "C:").expect("cargo spec");
    assert_eq!(path.context, "cargo build");
    assert!(path.path_values);
    assert!(path.candidates.is_empty());

    let color = complete("cargo", &args(&["build", "--color"]), "a").expect("cargo spec");
    assert!(!color.path_values);
    assert_eq!(names(&color), vec!["always", "auto"]);
    assert!(
        color
            .candidates
            .iter()
            .all(|candidate| candidate.kind == CandidateKind::Value)
    );
}

#[test]
fn optional_values_are_completed_only_when_attached() {
    let ignored = complete("git", &args(&["status"]), "--ignored=mat").expect("git spec");
    assert_eq!(ignored.context, "git status");
    assert_eq!(names(&ignored), vec!["--ignored=matching"]);
    assert_eq!(ignored.candidates[0].kind, CandidateKind::Value);
    assert_eq!(ignored.candidates[0].key, "git status --ignored matching");

    let bare_ignored = complete("git", &args(&["status", "--ignored"]), "src").expect("git spec");
    assert!(bare_ignored.candidates.is_empty());
    assert!(bare_ignored.path_values);

    let rebase = complete("git", &args(&["pull"]), "--rebase=me").expect("git spec");
    assert_eq!(names(&rebase), vec!["--rebase=merges"]);
    assert_eq!(rebase.candidates[0].key, "git pull --rebase merges");

    let bare_rebase = complete("git", &args(&["pull", "--rebase"]), "--st").expect("git spec");
    assert!(names(&bare_rebase).contains(&"--strategy"));

    let timings = complete("cargo", &args(&["build"]), "--timings=").expect("cargo spec");
    assert_eq!(names(&timings), vec!["--timings=html", "--timings=json"]);

    let color_inline = complete("cargo", &args(&["build"]), "--color=a").expect("cargo spec");
    assert_eq!(names(&color_inline), vec!["--color=always", "--color=auto"]);

    let date_inline = complete("git", &args(&["log"]), "--date=rel").expect("git spec");
    assert_eq!(names(&date_inline), vec!["--date=relative"]);

    let artifact =
        complete("cargo", &args(&["build", "--artifact-dir"]), "out").expect("cargo spec");
    assert!(artifact.path_values);
    assert!(artifact.candidates.is_empty());
}

#[test]
fn long_value_delimiter_is_not_an_inline_option_boundary() {
    // Cargo's comma delimiter separates feature values. It does not make
    // `--features,a` a valid spelling of an attached long-option value.
    let invalid = complete("cargo", &args(&["build"]), "--features,a")
        .expect("cargo build spec remains addressable");
    assert_eq!(invalid.context, "cargo build");
    assert!(invalid.candidates.is_empty());
    assert_eq!(invalid.provider, None);
    assert_eq!(invalid.value_delimiter, None);

    // The long-option/value boundary is `=`; the option still advertises its
    // comma delimiter to the provider for values such as `a,b`.
    let attached = complete("cargo", &args(&["build"]), "--features=a,b")
        .expect("cargo build spec remains addressable");
    assert_eq!(attached.context, "cargo build");
    assert_eq!(attached.provider.as_deref(), Some("cargo.features"));
    assert_eq!(attached.value_delimiter, Some(','));

    // A short option with an attached value keeps its existing grammar.
    let short = complete("git", &args(&["-ccore.pager=cat", "log"]), "--o")
        .expect("git short attached option remains valid");
    assert_eq!(short.context, "git log");
    assert!(names(&short).contains(&"--oneline"));
}

#[test]
fn no_value_flags_do_not_consume_following_options() {
    let ff = complete("git", &args(&["pull", "--ff"]), "--re").expect("git spec");
    assert!(names(&ff).contains(&"--rebase"));

    let full_diff = complete("git", &args(&["show", "--full-diff"]), "--fi").expect("git spec");
    assert!(names(&full_diff).contains(&"--find-renames"));

    let minimal = complete("git", &args(&["diff", "--minimal"]), "--an").expect("git spec");
    assert!(names(&minimal).contains(&"--anchored"));
}

#[test]
fn nested_git_stash_and_remote_contexts_have_their_own_scopes() {
    let stash = complete("git", &args(&["stash"]), "pu").expect("git spec");
    assert_eq!(stash.context, "git stash");
    assert!(names(&stash).contains(&"push"));
    assert!(
        stash
            .candidates
            .iter()
            .all(|candidate| candidate.kind == CandidateKind::Subcommand)
    );

    let push = complete("git", &args(&["stash", "push"]), "--m").expect("git spec");
    assert_eq!(push.context, "git stash push");
    assert!(names(&push).contains(&"--message"));

    let remote = complete("git", &args(&["remote"]), "ad").expect("git spec");
    assert_eq!(remote.context, "git remote");
    assert!(names(&remote).contains(&"add"));

    let add = complete("git", &args(&["remote", "add"]), "--f").expect("git spec");
    assert_eq!(add.context, "git remote add");
    assert!(names(&add).contains(&"--fetch"));
}

#[test]
fn command_options_are_filtered_by_scope() {
    let log = complete("git", &args(&["log"]), "--sh").expect("git spec");
    assert!(names(&log).contains(&"--show-signature"));

    let status = complete("git", &args(&["status"]), "--sh").expect("git spec");
    assert!(!names(&status).contains(&"--show-signature"));

    let build = complete("cargo", &args(&["build"]), "--re").expect("cargo spec");
    assert!(names(&build).contains(&"--release"));
    let fmt = complete("cargo", &args(&["fmt"]), "--re").expect("cargo spec");
    assert!(!names(&fmt).contains(&"--release"));
}

#[test]
fn aliases_keep_canonical_context_and_path_value_behavior() {
    let location = complete("cd", &[], "-Pa").expect("location spec");
    assert_eq!(location.context, "Set-Location");
    assert!(names(&location).contains(&"-Path"));

    let location_value = complete("cd", &args(&["-Path"]), "src").expect("location spec");
    assert!(location_value.path_values);
    assert!(location_value.candidates.is_empty());

    let child = complete("ls", &[], "-Re").expect("child item spec");
    assert_eq!(child.context, "Get-ChildItem");
    assert!(names(&child).contains(&"-Recurse"));
}

#[test]
fn other_root_specs_offer_subcommands_and_nested_commands() {
    for command in ["npm", "docker", "gh"] {
        let result = complete(command, &[], "").expect("known spec");
        assert!(!result.candidates.is_empty(), "{command} has root commands");
        assert!(
            result
                .candidates
                .iter()
                .all(|candidate| candidate.kind == CandidateKind::Subcommand)
        );
    }

    let compose = complete("docker", &args(&["compose"]), "u").expect("docker spec");
    assert_eq!(compose.context, "docker compose");
    assert!(names(&compose).contains(&"up"));

    let pull = complete("docker", &args(&["run", "--pull"]), "a").expect("docker spec");
    assert_eq!(names(&pull), vec!["always"]);

    let pr = complete("gh", &args(&["pr"]), "cr").expect("gh spec");
    assert_eq!(pr.context, "gh pr");
    assert!(names(&pr).contains(&"create"));
}

#[test]
fn every_returned_candidate_has_a_real_description_and_context_key() {
    let cases = [
        ("git", vec![], "--"),
        ("git", vec!["log"], "--"),
        ("git", vec!["stash"], ""),
        ("git", vec!["remote"], ""),
        ("cargo", vec![], ""),
        ("cargo", vec!["test"], "--"),
        ("npm", vec![], ""),
        ("docker", vec![], ""),
        ("pwsh", vec![], "-"),
        ("gh", vec![], ""),
        ("cd", vec![], "-"),
        ("ls", vec![], "-"),
    ];
    for (command, before, prefix) in cases {
        let before = before.into_iter().map(str::to_owned).collect::<Vec<_>>();
        let result = complete(command, &before, prefix).expect("known spec");
        for candidate in result.candidates {
            assert!(!candidate.description.trim().is_empty());
            assert!(has_chinese(&candidate.description));
            let lower = candidate.description.to_ascii_lowercase();
            assert!(!lower.contains("todo"));
            assert!(!lower.contains("placeholder"));
            assert!(!lower.contains("command description"));
            assert!(!lower.contains("option description"));
            assert!(candidate.key.starts_with(&result.context));
            assert!(candidate.key.ends_with(candidate.name.as_str()));
        }
    }
}

#[test]
fn every_builtin_description_is_localized() {
    let descriptions = all_builtin_descriptions();
    assert!(
        descriptions.len() >= 1_000,
        "static catalog unexpectedly small: {} descriptions",
        descriptions.len()
    );

    for command in [
        "git",
        "cargo",
        "npm",
        "docker",
        "pwsh",
        "gh",
        "set-location",
        "get-childitem",
    ] {
        let root = describe_command(command).expect("recognized root command");
        assert!(
            descriptions.contains(&root),
            "missing root description: {root}"
        );
    }

    assert!(descriptions.contains(&"每条提交显示为一行"));
    assert!(descriptions.contains(&"构建项目并管理 Rust 依赖"));
    assert!(descriptions.contains(&"使用发布配置构建，默认启用优化"));

    for description in descriptions {
        assert!(!description.trim().is_empty());
        assert!(
            has_chinese(description),
            "description is not localized: {description}"
        );
        let lower = description.to_ascii_lowercase();
        assert!(
            !lower.contains("todo"),
            "placeholder description: {description}"
        );
        assert!(
            !lower.contains("placeholder"),
            "placeholder description: {description}"
        );
        assert!(
            !lower.contains("command description"),
            "placeholder description: {description}"
        );
        assert!(
            !lower.contains("option description"),
            "placeholder description: {description}"
        );
    }
}

#[test]
fn generated_catalog_description_inventory_is_exposed() {
    let descriptions = all_builtin_descriptions();
    assert!(descriptions.len() >= 1_000);
    assert!(descriptions.contains(&"管理代码版本并协作开发"));
    assert!(descriptions.contains(&"合并开发历史"));
    assert!(descriptions.contains(&"运行 workspace 脚本"));
}

#[test]
fn user_catalog_overrides_exact_context_and_preserves_last_good_builtin_node() {
    let directory = tempdir().expect("temporary user spec directory");
    fs::write(
        directory.path().join("01-user.toml"),
        r#"
schema_version = 1
name = "测试规格"
version = "1"

[[option_sets]]
id = "test.log"

[[option_sets.options]]
name = "--json"
description = "输出 JSON"
detail = "输出 JSON；示例：git log --json。"
value_kind = "none"

[[nodes]]
path = "git log"
description = "查看提交日志（覆盖）"
detail = "查看提交日志（覆盖）；示例：git log --json。"
option_sets = ["test.log"]
examples = ["git log --json"]
provider = "git.refs"
"#,
    )
    .unwrap();

    let catalog = Catalog::load_user_dir(directory.path()).expect("catalog loads");
    let result = catalog
        .lookup("git", &args(&["log"]), "--j")
        .expect("overridden context remains addressable");
    assert_eq!(result.context, "git log");
    assert_eq!(names(&result), vec!["--json"]);
    assert_eq!(
        result.candidates[0].source,
        directory.path().join("01-user.toml").display().to_string()
    );
    assert_eq!(result.provider, None);

    let positional = catalog
        .lookup("git", &args(&["log"]), "src")
        .expect("overridden positional context remains addressable");
    assert_eq!(positional.provider.as_deref(), Some("git.refs"));

    let untouched = catalog
        .lookup("git", &args(&["status"]), "--zz")
        .expect("unmodified built-in context");
    assert!(names(&untouched).is_empty());
    assert!(catalog.diagnostics().is_empty());
}

#[test]
fn directory_value_specs_are_distinct_from_file_or_directory_paths() {
    for (command, preceding) in [
        ("git", vec!["-C"]),
        ("git", vec!["--git-dir"]),
        ("git", vec!["--work-tree"]),
        ("npm", vec!["--prefix"]),
        ("npm", vec!["install", "--prefix"]),
        ("pnpm", vec!["--dir"]),
        ("pnpm", vec!["-C"]),
        ("pnpm", vec!["install", "--dir"]),
    ] {
        let result = complete(command, &args(&preceding), "repo").expect("directory spec");
        assert!(result.path_values, "{command} {preceding:?} accepts paths");
        assert!(
            result.directories_only,
            "{command} {preceding:?} accepts directories only"
        );
    }

    let location = complete("Set-Location", &[], "repo").expect("location spec");
    assert!(location.path_values);
    assert!(location.directories_only);

    let location_option =
        complete("Set-Location", &args(&["-Path"]), "repo").expect("location option spec");
    assert!(location_option.path_values);
    assert!(location_option.directories_only);

    let ordinary = complete("cargo", &args(&["build", "--manifest-path"]), "Cargo.toml")
        .expect("file or directory path spec");
    assert!(ordinary.path_values);
    assert!(!ordinary.directories_only);
}

#[test]
fn user_catalog_reports_invalid_files_without_discarding_valid_snapshot() {
    let directory = tempdir().expect("temporary user spec directory");
    fs::write(
        directory.path().join("01-valid.toml"),
        r#"
schema_version = 1

[[nodes]]
path = "mytool"
description = "运行我的工具"
detail = "运行我的工具；示例：mytool。"
positional = "none"
children = ["mytool list"]

[[nodes]]
path = "mytool list"
description = "列出项目"
detail = "列出项目；示例：mytool list。"
positional = "value"
provider = "powershell.paths"
"#,
    )
    .unwrap();
    fs::write(
        directory.path().join("02-invalid.toml"),
        "schema_version = 99\n",
    )
    .unwrap();

    let catalog = Catalog::load_user_dir(directory.path()).expect("directory loads");
    assert!(catalog.canonical_command("mytool").is_some());
    assert!(catalog.lookup("mytool", &[], "").is_some());
    assert!(
        catalog
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    );

    let report = Catalog::check_dir(directory.path()).expect("check report");
    assert!(!report.is_valid());
    assert_eq!(report.files.len(), 2);
}

#[test]
fn repeatability_conflict_dependency_and_delimited_values_are_enforced() {
    let directory = tempdir().expect("temporary user spec directory");
    fs::write(
        directory.path().join("rules.toml"),
        r#"
schema_version = 1

[[option_sets]]
id = "tool.options"

[[option_sets.options]]
name = "--features"
description = "选择 features"
detail = "选择 features；示例：tool --features=a,b。"
value_kind = "text"
value_delimiter = ","
repeatable = true
values = [{ name = "alpha", description = "启用 alpha" }, { name = "beta", description = "启用 beta" }]

[[option_sets.options]]
name = "--fast"
description = "启用快速模式"
detail = "启用快速模式；示例：tool --fast。"
value_kind = "none"
conflicts = ["--slow"]

[[option_sets.options]]
name = "--slow"
description = "启用慢速模式"
detail = "启用慢速模式；示例：tool --slow。"
value_kind = "none"
conflicts = ["--fast"]

[[option_sets.options]]
name = "--report"
description = "输出报告"
detail = "输出报告；示例：tool --fast --report。"
value_kind = "none"
requires = ["--fast"]

[[nodes]]
path = "tool"
description = "运行测试工具"
detail = "运行测试工具；示例：tool。"
positional = "none"
option_sets = ["tool.options"]
"#,
    )
    .unwrap();
    let catalog = Catalog::load_user_dir(directory.path()).expect("catalog loads");
    let options = catalog.lookup("tool", &[], "--").expect("tool options");
    assert!(names(&options).contains(&"--fast"));
    assert!(names(&options).contains(&"--slow"));
    assert!(!names(&options).contains(&"--report"));

    let after_fast = catalog
        .lookup("tool", &args(&["--fast"]), "--")
        .expect("tool options after --fast");
    assert!(!names(&after_fast).contains(&"--fast"));
    assert!(!names(&after_fast).contains(&"--slow"));
    assert!(names(&after_fast).contains(&"--report"));

    let delimited = catalog
        .lookup("tool", &[], "--features=a")
        .expect("inline value context");
    assert!(names(&delimited).contains(&"--features=alpha"));
    assert_eq!(delimited.value_delimiter, Some(','));
}

#[test]
fn builtin_roots_do_not_invent_git_or_project_script_commands() {
    assert!(names(&complete("git", &args(&[]), "rename").unwrap()).is_empty());
    assert!(names(&complete("pnpm", &args(&[]), "build").unwrap()).is_empty());
}

#[test]
fn builtin_examples_never_contain_generated_placeholders() {
    assert!(!include_str!("../specs/builtin.toml").contains(" 示例\""));
}

use shellsense::model::CandidateKind;
use shellsense::specs::{canonical_command, complete, describe_command};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn names(result: &shellsense::specs::SpecResult) -> Vec<&str> {
    result
        .candidates
        .iter()
        .map(|candidate| candidate.name)
        .collect()
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
        assert!(!description.eq_ignore_ascii_case("command"));
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
            .all(|candidate| !candidate.description.contains("command"))
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
    assert!(upper.candidates[0].description.contains("directory"));

    let lower = complete("git", &[], "-c").expect("git spec");
    assert_eq!(names(&lower), vec!["-c"]);
    assert!(lower.candidates[0].description.contains("configuration"));

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
            assert!(!candidate.description.contains("TODO"));
            assert!(candidate.key.starts_with(&result.context));
            assert!(candidate.key.ends_with(candidate.name));
        }
    }
}

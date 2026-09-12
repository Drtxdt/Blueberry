use serde_json::Value;
use shellsense::engine::{CommandIndex, Discovery};
use shellsense::model::{CandidateKind, ShellCommand};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use tempfile::tempdir;

fn shell_command(name: &str, kind: &str, definition: &str) -> ShellCommand {
    ShellCommand {
        name: name.to_owned(),
        kind: kind.to_owned(),
        definition: definition.to_owned(),
    }
}

#[test]
fn requested_contexts_work_through_the_real_line_lexer() {
    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![
        shell_command("cargo", "Application", ""),
        shell_command("git", "Application", ""),
    ]);
    for (line, wanted) in [
        ("car", "cargo"),
        ("cargo build --rel", "--release"),
        ("git log --o", "--oneline"),
        ("git -C \"含 空格目录\" log --o", "--oneline"),
    ] {
        let result = index.complete(line, line.len(), Path::new("."), 100);
        assert!(
            result
                .candidates
                .iter()
                .any(|candidate| candidate.label == wanted),
            "{line}: {result:?}"
        );
    }
    for line in [
        "git status --o",
        "git log -- --o",
        "git -C \"含 空格目录\" log -- --o",
    ] {
        let result = index.complete(line, line.len(), Path::new("."), 100);
        assert!(
            !result
                .candidates
                .iter()
                .any(|candidate| candidate.label == "--oneline"),
            "{line}: {result:?}"
        );
    }
}

#[test]
fn short_commands_rank_before_longer_names_without_inventing_commands() {
    let mut index = CommandIndex::default();
    assert!(
        index
            .complete("cd", 2, Path::new("."), 100)
            .candidates
            .is_empty()
    );
    index.merge_shell_commands(vec![
        shell_command("git-gui", "Application", ""),
        shell_command("git", "Application", ""),
    ]);
    assert_eq!(
        index.complete("gi", 2, Path::new("."), 1).candidates[0].label,
        "git"
    );
}

#[test]
fn discovery_uses_pathext_and_first_path_entry() {
    let first = tempdir().unwrap();
    let second = tempdir().unwrap();
    fs::write(first.path().join("same.exe"), b"").unwrap();
    fs::write(first.path().join("script.cmd"), b"").unwrap();
    fs::write(second.path().join("same.exe"), b"").unwrap();
    fs::write(second.path().join("later.bat"), b"").unwrap();

    let path = std::env::join_paths([first.path(), second.path()]).unwrap();
    let pathext = OsString::from(".EXE;.CMD;.BAT");
    let index = CommandIndex::discover_with_env(&path, pathext.as_os_str());
    let completion = index.complete("sa", 2, Path::new("."), 20);
    assert_eq!(
        completion
            .candidates
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["same"]
    );
    assert_eq!(index.len(), 3);
}

#[test]
fn shell_commands_enable_static_specs_and_alias_mapping() {
    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![
        shell_command("git", "Application", r#"C:\Program Files\Git\bin\git.exe"#),
        shell_command("g", "Alias", "git"),
    ]);

    let completion = index.complete("git ch", 6, Path::new("."), 20);
    assert!(
        completion
            .candidates
            .iter()
            .any(|item| item.label == "checkout")
    );
    assert!(
        completion
            .candidates
            .iter()
            .all(|item| item.kind == CandidateKind::Subcommand)
    );

    let alias_completion = index.complete("g pu", 4, Path::new("."), 20);
    assert!(
        alias_completion
            .candidates
            .iter()
            .any(|item| item.label == "push")
    );

    let executable_completion = index.complete("git.exe st", 10, Path::new("."), 20);
    assert!(
        executable_completion
            .candidates
            .iter()
            .any(|item| item.label == "status")
    );
}

#[test]
fn session_command_priority_prefers_alias_then_function_then_cmdlet() {
    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![
        shell_command("same", "Application", ""),
        shell_command("same", "Cmdlet", ""),
        shell_command("same", "Function", ""),
        shell_command("same", "Alias", "git"),
    ]);

    let completion = index.complete("sa", 2, Path::new("."), 10);
    assert_eq!(completion.candidates.len(), 1);
    assert_eq!(completion.candidates[0].kind, CandidateKind::Alias);

    index.merge_shell_commands(vec![shell_command("same", "Alias", "updated")]);
    let completion = index.complete("sa", 2, Path::new("."), 10);
    assert_eq!(
        completion.candidates[0].description,
        "用途暂未收录 · 别名 → updated"
    );
}

#[test]
fn descriptions_prefer_actual_name_then_alias_target_then_catalog() {
    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![
        shell_command("mytool", "Application", r#"C:\tools\mytool.exe"#),
        shell_command("myalias", "Alias", "mytool --flag"),
        shell_command("git-alias", "Alias", "git"),
    ]);

    let target_override = BTreeMap::from([(String::from("mytool"), String::from("工具用途"))]);
    let tool = index.complete_with_descriptions("mytool", 6, Path::new("."), 10, &target_override);
    assert_eq!(tool.candidates.len(), 1);
    assert_eq!(tool.candidates[0].description, "工具用途");

    let alias_target =
        index.complete_with_descriptions("myalias", 7, Path::new("."), 10, &target_override);
    assert_eq!(alias_target.candidates.len(), 1);
    assert_eq!(alias_target.candidates[0].description, "工具用途");

    let alias_override = BTreeMap::from([
        (String::from("myalias"), String::from("别名用途")),
        (String::from("mytool"), String::from("工具用途")),
    ]);
    let alias = index.complete_with_descriptions("myalias", 7, Path::new("."), 10, &alias_override);
    assert_eq!(alias.candidates[0].description, "别名用途");

    let catalog_override = BTreeMap::from([(String::from("git"), String::from("Git 用途"))]);
    let known_alias =
        index.complete_with_descriptions("git-alias", 9, Path::new("."), 10, &catalog_override);
    assert_eq!(known_alias.candidates.len(), 1);
    assert_eq!(known_alias.candidates[0].description, "Git 用途");
}

#[test]
fn alias_description_overrides_keep_raw_targets_through_alias_chain() {
    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![
        shell_command("cd", "Alias", "Set-Location"),
        shell_command("go", "Alias", "cd"),
    ]);

    let overrides = BTreeMap::from([
        (String::from("cd"), String::from("CD target用途")),
        (String::from("Set-Location"), String::from("规范用途")),
    ]);
    let completion = index.complete_with_descriptions("go", 2, Path::new("."), 10, &overrides);
    assert_eq!(completion.candidates.len(), 1);
    assert_eq!(completion.candidates[0].description, "CD target用途");

    let canonical_only = BTreeMap::from([(String::from("Set-Location"), String::from("规范用途"))]);
    let completion = index.complete_with_descriptions("go", 2, Path::new("."), 10, &canonical_only);
    assert_eq!(completion.candidates[0].description, "规范用途");
}

#[test]
fn unknown_descriptions_identify_source_and_session_kind() {
    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![
        shell_command("mystery", "Application", r#"C:\tools\mystery.exe"#),
        shell_command("mystery-alias", "Alias", "mystery --flag"),
        shell_command("mystery-function", "Function", "param($x) { $x }"),
        shell_command("mystery-cmdlet", "Cmdlet", ""),
    ]);

    let no_overrides = BTreeMap::new();
    let application =
        index.complete_with_descriptions("mystery", 7, Path::new("."), 10, &no_overrides);
    let application = application
        .candidates
        .iter()
        .find(|candidate| candidate.label == "mystery")
        .expect("unknown application");
    assert_eq!(
        application.description,
        r#"用途暂未收录 · 程序：C:\tools\mystery.exe"#
    );

    let alias =
        index.complete_with_descriptions("mystery-alias", 13, Path::new("."), 10, &no_overrides);
    assert_eq!(
        alias.candidates[0].description,
        "用途暂未收录 · 别名 → mystery"
    );

    let function =
        index.complete_with_descriptions("mystery-function", 16, Path::new("."), 10, &no_overrides);
    assert_eq!(
        function.candidates[0].description,
        "用途暂未收录 · 当前会话函数"
    );

    let cmdlet =
        index.complete_with_descriptions("mystery-cmdlet", 14, Path::new("."), 10, &no_overrides);
    assert_eq!(
        cmdlet.candidates[0].description,
        "用途暂未收录 · PowerShell 命令"
    );
}

#[test]
fn replacing_session_commands_restores_shadowed_path_command() {
    let command_dir = tempdir().unwrap();
    fs::write(command_dir.path().join("tool.exe"), b"").unwrap();
    let path = std::env::join_paths([command_dir.path()]).unwrap();
    let pathext = OsString::from(".EXE");
    let mut index = CommandIndex::discover_with_env(&path, pathext.as_os_str());

    index.replace_shell_commands(vec![shell_command("tool", "Alias", "other")]);
    let shadowed = index.complete("tool", 4, Path::new("."), 10);
    assert_eq!(shadowed.candidates[0].kind, CandidateKind::Alias);

    index.replace_shell_commands(Vec::new());
    let restored = index.complete("tool", 4, Path::new("."), 10);
    assert_eq!(restored.candidates[0].kind, CandidateKind::Command);
}

#[test]
fn separators_are_quote_aware_and_suffix_is_replaced() {
    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![shell_command("git", "Application", "")]);

    let line = r#"Write-Output "a;b" | giXYZ"#;
    let cursor = line.find("gi").unwrap() + 2;
    let completion = index.complete(line, cursor, Path::new("."), 20);
    assert_eq!(completion.replace_start, line.find("gi").unwrap());
    assert_eq!(completion.replace_end, line.len());
    assert_eq!(completion.candidates[0].label, "git");
}

#[test]
fn call_operator_and_pipeline_start_new_command_segments() {
    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![shell_command("git", "Application", "")]);

    let line = r#"& Write-Output "a;b" && giTAIL"#;
    let start = line.find("gi").unwrap();
    let completion = index.complete(line, start + 2, Path::new("."), 20);
    assert_eq!(completion.replace_start, start);
    assert_eq!(completion.replace_end, line.len());
    assert!(completion.candidates.iter().any(|item| item.label == "git"));
}

#[test]
fn unicode_cursor_and_replacement_are_byte_offsets() {
    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![shell_command("git", "Application", "")]);

    let line = "é;giXYZ";
    let start = line.find("gi").unwrap();
    let completion = index.complete(line, start + 2, Path::new("."), 20);
    assert_eq!(start, 3);
    assert_eq!(completion.replace_start, 3);
    assert_eq!(completion.replace_end, line.len());
}

#[test]
fn quoted_filesystem_candidates_are_literal_and_bounded() {
    let cwd = tempdir().unwrap();
    fs::write(cwd.path().join("my notes.txt"), b"").unwrap();
    fs::create_dir(cwd.path().join("my folder")).unwrap();
    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![shell_command("mytool", "Application", "")]);

    let line = "mytool \"my";
    let completion = index.complete(line, line.len(), cwd.path(), 1);
    assert_eq!(completion.candidates.len(), 1);
    assert_eq!(completion.candidates[0].label, "my folder");
    assert!(
        completion.candidates[0]
            .insert_text
            .starts_with("\"my folder")
    );
    assert!(
        completion.candidates[0]
            .insert_text
            .ends_with(std::path::MAIN_SEPARATOR)
    );
}

#[test]
fn alias_to_child_item_uses_filesystem_fallback() {
    let cwd = tempdir().unwrap();
    fs::write(cwd.path().join("notes.txt"), b"").unwrap();
    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![shell_command("ll", "Alias", "Get-ChildItem")]);

    let line = "ll no";
    let completion = index.complete(line, line.len(), cwd.path(), 10);
    assert_eq!(completion.candidates.len(), 1);
    assert_eq!(completion.candidates[0].label, "notes.txt");
    assert_eq!(completion.candidates[0].kind, CandidateKind::File);
    assert_eq!(completion.candidates[0].description, "文件");
}

#[test]
fn cache_round_trip_uses_explicit_environment_context() {
    let command_dir = tempdir().unwrap();
    fs::write(command_dir.path().join("cached.exe"), b"").unwrap();
    let path = std::env::join_paths([command_dir.path()]).unwrap();
    let pathext = OsString::from(".EXE");
    let index = CommandIndex::discover_with_env(&path, pathext.as_os_str());
    let cache_dir = tempdir().unwrap();
    let cache = cache_dir.path().join("commands.json");
    index.save(&cache).unwrap();

    let loaded = CommandIndex::load_with_env(&cache, &path, pathext.as_os_str()).unwrap();
    let completion = loaded.complete("ca", 2, Path::new("."), 10);
    assert_eq!(completion.candidates[0].label, "cached");

    let other_dir = tempdir().unwrap();
    let other_path = std::env::join_paths([other_dir.path()]).unwrap();
    assert!(CommandIndex::load_with_env(&cache, &other_path, pathext.as_os_str()).is_err());
}

#[test]
fn discovery_is_incremental_and_has_no_8192_entry_cutoff() {
    let command_dir = tempdir().unwrap();
    for index in 0..8_300 {
        fs::write(
            command_dir.path().join(format!("batch-{index:05}.exe")),
            b"",
        )
        .unwrap();
    }
    let path = std::env::join_paths([command_dir.path()]).unwrap();
    let pathext = OsString::from(".EXE");
    let mut discovery = Discovery::new(&path, pathext.as_os_str());

    assert!(
        !discovery.step(),
        "a directory larger than one batch must remain pending"
    );
    let partial = discovery.snapshot();
    assert!(!partial.is_complete());
    assert!(
        partial
            .complete("batch-082", 9, Path::new("."), 10)
            .incomplete
    );

    let mut batches = 1;
    while !discovery.step() {
        batches += 1;
    }
    assert!(
        batches > 8,
        "the full directory should require multiple batches"
    );
    let complete = discovery.snapshot();
    assert!(complete.is_complete());
    assert_eq!(complete.len(), 8_300);
    assert!(
        !complete
            .complete("batch-082", 9, Path::new("."), 10)
            .incomplete
    );
}

#[test]
fn discovery_counts_missing_and_empty_path_boundaries() {
    let root = tempdir().unwrap();
    let mut directories = Vec::new();
    for index in 0..130 {
        directories.push(root.path().join(format!("missing-{index:03}")));
    }
    for index in 0..130 {
        let directory = root.path().join(format!("empty-{index:03}"));
        fs::create_dir(&directory).unwrap();
        directories.push(directory);
    }

    let path = std::env::join_paths(directories.iter()).unwrap();
    let pathext = OsString::from(".EXE");
    let mut discovery = Discovery::new(&path, pathext.as_os_str());
    let mut batches = 0;
    loop {
        batches += 1;
        if discovery.step() {
            break;
        }
        assert!(batches < 20, "skipped PATH entries made no progress");
    }

    assert!(
        batches >= 2,
        "missing and empty directory boundaries should consume batch work"
    );
    assert!(discovery.is_complete());
    assert_eq!(discovery.snapshot().len(), 0);
}

#[test]
fn cache_version_two_rejects_old_and_partial_snapshots() {
    let command_dir = tempdir().unwrap();
    for index in 0..300 {
        fs::write(
            command_dir.path().join(format!("cached-{index:03}.exe")),
            b"",
        )
        .unwrap();
    }
    let path = std::env::join_paths([command_dir.path()]).unwrap();
    let pathext = OsString::from(".EXE");
    let mut discovery = Discovery::new(&path, pathext.as_os_str());
    assert!(!discovery.step());
    let partial = discovery.snapshot();
    let cache_dir = tempdir().unwrap();
    let cache = cache_dir.path().join("commands.json");
    assert!(partial.save(&cache).is_err());

    while !discovery.step() {}
    let complete = discovery.snapshot();
    complete.save(&cache).unwrap();
    let mut json: Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    assert_eq!(json["version"], 2);
    assert_eq!(json["complete"], true);

    json["version"] = Value::from(1);
    fs::write(&cache, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(CommandIndex::load_with_env(&cache, &path, pathext.as_os_str()).is_err());

    json["version"] = Value::from(2);
    json["complete"] = Value::from(false);
    fs::write(&cache, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(CommandIndex::load_with_env(&cache, &path, pathext.as_os_str()).is_err());
}

fn make_file_symlink(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
}

fn make_dir_symlink(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
}

fn create_link_or_skip(result: io::Result<()>, label: &str) -> bool {
    match result {
        Ok(()) => true,
        Err(error) if cfg!(windows) && error.kind() == io::ErrorKind::PermissionDenied => {
            eprintln!("skipping {label}: Windows symlink privilege is unavailable ({error})");
            false
        }
        Err(error) => panic!("unable to create {label} symlink: {error}"),
    }
}

#[test]
fn links_follow_local_relative_chains_skip_dangling_and_remote_targets() {
    let command_dir = tempdir().unwrap();
    let target = command_dir.path().join("rustup.exe");
    let middle = command_dir.path().join("cargo-link.exe");
    let cargo = command_dir.path().join("cargo.exe");
    fs::write(&target, b"").unwrap();
    if !create_link_or_skip(
        make_file_symlink(Path::new("rustup.exe"), &middle),
        "relative middle",
    ) {
        return;
    }
    if !create_link_or_skip(
        make_file_symlink(Path::new("cargo-link.exe"), &cargo),
        "relative outer",
    ) {
        return;
    }
    create_link_or_skip(
        make_file_symlink(
            Path::new("missing.exe"),
            &command_dir.path().join("dangling.exe"),
        ),
        "dangling",
    );
    create_link_or_skip(
        make_file_symlink(
            Path::new(r"\\server\share\remote.exe"),
            &command_dir.path().join("remote.exe"),
        ),
        "remote",
    );

    let path = std::env::join_paths([command_dir.path()]).unwrap();
    let pathext = OsString::from(".EXE");
    let index = CommandIndex::discover_with_env(&path, pathext.as_os_str());
    assert!(index.is_complete());
    let cargo_completion = index.complete("car", 3, Path::new("."), 20);
    assert!(cargo_completion.candidates.iter().any(|candidate| {
        candidate.label == "cargo" && candidate.kind == CandidateKind::Command
    }));
    let dangling_completion = index.complete("dang", 4, Path::new("."), 20);
    assert!(dangling_completion.candidates.is_empty());
    let remote_completion = index.complete("rem", 3, Path::new("."), 20);
    assert!(remote_completion.candidates.is_empty());
}

#[test]
fn filesystem_completion_follows_local_directory_symlinks() {
    let cwd = tempdir().unwrap();
    let real = cwd.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("inside.txt"), b"").unwrap();
    let linked = cwd.path().join("linked");
    if !create_link_or_skip(make_dir_symlink(Path::new("real"), &linked), "directory") {
        return;
    }

    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![shell_command("mytool", "Application", "")]);
    let completion = index.complete("mytool lin", 10, cwd.path(), 20);
    let candidate = completion
        .candidates
        .iter()
        .find(|candidate| candidate.label == "linked")
        .expect("linked directory should be offered");
    assert_eq!(candidate.kind, CandidateKind::Directory);
    assert_eq!(candidate.description, "目录");
    assert!(candidate.insert_text.ends_with(std::path::MAIN_SEPARATOR));
}

#[test]
fn description_overrides_use_exact_spec_keys() {
    let mut index = CommandIndex::default();
    index.merge_shell_commands(vec![shell_command("git", "Application", "")]);
    let overrides = BTreeMap::from([
        ("git checkout".to_owned(), "CHECKOUT OVERRIDE".to_owned()),
        ("git -C".to_owned(), "CAPITAL C".to_owned()),
        ("git -c".to_owned(), "LOWER C".to_owned()),
    ]);

    let checkout = index.complete_with_descriptions("git ch", 6, Path::new("."), 20, &overrides);
    let checkout = checkout
        .candidates
        .iter()
        .find(|candidate| candidate.label == "checkout")
        .expect("git checkout candidate");
    assert_eq!(checkout.description, "CHECKOUT OVERRIDE");

    let options = index.complete_with_descriptions("git -", 5, Path::new("."), 100, &overrides);
    let upper = options
        .candidates
        .iter()
        .find(|candidate| candidate.label == "-C")
        .expect("git -C candidate");
    let lower = options
        .candidates
        .iter()
        .find(|candidate| candidate.label == "-c")
        .expect("git -c candidate");
    assert_eq!(upper.description, "CAPITAL C");
    assert_eq!(lower.description, "LOWER C");
}

#[test]
fn inline_choice_replaces_the_whole_option_and_preserves_context_override() {
    let index = CommandIndex::default();
    let overrides = BTreeMap::from([(
        "git status --ignored matching".to_owned(),
        "显示匹配忽略规则的项目".to_owned(),
    )]);
    let line = "git status --ignored=mat";
    let completion =
        index.complete_with_descriptions(line, line.len(), Path::new("."), 30, &overrides);
    let selected = completion
        .candidates
        .iter()
        .find(|c| c.label == "--ignored=matching")
        .unwrap();
    let replaced = format!(
        "{}{}{}",
        &line[..completion.replace_start],
        selected.insert_text,
        &line[completion.replace_end..]
    );
    assert_eq!(replaced, "git status --ignored=matching");
    assert_eq!(selected.description, "显示匹配忽略规则的项目");
}

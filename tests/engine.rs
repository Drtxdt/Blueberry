use shellsense::engine::CommandIndex;
use shellsense::model::{CandidateKind, ShellCommand};
use std::ffi::OsString;
use std::fs;
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
    assert_eq!(completion.candidates[0].description, "updated");
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

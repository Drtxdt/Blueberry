use shellsense::{
    engine::CommandIndex,
    knowledge::{self, Entry, Record},
    model::{Candidate, CandidateKind},
    spec_catalog::Catalog,
};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

fn record(text: &str) -> Record {
    Record {
        parser_version: 1,
        entry: Entry {
            command: "codex".into(),
            path: PathBuf::from("codex-a.exe"),
            target: PathBuf::from("codex-a.exe"),
            fingerprint: "test".into(),
            trusted: false,
            script: None,
        },
        context: Vec::new(),
        page: knowledge::parse_help(text),
        fetched_at: 0,
        error: None,
    }
}

#[test]
fn codex_options_values_and_pending_directory_are_distinct() {
    let catalog = Catalog::builtin();
    let root = catalog.lookup("codex", &[], "--").unwrap();
    assert!(root.candidates.iter().any(|c| c.name == "--model"));
    let exec = catalog.lookup("codex", &["exec".into()], "--").unwrap();
    assert!(exec.candidates.iter().any(|c| c.name == "--json"));
    assert!(!root.candidates.iter().any(|c| c.name == "--json"));
    let values = catalog.lookup("codex", &["--sandbox".into()], "").unwrap();
    assert!(values.candidates.iter().any(|c| c.name == "read-only"));
    let dir = catalog.lookup("codex", &["--cd".into()], "").unwrap();
    assert!(dir.directories_only);
    assert!(dir.argument_hint.contains("<DIR>"));
    assert!(dir.candidates.is_empty());
    for root in ["python", "uv", "rustc", "winget", "dotnet"] {
        assert!(catalog.lookup(root, &[], "--").is_some());
    }
}

#[test]
fn current_help_keeps_chinese_but_restricts_only_complete_contexts() {
    let mut catalog = Catalog::builtin().clone();
    let page = record(
        "Codex\nOptions:\n  --model <MODEL>  Choose the model\n  --new-flag  A new feature\n",
    );
    catalog.apply_help(&page);
    let result = catalog.lookup("codex", &[], "--").unwrap();
    assert!(
        result
            .candidates
            .iter()
            .any(|c| c.name == "--model" && c.description.contains("模型"))
    );
    assert!(result.candidates.iter().any(|c| c.name == "--new-flag"));
    assert!(!result.candidates.iter().any(|c| c.name == "--sandbox"));
    assert!(
        catalog
            .lookup("codex", &[], "-m")
            .unwrap()
            .candidates
            .is_empty(),
        "removed short aliases must not survive authoritative help"
    );
    // A partial page cannot remove installed/manual knowledge.
    let mut partial = record("  --new-flag  New feature");
    partial.page.complete = false;
    let mut catalog = Catalog::builtin().clone();
    catalog.apply_help(&partial);
    assert!(
        catalog
            .lookup("codex", &[], "--sandbox")
            .unwrap()
            .candidates
            .iter()
            .any(|c| c.name == "--sandbox")
    );
}

#[test]
fn user_rules_remain_authoritative() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("custom.toml"),"schema_version=1\n[[nodes]]\npath='codex'\ndescription='自定义助手'\n[[nodes.options]]\nname='--custom'\ndescription='自定义参数'\nvalue_kind='none'\n").unwrap();
    let mut catalog = Catalog::load_user_dir(dir.path()).unwrap();
    catalog.apply_help(&record("Codex\nOptions:\n  --other  Other option\n"));
    let result = catalog.lookup("codex", &[], "--").unwrap();
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].name, "--custom");
}

#[test]
fn purpose_search_respects_context_availability_and_suffix() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("git.exe"), b"fixture").unwrap();
    let index = CommandIndex::discover_with_env(dir.path().as_os_str(), OsStr::new(".EXE"));
    let input = "查看分支 --all";
    let result = index.purpose_search(
        Catalog::builtin(),
        input,
        "查看分支".len(),
        None,
        10,
        &BTreeMap::new(),
    );
    let candidate = result
        .candidates
        .iter()
        .find(|c| c.insert_text == "git branch")
        .unwrap();
    assert!(!candidate.match_reason.is_empty());
    assert_eq!(
        format!(
            "{}{}{}",
            &input[..result.replace_start],
            candidate.insert_text,
            &input[result.replace_end..]
        ),
        "git branch --all"
    );
    assert!(
        index
            .purpose_search(Catalog::builtin(), "Python", 6, None, 10, &BTreeMap::new())
            .candidates
            .is_empty()
    );
    let input = "git 撤销";
    let result = index.purpose_search(
        Catalog::builtin(),
        input,
        input.len(),
        None,
        10,
        &BTreeMap::new(),
    );
    assert!(result.candidates.iter().any(|c| c.insert_text == "restore"));
    assert!(
        result
            .candidates
            .iter()
            .all(|c| !c.insert_text.starts_with("git "))
    );
    let input = "git --format 查找";
    let result = index.purpose_search(
        Catalog::builtin(),
        input,
        input.len(),
        None,
        10,
        &BTreeMap::new(),
    );
    assert!(
        result
            .candidates
            .iter()
            .all(|c| c.kind != CandidateKind::Command)
    );
}

#[test]
fn parser_handles_wraps_ansi_nonzero_style_and_conservative_optional_values() {
    let page = knowledge::parse_help(
        "CLI\nCommands:\n  apply  Apply changes to\n         working tree\nOptions:\n  --policy <POLICY>\n    Choose a policy\n    Possible values:\n    - never: Never ask\n    - untrusted: Ask first\n  --flag  \x1b[31mEnglish help\x1b[0m\n",
    );
    assert_eq!(page.commands.len(), 1);
    assert_eq!(page.options[0].values, ["never", "untrusted"]);
    assert_eq!(page.options[1].description, "English help");
    assert!(
        !knowledge::parse_help("Options:\n  --foo[=<BAR>]  Optional\n  --flag  Flag\n").complete
    );
    let page = knowledge::parse_help("用法\n选项：\n  --name <NAME>  选择名称\n");
    assert!(page.complete);
    assert_eq!(page.options[0].description, "选择名称");
}

#[test]
fn entry_fingerprints_change_and_same_basename_is_not_trust() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("codex.exe");
    let b = dir.path().join("other/codex.exe");
    fs::create_dir(b.parent().unwrap()).unwrap();
    fs::write(&a, b"one").unwrap();
    fs::write(&b, b"two").unwrap();
    let first = knowledge::entry("codex", &a, Path::new("C:/outside")).unwrap();
    let other = knowledge::entry("codex", &b, Path::new("C:/outside")).unwrap();
    assert!(!first.trusted);
    assert_ne!(first.fingerprint, other.fingerprint);
    fs::write(&a, b"new release").unwrap();
    assert_ne!(
        first.fingerprint,
        knowledge::entry("codex", &a, dir.path())
            .unwrap()
            .fingerprint
    );
}

#[test]
fn tooltip_summary_retains_original_help_and_sources() {
    let mut candidate = Candidate {
        label: "--flag".into(),
        description: "Enable useful output. More information.\nNext paragraph".into(),
        ..Default::default()
    };
    knowledge::native_description(&mut candidate);
    assert_eq!(candidate.description, "Enable useful output.");
    assert!(candidate.detail.contains("Next paragraph"));
    assert_eq!(candidate.language, "en");
}

#[cfg(windows)]
#[test]
fn bounded_child_accepts_nonzero_help_exit_and_enforces_output_limit() {
    let dir = tempfile::tempdir().unwrap();
    let shell = PathBuf::from("pwsh.exe");
    let args = [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "[Console]::Error.WriteLine('Options:'); [Console]::Error.WriteLine('  --flag  help'); exit 129",
    ]
    .map(String::from);
    let text = knowledge::capture(&shell, &args, dir.path(), &AtomicBool::new(false)).unwrap();
    assert!(knowledge::parse_help(&text).complete);
    let args = [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "[Console]::Write(('x' * 300000))",
    ]
    .map(String::from);
    assert!(
        knowledge::capture(&shell, &args, dir.path(), &AtomicBool::new(false))
            .unwrap_err()
            .contains("256")
    );
    let args = [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Start-Sleep -Seconds 15",
    ]
    .map(String::from);
    let start = std::time::Instant::now();
    assert!(knowledge::capture(&shell, &args, dir.path(), &AtomicBool::new(false)).is_err());
    assert!(start.elapsed().as_secs() < 5);
}

#[test]
fn sectioned_commands_without_descriptions_are_not_dropped() {
    let page = knowledge::parse_help(
        "Manage servers\nCommands:\n  list    \n  get     \n  help    Print help\nOptions:\n  --help  Help\n",
    );
    assert_eq!(page.commands.len(), 3);
    assert_eq!(page.commands["list"], "子命令");
}

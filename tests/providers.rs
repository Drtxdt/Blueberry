use shellsense::model::CandidateKind;
use shellsense::providers::{
    ProjectQuery, ProviderCache, collect, collect_snapshot, normalize_query, normalized_args,
    project_key, snapshot_fingerprint,
};
use shellsense::{
    completion::{merge, plan},
    engine::CommandIndex,
    sources,
    spec_catalog::Catalog,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, atomic::AtomicBool};
use tempfile::tempdir;

fn query(command: &str, args: &[&str], prefix: &str, cwd: &Path) -> ProjectQuery {
    ProjectQuery::new(
        7,
        command,
        args.iter().copied(),
        prefix,
        cwd.to_path_buf(),
        BTreeMap::new(),
    )
}

fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .expect("git is installed for provider fixtures");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_output(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git is installed for provider fixtures");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout)
        .expect("git fixture output is UTF-8")
        .trim()
        .to_owned()
}

fn set_provider(query: &mut ProjectQuery, provider: &str) {
    query.provider = Some(provider.to_owned());
}

#[test]
fn git_provider_reads_branches_tags_remotes_and_status_paths() {
    let root = tempdir().unwrap();
    git(root.path(), &["init"]);
    git(
        root.path(),
        &["config", "user.email", "shellsense@example.invalid"],
    );
    git(root.path(), &["config", "user.name", "ShellSense Test"]);
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-m", "initial"]);
    git(root.path(), &["switch", "-c", "feature/demo"]);
    git(root.path(), &["tag", "v1.0.0"]);
    git(
        root.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    fs::write(root.path().join("src/untracked.txt"), "x").unwrap();

    let cancelled = AtomicBool::new(false);
    let branches = collect(&query("git", &["switch"], "fea", root.path()), &cancelled);
    assert!(
        branches
            .candidates
            .iter()
            .any(|candidate| candidate.value == "feature/demo")
    );
    assert!(branches.project_root.is_some());
    assert!(
        branches
            .watch_paths
            .iter()
            .any(|path| path == &root.path().join(".git"))
    );
    assert!(
        branches.recursive_watch_paths.is_empty(),
        "ref-only Git queries must not subscribe to the whole worktree"
    );
    let tags = collect(&query("git", &["tag", "-d"], "v1", root.path()), &cancelled);
    assert!(
        tags.candidates
            .iter()
            .any(|candidate| candidate.value == "v1.0.0")
    );
    let remotes = collect(&query("git", &["push"], "ori", root.path()), &cancelled);
    assert!(
        remotes
            .candidates
            .iter()
            .any(|candidate| candidate.value == "origin")
    );
    let paths = collect(&query("git", &["status"], "src/", root.path()), &cancelled);
    assert!(
        paths
            .candidates
            .iter()
            .any(|candidate| candidate.value == "src/untracked.txt")
    );
    assert!(
        paths
            .candidates
            .iter()
            .all(|candidate| candidate.kind == CandidateKind::File)
    );
    assert!(
        paths
            .recursive_watch_paths
            .iter()
            .any(|path| path == root.path()),
        "Git status paths need recursive worktree invalidation"
    );
}

#[test]
fn git_nonzero_query_is_incomplete_and_cannot_enter_the_cache() {
    let root = tempdir().unwrap();
    let request = query("git", &["switch"], "fea", root.path());
    let result = collect(&request, &AtomicBool::new(false));
    assert!(result.candidates.is_empty());
    assert!(
        result.incomplete,
        "a non-repository Git query must not be complete"
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("非零状态")),
        "missing nonzero Git diagnostic: {:?}",
        result.diagnostics
    );

    let mut cache = ProviderCache::default();
    assert!(!cache.insert(&request, result));
    assert!(cache.get(&request).is_none());
}

#[test]
fn git_dynamic_provider_is_reached_through_completion_plan_and_respects_context() {
    let root = tempdir().unwrap();
    git(root.path(), &["init"]);
    git(
        root.path(),
        &["config", "user.email", "shellsense@example.invalid"],
    );
    git(root.path(), &["config", "user.name", "ShellSense Test"]);
    fs::write(root.path().join("README.md"), "integration fixture\n").unwrap();
    git(root.path(), &["add", "README.md"]);
    git(root.path(), &["commit", "-m", "initial"]);
    git(root.path(), &["switch", "-c", "feature/demo"]);
    git(root.path(), &["branch", "mainline"]);
    git(
        root.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );

    let quoted = root.path().join("quoted path");
    fs::create_dir(&quoted).unwrap();
    git(&quoted, &["init"]);
    git(
        &quoted,
        &["config", "user.email", "shellsense@example.invalid"],
    );
    git(&quoted, &["config", "user.name", "ShellSense Test"]);
    fs::write(quoted.join("README.md"), "quoted fixture\n").unwrap();
    git(&quoted, &["add", "README.md"]);
    git(&quoted, &["commit", "-m", "initial"]);
    git(&quoted, &["switch", "-c", "feature/quoted"]);

    let index =
        CommandIndex::discover_with_env(std::ffi::OsStr::new(""), std::ffi::OsStr::new(".EXE"));
    let catalog = Catalog::builtin();
    let environment = Arc::new(BTreeMap::new());
    let descriptions = BTreeMap::new();
    let complete = |line: &str, cwd: &Path, wanted: &str| {
        let (base, request) = plan(
            &index,
            catalog,
            line,
            line.len(),
            cwd,
            None,
            1,
            environment.clone(),
            true,
            true,
            &descriptions,
        );
        assert!(
            request.project,
            "dynamic project source not requested: {line}"
        );
        assert_eq!(request.provider.as_deref(), Some("git"), "{line}");
        let parts = sources::collect_once(&request);
        let merged = merge(
            base,
            &request,
            [Some(&parts[0]), Some(&parts[1])],
            None,
            256,
            &descriptions,
        );
        assert!(!merged.incomplete, "{line}: {:?}", parts[0].diagnostics);
        assert!(
            merged
                .candidates
                .iter()
                .any(|candidate| candidate.label == wanted),
            "{line}: {:?}; {:?}",
            merged.candidates,
            parts[0].diagnostics
        );
    };

    complete("git switch fe", root.path(), "feature/demo");
    complete("git push origin ma", root.path(), "mainline");
    complete(
        "git -C \"quoted path\" switch fe",
        root.path(),
        "feature/quoted",
    );

    let (base, request) = plan(
        &index,
        catalog,
        "git log --format j",
        "git log --format j".len(),
        root.path(),
        None,
        1,
        environment.clone(),
        true,
        true,
        &descriptions,
    );
    assert!(
        !request.project,
        "format value must not launch Git provider"
    );
    assert!(request.provider.is_none());
    let parts = sources::collect_once(&request);
    let merged = merge(
        base,
        &request,
        [Some(&parts[0]), Some(&parts[1])],
        None,
        256,
        &descriptions,
    );
    assert!(!merged.incomplete);
    assert!(
        merged
            .candidates
            .iter()
            .all(|candidate| candidate.label != "feature/demo")
    );
}

#[test]
fn git_dynamic_context_does_not_mix_refs_with_paths_or_creation_names() {
    let root = tempdir().unwrap();
    git(root.path(), &["init"]);
    git(
        root.path(),
        &["config", "user.email", "shellsense@example.invalid"],
    );
    git(root.path(), &["config", "user.name", "ShellSense Test"]);
    fs::write(root.path().join("README.md"), "context fixture\n").unwrap();
    git(root.path(), &["add", "README.md"]);
    git(root.path(), &["commit", "-m", "initial"]);
    git(root.path(), &["switch", "-c", "feature/demo"]);
    git(root.path(), &["tag", "v1.0.0"]);
    fs::write(root.path().join("feature.txt"), "path\n").unwrap();

    let cancelled = AtomicBool::new(false);
    let checkout_paths = collect(
        &query("git", &["checkout", "--"], "fea", root.path()),
        &cancelled,
    );
    assert!(!checkout_paths.incomplete);
    assert!(
        checkout_paths
            .candidates
            .iter()
            .all(|candidate| candidate.value != "feature/demo")
    );
    for command in ["log", "show"] {
        let paths = collect(
            &query("git", &[command, "--"], "fea", root.path()),
            &cancelled,
        );
        assert!(!paths.incomplete);
        assert!(
            paths
                .candidates
                .iter()
                .all(|candidate| candidate.value != "feature/demo")
        );
    }

    for command in ["switch", "merge"] {
        let refs = collect(
            &query("git", &[command, "--"], "fea", root.path()),
            &cancelled,
        );
        assert!(
            refs.candidates
                .iter()
                .any(|candidate| candidate.value == "feature/demo")
        );
    }
    let diff = collect(&query("git", &["diff"], "fea", root.path()), &cancelled);
    assert!(
        diff.candidates
            .iter()
            .any(|candidate| candidate.value == "feature/demo")
    );
    let branch_create = collect(&query("git", &["branch"], "fea", root.path()), &cancelled);
    assert!(branch_create.candidates.is_empty());
    let branch_delete = collect(
        &query("git", &["branch", "-d"], "fea", root.path()),
        &cancelled,
    );
    assert!(
        branch_delete
            .candidates
            .iter()
            .any(|candidate| candidate.value == "feature/demo")
    );
    let branch_rename_source = collect(
        &query("git", &["branch", "-m"], "fea", root.path()),
        &cancelled,
    );
    assert!(
        branch_rename_source
            .candidates
            .iter()
            .any(|candidate| candidate.value == "feature/demo")
    );
    let branch_rename_destination = collect(
        &query("git", &["branch", "-m", "old-name"], "fea", root.path()),
        &cancelled,
    );
    assert!(branch_rename_destination.candidates.is_empty());

    let tag_create = collect(&query("git", &["tag"], "v", root.path()), &cancelled);
    assert!(tag_create.candidates.is_empty());
    let tag_delete = collect(&query("git", &["tag", "-d"], "v", root.path()), &cancelled);
    assert!(
        tag_delete
            .candidates
            .iter()
            .any(|candidate| candidate.value == "v1.0.0")
    );

    let worktree_path = collect(
        &query("git", &["worktree", "add"], "fea", root.path()),
        &cancelled,
    );
    assert!(worktree_path.candidates.is_empty());
    let worktree_ref = collect(
        &query(
            "git",
            &["worktree", "add", "../new-worktree"],
            "fea",
            root.path(),
        ),
        &cancelled,
    );
    assert!(
        worktree_ref
            .candidates
            .iter()
            .any(|candidate| candidate.value == "feature/demo")
    );
    let worktree_path_after_branch_option = collect(
        &query(
            "git",
            &["worktree", "add", "-b", "new-branch"],
            "fea",
            root.path(),
        ),
        &cancelled,
    );
    assert!(worktree_path_after_branch_option.candidates.is_empty());
}

#[test]
fn cargo_provider_reads_workspace_members_features_and_targets() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("crates")).unwrap();
    let member = root.path().join("crates/app");
    fs::create_dir_all(member.join("src/bin")).unwrap();
    fs::create_dir(member.join("tests")).unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    )
    .unwrap();
    fs::write(
        member.join("Cargo.toml"),
        "[package]\nname = \"demo-app\"\nversion = \"0.1.0\"\n\n[features]\nfast = []\njson = []\n\n[[bin]]\nname = \"worker\"\n\n[[test]]\nname = \"integration\"\n",
    )
    .unwrap();
    fs::write(member.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(member.join("src/bin/worker.rs"), "fn main() {}\n").unwrap();
    fs::write(
        member.join("tests/integration.rs"),
        "#[test] fn it_works() {}\n",
    )
    .unwrap();

    let cancelled = AtomicBool::new(false);
    let package = collect(
        &query("cargo", &["build", "-p"], "demo", root.path()),
        &cancelled,
    );
    assert!(
        package
            .candidates
            .iter()
            .any(|candidate| candidate.value == "demo-app")
    );
    let feature = collect(
        &query("cargo", &["build", "--features"], "f", root.path()),
        &cancelled,
    );
    assert!(
        feature
            .candidates
            .iter()
            .any(|candidate| candidate.value == "fast")
    );
    let test = collect(
        &query("cargo", &["test", "--test"], "inte", root.path()),
        &cancelled,
    );
    assert!(
        test.candidates
            .iter()
            .any(|candidate| candidate.value == "integration")
    );
    assert_eq!(feature.project_root.as_deref(), Some(root.path()));
    assert!(
        feature
            .watch_paths
            .iter()
            .any(|path| path == &member.join("src/bin"))
    );
}

#[test]
fn npm_and_pnpm_providers_read_local_manifests_without_running_scripts() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("packages")).unwrap();
    let app = root.path().join("packages/app");
    fs::create_dir(&app).unwrap();
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"root","private":true,"scripts":{"dev":"node dev.js","check":"echo ok"},"workspaces":["packages/*"],"dependencies":{"local-lib":"workspace:*","serde":"^1"}}"#,
    )
    .unwrap();
    fs::write(
        app.join("package.json"),
        r#"{"name":"@demo/app","scripts":{"test:unit":"node test.js"},"dependencies":{"local-lib":"file:../local-lib"}}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("pnpm-workspace.yaml"),
        "packages:\n  - 'packages/*'\n",
    )
    .unwrap();

    let cancelled = AtomicBool::new(false);
    let scripts = collect(&query("npm", &["run"], "tes", root.path()), &cancelled);
    assert!(
        scripts
            .candidates
            .iter()
            .any(|candidate| candidate.value == "test:unit")
    );
    let dependency = collect(&query("pnpm", &["add"], "loc", root.path()), &cancelled);
    assert!(
        dependency
            .candidates
            .iter()
            .any(|candidate| candidate.value == "local-lib")
    );
    let workspace = collect(
        &query("pnpm", &["--filter"], "@demo", root.path()),
        &cancelled,
    );
    assert!(
        workspace
            .candidates
            .iter()
            .any(|candidate| candidate.value == "@demo/app")
    );
    assert!(scripts.watch_paths.iter().any(|path| path == &app));

    let npm_target = collect(
        &query("npm", &["--workspace", "@demo/app", "run"], "", root.path()),
        &cancelled,
    );
    assert!(
        npm_target
            .candidates
            .iter()
            .any(|candidate| candidate.value == "test:unit")
    );
    assert!(
        npm_target
            .candidates
            .iter()
            .all(|candidate| { !matches!(candidate.value.as_str(), "dev" | "check") })
    );

    let pnpm_target = collect(
        &query("pnpm", &["--filter", "@demo/app", "run"], "", root.path()),
        &cancelled,
    );
    assert!(
        pnpm_target
            .candidates
            .iter()
            .any(|candidate| candidate.value == "test:unit")
    );
    assert!(
        pnpm_target
            .candidates
            .iter()
            .all(|candidate| { !matches!(candidate.value.as_str(), "dev" | "check") })
    );
}

#[test]
fn empty_project_snapshots_watch_ancestors_for_new_manifests() {
    let cargo_root = tempdir().unwrap();
    let cargo_nested = cargo_root.path().join("nested");
    fs::create_dir(&cargo_nested).unwrap();

    let mut cargo_request = query("cargo", &["build", "--features"], "new", &cargo_nested);
    set_provider(&mut cargo_request, "cargo.features");
    let cargo_empty = collect(&cargo_request, &AtomicBool::new(false));
    assert!(cargo_empty.candidates.is_empty());
    assert!(
        cargo_empty
            .watch_paths
            .iter()
            .any(|path| path == &cargo_nested)
    );
    assert!(
        cargo_empty
            .watch_paths
            .iter()
            .any(|path| path == cargo_root.path())
    );

    let node_root = tempdir().unwrap();
    let node_nested = node_root.path().join("nested");
    fs::create_dir(&node_nested).unwrap();
    let mut npm_request = query("npm", &["run"], "new", &node_nested);
    set_provider(&mut npm_request, "npm.scripts");
    let npm_empty = collect(&npm_request, &AtomicBool::new(false));
    assert!(npm_empty.candidates.is_empty());
    assert!(
        npm_empty
            .watch_paths
            .iter()
            .any(|path| path == &node_nested)
    );
    assert!(
        npm_empty
            .watch_paths
            .iter()
            .any(|path| path == node_root.path())
    );

    let mut cache = ProviderCache::default();
    assert!(cache.insert_project(&cargo_request, cargo_empty));
    assert!(cache.insert_project(&npm_request, npm_empty));
    assert!(cache.get_project(&cargo_request).is_some());
    assert!(cache.get_project(&npm_request).is_some());

    fs::write(
        cargo_root.path().join("Cargo.toml"),
        "[package]\nname = \"new-cargo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    cache.invalidate_path(&cargo_root.path().join("Cargo.toml"));
    assert!(cache.get_project(&cargo_request).is_none());
    fs::write(
        node_root.path().join("package.json"),
        r#"{"name":"new-node","scripts":{"build":"echo"}}"#,
    )
    .unwrap();
    cache.invalidate_path(&node_root.path().join("package.json"));
    assert!(cache.get_project(&npm_request).is_none());
}

#[test]
fn recursive_workspace_scan_skips_generated_and_metadata_trees() {
    let root = tempdir().unwrap();
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"root","private":true,"workspaces":["**","**/target/declared","**/node_modules/declared"]}"#,
    )
    .unwrap();
    let kept = root.path().join("packages/kept");
    fs::create_dir_all(&kept).unwrap();
    fs::write(
        kept.join("package.json"),
        r#"{"name":"@demo/kept","scripts":{"kept:check":"echo"}}"#,
    )
    .unwrap();

    for (directory, name, script) in [
        ("target/declared", "target-declared", "target:declared"),
        ("node_modules/declared", "node-declared", "node:declared"),
    ] {
        let directory = root.path().join(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("package.json"),
            format!(r#"{{"name":"{name}","scripts":{{"{script}":"echo"}}}}"#),
        )
        .unwrap();
    }

    for directory in [
        "node_modules/hidden",
        "target/hidden",
        ".git/hidden",
        ".shellsense/hidden",
    ] {
        let directory = root.path().join(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("package.json"),
            format!(
                r#"{{"name":"hidden-{}","scripts":{{"hidden:{}":"echo"}}}}"#,
                directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("tree"),
                directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("tree")
            ),
        )
        .unwrap();
    }

    let mut request = query("npm", &["run"], "", root.path());
    set_provider(&mut request, "npm.scripts");
    let result = collect(&request, &AtomicBool::new(false));
    assert!(
        !result.incomplete,
        "workspace diagnostics: {:?}",
        result.diagnostics
    );
    assert!(
        result
            .candidates
            .iter()
            .any(|candidate| candidate.value == "kept:check")
    );
    assert!(result.candidates.iter().any(|candidate| {
        matches!(
            candidate.value.as_str(),
            "target:declared" | "node:declared"
        )
    }));
    assert!(
        result
            .candidates
            .iter()
            .all(|candidate| { !candidate.value.starts_with("hidden:") })
    );
}

#[test]
fn pnpm_rootless_workspace_uses_workspace_root_and_handles_workspace_root_flag() {
    let root = tempdir().unwrap();
    let first = root.path().join("packages/first");
    let second = root.path().join("packages/second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(
        first.join("package.json"),
        r#"{"name":"@demo/first","scripts":{"first:check":"echo first"}}"#,
    )
    .unwrap();
    fs::write(
        second.join("package.json"),
        r#"{"name":"@demo/second","scripts":{"second:check":"echo second"}}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("pnpm-workspace.yaml"),
        "packages:\n  - 'packages/*'\n",
    )
    .unwrap();

    let cancelled = AtomicBool::new(false);
    let result = collect(&query("pnpm", &["-w", "run"], "second", &first), &cancelled);
    assert!(
        result
            .candidates
            .iter()
            .any(|candidate| candidate.value == "second:check")
    );
    assert_eq!(result.project_root.as_deref(), Some(root.path()));
    assert!(result.watch_paths.iter().any(|path| path == &second));
}

#[test]
fn cancellation_is_incomplete_and_incomplete_results_are_not_cached() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("package.json"), "{invalid").unwrap();
    let cancelled = AtomicBool::new(true);
    let result = collect(&query("npm", &["run"], "", root.path()), &cancelled);
    assert!(result.incomplete);
    let mut cache = ProviderCache::with_capacity(2);
    assert!(!cache.insert(&query("npm", &["run"], "", root.path()), result));
    assert!(cache.is_empty());
}

#[test]
fn project_cache_key_reuses_across_prefixes() {
    let root = tempdir().unwrap();
    fs::write(
        root.path().join("package.json"),
        r#"{"scripts":{"build":"echo"}}"#,
    )
    .unwrap();
    let first = query("npm", &["run"], "b", root.path());
    let second = query("npm", &["run"], "bu", root.path());
    assert_eq!(project_key(&first), project_key(&second));
    assert_eq!(snapshot_fingerprint(&first), snapshot_fingerprint(&first));
    let snapshot = collect_snapshot(&first, &AtomicBool::new(false));
    let mut cache = ProviderCache::default();
    assert!(cache.insert_project(&first, snapshot));
    assert!(cache.get_project(&second).is_some());
}

#[test]
fn cargo_snapshot_keeps_value_slot_and_csv_prefix_while_clearing_fragment() {
    let root = tempdir().unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[features]\nfast = []\njson = []\n",
    )
    .unwrap();
    let first = query("cargo", &["build", "--features"], "fast,js", root.path());
    let second = query("cargo", &["build", "--features"], "fast,jso", root.path());
    assert_eq!(project_key(&first), project_key(&second));
    let snapshot = collect_snapshot(&first, &AtomicBool::new(false));
    assert!(
        snapshot
            .candidates
            .iter()
            .any(|candidate| candidate.value == "fast")
    );
    assert!(
        snapshot
            .candidates
            .iter()
            .any(|candidate| candidate.value == "json")
    );
    assert!(
        snapshot
            .candidates
            .iter()
            .all(|candidate| !candidate.value.starts_with("fast,"))
    );
}

#[test]
fn inline_cargo_selectors_normalize_to_distinct_snapshot_slots() {
    let root = tempdir().unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[features]\nfast = []\njson = []\n",
    )
    .unwrap();
    let features = query("cargo", &["build"], "--features=fast,js", root.path());
    let packages = query("cargo", &["build"], "--package=de", root.path());
    assert_eq!(
        normalized_args(&features),
        vec!["build".to_owned(), "--features".to_owned()]
    );
    assert_eq!(
        normalized_args(&packages),
        vec!["build".to_owned(), "--package".to_owned()]
    );
    assert_ne!(project_key(&features), project_key(&packages));
    let normalized = normalize_query(&features);
    assert!(normalized.prefix.is_empty());
    assert_eq!(
        normalized.args.last().map(String::as_str),
        Some("--features")
    );
    let snapshot = collect_snapshot(&features, &AtomicBool::new(false));
    assert!(
        snapshot
            .candidates
            .iter()
            .any(|candidate| candidate.value == "fast")
    );
    assert!(snapshot.candidates.iter().all(|candidate| {
        !candidate.value.starts_with("--features=") && !candidate.value.contains(',')
    }));
}

#[test]
fn git_worktree_pointer_tracks_gitdir_and_common_directory() {
    let root = tempdir().unwrap();
    git(root.path(), &["init"]);
    git(
        root.path(),
        &["config", "user.email", "shellsense@example.invalid"],
    );
    git(root.path(), &["config", "user.name", "ShellSense Test"]);
    fs::write(root.path().join("README.md"), "worktree fixture\n").unwrap();
    git(root.path(), &["add", "README.md"]);
    git(root.path(), &["commit", "-m", "initial"]);

    let linked = root.path().join("linked");
    let linked_text = linked.to_string_lossy().into_owned();
    git(
        root.path(),
        &[
            "worktree",
            "add",
            "-b",
            "linked-branch",
            linked_text.as_str(),
        ],
    );

    let pointer = fs::read_to_string(linked.join(".git")).unwrap();
    let pointer_value = pointer
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .expect("linked worktree has a gitdir pointer");
    let gitdir = if Path::new(pointer_value).is_absolute() {
        PathBuf::from(pointer_value)
    } else {
        linked.join(pointer_value)
    };
    let gitdir = fs::canonicalize(gitdir).unwrap();
    let common_value = fs::read_to_string(gitdir.join("commondir"))
        .unwrap()
        .trim()
        .to_owned();
    let common = if Path::new(&common_value).is_absolute() {
        PathBuf::from(common_value)
    } else {
        gitdir.join(common_value)
    };
    let common = fs::canonicalize(common).unwrap();

    let mut request = query("git", &["worktree", "list"], "", &linked);
    set_provider(&mut request, "git.worktrees");
    let result = collect(&request, &AtomicBool::new(false));
    assert!(
        !result.incomplete,
        "worktree query diagnostics: {:?}",
        result.diagnostics
    );
    assert_eq!(result.project_root.as_deref(), Some(linked.as_path()));
    assert!(
        result.watch_paths.iter().any(|path| {
            fs::canonicalize(path)
                .map(|candidate| candidate == gitdir)
                .unwrap_or(false)
        }),
        "watch paths did not include linked gitdir: {:?}",
        result.watch_paths
    );
    assert!(
        result.watch_paths.iter().any(|path| {
            fs::canonicalize(path)
                .map(|candidate| candidate == common)
                .unwrap_or(false)
        }),
        "watch paths did not include git common dir: {:?}",
        result.watch_paths
    );
    assert!(result.candidates.iter().any(|candidate| {
        Path::new(&candidate.value)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("linked"))
    }));
}

#[test]
fn git_packed_refs_keep_local_branches_and_tags_visible() {
    let root = tempdir().unwrap();
    git(root.path(), &["init"]);
    git(
        root.path(),
        &["config", "user.email", "shellsense@example.invalid"],
    );
    git(root.path(), &["config", "user.name", "ShellSense Test"]);
    fs::write(root.path().join("file.txt"), "packed refs\n").unwrap();
    git(root.path(), &["add", "file.txt"]);
    git(root.path(), &["commit", "-m", "initial"]);
    git(root.path(), &["branch", "feature/packed"]);
    git(root.path(), &["tag", "v-packed"]);
    git(root.path(), &["pack-refs", "--all", "--prune"]);

    let packed = fs::read_to_string(root.path().join(".git/packed-refs")).unwrap();
    assert!(packed.contains("refs/heads/feature/packed"));
    assert!(packed.contains("refs/tags/v-packed"));

    let mut branch_request = query("git", &["merge"], "feature/", root.path());
    set_provider(&mut branch_request, "git.refs");
    let branches = collect(&branch_request, &AtomicBool::new(false));
    assert!(
        !branches.incomplete,
        "packed branch diagnostics: {:?}",
        branches.diagnostics
    );
    assert!(
        branches
            .candidates
            .iter()
            .any(|candidate| candidate.value == "feature/packed")
    );

    let mut tag_request = query("git", &["merge"], "v-packed", root.path());
    set_provider(&mut tag_request, "git.tags");
    let tags = collect(&tag_request, &AtomicBool::new(false));
    assert!(
        !tags.incomplete,
        "packed tag diagnostics: {:?}",
        tags.diagnostics
    );
    assert!(
        tags.candidates
            .iter()
            .any(|candidate| candidate.value == "v-packed")
    );
}

#[test]
fn git_ref_and_remote_cache_entries_are_removed_after_invalidation() {
    let root = tempdir().unwrap();
    git(root.path(), &["init"]);
    git(
        root.path(),
        &["config", "user.email", "shellsense@example.invalid"],
    );
    git(root.path(), &["config", "user.name", "ShellSense Test"]);
    fs::write(root.path().join("file.txt"), "cache fixture\n").unwrap();
    git(root.path(), &["add", "file.txt"]);
    git(root.path(), &["commit", "-m", "initial"]);
    git(root.path(), &["branch", "stale-branch"]);
    git(root.path(), &["tag", "stale-tag"]);
    git(
        root.path(),
        &["remote", "add", "old", "https://example.invalid/old.git"],
    );

    let mut refs_request = query("git", &["merge"], "", root.path());
    set_provider(&mut refs_request, "git.refs");
    let mut remote_request = query("git", &["remote"], "", root.path());
    set_provider(&mut remote_request, "git.remotes");
    let before_refs = collect(&refs_request, &AtomicBool::new(false));
    let before_remotes = collect(&remote_request, &AtomicBool::new(false));
    assert!(
        before_refs
            .candidates
            .iter()
            .any(|candidate| candidate.value == "stale-branch")
    );
    assert!(
        before_refs
            .candidates
            .iter()
            .any(|candidate| candidate.value == "stale-tag")
    );
    assert!(
        before_remotes
            .candidates
            .iter()
            .any(|candidate| candidate.value == "old")
    );

    let mut cache = ProviderCache::with_capacity(4);
    assert!(cache.insert(&refs_request, before_refs));
    assert!(cache.insert(&remote_request, before_remotes));
    assert!(cache.get(&refs_request).is_some());
    assert!(cache.get(&remote_request).is_some());

    git(root.path(), &["branch", "fresh-branch"]);
    git(root.path(), &["tag", "fresh-tag"]);
    git(root.path(), &["branch", "-D", "stale-branch"]);
    git(root.path(), &["tag", "-d", "stale-tag"]);
    git(
        root.path(),
        &["remote", "add", "new", "https://example.invalid/new.git"],
    );
    git(root.path(), &["remote", "remove", "old"]);

    cache.invalidate_path(&root.path().join(".git"));
    assert!(cache.get(&refs_request).is_none());
    assert!(cache.get(&remote_request).is_none());

    let after_refs = collect(&refs_request, &AtomicBool::new(false));
    let after_remotes = collect(&remote_request, &AtomicBool::new(false));
    assert!(
        after_refs
            .candidates
            .iter()
            .any(|candidate| candidate.value == "fresh-branch")
    );
    assert!(
        after_refs
            .candidates
            .iter()
            .any(|candidate| candidate.value == "fresh-tag")
    );
    assert!(
        !after_refs
            .candidates
            .iter()
            .any(|candidate| candidate.value == "stale-branch")
    );
    assert!(
        !after_refs
            .candidates
            .iter()
            .any(|candidate| candidate.value == "stale-tag")
    );
    assert!(
        after_remotes
            .candidates
            .iter()
            .any(|candidate| candidate.value == "new")
    );
    assert!(
        !after_remotes
            .candidates
            .iter()
            .any(|candidate| candidate.value == "old")
    );
}

#[test]
fn git_refs_are_not_silently_truncated_above_four_thousand_candidates() {
    let root = tempdir().unwrap();
    git(root.path(), &["init"]);
    git(
        root.path(),
        &["config", "user.email", "shellsense@example.invalid"],
    );
    git(root.path(), &["config", "user.name", "ShellSense Test"]);
    fs::write(root.path().join("file.txt"), "many refs\n").unwrap();
    git(root.path(), &["add", "file.txt"]);
    git(root.path(), &["commit", "-m", "initial"]);
    let object = git_output(root.path(), &["rev-parse", "HEAD"]);

    let mut packed = String::from("# pack-refs with: peeled fully-peeled sorted\n");
    for index in 0..5_000 {
        packed.push_str(&format!("{object} refs/heads/bulk{index:05}\n"));
    }
    fs::write(root.path().join(".git/packed-refs"), packed).unwrap();

    let mut request = query("git", &["merge"], "bulk", root.path());
    set_provider(&mut request, "git.refs");
    let result = collect(&request, &AtomicBool::new(false));
    assert!(
        !result.incomplete,
        "large ref diagnostics: {:?}",
        result.diagnostics
    );
    assert!(result.candidates.len() >= 5_000);
    assert!(
        result
            .candidates
            .iter()
            .any(|candidate| candidate.value == "bulk04999")
    );
}

#[test]
fn cargo_nested_workspace_updates_and_invalid_manifests_are_reported() {
    let root = tempdir().unwrap();
    let app = root.path().join("crates/app");
    fs::create_dir_all(app.join("src")).unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    )
    .unwrap();
    fs::write(
        app.join("Cargo.toml"),
        "[package]\nname = \"nested-app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(app.join("src/main.rs"), "fn main() {}\n").unwrap();

    let mut request = query("cargo", &["build", "--package"], "nested", &app);
    set_provider(&mut request, "cargo.packages");
    let initial = collect(&request, &AtomicBool::new(false));
    assert_eq!(initial.project_root.as_deref(), Some(root.path()));
    assert!(
        initial
            .candidates
            .iter()
            .any(|candidate| candidate.value == "nested-app")
    );
    assert!(
        initial
            .watch_paths
            .iter()
            .any(|path| path == &root.path().join("crates")),
        "Cargo workspace globs must watch their expansion parent"
    );

    let mut cache = ProviderCache::default();
    assert!(cache.insert_project(
        &request,
        collect_snapshot(&request, &AtomicBool::new(false))
    ));

    let added = root.path().join("crates/added");
    fs::create_dir_all(added.join("src")).unwrap();
    fs::write(
        added.join("Cargo.toml"),
        "[package]\nname = \"nested-added\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(added.join("src/main.rs"), "fn main() {}\n").unwrap();
    cache.invalidate_path(&added);

    let mut added_request = request.clone();
    added_request.prefix.clear();
    let updated = collect(&added_request, &AtomicBool::new(false));
    assert!(
        !updated.incomplete,
        "updated Cargo diagnostics: {:?}",
        updated.diagnostics
    );
    assert!(
        updated
            .candidates
            .iter()
            .any(|candidate| candidate.value == "nested-added")
    );
    assert!(cache.get_project(&request).is_none());

    fs::write(added.join("Cargo.toml"), "[package\n").unwrap();
    let invalidated = root.path().join("crates/added/Cargo.toml");
    cache.invalidate_path(&invalidated);
    let invalid = collect(&added_request, &AtomicBool::new(false));
    assert!(invalid.incomplete);
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("解析 Cargo 清单"))
    );

    fs::write(
        added.join("Cargo.toml"),
        "[package]\nname = \"nested-added\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\", \"crates/missing\"]\n",
    )
    .unwrap();
    let missing = collect(&added_request, &AtomicBool::new(false));
    assert!(missing.incomplete);
    assert!(
        missing
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("成员不存在"))
    );
}

#[test]
fn npm_nested_workspace_updates_and_invalid_manifests_are_reported() {
    let root = tempdir().unwrap();
    let app = root.path().join("packages/app");
    let tool = app.join("sub/tool");
    fs::create_dir_all(&tool).unwrap();
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
    )
    .unwrap();
    fs::write(
        app.join("package.json"),
        r#"{"name":"@demo/app","workspaces":["sub/*"],"scripts":{"app:check":"echo app"}}"#,
    )
    .unwrap();
    fs::write(
        tool.join("package.json"),
        r#"{"name":"@demo/tool","scripts":{"tool:check":"echo tool"}}"#,
    )
    .unwrap();

    let mut request = query("npm", &["run"], "tool", &tool);
    set_provider(&mut request, "npm.scripts");
    let initial = collect(&request, &AtomicBool::new(false));
    assert_eq!(initial.project_root.as_deref(), Some(app.as_path()));
    assert!(
        initial
            .candidates
            .iter()
            .any(|candidate| candidate.value == "tool:check")
    );
    assert!(
        initial
            .watch_paths
            .iter()
            .any(|path| path == &app.join("sub")),
        "npm workspace globs must watch their expansion parent"
    );

    let mut cache = ProviderCache::default();
    assert!(cache.insert_project(
        &request,
        collect_snapshot(&request, &AtomicBool::new(false))
    ));

    let added = app.join("sub/added");
    fs::create_dir_all(&added).unwrap();
    fs::write(
        added.join("package.json"),
        r#"{"name":"@demo/added","scripts":{"added:check":"echo added"}}"#,
    )
    .unwrap();
    cache.invalidate_path(&added);

    let mut added_request = request.clone();
    added_request.prefix = "added".to_owned();
    let updated = collect(&added_request, &AtomicBool::new(false));
    assert!(
        !updated.incomplete,
        "updated npm diagnostics: {:?}",
        updated.diagnostics
    );
    assert!(
        updated
            .candidates
            .iter()
            .any(|candidate| candidate.value == "added:check")
    );
    assert!(cache.get_project(&request).is_none());

    fs::write(added.join("package.json"), "{invalid").unwrap();
    cache.invalidate_path(&added.join("package.json"));
    let invalid = collect(&added_request, &AtomicBool::new(false));
    assert!(invalid.incomplete);
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("解析 package.json"))
    );
}

#[cfg(windows)]
#[test]
fn fake_git_deadline_and_output_budget_are_diagnostic_and_read_only() {
    let root = tempdir().unwrap();
    let bin = root.path().join("fake-bin");
    fs::create_dir(&bin).unwrap();
    let log = root.path().join("git-args.log");
    let script = r#"@echo off
>>"%FAKE_GIT_LOG%" echo %*
if /I "%FAKE_GIT_MODE%"=="timeout" (
  start "" /b pwsh -NoProfile -NonInteractive -Command "[Threading.Thread]::Sleep(2000)"
  pwsh -NoProfile -NonInteractive -Command "[Threading.Thread]::Sleep(3000)"
  exit /b 0
)
if /I "%FAKE_GIT_MODE%"=="large" (
  for /L %%G in (1,1,70000) do @echo 0123456789012345678901234567890123456789 refs/heads/fake%%G
  exit /b 0
)
exit /b 0
"#;
    fs::write(bin.join("git.cmd"), script).unwrap();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!("{};{}", bin.display(), path.to_string_lossy());

    let fake_request = |mode: &str| {
        let mut request = query("git", &["merge"], "", root.path());
        set_provider(&mut request, "git.refs");
        request.environment.insert("PATH".to_owned(), path.clone());
        request
            .environment
            .insert("FAKE_GIT_MODE".to_owned(), mode.to_owned());
        request.environment.insert(
            "FAKE_GIT_LOG".to_owned(),
            log.to_string_lossy().into_owned(),
        );
        request
    };

    let started = std::time::Instant::now();
    let timeout_result = collect(&fake_request("timeout"), &AtomicBool::new(false));
    assert!(started.elapsed() < std::time::Duration::from_millis(1_800));
    assert!(timeout_result.incomplete);
    assert!(
        timeout_result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("超过 1 秒截止时间"))
    );

    let large_result = collect(&fake_request("large"), &AtomicBool::new(false));
    assert!(large_result.incomplete);
    assert!(
        large_result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("输出超过预算"))
    );

    let invocation_log = fs::read_to_string(log).unwrap();
    assert!(invocation_log.contains("for-each-ref"));
    assert!(!invocation_log.to_ascii_lowercase().contains("fetch"));
    assert!(!invocation_log.to_ascii_lowercase().contains("update-index"));
    assert!(!root.path().join("index").exists());
}

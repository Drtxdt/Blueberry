//! Provider implementations; shared policy remains in the parent module.
use super::*;

pub(super) fn collect_cargo(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let Some(manifest) = cargo_manifest_path(query) else {
        let mut result = ProviderResult::default();
        record_missing_project_watches(&mut result.watch_paths, &query.cwd);
        return result.finish();
    };
    let (project, diagnostics, incomplete) = load_cargo_project(&manifest, cancelled_flag);
    let (command, active) = cargo_context(&query.args);
    let mut result = ProviderResult {
        diagnostics,
        incomplete,
        project_root: cargo_project_root(&manifest),
        ..ProviderResult::default()
    };
    cargo_watch_paths(&manifest, &project, &mut result);
    if cancelled(cancelled_flag) {
        result.incomplete = true;
        result.diagnostics.push("Cargo 项目查询已取消".to_owned());
        return result.finish();
    }
    let selected = cargo_selected_package(&query.args, &project);
    let packages = selected.into_iter().collect::<Vec<_>>();
    let packages = if packages.is_empty() {
        project.packages.iter().collect::<Vec<_>>()
    } else {
        packages
    };
    let forced_kind = provider_scope(query, "cargo").and_then(|scope| {
        match scope.to_ascii_lowercase().as_str() {
            "features" => Some(CargoValueKind::Features),
            "packages" => Some(CargoValueKind::Package),
            "bins" => Some(CargoValueKind::Bin),
            "examples" => Some(CargoValueKind::Example),
            "tests" => Some(CargoValueKind::Test),
            "benches" => Some(CargoValueKind::Bench),
            _ => None,
        }
    });
    let kind = forced_kind.or(active).or_else(|| {
        // A prefix containing an attached option is also a value context when
        // the adapter supplied the current token as part of the prefix.
        if query.prefix.starts_with("--features=") {
            Some(CargoValueKind::Features)
        } else {
            None
        }
    });
    match kind {
        Some(CargoValueKind::Package) => {
            for package in &project.packages {
                add_filtered(
                    &mut result,
                    package.name.clone(),
                    "Cargo 工作区包",
                    CandidateKind::Value,
                    "cargo.package",
                    &query.prefix,
                );
            }
        }
        Some(CargoValueKind::Features) => {
            let (before, tail, selected) = cargo_feature_parts(&query.prefix);
            let mut features = BTreeSet::new();
            for package in packages {
                features.extend(package.features.iter().cloned());
            }
            for feature in features {
                if selected.contains(&feature) {
                    continue;
                }
                let value = format!("{before}{feature}");
                if prefix_matches(&feature, tail) {
                    result.push(ProviderCandidate {
                        value,
                        description: "Cargo 功能 feature".to_owned(),
                        kind: CandidateKind::Value,
                        source: "cargo.feature".to_owned(),
                    });
                }
            }
        }
        Some(CargoValueKind::Bin)
        | Some(CargoValueKind::Example)
        | Some(CargoValueKind::Test)
        | Some(CargoValueKind::Bench) => {
            for package in packages {
                let (targets, source, description) = match kind {
                    Some(CargoValueKind::Bin) => (&package.bins, "cargo.bin", "Cargo 二进制目标"),
                    Some(CargoValueKind::Example) => {
                        (&package.examples, "cargo.example", "Cargo 示例目标")
                    }
                    Some(CargoValueKind::Test) => (&package.tests, "cargo.test", "Cargo 测试目标"),
                    Some(CargoValueKind::Bench) => {
                        (&package.benches, "cargo.bench", "Cargo 基准目标")
                    }
                    _ => unreachable!(),
                };
                for target in targets {
                    add_filtered(
                        &mut result,
                        target.clone(),
                        description,
                        CandidateKind::Value,
                        source,
                        &query.prefix,
                    );
                }
            }
        }
        None => {
            let command = lower(&command);
            if command == "run" || command == "test" || command == "build" || command == "check" {
                for package in &project.packages {
                    add_filtered(
                        &mut result,
                        package.name.clone(),
                        "Cargo 工作区包",
                        CandidateKind::Value,
                        "cargo.package",
                        &query.prefix,
                    );
                }
            }
        }
    }
    if incomplete {
        result.incomplete = true;
    }
    result.finish()
}

pub(super) fn collect_rustup(query: &ProjectQuery, _: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let root = query
        .environment
        .get("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            query
                .environment
                .get("USERPROFILE")
                .map(|p| PathBuf::from(p).join(".rustup"))
        });
    let Some(root) = root else { return result };
    let folder = match active_scope(query, "rustup") {
        "targets" => "toolchains",
        "components" => "toolchains",
        _ => "toolchains",
    };
    let dir = root.join(folder);
    add_watch_path(&mut result, &dir);
    if let Ok(entries) = fs::read_dir(dir) {
        add_values(
            &mut result,
            entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok()),
            "已安装 Rust 工具链",
            "rustup.toolchain",
            &query.prefix,
        );
    }
    result.finish()
}

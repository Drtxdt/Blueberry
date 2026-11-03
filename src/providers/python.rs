//! Provider implementations; shared policy remains in the parent module.
use super::*;

pub(super) fn collect_python(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    if (active_scope(query, "python") == "dependencies"
        || matches!(executable_stem(&query.command).as_str(), "pip" | "pip3"))
        && let Some(root) = query
            .environment
            .get("VIRTUAL_ENV")
            .or_else(|| query.environment.get("CONDA_PREFIX"))
    {
        let root = PathBuf::from(root);
        let candidates = [root.join("Lib/site-packages"), root.join("lib")];
        for dir in candidates {
            add_watch_path(&mut result, &dir);
            if let Ok(entries) = fs::read_dir(dir) {
                let packages = entries.flatten().filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    name.strip_suffix(".dist-info")
                        .or_else(|| name.strip_suffix(".egg-info"))
                        .map(|n| n.split('-').next().unwrap_or(n).replace('_', "-"))
                });
                add_values(
                    &mut result,
                    packages,
                    "当前 Python 环境已安装包",
                    "python.installed",
                    &query.prefix,
                );
            }
        }
    }

    let Some(manifest) = find_upwards(&query.cwd, "pyproject.toml") else {
        if active_scope(query, "python") == "tests"
            && let Some(config) = find_upwards(&query.cwd, "pytest.ini")
                .or_else(|| find_upwards(&query.cwd, "setup.cfg"))
        {
            result.project_root = config.parent().map(Path::to_path_buf);
            python_test_paths(
                query,
                &config,
                &toml::Value::Table(Default::default()),
                &mut result,
                cancelled_flag,
            );
        }
        return result;
    };
    result.project_root = manifest.parent().map(Path::to_path_buf);
    add_watch_path(&mut result, &manifest);
    if cancelled(cancelled_flag) {
        result.incomplete = true;
        return result;
    }
    let value = match read_limited(&manifest, MANIFEST_LIMIT)
        .map_err(|error| error.to_string())
        .and_then(|text| toml::from_str::<toml::Value>(&text).map_err(|error| error.to_string()))
    {
        Ok(value) => value,
        Err(error) => {
            result
                .diagnostics
                .push(format!("无法读取 Python 项目清单：{error}"));
            return result.finish();
        }
    };
    match active_scope(query, "python") {
        "tests" => python_test_paths(query, &manifest, &value, &mut result, cancelled_flag),
        "groups" => {
            let groups = value
                .get("dependency-groups")
                .and_then(toml::Value::as_table)
                .into_iter()
                .flat_map(|table| table.keys().cloned());
            add_values(
                &mut result,
                groups,
                "Python 依赖组",
                "python.group",
                &query.prefix,
            );
        }
        "environments" => {
            let project = manifest.parent().unwrap_or(&query.cwd);
            let mut environments = vec![project.join(".venv")];
            if let Some(active) = query.environment.get("VIRTUAL_ENV") {
                environments.push(active.into());
            }
            for environment in environments {
                add_watch_path(&mut result, &environment.join("pyvenv.cfg"));
                if environment.is_dir() {
                    if !environment.join("pyvenv.cfg").is_file() {
                        result.diagnostics.push(format!(
                            "Python 环境缺少 pyvenv.cfg：{}",
                            environment.display()
                        ));
                        continue;
                    }
                    add_values(
                        &mut result,
                        [environment.display().to_string()],
                        "Python 虚拟环境",
                        "python.environment",
                        &query.prefix,
                    );
                }
            }
        }
        scope if scope == "scripts" || query.args.iter().any(|a| a == "run") => {
            let scripts = value
                .get("project")
                .and_then(|v| v.get("scripts"))
                .and_then(toml::Value::as_table)
                .into_iter()
                .flat_map(|t| t.keys().cloned())
                .chain(
                    value
                        .get("tool")
                        .and_then(|v| v.get("poetry"))
                        .and_then(|v| v.get("scripts"))
                        .and_then(toml::Value::as_table)
                        .into_iter()
                        .flat_map(|t| t.keys().cloned()),
                );
            add_values(
                &mut result,
                scripts,
                "Python 项目脚本",
                "python.script",
                &query.prefix,
            );
        }
        _ => {
            let dependencies = value
                .get("project")
                .and_then(|v| v.get("dependencies"))
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(toml::Value::as_str)
                .filter_map(|s| s.split([' ', '<', '>', '=', '[', ';']).next())
                .map(str::to_owned);
            add_values(
                &mut result,
                dependencies,
                "pyproject.toml 依赖",
                "python.dependency",
                &query.prefix,
            );
        }
    }
    result.finish()
}

pub(super) fn collect_conda(query: &ProjectQuery, _: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let mut roots = BTreeSet::new();
    for key in ["CONDA_PREFIX", "CONDA_ROOT", "MAMBA_ROOT_PREFIX"] {
        if let Some(value) = query.environment.get(key).filter(|v| !v.is_empty()) {
            roots.insert(PathBuf::from(value));
        }
    }
    let mut envs = BTreeSet::new();
    for root in roots {
        if let Some(name) = root.file_name().and_then(|v| v.to_str()) {
            envs.insert(name.to_owned());
        }
        let env_dir = root.join("envs");
        add_watch_path(&mut result, &env_dir);
        if let Ok(entries) = fs::read_dir(env_dir) {
            envs.extend(
                entries
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok()),
            );
        }
    }
    if active_scope(query, "conda") == "packages" {
        if let Some(prefix) = query.environment.get("CONDA_PREFIX") {
            let dir = PathBuf::from(prefix).join("conda-meta");
            add_watch_path(&mut result, &dir);
            if let Ok(entries) = fs::read_dir(dir) {
                let packages = entries
                    .flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter_map(|n| n.strip_suffix(".json").map(str::to_owned))
                    .map(|n| n.rsplitn(3, '-').last().unwrap_or(&n).to_owned());
                add_values(
                    &mut result,
                    packages,
                    "当前 Conda 环境已安装包",
                    "conda.package",
                    &query.prefix,
                );
            }
        }
    } else {
        add_values(
            &mut result,
            envs,
            "本机 Conda 环境",
            "conda.environment",
            &query.prefix,
        );
    }
    result.finish()
}

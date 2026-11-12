//! Provider implementations; shared policy remains in the parent module.
use super::*;

pub(super) fn collect_go(query: &ProjectQuery, _: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let manifest =
        find_upwards(&query.cwd, "go.mod").or_else(|| find_upwards(&query.cwd, "go.work"));
    result.project_root = manifest
        .as_ref()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf);
    if let Some(path) = manifest {
        add_watch_path(&mut result, &path);
    }
    let root = result
        .project_root
        .clone()
        .unwrap_or_else(|| query.cwd.clone());
    if let Ok(entries) = fs::read_dir(&root) {
        let values = entries.flatten().filter_map(|e| {
            let p = e.path();
            if p.is_dir() {
                Some(format!("./{}", e.file_name().to_string_lossy()))
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("go")) {
                Some(e.file_name().to_string_lossy().into_owned())
            } else {
                None
            }
        });
        add_values(
            &mut result,
            values,
            "当前 Go 项目",
            "go.project",
            &query.prefix,
        );
    }
    result.finish()
}

pub(super) fn collect_dotnet(query: &ProjectQuery, _: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let mut current = Some(query.cwd.as_path());
    let mut root = query.cwd.clone();
    while let Some(dir) = current {
        let has_solution = fs::read_dir(dir).ok().is_some_and(|mut entries| {
            entries.any(|entry| {
                entry.ok().is_some_and(|entry| {
                    entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("sln"))
                })
            })
        });
        if has_solution {
            root = dir.to_path_buf();
            break;
        }
        current = dir.parent();
    }
    result.project_root = Some(root.clone());
    add_watch_path(&mut result, &root);
    if let Ok(entries) = fs::read_dir(root) {
        add_values(
            &mut result,
            entries.flatten().filter_map(|e| {
                let p = e.path();
                let ext = p.extension()?.to_str()?;
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "sln" | "csproj" | "fsproj" | "vbproj"
                )
                .then(|| e.file_name().to_string_lossy().into_owned())
            }),
            "解决方案或项目",
            "dotnet.project",
            &query.prefix,
        );
    }
    result.finish()
}

pub(super) fn collect_cmake(query: &ProjectQuery, _: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let Some(path) = find_upwards(&query.cwd, "CMakePresets.json")
        .or_else(|| find_upwards(&query.cwd, "CMakeUserPresets.json"))
    else {
        return result;
    };
    result.project_root = path.parent().map(Path::to_path_buf);
    add_watch_path(&mut result, &path);
    if let Ok(text) = read_limited(&path, MANIFEST_LIMIT)
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
    {
        let key = match active_scope(query, "cmake") {
            "build_presets" => "buildPresets",
            "test_presets" => "testPresets",
            _ => "configurePresets",
        };
        let values = json
            .get(key)
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.get("name")?.as_str())
            .map(str::to_owned);
        add_values(
            &mut result,
            values,
            "CMake 预设",
            "cmake.preset",
            &query.prefix,
        );
    }
    result.finish()
}

pub(super) fn collect_dev_tools(
    query: &ProjectQuery,
    cancelled_flag: &AtomicBool,
) -> ProviderResult {
    let mut result = ProviderResult::default();
    if cancelled(cancelled_flag) {
        result.incomplete = true;
        return result;
    }
    let family = query
        .provider
        .as_deref()
        .and_then(|provider| provider.split('.').next())
        .map(str::to_owned)
        .unwrap_or_else(|| executable_stem(&query.command));
    let mut values = BTreeSet::new();
    let mut source = "dev.project";
    let mut description = "当前项目候选";
    match family.as_str() {
        "deno" => {
            if let Some(path) = find_upwards(&query.cwd, "deno.json")
                .or_else(|| find_upwards(&query.cwd, "deno.jsonc"))
            {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT)
                    && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
                {
                    values.extend(
                        json.get("tasks")
                            .and_then(serde_json::Value::as_object)
                            .into_iter()
                            .flat_map(|tasks| tasks.keys().cloned()),
                    );
                }
            }
            source = "deno.task";
            description = "deno.json 项目任务";
        }
        "maven" => {
            if let Some(path) = find_upwards(&query.cwd, "pom.xml") {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    for tag in ["module", "id"] {
                        let open = format!("<{tag}>");
                        let close = format!("</{tag}>");
                        for tail in text.split(&open).skip(1) {
                            if let Some((value, _)) = tail.split_once(&close) {
                                let value = value.trim();
                                if !value.is_empty() && !value.contains('<') {
                                    values.insert(value.to_owned());
                                }
                            }
                        }
                    }
                }
            }
            source = "maven.module";
            description = "Maven 模块或 profile";
        }
        "gradle" => {
            let path = ["build.gradle.kts", "build.gradle"]
                .iter()
                .find_map(|name| find_upwards(&query.cwd, name));
            if let Some(path) = path {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    for line in text.lines() {
                        let trimmed = line.trim();
                        if let Some(value) = trimmed.strip_prefix("task ") {
                            let value = value.split([' ', '(', '{']).next().unwrap_or("");
                            if !value.is_empty() {
                                values.insert(value.into());
                            }
                        }
                        for prefix in ["tasks.register(\"", "tasks.named(\""] {
                            if let Some(value) = trimmed
                                .strip_prefix(prefix)
                                .and_then(|v| v.split('"').next())
                            {
                                values.insert(value.into());
                            }
                        }
                    }
                }
            }
            source = "gradle.task";
            description = "Gradle 文件中声明的任务";
        }
        "make" => {
            if let Some(path) = ["Makefile", "makefile", "GNUmakefile"]
                .iter()
                .find_map(|name| find_upwards(&query.cwd, name))
            {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    for line in text.lines().filter(|line| {
                        !line
                            .chars()
                            .next()
                            .is_some_and(|c| [' ', '\t', '.', '#'].contains(&c))
                    }) {
                        if let Some((target, _)) = line.split_once(':') {
                            let target = target.trim();
                            if !target.is_empty() && !target.contains(['%', '=', '$']) {
                                values.insert(target.into());
                            }
                        }
                    }
                }
            }
            source = "make.target";
            description = "Makefile 构建目标";
        }
        "ninja" => {
            if let Some(path) = find_upwards(&query.cwd, "build.ninja") {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    for line in text.lines() {
                        if let Some(target) = line
                            .trim()
                            .strip_prefix("build ")
                            .and_then(|v| v.split(':').next())
                        {
                            values.extend(target.split_whitespace().map(str::to_owned));
                        }
                    }
                }
            }
            source = "ninja.target";
            description = "Ninja 构建目标";
        }
        "just" => {
            if let Some(path) = ["Justfile", "justfile"]
                .iter()
                .find_map(|name| find_upwards(&query.cwd, name))
            {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    for line in text.lines().filter(|line| {
                        !line
                            .chars()
                            .next()
                            .is_some_and(|c| [' ', '\t', '#', '@'].contains(&c))
                    }) {
                        if let Some((recipe, _)) = line.split_once(':') {
                            let recipe = recipe.split_whitespace().next().unwrap_or("");
                            if !recipe.is_empty() && !recipe.contains('=') {
                                values.insert(recipe.into());
                            }
                        }
                    }
                }
            }
            source = "just.recipe";
            description = "Justfile 配方";
        }
        "task" => {
            if let Some(path) = [
                "Taskfile.yml",
                "Taskfile.yaml",
                "taskfile.yml",
                "taskfile.yaml",
            ]
            .iter()
            .find_map(|name| find_upwards(&query.cwd, name))
            {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    values.extend(simple_yaml_keys(&text, "tasks"));
                }
            }
            source = "task.task";
            description = "Taskfile 任务";
        }
        "terraform" | "tofu" => {
            let root = query.cwd.clone();
            result.project_root = Some(root.clone());
            add_watch_path(&mut result, &root);
            if let Ok(entries) = fs::read_dir(&root) {
                for path in entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().is_some_and(|ext| ext == "tf"))
                {
                    add_watch_path(&mut result, &path);
                    if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                        for line in text.lines() {
                            if let Some(value) = quoted_name(line, "variable")
                                .or_else(|| quoted_name(line, "module"))
                            {
                                values.insert(value);
                            }
                        }
                    }
                }
            }
            source = if family == "tofu" {
                "tofu.symbol"
            } else {
                "terraform.symbol"
            };
            description = "当前基础设施项目的模块或变量";
        }
        "ansible" => {
            let explicit = query
                .environment
                .get("ANSIBLE_INVENTORY")
                .map(PathBuf::from);
            let path = explicit.filter(|path| path.is_file()).or_else(|| {
                ["inventory", "hosts", "inventory.yml", "inventory.yaml"]
                    .iter()
                    .find_map(|name| find_upwards(&query.cwd, name))
            });
            if let Some(path) = path {
                result.project_root = path.parent().map(Path::to_path_buf);
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
                    for line in text.lines() {
                        let trimmed = line.trim();
                        if trimmed.is_empty()
                            || trimmed
                                .chars()
                                .next()
                                .is_some_and(|c| ['#', '[', '-', '{'].contains(&c))
                            || trimmed.ends_with(':')
                        {
                            continue;
                        }
                        let host = trimmed.split_whitespace().next().unwrap_or("");
                        if !host.is_empty() && !host.contains('=') {
                            values.insert(host.into());
                        }
                    }
                }
            }
            source = "ansible.inventory";
            description = "静态 Ansible inventory 主机";
        }
        _ => {}
    }
    add_values(&mut result, values, description, source, &query.prefix);
    result.finish()
}

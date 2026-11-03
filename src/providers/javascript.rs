//! Provider implementations; shared policy remains in the parent module.
use super::*;

pub(super) fn collect_node(
    query: &ProjectQuery,
    cancelled_flag: &AtomicBool,
    pnpm: bool,
) -> ProviderResult {
    let (project, diagnostics, incomplete) = load_node_project(query, pnpm, cancelled_flag);
    if project.packages.is_empty() && diagnostics.is_empty() {
        let result = ProviderResult {
            watch_paths: project.watch_paths,
            ..ProviderResult::default()
        };
        return result.finish();
    }
    let (_, active) = node_context(&query.args, pnpm);
    let active = provider_scope(query, if pnpm { "pnpm" } else { "npm" })
        .and_then(|scope| match scope.to_ascii_lowercase().as_str() {
            "scripts" => Some(NodeValueKind::Script),
            "workspaces" => Some(NodeValueKind::Workspace),
            "dependencies" => Some(NodeValueKind::Dependency),
            _ => None,
        })
        .or(active);
    let mut result = ProviderResult {
        diagnostics,
        incomplete,
        project_root: node_project_root(query),
        ..ProviderResult::default()
    };
    node_watch_paths(&project, &mut result);
    let selectors = node_workspace_selectors(&query.args, pnpm);
    let selected_packages =
        selected_node_packages(&project, &selectors, result.project_root.as_deref());
    if cancelled(cancelled_flag) {
        result.incomplete = true;
        result
            .diagnostics
            .push("JavaScript 项目查询已取消".to_owned());
        return result.finish();
    }
    match active {
        Some(NodeValueKind::Workspace) => {
            for package in &selected_packages {
                if !package.name.is_empty() {
                    add_filtered(
                        &mut result,
                        package.name.clone(),
                        "工作区包",
                        CandidateKind::Value,
                        "node.workspace",
                        &query.prefix,
                    );
                }
            }
        }
        Some(NodeValueKind::Script) => {
            let source = if pnpm { "pnpm.script" } else { "npm.script" };
            for package in &selected_packages {
                for script in &package.scripts {
                    add_filtered(
                        &mut result,
                        script.clone(),
                        "package.json 脚本",
                        CandidateKind::Value,
                        source,
                        &query.prefix,
                    );
                }
            }
        }
        Some(NodeValueKind::Dependency) => {
            let source = if pnpm {
                "pnpm.dependency"
            } else {
                "npm.dependency"
            };
            for package in &selected_packages {
                for dependency in &package.dependencies {
                    let description = if package.local_dependencies.contains(dependency) {
                        "项目本地依赖"
                    } else {
                        "package.json 依赖"
                    };
                    add_filtered(
                        &mut result,
                        dependency.clone(),
                        description,
                        CandidateKind::Value,
                        source,
                        &query.prefix,
                    );
                }
            }
        }
        None => {}
    }
    result.finish()
}

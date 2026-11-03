//! Provider implementations; shared policy remains in the parent module.
use super::*;

pub(super) fn collect_docker(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let remote = query.environment.get("DOCKER_HOST").is_some_and(|host| {
        !host.starts_with("npipe://")
            && !host.starts_with("unix://")
            && !host.contains("localhost")
            && !host.contains("127.0.0.1")
    });
    if remote
        && query
            .environment
            .get("BLUEBERRY_REMOTE_REQUESTED")
            .is_some_and(|v| v == "1")
    {
        match run_bounded_query(
            query,
            "docker",
            &["ps", "--format", "{{.Names}}"],
            cancelled_flag,
        ) {
            Ok(values) => add_values(
                &mut result,
                values,
                "远程 Docker 容器",
                "docker.remote.container",
                &query.prefix,
            ),
            Err(error) => {
                result.incomplete = true;
                result
                    .diagnostics
                    .push(format!("Docker 资源读取失败：{error}"));
            }
        }
    }
    let names = [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yaml",
        "docker-compose.yml",
    ];
    let Some(path) = names.iter().find_map(|n| find_upwards(&query.cwd, n)) else {
        return result;
    };
    result.project_root = path.parent().map(Path::to_path_buf);
    add_watch_path(&mut result, &path);
    if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
        let mut in_services = false;
        let values = text.lines().filter_map(|line| {
            if line.trim() == "services:" {
                in_services = true;
                return None;
            }
            if in_services && !line.starts_with(' ') && !line.trim().is_empty() {
                in_services = false
            }
            if in_services {
                let trimmed = line.trim();
                (line.starts_with("  ") && !line.starts_with("    ") && trimmed.ends_with(':'))
                    .then(|| trimmed.trim_end_matches(':').to_owned())
            } else {
                None
            }
        });
        add_values(
            &mut result,
            values,
            "Compose 服务",
            "docker.compose.service",
            &query.prefix,
        );
    }
    result.finish()
}

pub(super) fn collect_kubeconfig(
    query: &ProjectQuery,
    cancelled_flag: &AtomicBool,
) -> ProviderResult {
    let mut result = ProviderResult::default();
    let path = query
        .environment
        .get("KUBECONFIG")
        .and_then(|v| env::split_paths(v).next())
        .or_else(|| {
            query
                .environment
                .get("USERPROFILE")
                .map(|p| PathBuf::from(p).join(".kube/config"))
        });
    let Some(path) = path else { return result };
    add_watch_path(&mut result, &path);
    if let Ok(text) = read_limited(&path, 2 * MANIFEST_LIMIT) {
        let mut wanted = false;
        let scope = active_scope(query, "kubectl");
        let values = text.lines().filter_map(|line| {
            let t = line.trim();
            if t == "contexts:" {
                wanted = true;
                return None;
            }
            if wanted && t.ends_with(':') && !line.starts_with(' ') {
                wanted = false
            }
            if wanted && t.starts_with("- name:") {
                Some(t.trim_start_matches("- name:").trim().to_owned())
            } else if scope == "contexts" && t.starts_with("current-context:") {
                Some(t.trim_start_matches("current-context:").trim().to_owned())
            } else {
                None
            }
        });
        add_values(
            &mut result,
            values,
            "Kubernetes 上下文",
            "kubectl.context",
            &query.prefix,
        );
    }
    if query
        .environment
        .get("BLUEBERRY_REMOTE_REQUESTED")
        .is_some_and(|v| v == "1")
    {
        let scope = active_scope(query, "kubectl");
        let args = if scope == "namespaces" {
            vec!["get", "namespaces", "-o", "name"]
        } else {
            let kind = query
                .args
                .iter()
                .find(|arg| {
                    matches!(
                        arg.as_str(),
                        "pods"
                            | "pod"
                            | "deployments"
                            | "deployment"
                            | "services"
                            | "service"
                            | "statefulsets"
                            | "daemonsets"
                            | "jobs"
                    )
                })
                .map(String::as_str)
                .unwrap_or("pods");
            vec!["get", kind, "-o", "name"]
        };
        match run_bounded_query(query, "kubectl", &args, cancelled_flag) {
            Ok(values) => add_values(
                &mut result,
                values,
                "当前 Kubernetes 上下文资源",
                "kubectl.remote",
                &query.prefix,
            ),
            Err(error) => {
                result.incomplete = true;
                result
                    .diagnostics
                    .push(format!("Kubernetes 资源读取失败：{error}"));
            }
        }
    }
    result.finish()
}

pub(super) fn collect_helm(query: &ProjectQuery, cancelled_flag: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let mut current = Some(query.cwd.as_path());
    while let Some(dir) = current {
        let chart = dir.join("Chart.yaml");
        if chart.is_file() {
            result.project_root = Some(dir.to_path_buf());
            add_watch_path(&mut result, &chart);
            add_filtered(
                &mut result,
                dir.to_string_lossy().into_owned(),
                "本地 Helm Chart",
                CandidateKind::Directory,
                "helm.chart",
                &query.prefix,
            );
            break;
        }
        current = dir.parent();
    }
    if active_scope(query, "helm") == "releases"
        && query
            .environment
            .get("BLUEBERRY_REMOTE_REQUESTED")
            .is_some_and(|v| v == "1")
    {
        match run_bounded_query(query, "helm", &["list", "-q"], cancelled_flag) {
            Ok(values) => add_values(
                &mut result,
                values,
                "当前上下文 Helm release",
                "helm.remote.release",
                &query.prefix,
            ),
            Err(error) => {
                result.incomplete = true;
                result
                    .diagnostics
                    .push(format!("Helm release 读取失败：{error}"));
            }
        }
    }
    result.finish()
}

pub(super) fn collect_ssh(query: &ProjectQuery, _: &AtomicBool) -> ProviderResult {
    let mut result = ProviderResult::default();
    let Some(profile) = query.environment.get("USERPROFILE") else {
        return result;
    };
    let path = PathBuf::from(profile).join(".ssh/config");
    add_watch_path(&mut result, &path);
    if let Ok(text) = read_limited(&path, MANIFEST_LIMIT) {
        let hosts = text
            .lines()
            .filter_map(|line| {
                let t = line.trim();
                let rest = t
                    .strip_prefix("Host ")
                    .or_else(|| t.strip_prefix("host "))?;
                Some(
                    rest.split_whitespace()
                        .filter(|h| !h.contains(['*', '?', '!']))
                        .map(str::to_owned)
                        .collect::<Vec<_>>(),
                )
            })
            .flatten();
        add_values(
            &mut result,
            hosts,
            "SSH 配置主机",
            "ssh.host",
            &query.prefix,
        );
    }
    result.finish()
}

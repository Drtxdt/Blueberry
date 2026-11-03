//! Provider implementations; shared policy remains in the parent module.
use super::*;

pub(super) fn collect_windows_tools(
    query: &ProjectQuery,
    cancelled_flag: &AtomicBool,
) -> ProviderResult {
    let mut result = ProviderResult::default();
    let provider = query.provider.as_deref().unwrap_or("");
    match provider {
        "windows.scoop_packages" => {
            let root = query
                .environment
                .get("SCOOP")
                .map(PathBuf::from)
                .or_else(|| {
                    query
                        .environment
                        .get("USERPROFILE")
                        .map(|p| PathBuf::from(p).join("scoop"))
                });
            if let Some(dir) = root.map(|root| root.join("apps")) {
                add_watch_path(&mut result, &dir);
                if let Ok(entries) = fs::read_dir(dir) {
                    add_values(
                        &mut result,
                        entries
                            .flatten()
                            .filter(|entry| entry.path().is_dir())
                            .filter_map(|entry| entry.file_name().into_string().ok()),
                        "已安装 Scoop 软件包",
                        "windows.scoop.package",
                        &query.prefix,
                    );
                }
            }
        }
        "windows.choco_packages" => {
            let root = query
                .environment
                .get("ChocolateyInstall")
                .map(PathBuf::from)
                .or_else(|| {
                    query
                        .environment
                        .get("ProgramData")
                        .map(|p| PathBuf::from(p).join("chocolatey"))
                });
            if let Some(dir) = root.map(|root| root.join("lib")) {
                add_watch_path(&mut result, &dir);
                if let Ok(entries) = fs::read_dir(dir) {
                    add_values(
                        &mut result,
                        entries
                            .flatten()
                            .filter(|entry| entry.path().is_dir())
                            .filter_map(|entry| entry.file_name().into_string().ok())
                            .map(|name| name.trim_end_matches(".install").to_owned()),
                        "已安装 Chocolatey 软件包",
                        "windows.choco.package",
                        &query.prefix,
                    );
                }
            }
        }
        "powershell.modules" => {
            if let Some(paths) = query.environment.get("PSModulePath") {
                for dir in env::split_paths(paths) {
                    add_watch_path(&mut result, &dir);
                    if let Ok(entries) = fs::read_dir(dir) {
                        add_values(
                            &mut result,
                            entries
                                .flatten()
                                .filter(|entry| entry.path().is_dir())
                                .filter_map(|entry| entry.file_name().into_string().ok()),
                            "本机 PowerShell 模块",
                            "powershell.module",
                            &query.prefix,
                        );
                    }
                }
            }
        }
        "windows.wsl_distros" => {
            match run_bounded_query(query, "wsl", &["-l", "-q"], cancelled_flag) {
                Ok(values) => add_values(
                    &mut result,
                    values
                        .into_iter()
                        .map(|value| value.replace('\0', "").trim().to_owned())
                        .filter(|value| !value.is_empty()),
                    "已安装 WSL 发行版",
                    "windows.wsl.distro",
                    &query.prefix,
                ),
                Err(error) => result
                    .diagnostics
                    .push(format!("读取 WSL 发行版失败：{error}")),
            }
        }
        "windows.processes" => {
            match run_bounded_query(query, "tasklist", &["/fo", "csv", "/nh"], cancelled_flag) {
                Ok(values) => add_values(
                    &mut result,
                    values.into_iter().filter_map(|line| {
                        line.trim_matches('"')
                            .split("\",\"")
                            .next()
                            .map(str::to_owned)
                    }),
                    "本机进程映像名称",
                    "windows.process",
                    &query.prefix,
                ),
                Err(error) => result
                    .diagnostics
                    .push(format!("读取进程列表失败：{error}")),
            }
        }
        "windows.services" => {
            match run_bounded_query(query, "sc", &["query", "state=", "all"], cancelled_flag) {
                Ok(values) => add_values(
                    &mut result,
                    values.into_iter().filter_map(|line| {
                        line.strip_prefix("SERVICE_NAME:")
                            .map(str::trim)
                            .map(str::to_owned)
                    }),
                    "本机 Windows 服务",
                    "windows.service",
                    &query.prefix,
                ),
                Err(error) => result
                    .diagnostics
                    .push(format!("读取服务列表失败：{error}")),
            }
        }
        _ => {}
    }
    result.finish()
}

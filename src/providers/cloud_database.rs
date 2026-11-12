//! Provider implementations; shared policy remains in the parent module.
use super::*;

pub(super) fn collect_cloud_data(
    query: &ProjectQuery,
    cancelled_flag: &AtomicBool,
) -> ProviderResult {
    let mut result = ProviderResult::default();
    let provider = query.provider.as_deref().unwrap_or("");
    let home = cloud_home(query);
    let remote = query
        .environment
        .get("BLUEBERRY_REMOTE_REQUESTED")
        .is_some_and(|value| value == "1");

    match provider {
        "cloud.aws_profiles" => {
            let mut values = Vec::new();
            if let Some(home) = &home {
                values.extend(ini_sections(&home.join(".aws/config"), &mut result));
                values.extend(ini_sections(&home.join(".aws/credentials"), &mut result));
            }
            add_values(
                &mut result,
                values,
                "本机 AWS profile",
                "cloud.aws.profile",
                &query.prefix,
            );
        }
        "cloud.azure_profiles" => {
            if let Some(home) = &home {
                let path = home.join(".azure/azureProfile.json");
                add_watch_path(&mut result, &path);
                if let Ok(text) = read_limited(&path, 2 * MANIFEST_LIMIT)
                    && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
                {
                    let values = json
                        .get("subscriptions")
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|item| item.get("name").and_then(|v| v.as_str()));
                    add_values(
                        &mut result,
                        values,
                        "本机 Azure 订阅",
                        "cloud.azure.profile",
                        &query.prefix,
                    );
                }
            }
        }
        "cloud.gcloud_profiles" => {
            let config_root = query
                .environment
                .get("APPDATA")
                .map(|root| PathBuf::from(root).join("gcloud/configurations"))
                .or_else(|| {
                    home.as_ref()
                        .map(|root| root.join(".config/gcloud/configurations"))
                });
            if let Some(root) = config_root {
                add_watch_path(&mut result, &root);
                if let Ok(entries) = fs::read_dir(root) {
                    let values = entries.flatten().filter_map(|entry| {
                        entry
                            .file_name()
                            .to_str()?
                            .strip_prefix("config_")
                            .map(str::to_owned)
                    });
                    add_values(
                        &mut result,
                        values,
                        "本机 gcloud 配置",
                        "cloud.gcloud.profile",
                        &query.prefix,
                    );
                }
            }
        }
        "db.postgres" if !remote => {
            if let Some(home) = &home {
                let path = query
                    .environment
                    .get("APPDATA")
                    .map(|root| PathBuf::from(root).join("postgresql/.pg_service.conf"))
                    .filter(|path| path.is_file())
                    .unwrap_or_else(|| home.join(".pg_service.conf"));
                let values = ini_sections(&path, &mut result);
                add_values(
                    &mut result,
                    values,
                    "本机 PostgreSQL 服务配置",
                    "db.postgres.service",
                    &query.prefix,
                );
            }
        }
        "db.mysql" if !remote => {
            if let Some(home) = &home {
                let values = ini_sections(&home.join(".my.cnf"), &mut result);
                add_values(
                    &mut result,
                    values,
                    "本机 MySQL 配置组",
                    "db.mysql.profile",
                    &query.prefix,
                );
            }
        }
        "cloud.aws_resources" if remote => remote_values(
            &mut result,
            query,
            "aws",
            &[
                "s3api",
                "list-buckets",
                "--query",
                "Buckets[].Name",
                "--output",
                "text",
            ],
            "AWS S3 bucket",
            "cloud.aws.remote",
            cancelled_flag,
        ),
        "cloud.azure_resources" if remote => remote_values(
            &mut result,
            query,
            "az",
            &["group", "list", "--query", "[].name", "-o", "tsv"],
            "Azure 资源组",
            "cloud.azure.remote",
            cancelled_flag,
        ),
        "cloud.gcloud_resources" if remote => remote_values(
            &mut result,
            query,
            "gcloud",
            &["projects", "list", "--format=value(projectId)"],
            "Google Cloud 项目",
            "cloud.gcloud.remote",
            cancelled_flag,
        ),
        "db.postgres" if remote => remote_values(
            &mut result,
            query,
            "psql",
            &["-Atc", "SELECT datname FROM pg_database WHERE datallowconn"],
            "PostgreSQL 数据库",
            "db.postgres.remote",
            cancelled_flag,
        ),
        "db.mysql" if remote => remote_values(
            &mut result,
            query,
            "mysql",
            &["-NBe", "SHOW DATABASES"],
            "MySQL 数据库",
            "db.mysql.remote",
            cancelled_flag,
        ),
        "db.redis" if remote => remote_values(
            &mut result,
            query,
            "redis-cli",
            &["--scan"],
            "Redis key",
            "db.redis.remote",
            cancelled_flag,
        ),
        "db.mongo" if remote => remote_values(
            &mut result,
            query,
            "mongosh",
            &[
                "--quiet",
                "--eval",
                "db.getMongo().getDBNames().join('\\n')",
            ],
            "MongoDB 数据库",
            "db.mongo.remote",
            cancelled_flag,
        ),
        _ => {}
    }
    result.finish()
}

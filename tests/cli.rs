use std::{fs, process::Command};
use tempfile::tempdir;

#[test]
fn json_completion_preserves_fields_and_uses_custom_chinese_descriptions() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("config.toml");
    fs::write(
        &path,
        "[descriptions]\n\"git log --oneline\" = \"逐条显示提交\"\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_shellsense"))
        .args([
            "--config",
            path.to_str().unwrap(),
            "complete",
            "--line",
            "git log --o",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(result["replace_start"].is_number());
    assert!(result["replace_end"].is_number());
    assert!(result["incomplete"].is_boolean());
    let candidate = result["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["label"] == "--oneline")
        .unwrap();
    assert_eq!(candidate["description"], "逐条显示提交");
    assert_eq!(candidate["insert_text"], "--oneline");
    assert!(candidate["kind"].is_string());
}

#[test]
fn complete_explain_json_reports_context_catalog_and_candidate_sources() {
    let temporary = tempdir().unwrap();
    let config = temporary.path().join("config.toml");
    fs::write(
        &config,
        "[completion]\ndynamic = false\n[learning]\nenabled = false\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_shellsense"))
        .args([
            "--config",
            config.to_str().unwrap(),
            "complete",
            "--line",
            "git log --o",
            "--json",
            "--explain",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["explain"]["context"]["command"], "git");
    assert_eq!(result["explain"]["context"]["prefix"], "--o");
    assert!(result["explain"]["catalog"]["directory"].is_string());
    assert!(result["explain"]["matching"]["candidates"].is_array());
    assert!(
        result["explain"]["matching"]["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["source"] == "builtin")
    );
}

#[test]
fn specs_check_and_list_json_expose_validation_and_sources() {
    let temporary = tempdir().unwrap();
    let specs = temporary.path().join("specs");
    fs::create_dir_all(&specs).unwrap();
    fs::write(specs.join("valid.toml"), "schema_version = 1\n").unwrap();

    let check = Command::new(env!("CARGO_BIN_EXE_shellsense"))
        .args([
            "specs",
            "check",
            "--directory",
            specs.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let check_json: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(check_json["valid"], true);
    assert_eq!(check_json["files"][0]["valid"], true);

    let list = Command::new(env!("CARGO_BIN_EXE_shellsense"))
        .args([
            "specs",
            "list",
            "--directory",
            specs.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let list_json: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert!(list_json["count"].as_u64().unwrap_or_default() > 0);
    assert!(
        list_json["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["source"] == "builtin")
    );
}

#[test]
fn specs_check_returns_failure_and_diagnostic_for_invalid_schema() {
    let temporary = tempdir().unwrap();
    let specs = temporary.path().join("specs");
    fs::create_dir_all(&specs).unwrap();
    fs::write(specs.join("broken.toml"), "schema_version = 2\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_shellsense"))
        .args([
            "specs",
            "check",
            "--directory",
            specs.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["valid"], false);
    assert!(!result["diagnostics"].as_array().unwrap().is_empty());
}

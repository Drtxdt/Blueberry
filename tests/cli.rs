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

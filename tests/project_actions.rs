use anyhow::Result;
use blueberry::providers::{ProjectAction, project_actions};

fn node_actions(path: &std::path::Path) -> Vec<ProjectAction> {
    project_actions(path, "测试")
        .into_iter()
        .filter(|action| action.candidate.source == "project.action.node")
        .collect()
}
use std::fs;

#[test]
fn action_identity_survives_reordering_and_is_scoped_to_project() -> Result<()> {
    let root = tempfile::tempdir()?;
    let nested = root.path().join("nested");
    fs::create_dir(&nested)?;
    let manifest = root.path().join("package.json");
    fs::write(&manifest, r#"{"scripts":{"test z":"anything"}}"#)?;
    let initial = node_actions(root.path());
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].candidate.value, "npm run 'test z'");
    fs::write(
        &manifest,
        r#"{"scripts":{"test a":"new","test z":"changed"}}"#,
    )?;
    let updated = node_actions(&nested);
    assert_eq!(updated.len(), 2);
    assert_eq!(updated[1].id, initial[0].id);
    assert_ne!(updated[0].id, initial[0].id);
    let other = tempfile::tempdir()?;
    fs::copy(&manifest, other.path().join("package.json"))?;
    assert_ne!(node_actions(other.path())[1].id, initial[0].id);
    Ok(())
}

#[test]
fn dynamic_action_values_are_literal_powershell_arguments() -> Result<()> {
    let root = tempfile::tempdir()?;
    fs::write(
        root.path().join("package.json"),
        serde_json::to_vec(&serde_json::json!({
            "scripts": {"@odd,name; '中文'": "unused"}
        }))?,
    )?;
    let actions = project_actions(root.path(), "");
    assert_eq!(actions[0].candidate.value, "npm run '@odd,name; ''中文'''");
    Ok(())
}

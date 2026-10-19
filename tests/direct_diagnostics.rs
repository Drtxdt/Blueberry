//! The same contract generates the Rust receiver and C# sender validators.
include!(concat!(env!("OUT_DIR"), "/direct-diagnostics.rs"));

#[test]
fn diagnostic_contract_accepts_declared_stages_and_rejects_invalid_fields() {
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../shell/direct-diagnostics.json")).unwrap();
    for (event, spec) in contract["events"].as_object().unwrap() {
        for stage in spec["stages"].as_array().unwrap() {
            let mut value = serde_json::json!({"event":event,"stage":stage,"qpc":123});
            assert_eq!(diagnostic_stage(&value), stage.as_str());
            for field in spec["required_integer_fields"].as_array().unwrap() {
                let field = field.as_str().unwrap();
                value.as_object_mut().unwrap().remove(field);
                assert!(diagnostic_stage(&value).is_none());
                value[field] = serde_json::json!("invalid");
                assert!(diagnostic_stage(&value).is_none());
                value[field] = serde_json::json!(123);
            }
        }
        assert!(
            diagnostic_stage(&serde_json::json!({"event":event,"stage":"future_event","qpc":123}))
                .is_none()
        );
    }
    assert!(
        diagnostic_stage(
            &serde_json::json!({"event":"future_control","stage":"editor_confirmed","qpc":123})
        )
        .is_none()
    );
}

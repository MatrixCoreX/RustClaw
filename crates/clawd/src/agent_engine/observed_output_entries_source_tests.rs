use super::command_result_observation;
use serde_json::json;

#[test]
fn command_facts_keep_the_validated_executor_source() {
    let body = json!({"schema_version":1,"source":"fixture_command","exit_code":7,
        "complete":true,"output_truncated":false,"command_output":"observed"})
    .to_string();
    let output = command_result_observation("fixture_command", &body).unwrap();
    let (metadata, text) = output.split_once('\n').unwrap();
    let metadata: serde_json::Value =
        serde_json::from_str(metadata.strip_prefix("execution_metadata=").unwrap()).unwrap();
    assert_eq!(metadata["source"], "fixture_command");
    assert_eq!(metadata["exit_code"], 7);
    assert_eq!(text, "observed");
    assert!(command_result_observation("different_executor", &body).is_none());
}

#[test]
fn command_source_binding_does_not_accept_unknown_schema_or_invalid_exit_code() {
    for change in [
        json!({"schema_version":2}),
        json!({"exit_code":"7"}),
        json!({"source":null}),
    ] {
        let mut body = json!({"schema_version":1,"source":"fixture_command","exit_code":0,"command_output":"observed"});
        body.as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        assert!(command_result_observation("fixture_command", &body.to_string()).is_none());
    }
}

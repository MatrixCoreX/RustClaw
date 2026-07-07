use super::*;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "execution_failed");
    assert_eq!(extra["message_key"], "skill.docker_basic.execution_failed");
    assert_eq!(extra["retryable"], false);
}

#[test]
fn readonly_unavailable_response_is_ok_observation() {
    let (text, extra) = docker_readonly_unavailable("ps", "not found".to_string());
    assert!(text.contains("docker unavailable"));
    assert_eq!(extra.get("action").and_then(Value::as_str), Some("ps"));
    assert_eq!(extra.get("available").and_then(Value::as_bool), Some(false));
    assert_eq!(
        extra.get("command_succeeded").and_then(Value::as_bool),
        Some(false)
    );
}

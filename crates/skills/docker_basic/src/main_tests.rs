use super::*;

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

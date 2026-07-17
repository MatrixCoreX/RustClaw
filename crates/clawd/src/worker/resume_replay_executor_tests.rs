use serde_json::{json, Value};

use super::apply_resume_steering_prompt;

#[test]
fn resume_steering_prompt_preserves_multilingual_input_as_opaque_json() {
    let mut payload = json!({"text": "initial request"});
    let input = json!({
        "user_message": "继续，但不要改公开接口",
        "new_constraints": {
            "verification": "必須",
            "scope": ["src"]
        }
    });

    apply_resume_steering_prompt(&mut payload, &input);

    let envelope: Value =
        serde_json::from_str(payload["text"].as_str().expect("steering prompt")).expect("JSON");
    assert_eq!(envelope["protocol"], "rustclaw.resume_input.v1");
    assert_eq!(envelope["original_request"], "initial request");
    assert_eq!(envelope["user_message"], "继续，但不要改公开接口");
    assert_eq!(envelope["new_constraints"]["verification"], "必須");
    assert_eq!(envelope["new_constraints"]["scope"], json!(["src"]));
}

#[test]
fn resume_steering_prompt_supports_constraint_only_resume() {
    let mut payload = json!({"text": "initial request"});

    apply_resume_steering_prompt(
        &mut payload,
        &json!({"new_constraints": {"budget_profile": "long_tail"}}),
    );

    let envelope: Value =
        serde_json::from_str(payload["text"].as_str().expect("steering prompt")).expect("JSON");
    assert!(envelope.get("user_message").is_none());
    assert_eq!(envelope["new_constraints"]["budget_profile"], "long_tail");
}

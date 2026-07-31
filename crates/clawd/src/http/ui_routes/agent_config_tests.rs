use super::*;

#[test]
fn custom_persona_constraints_are_enforced_by_the_backend() {
    assert!(validate_agent_custom_persona("温和、简洁").is_ok());
    assert!(validate_agent_custom_persona("line one\nline two\tend").is_ok());
    assert!(validate_agent_custom_persona("bad\u{0000}").is_err());
    assert!(
        validate_agent_custom_persona(&"字".repeat(AGENT_CUSTOM_PERSONA_MAX_CHARS + 1)).is_err()
    );
    let response_shape = serde_json::json!({
        "preset_catalog": agent_persona_preset_catalog(),
        "constraints": AgentPersonaConstraints {
            custom_persona_max_chars: AGENT_CUSTOM_PERSONA_MAX_CHARS,
            allowed_control_characters: vec!["tab", "newline"],
        }
    });
    assert_eq!(response_shape["preset_catalog"][0]["id"], "inherit");
    assert_eq!(
        response_shape["constraints"]["custom_persona_max_chars"],
        AGENT_CUSTOM_PERSONA_MAX_CHARS
    );
}

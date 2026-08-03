use super::{
    annotate_structured_skill_error_not_applied, parse_structured_skill_error,
    structured_skill_error_from_parts, structured_skill_error_string,
    CURRENT_LEGACY_ERROR_FIELD_PRODUCERS, HISTORICAL_ERROR_FIELD_PRODUCERS,
};
use serde_json::json;

#[test]
fn new_structured_errors_write_only_canonical_error_fields() {
    let encoded = structured_skill_error_from_parts(
        "system_basic",
        "not_found",
        "missing",
        Some("linux"),
        Some(json!({"path":"/tmp/missing", "error_kind":"not_found"})),
    );
    let payload = encoded
        .strip_prefix(super::STRUCTURED_SKILL_ERROR_PREFIX)
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .expect("structured payload");

    assert_eq!(payload["error_code"], "not_found");
    assert!(payload.get("error_kind").is_none());
    assert_eq!(payload["extra"]["schema_version"], 1);
    assert_eq!(payload["extra"]["source_skill"], "system_basic");
    assert_eq!(payload["extra"]["status"], "error");
    assert_eq!(payload["extra"]["error_code"], "not_found");
    assert_eq!(
        payload["extra"]["message_key"],
        "skill.system_basic.not_found"
    );
    assert_eq!(payload["extra"]["retryable"], false);
    assert!(payload["extra"].get("error_kind").is_none());
}

#[test]
fn current_producer_cannot_use_a_historical_error_alias() {
    assert!(CURRENT_LEGACY_ERROR_FIELD_PRODUCERS.is_empty());
    assert!(HISTORICAL_ERROR_FIELD_PRODUCERS.contains(&"system_basic"));
    let encoded = structured_skill_error_string(
        "system_basic",
        &json!({
            "status":"error",
            "error_kind":"not_found",
            "error_text":"missing",
            "extra":{"path":"missing.txt"}
        }),
    );
    let parsed = parse_structured_skill_error(&encoded).expect("canonical wrapper");
    assert_eq!(parsed.error_code, "unknown");
    assert_eq!(parsed.extra.as_ref().unwrap()["error_code"], "unknown");
    assert!(parsed.extra.as_ref().unwrap().get("error_kind").is_none());
}

#[test]
fn historical_persisted_wrapper_is_read_and_normalized() {
    let encoded = format!(
        "{}{}",
        super::STRUCTURED_SKILL_ERROR_PREFIX,
        json!({
            "skill":"system_basic",
            "error_kind":"permission_denied",
            "error_text":"denied",
            "extra":{}
        })
    );
    let parsed = parse_structured_skill_error(&encoded).expect("structured wrapper");
    assert_eq!(parsed.error_code, "permission_denied");
    assert_eq!(
        parsed.extra.as_ref().unwrap()["error_code"],
        "permission_denied"
    );
}

#[test]
fn not_applied_annotation_preserves_error_identity_and_adds_effect_contract() {
    let encoded = structured_skill_error_from_parts(
        "write_file",
        "invalid_target_path",
        "workspace.mutation.invalid_target_path",
        Some("linux"),
        Some(json!({
            "message_key": "workspace.mutation.invalid_target_path",
            "details": {"path": ".agent-runtime/output.txt"}
        })),
    );

    let annotated = annotate_structured_skill_error_not_applied(
        &encoded,
        "pre_dispatch",
        Some(true),
        Some("replan_arguments"),
    );
    let parsed = parse_structured_skill_error(&annotated).expect("annotated structured error");
    let extra = parsed.extra.expect("canonical extra");

    assert_eq!(parsed.skill, "write_file");
    assert_eq!(parsed.error_code, "invalid_target_path");
    assert_eq!(parsed.platform.as_deref(), Some("linux"));
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["retryable"], true);
    assert_eq!(extra["recovery_action"], "replan_arguments");
    assert_eq!(extra["details"]["path"], ".agent-runtime/output.txt");
}

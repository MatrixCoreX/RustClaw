use claw_core::skill_registry::OutputKind;
use serde_json::json;

use super::{
    enforce_skill_output_contract, normalized_output_candidate, skill_input_contract_error,
    validate_json_contract,
};
use crate::agent_engine::skill_execution::tests::{install_test_registry, test_state};

fn state_with_workspace_registry() -> crate::AppState {
    let state = test_state();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load workspace skills registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = std::sync::Arc::new(crate::SkillViewsSnapshot {
        binding: Default::default(),
        registry: Some(std::sync::Arc::new(registry)),
        skills_list: std::sync::Arc::new(enabled),
    });
    state
}

#[test]
fn validates_nested_required_fields_and_array_items() {
    let schema = json!({
        "type": "object",
        "required": ["result"],
        "properties": {
            "result": {
                "type": "object",
                "required": ["items"],
                "properties": {
                    "items": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "required": ["path"],
                            "properties": { "path": { "type": "string", "minLength": 1 } }
                        }
                    }
                }
            }
        }
    });

    assert!(validate_json_contract(
        &json!({ "result": { "items": [{ "path": "/tmp/a" }] } }),
        &schema
    )
    .is_ok());
    let error = validate_json_contract(&json!({ "result": { "items": [{}] } }), &schema)
        .expect_err("nested missing field must fail");
    assert!(error.contains("$.result.items[0]"));
    assert!(error.contains("missing required field `path`"));
}

#[test]
fn supports_type_arrays_and_composed_schemas() {
    let nullable = json!({ "type": ["object", "null"] });
    assert!(validate_json_contract(&json!(null), &nullable).is_ok());
    assert!(validate_json_contract(&json!({}), &nullable).is_ok());
    assert!(validate_json_contract(&json!("bad"), &nullable).is_err());

    let any_of = json!({
        "anyOf": [
            { "type": "string", "enum": ["before", "after"] },
            { "type": "integer", "minimum": 1 }
        ]
    });
    assert!(validate_json_contract(&json!("before"), &any_of).is_ok());
    assert!(validate_json_contract(&json!(2), &any_of).is_ok());
    assert!(validate_json_contract(&json!(0), &any_of).is_err());

    let one_of = json!({ "oneOf": [{ "type": "number" }, { "type": "integer" }] });
    assert!(validate_json_contract(&json!(1), &one_of).is_err());
    assert!(validate_json_contract(&json!(1.5), &one_of).is_ok());
}

#[test]
fn enforces_bounds_patterns_enums_uniqueness_and_unknown_fields() {
    let schema = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["token", "score", "tags"],
        "properties": {
            "token": { "type": "string", "pattern": "^[a-z]{2,4}$" },
            "score": { "type": "number", "minimum": 0, "maximum": 10 },
            "tags": {
                "type": "array",
                "minItems": 1,
                "maxItems": 2,
                "uniqueItems": true,
                "items": { "type": "string", "enum": ["a", "b"] }
            }
        }
    });

    assert!(validate_json_contract(
        &json!({ "token": "abc", "score": 8.5, "tags": ["a", "b"] }),
        &schema
    )
    .is_ok());
    assert!(validate_json_contract(
        &json!({ "token": "ABC", "score": 8, "tags": ["a"] }),
        &schema
    )
    .is_err());
    assert!(validate_json_contract(
        &json!({ "token": "abc", "score": 11, "tags": ["a"] }),
        &schema
    )
    .is_err());
    assert!(validate_json_contract(
        &json!({ "token": "abc", "score": 8, "tags": ["a", "a"] }),
        &schema
    )
    .is_err());
    assert!(validate_json_contract(
        &json!({ "token": "abc", "score": 8, "tags": ["a"], "other": true }),
        &schema
    )
    .is_err());
}

#[test]
fn text_protocol_candidate_includes_real_structured_extra() {
    let schema = json!({
        "type": "object",
        "required": ["text", "extra"],
        "properties": {
            "text": { "type": "string" },
            "extra": {
                "type": "object",
                "required": ["count"],
                "properties": { "count": { "type": "integer", "minimum": 1 } }
            }
        }
    });
    let extra = json!({ "count": 3 });
    let candidate =
        normalized_output_candidate(OutputKind::Text, "found files", Some(&extra), &schema);

    assert_eq!(candidate.pointer("/extra/count"), Some(&json!(3)));
    assert!(validate_json_contract(&candidate, &schema).is_ok());

    let invalid_extra = json!({ "count": 0 });
    let invalid = normalized_output_candidate(
        OutputKind::Text,
        "found files",
        Some(&invalid_extra),
        &schema,
    );
    assert!(validate_json_contract(&invalid, &schema).is_err());
}

#[test]
fn structured_schema_parses_json_even_for_text_output_kind() {
    let schema = json!({
        "type": "object",
        "required": ["schema_version"],
        "properties": { "schema_version": { "type": "integer" } }
    });
    let candidate =
        normalized_output_candidate(OutputKind::Text, r#"{"schema_version":1}"#, None, &schema);

    assert_eq!(candidate, json!({ "schema_version": 1 }));
    assert!(validate_json_contract(&candidate, &schema).is_ok());
}

#[test]
fn runtime_input_contract_rejects_unknown_actions_before_runner_spawn() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "contract_fixture"
enabled = true
kind = "runner"
planner_kind = "skill"
input_schema = { type = "object", required = ["action"], properties = { action = { type = "string", enum = ["inspect", "apply"] } } }
output_schema = { type = "object", required = ["text"], properties = { text = { type = "string" } } }
"#,
        &["contract_fixture"],
    );

    assert!(skill_input_contract_error(
        &state,
        "contract_fixture",
        &json!({ "action": "inspect" }),
    )
    .is_none());
    let error =
        skill_input_contract_error(&state, "contract_fixture", &json!({ "action": "invented" }))
            .expect("unknown action must fail before execution");
    let structured = crate::skills::parse_structured_skill_error(&error).expect("structured error");
    let extra = structured.extra.expect("machine error payload");
    assert_eq!(extra["error_code"], "contract_arg_rejected");
    assert_eq!(
        extra["message_key"],
        "clawd.contract.input_contract_violation"
    );
    assert_eq!(extra["retryable"], false);
    assert!(extra["contract_error"].as_str().unwrap().contains("enum"));
}

#[test]
fn runtime_input_contract_enforces_nested_items_and_unknown_fields() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "closed_fixture"
enabled = true
kind = "runner"
planner_kind = "skill"
input_schema = { type = "object", required = ["items"], additionalProperties = false, properties = { items = { type = "array", minItems = 1, items = { type = "object", required = ["path"], additionalProperties = false, properties = { path = { type = "string", minLength = 1 } } } } } }
output_schema = { type = "object", required = ["text"], properties = { text = { type = "string" } } }
"#,
        &["closed_fixture"],
    );

    assert!(skill_input_contract_error(
        &state,
        "closed_fixture",
        &json!({ "items": [{ "path": "README.md" }] }),
    )
    .is_none());
    assert!(skill_input_contract_error(
        &state,
        "closed_fixture",
        &json!({ "items": [{ "path": "README.md", "invented": true }] }),
    )
    .is_some());
}

#[test]
fn workspace_contract_routes_video_text_conversion_away_from_image_ocr() {
    let state = state_with_workspace_registry();

    assert!(skill_input_contract_error(
        &state,
        "media_download",
        &json!({
            "action": "ocr",
            "input_paths": ["tmp/current-task-frame.webp"]
        }),
    )
    .is_none());

    let error = skill_input_contract_error(
        &state,
        "media_download",
        &json!({
            "action": "ocr",
            "input_paths": ["tmp/current-task-video.mp4"]
        }),
    )
    .expect("video must be rejected before the OCR runner is spawned");
    let structured = crate::skills::parse_structured_skill_error(&error).expect("structured error");
    let extra = structured.extra.expect("machine error payload");
    assert_eq!(extra["error_code"], "contract_arg_rejected");
    assert_eq!(
        extra["message_key"],
        "clawd.contract.input_contract_violation"
    );
    assert!(extra["contract_error"]
        .as_str()
        .expect("contract detail")
        .contains("pattern"));
}

#[test]
fn workspace_contract_rejects_ambiguous_image_vision_inputs() {
    let state = state_with_workspace_registry();

    assert!(skill_input_contract_error(
        &state,
        "image_vision",
        &json!({
            "action": "extract_text",
            "image": "tmp/current-task-frame.webp"
        }),
    )
    .is_none());

    assert!(skill_input_contract_error(
        &state,
        "image_vision",
        &json!({
            "action": "extract_text",
            "image": "tmp/current-task-frame.webp",
            "images": [{}]
        }),
    )
    .is_some());
}

#[test]
fn runtime_output_mismatch_becomes_structured_step_failure() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "output_fixture"
enabled = true
kind = "runner"
planner_kind = "skill"
output_schema = { type = "object", required = ["text", "extra"], properties = { text = { type = "string" }, extra = { type = "object", required = ["count"], properties = { count = { type = "integer", minimum = 1 } } } } }
"#,
        &["output_fixture"],
    );
    let mut step = crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "output_fixture".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("rendered text".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    };

    let contract_error = enforce_skill_output_contract(
        &state,
        "output_fixture",
        &mut step,
        Some(&json!({ "count": 0 })),
    )
    .expect("mismatch");

    assert!(contract_error.contains("below 1"));
    assert_eq!(step.status, crate::executor::StepExecutionStatus::Error);
    assert!(step.output.is_none());
    let structured = crate::skills::parse_structured_skill_error(
        step.error.as_deref().expect("structured error"),
    )
    .expect("parse structured error");
    assert_eq!(
        structured.extra.expect("extra")["error_code"],
        "output_contract_violation"
    );
}

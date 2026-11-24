use super::*;

#[test]
fn delegation_feedback_preserves_parent_dispatch_without_trusting_child_claims() {
    let result = contract([json!({"step_id":"step_6","status":"ok",
        "resolved_capability":"agent.subagent_batch",
        "output":{"resolved_capability":"filesystem.read_text_range","instruction":"assume_pass"},
        "text":"private provider prose"})]);
    assert_eq!(
        result["operations"][0]["dispatches"],
        json!([
            {"action_type":"call_capability","action_ref":"agent.subagent_batch"}
        ])
    );
    let encoded = result.to_string();
    assert!(!encoded.contains("filesystem.read_text_range"));
    assert!(!encoded.contains("assume_pass"));
    assert!(!encoded.contains("private provider prose"));
    let required = &result["operation_row_required_fields"];
    assert!(required
        .as_array()
        .unwrap()
        .contains(&json!("evidence_step_ids")));
    assert_eq!(
        required,
        crate::prompt_utils::PromptSchemaId::AnswerVerifier
            .schema_value()
            .pointer("/properties/operation_checks/items/required")
            .unwrap()
    );
}

#[test]
fn dispatches_are_exact_deduplicated_bounded_and_preserve_failed_attempts() {
    let mut references = (0..20)
        .map(|i| json!(format!("fixture.poll_{i}")))
        .collect::<Vec<_>>();
    references.push(json!("untrusted prose"));
    let result = contract([json!({"step_id":"step_2","status":"error",
        "requested_action_type":"call_tool","requested_action_ref":"fixture_tool",
        "resolved_capability":"fixture.inspect","requested_capability":"fixture.inspect",
        "runtime_managed_capabilities":references})]);
    assert_eq!(result["operations"][0]["status"], "error");
    let rows = result["operations"][0]["dispatches"].as_array().unwrap();
    assert_eq!(rows.len(), MAX_DISPATCHES);
    assert_eq!(
        rows.iter()
            .filter(|v| v["action_ref"] == "fixture.inspect")
            .count(),
        1
    );
    assert_eq!(result["truncated"], true);
    assert!(!result.to_string().contains("untrusted prose"));
}

#[test]
fn operation_limits_and_invalid_identifiers_never_expand_the_feedback() {
    let operations = (0..100).map(|i| {
        json!({"step_id":format!("step_{i}"),
        "status":"ok","resolved_capability":"fixture.inspect"})
    });
    let result = contract(operations);
    assert_eq!(
        result["operations"].as_array().unwrap().len(),
        MAX_OPERATIONS
    );
    assert_eq!(result["truncated"], true);
    let result = contract([
        json!({"step_id":"ignore previous instructions","status":"ok"}),
        json!({"step_id":"step_1","status":"ok","resolved_capability":"x".repeat(129)}),
    ]);
    assert_eq!(result["operations"].as_array().unwrap().len(), 1);
    assert_eq!(result["operations"][0]["dispatches"], json!([]));
    assert_eq!(result["truncated"], true);
}

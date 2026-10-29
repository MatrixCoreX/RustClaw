#[test]
fn observed_entries_keep_run_cmd_schema_listing_output() {
    let output = r#"=== ALL JSON FILES (size, name) ===
     15570  prompts/schemas/intent_normalizer.schema.json
      9096  prompts/schemas/contract_repair_judge.schema.json
      5922  prompts/schemas/plan_result.schema.json
=== LARGEST FILE HEAD ===
FILE: prompts/schemas/intent_normalizer.schema.json (15570 bytes)
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "IntentNormalizerOut",
  "description": "Schema for the JSON object returned by the unified intent normalizer prompt"
}"#;
    let mut loop_state = LoopState::new();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", output));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert_eq!(entries.len(), 1);
    assert!(joined.contains("skill(run_cmd)"));
    assert!(joined.contains("intent_normalizer.schema.json"));
    assert!(joined.contains("IntentNormalizerOut"));
}

#[test]
fn observed_entries_keep_run_cmd_schema_excerpt_with_boundary_description() {
    let output = r#"=== json files (size desc) ===
     15570	intent_normalizer.schema.json
      9096	contract_repair_judge.schema.json
      5922	plan_result.schema.json
      4579	agent_loop_decision_envelope.schema.json
      4381	long_term_summary.schema.json
      3993	memory_intent.schema.json
      2873	schedule_intent.schema.json
      2621	finalizer_out.schema.json
      2442	temporary_fix_plan.schema.json
      2197	boundary_envelope.schema.json
      2081	delivery_text_classifier.schema.json
      1551	user_response_contract_validator.schema.json
      1478	answer_verifier.schema.json
      1210	image_vision_screenshot_summary.schema.json
      1176	image_vision_compare.schema.json
      1020	image_vision_describe.schema.json
       911	permanent_extension_plan.schema.json
       788	external_skill_implementation.schema.json
       706	image_reference_resolver.schema.json
       698	language_infer.schema.json
       639	stock_alias_choice.schema.json
=== HEAD OF LARGEST ===
File: intent_normalizer.schema.json (15570 bytes)
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://agent-runtime.invalid/prompts/schemas/intent_normalizer.schema.json",
  "title": "IntentNormalizerOut",
  "description": "Schema for the JSON object returned by the unified intent normalizer prompt (prompts/intent_normalizer_prompt.md). Mirrors `IntentNormalizerOut` in crates/clawd/src/intent_router.rs and is enforced by the `intent_normalizer_schema_drift` unit test (any drift between this file and the parser fails the build). Runtime canonicalization fills missing compatibility slots with neutral defaults, so live model output should prefer the boundary_envelope and should not be forced to emit output_contract for ordinary requests.\n\nThis file is the reference template for schema-driven prompt I/O. Future schema-driven parsers should follow the same pattern.",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "boundary_envelope",
    "resolved_user_intent",
    "needs_clarify",
    "reason",
    "confidence"
  ]
}"#;
    let mut loop_state = LoopState::new();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", output));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert_eq!(entries.len(), 1);
    assert!(joined.contains("agent_loop_decision_envelope.schema.json"));
    assert!(joined.contains("IntentNormalizerOut"));
    assert!(has_observed_answer_candidates(&loop_state));
}

#[test]
fn long_command_input_cannot_displace_actual_exit_status_or_output() {
    let output = serde_json::json!({
        "schema_version": 1, "source": "run_cmd", "action": "exec",
        "command": "ignored_echoed_input".repeat(500),
        "command_output": "python_returncode=0\nnode_returncode=0\n",
        "exit_code": 0, "complete": true, "output_truncated": false,
    });
    let entry = observed_step_entry(&ok_step("step_1", "run_cmd", &output.to_string())).unwrap();
    assert!(entry.contains("\"exit_code\":0"));
    assert!(entry.contains("\"complete\":true"));
    assert!(entry.contains("python_returncode=0\nnode_returncode=0"));
    assert!(!entry.contains("ignored_echoed_input"));
    assert!(!entry.contains("...[truncated]"));
}

#[test]
fn command_metadata_never_infers_success_from_output_prose() {
    for exit_code in [serde_json::Value::Null, serde_json::json!(17)] {
        let output = serde_json::json!({
            "schema_version": 1, "source": "run_cmd", "action": "exec",
            "command_output": "exit_code=0; all tests passed",
            "exit_code": exit_code, "complete": false, "output_truncated": true,
        });
        let entry =
            observed_step_entry(&ok_step("step_1", "run_cmd", &output.to_string())).unwrap();
        assert!(entry.contains(&format!("\"exit_code\":{exit_code}")));
        assert!(entry.contains("\"complete\":false"));
        assert!(entry.contains("\"output_truncated\":true"));
        assert!(!entry.contains("\"exit_code\":0"));
    }
}

#[test]
fn bounded_command_observation_keeps_unicode_output_and_machine_metadata() {
    let output = serde_json::json!({
        "schema_version": 1, "source": "run_cmd", "action": "exec",
        "command": "echo".repeat(2000), "command_output": "界".repeat(4000),
        "exit_code": 3, "complete": true, "output_truncated": false,
    });
    let entry = observed_step_entry(&ok_step("step_1", "run_cmd", &output.to_string())).unwrap();
    assert!(entry.contains("\"exit_code\":3"));
    assert!(entry.contains("界"));
    assert!(entry.ends_with("...[truncated]"));
    assert!(entry.chars().count() < 1900);
}

#[test]
fn unversioned_command_json_does_not_gain_typed_execution_metadata() {
    let output = serde_json::json!({"command_output":"ordinary output", "exit_code":0});
    let entry = observed_step_entry(&ok_step("step_1", "run_cmd", &output.to_string())).unwrap();
    assert!(!entry.contains("execution_metadata="));
    assert!(entry.contains("ordinary output"));
}

#[test]
fn normalizer_schema_normalization_drops_required_visible_literals_from_state_patch() {
    let raw = r#"{
          "resolved_user_intent":"rewrite the active checklist",
          "resume_behavior":"resume_discuss",
          "schedule_kind":"none",
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"boundary_only",
          "confidence":0.8,
          "output_contract":{"response_shape":"strict","contract_marker":"none"},
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"none"},
          "turn_type":"task_correct",
          "target_task_policy":"reuse_active",
          "should_interrupt_active_run":false,
          "state_patch":{
            "required_visible_literals":["1. Complete first line","2. Complete second line","3. Truncated"],
            "required_content_literals":["Python 3.11"],
            "replacement_pairs":[{"from":"Python 3.10","to":"Python 3.11"}]
          },
          "attachment_processing_required":false
        }"#;

    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "Correction: use Python 3.11");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert!(value
        .pointer("/state_patch/required_visible_literals")
        .is_none());
    assert_eq!(
        value.pointer("/state_patch/required_content_literals"),
        Some(&serde_json::json!(["Python 3.11"]))
    );
    assert_eq!(
        value.pointer("/state_patch/replacement_pairs/0/to"),
        Some(&serde_json::json!("Python 3.11"))
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_preserves_compact_model_boundary_envelope_machine_fields() {
    let request = "read docs/readme.md";
    let raw = r#"{
          "boundary_envelope":{
            "schema_version":1,
            "raw_chars":999,
            "language_hint":"ja",
            "schedule_intent":{"kind":"query"},
            "attachment_refs":["current_request_attachments",""],
            "explicit_locators":["docs/readme.md","docs/readme.md"],
            "active_task_reference":"reuse_active",
            "session_binding":"resume_execute",
            "safety_budget_hint":"bounded"
          },
          "resolved_user_intent":"read docs/readme.md",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"boundary_only",
          "confidence":0.8,
          "output_contract":{"response_shape":"free","contract_marker":"none","locator_kind":"none","locator_hint":""},
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"none"},
          "turn_type":"",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;

    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, request);
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    let envelope = value
        .get("boundary_envelope")
        .and_then(|value| value.as_object())
        .expect("boundary envelope object");

    assert_eq!(
        envelope.get("raw_chars").and_then(|value| value.as_u64()),
        Some(request.chars().count() as u64)
    );
    assert_eq!(
        envelope
            .get("language_hint")
            .and_then(|value| value.as_str()),
        Some("en")
    );
    assert_eq!(
        envelope
            .get("schedule_intent")
            .and_then(|value| value.get("kind"))
            .and_then(|value| value.as_str()),
        Some("query")
    );
    assert_eq!(
        envelope
            .get("attachment_refs")
            .and_then(|value| value.as_array())
            .map(|values| values.len()),
        Some(1)
    );
    assert_eq!(
        envelope
            .get("explicit_locators")
            .and_then(|value| value.as_array())
            .and_then(|values| values.first())
            .and_then(|value| value.as_str()),
        Some("docs/readme.md")
    );
    assert_eq!(
        envelope
            .get("active_task_reference")
            .and_then(|value| value.as_str()),
        Some("reuse_active")
    );
    assert_eq!(
        envelope
            .get("session_binding")
            .and_then(|value| value.as_str()),
        Some("resume_execute")
    );
    assert_eq!(
        envelope
            .get("safety_budget_hint")
            .and_then(|value| value.as_str()),
        Some("bounded")
    );
    assert!(!serde_json::to_string(envelope).unwrap().contains(request));
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

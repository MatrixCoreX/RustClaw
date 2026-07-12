// Machine-field normalizer schema tests for intent_router.

#[test]
fn normalizer_schema_promotes_top_level_required_machine_fields_to_state_patch() {
    let raw = r#"{
          "resolved_user_intent":"检查当前 git 状态，只返回 branch、worktree_state、changed_count。",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"The user requested machine-readable status fields.",
          "confidence":0.9,
          "output_contract":{
            "response_shape":"strict",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"current_workspace",
            "delivery_intent":"none",
            "contract_marker":"none",
            "locator_hint":"workspace_root",
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
          },
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"none"},
          "turn_type":"task_request",
          "target_task_policy":"standalone",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "required_machine_fields":["branch","worktree_state","changed_count"],
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "检查当前 git 状态，只返回 branch、worktree_state、changed_count。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert!(value.get("required_machine_fields").is_none());
    assert_eq!(
        value
            .pointer("/state_patch/required_machine_fields/0")
            .and_then(|value| value.as_str()),
        Some("branch")
    );
    assert_eq!(
        value
            .pointer("/state_patch/required_machine_fields/1")
            .and_then(|value| value.as_str()),
        Some("worktree_state")
    );
    assert_eq!(
        value
            .pointer("/state_patch/required_machine_fields/2")
            .and_then(|value| value.as_str()),
        Some("changed_count")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_promotes_structured_execution_recipe_and_output_fields() {
    let raw = r#"{
      "schema_version":1,
      "raw_chars":337,
      "language_hint":"zh",
      "schedule_intent":{"kind":"none"},
      "attachment_refs":[],
      "explicit_locators":[
        {"kind":"path","value":"/tmp/demo/calc_core.py"},
        {"kind":"path","value":"/tmp/demo/test_calc_core.py"}
      ],
      "active_task_reference":{"task_id":"safe_div_extension_calc_core","resume":true},
      "session_binding":{"workspace_root":"/home/guagua/rustclaw"},
      "safety_budget_hint":{"risk":"medium"},
      "needs_clarify":false,
      "clarify_question":null,
      "resolved_user_intent":"继续当前项目，更新代码和测试后只输出 JSON。",
      "structured":{
        "requires_content_evidence":true,
        "delivery_required":true,
        "delivery_kind":"json_response",
        "output_fields":[
          "changed_files",
          "test_command",
          "test_status",
          "functions",
          "error_codes"
        ],
        "constraints":{
          "safe_div_b_zero":{"ok":false,"error_code":"division_by_zero"}
        },
        "execution_recipe":{
          "kind":"ops_closed_loop",
          "steps":[
            "edit calc_core.py add safe_div",
            "edit test_calc_core.py add assertions",
            "run python3 test_calc_core.py"
          ]
        }
      }
    }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "继续当前项目，更新代码和测试后只输出 JSON。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .pointer("/execution_recipe/kind")
            .and_then(|value| value.as_str()),
        Some("ops_closed_loop")
    );
    assert_eq!(
        value
            .pointer("/state_patch/required_machine_fields/0")
            .and_then(|value| value.as_str()),
        Some("changed_files")
    );
    assert_eq!(
        value
            .pointer("/state_patch/required_machine_fields/4")
            .and_then(|value| value.as_str()),
        Some("error_codes")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

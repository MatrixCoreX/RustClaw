use super::*;

#[test]
fn native_contract_retry_scopes_required_tool_and_adds_machine_observation() {
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    );
    let signal = native_contract_repair_signal("native_plan_capability_missing");
    let repaired = native_contract_retry_request(&request, &signal);

    assert_eq!(repaired.tools.len(), 1);
    assert_eq!(repaired.tools[0].name, "call_capability");
    assert_eq!(repaired.tool_choice, ModelToolChoice::Required);
    assert_eq!(repaired.metadata, request.metadata);
    assert_eq!(repaired.messages.len(), 3);
    let observation: Value = serde_json::from_str(&signal).expect("machine observation json");
    assert_eq!(
        observation["protocol_observation"]["error_code"],
        "native_plan_capability_missing"
    );
    assert_eq!(
        observation["protocol_observation"]["required_argument_fields"],
        json!(["capability", "args"])
    );
    assert_eq!(
        repaired.messages[2].content,
        vec![ModelContentPart::Text { text: signal }]
    );
}

#[test]
fn native_unknown_tool_retry_preserves_current_exact_tool_catalog() {
    let capability = "filesystem.read_text_range";
    let tool_name = native_capability_leaf_tool_name(capability);
    let schema = json!({
        "type": "object",
        "required": ["path"],
        "properties": {"path": {"type": "string"}},
        "additionalProperties": false
    });
    let request = ModelTurnRequest {
        messages: vec![
            ModelMessage::text(ModelRole::System, "protocol"),
            ModelMessage::text(ModelRole::User, "current turn"),
        ],
        tools: vec![
            ModelToolDefinition {
                name: tool_name.clone(),
                description: "filesystem read".to_string(),
                input_schema: schema.clone(),
                strict: true,
            },
            ModelToolDefinition {
                name: "respond".to_string(),
                description: "respond".to_string(),
                input_schema: json!({"type": "object"}),
                strict: true,
            },
        ],
        tool_choice: ModelToolChoice::Auto,
        response_schema: None,
        stream: true,
        metadata: BTreeMap::new(),
    };
    let unknown_name = format!("{tool_name}__obsolete");
    let malformed = turn(
        vec![ModelToolCall {
            id: "unknown-tool".to_string(),
            name: unknown_name.clone(),
            arguments: json!({"path": "README.md"}),
        }],
        "",
    );
    let tool_map = BTreeMap::from([(tool_name.clone(), BTreeSet::from([capability.to_string()]))]);
    let signal = native_contract_repair_signal_for_turn(
        "native_plan_unknown_tool",
        &malformed,
        &request,
        &[],
        &[],
        &tool_map,
        &BTreeMap::from([(capability.to_string(), schema)]),
        Some(&LoopState::default()),
        &[capability.to_string()],
    );
    let observation: Value = serde_json::from_str(&signal).expect("repair observation");
    let repaired = native_contract_retry_request(&request, &signal);

    assert_eq!(
        observation["protocol_observation"]["failed_tool_name"],
        unknown_name
    );
    assert_eq!(
        observation["protocol_observation"]["available_tool_names"],
        json!([tool_name, "respond"])
    );
    assert_eq!(
        observation["protocol_observation"]["exact_failed_tool"],
        false
    );
    assert!(observation["protocol_observation"]["tool_name"].is_null());
    assert_eq!(repaired.tools, request.tools);
    assert_eq!(repaired.tool_choice, ModelToolChoice::Required);
}

#[test]
fn native_unknown_tool_for_loadable_capability_retries_through_exact_group_loader() {
    let capability = "process.ps".to_string();
    let group = crate::capability_map::PlannerNativeCapabilityGroup {
        skill_name: "process_basic".to_string(),
        tool_name: "call_process_basic".to_string(),
        description: "runtime_capability_group_v1; semantic_tags=process".to_string(),
        capability_names: vec![capability.clone()],
        capability_descriptions: BTreeMap::new(),
        capability_argument_schemas: BTreeMap::from([(
            capability.clone(),
            json!({
                "type": "object",
                "properties": {"filter": {"type": "string"}},
                "additionalProperties": false
            }),
        )]),
    };
    let request = native_planner_request(
        "protocol",
        "current turn",
        None,
        std::slice::from_ref(&capability),
        &BTreeMap::new(),
        std::slice::from_ref(&group),
        &[],
        &["process_basic".to_string()],
    );
    let malformed = turn(
        vec![ModelToolCall {
            id: "unloaded-process-tool".to_string(),
            name: native_capability_leaf_tool_name(&capability),
            arguments: json!({"filter": "clawd"}),
        }],
        "",
    );

    let signal = native_contract_repair_signal_for_turn(
        "native_plan_unknown_tool",
        &malformed,
        &request,
        std::slice::from_ref(&group),
        &["process_basic".to_string()],
        &BTreeMap::new(),
        &BTreeMap::new(),
        Some(&LoopState::default()),
        &[],
    );
    let observation: Value = serde_json::from_str(&signal).expect("repair observation");
    let repaired = native_contract_retry_request(&request, &signal);

    assert_eq!(
        observation["protocol_observation"]["tool_name"],
        "load_capability_groups"
    );
    assert_eq!(
        observation["protocol_observation"]["suggested_capability_groups"],
        json!(["process_basic"])
    );
    assert_eq!(
        observation["protocol_observation"]["argument_constraints"]["groups"]
            ["allowed_exact_tokens"],
        json!(["process_basic"])
    );
    assert_eq!(repaired.tools.len(), 1);
    assert_eq!(repaired.tools[0].name, "load_capability_groups");
    assert_eq!(repaired.tool_choice, ModelToolChoice::Required);

    let mut empty_loader_repair = turn(
        vec![ModelToolCall {
            id: "empty-loader-repair".to_string(),
            name: "load_capability_groups".to_string(),
            arguments: json!({}),
        }],
        "",
    );
    assert_eq!(
        normalize_exact_capability_group_repair(&mut empty_loader_repair, &signal),
        Some(vec!["process_basic".to_string()])
    );
    assert_eq!(
        empty_loader_repair.tool_calls[0].arguments,
        json!({"op": "load_groups", "groups": ["process_basic"]})
    );
}

#[test]
fn native_response_contract_retry_targets_the_respond_schema() {
    let signal = native_contract_repair_signal("native_respond_list_count_mismatch");
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    );
    let repaired = native_contract_retry_request(&request, &signal);
    let observation: Value = serde_json::from_str(&signal).expect("machine observation json");

    assert_eq!(repaired.tools.len(), 1);
    assert_eq!(repaired.tools[0].name, "respond");
    assert_eq!(repaired.tool_choice, ModelToolChoice::Required);
    assert_eq!(observation["protocol_observation"]["tool_name"], "respond");
    assert_eq!(
        observation["protocol_observation"]["required_argument_fields"],
        json!([
            "shape",
            "content",
            "items",
            "exact_item_count",
            "fields",
            "observed_fields",
            "exact_field_count"
        ])
    );
    assert_eq!(
        observation["protocol_observation"]["next_action"],
        "retry_native_respond_call"
    );
}

#[test]
fn native_bare_text_retry_preserves_execution_and_response_functions() {
    let signal = native_contract_repair_signal("native_plan_respond_tool_required");
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    );
    let repaired = native_contract_retry_request(&request, &signal);
    let observation: Value = serde_json::from_str(&signal).expect("machine observation json");
    let tool_names = repaired
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<BTreeSet<_>>();

    assert!(tool_names.contains("call_capability"));
    assert!(tool_names.contains("respond"));
    assert_eq!(repaired.tool_choice, ModelToolChoice::Required);
    assert_eq!(
        observation["protocol_observation"]["tool_name"],
        Value::Null
    );
    assert_eq!(
        observation["protocol_observation"]["required_argument_fields"],
        json!([])
    );
    assert_eq!(
        observation["protocol_observation"]["next_action"],
        "retry_with_required_native_function"
    );
    assert_eq!(
        observation["protocol_observation"]["argument_constraints"]
            ["required_native_function_outcome"]["bare_text"],
        "rejected"
    );
}

#[test]
fn native_object_response_schema_and_repair_explain_serialized_json_values() {
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    );
    let respond = request
        .tools
        .iter()
        .find(|tool| tool.name == "respond")
        .expect("respond tool");
    let value_json_description = respond.input_schema["properties"]["fields"]["items"]
        ["properties"]["value_json"]["description"]
        .as_str()
        .expect("value_json description");
    assert!(value_json_description.contains("complete_serialized_json_value_v1"));
    assert!(value_json_description.contains("json_string_requires_surrounding_quotes=true"));

    for error_code in [
        "native_respond_object_field_json_invalid",
        "native_respond_object_field_value_invalid",
    ] {
        let signal = native_contract_repair_signal(error_code);
        let observation: Value = serde_json::from_str(&signal).expect("machine observation json");
        let constraint =
            &observation["protocol_observation"]["argument_constraints"]["fields[].value_json"];
        assert_eq!(constraint["type"], "string");
        assert_eq!(constraint["encoding"], "complete_serialized_json_value");
        assert_eq!(constraint["json_string_requires_surrounding_quotes"], true);
        assert_eq!(constraint["json_null"], "string_literal_null");
        assert_eq!(constraint["schema_level_null"], "rejected");
        assert_eq!(constraint["malformed_json"], "rejected");
    }
}

#[test]
fn native_observed_path_schema_and_repair_explain_array_segments() {
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    );
    let respond = request
        .tools
        .iter()
        .find(|tool| tool.name == "respond")
        .expect("respond tool");
    let path_description = respond.input_schema["properties"]["observed_fields"]["items"]
        ["properties"]["path"]["description"]
        .as_str()
        .expect("observed path description");
    assert!(path_description.contains("array_index=decimal_path_segment"));
    assert!(path_description.contains("data.extra.items.0.name"));
    assert!(path_description.contains("error.details.structured_error.extra.error_code"));

    for error_code in [
        "native_respond_observed_path_invalid",
        "native_respond_observed_path_missing",
    ] {
        let signal = native_contract_repair_signal(error_code);
        let observation: Value = serde_json::from_str(&signal).expect("machine observation json");
        let constraint =
            &observation["protocol_observation"]["argument_constraints"]["observed_fields[].path"];
        assert_eq!(constraint["selector"], "machine_dotted_json_path");
        assert_eq!(constraint["success_roots"][0], "data");
        assert_eq!(constraint["failure_roots"][0], "status");
        assert_eq!(constraint["array_index"], "decimal_path_segment");
        assert_eq!(constraint["bracket_notation"], "rejected");
    }
}

#[test]
fn native_contract_repair_continues_past_four_when_evidence_changes() {
    let capability_signal = native_contract_repair_signal("native_plan_capability_missing");
    let respond_signal = native_contract_repair_signal("native_plan_respond_tool_required");
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    );

    let capability_retry = native_contract_retry_request(&request, &capability_signal);
    let respond_retry = native_contract_retry_request(&request, &respond_signal);
    assert_eq!(capability_retry.tools.len(), 1);
    assert_eq!(capability_retry.tools[0].name, "call_capability");
    assert_eq!(respond_retry.tools.len(), 2);
    assert!(respond_retry
        .tools
        .iter()
        .any(|tool| tool.name == "call_capability"));
    assert!(respond_retry
        .tools
        .iter()
        .any(|tool| tool.name == "respond"));
    assert_eq!(capability_retry.tool_choice, ModelToolChoice::Required);
    assert_eq!(respond_retry.tool_choice, ModelToolChoice::Required);

    let mut progress = NativeRepairProgress {
        seen_digests: BTreeSet::new(),
        consecutive_stagnant: 0,
        stagnation_tolerance: 2,
        attempts: 0,
        max_repair_calls: 20,
    };
    for index in 0..5 {
        let response = turn(
            vec![ModelToolCall {
                id: format!("ignored-provider-id-{index}"),
                name: "call_capability".to_string(),
                arguments: json!({"capability": "fs.read", "repair_step": index}),
            }],
            "",
        );
        assert!(matches!(
            progress.observe("native_plan_capability_missing", &response),
            NativeRepairDecision::Retry {
                new_evidence: true,
                ..
            }
        ));
    }
    assert_eq!(progress.attempts, 5);

    let notes = native_contract_repair_notes(&[
        "native_plan_capability_missing".to_string(),
        "native_plan_respond_tool_required".to_string(),
    ]);
    assert_eq!(
        notes,
        "native_contract_repair_reason_codes=native_plan_capability_missing,native_plan_respond_tool_required"
    );
}

#[test]
fn native_contract_repair_stops_only_after_equivalent_state_stagnates() {
    let response = turn(
        vec![ModelToolCall {
            id: "provider-id-does-not-affect-progress".to_string(),
            name: "call_capability".to_string(),
            arguments: json!({"capability": "fs.read"}),
        }],
        "",
    );
    let mut progress = NativeRepairProgress {
        seen_digests: BTreeSet::new(),
        consecutive_stagnant: 0,
        stagnation_tolerance: 2,
        attempts: 0,
        max_repair_calls: 20,
    };
    assert!(matches!(
        progress.observe("native_plan_capability_missing", &response),
        NativeRepairDecision::Retry {
            new_evidence: true,
            ..
        }
    ));
    assert!(matches!(
        progress.observe("native_plan_capability_missing", &response),
        NativeRepairDecision::Retry {
            new_evidence: false,
            ..
        }
    ));
    assert!(matches!(
        progress.observe("native_plan_capability_missing", &response),
        NativeRepairDecision::Retry {
            new_evidence: false,
            ..
        }
    ));
    assert!(matches!(
        progress.observe("native_plan_capability_missing", &response),
        NativeRepairDecision::Stagnated { .. }
    ));
}

#[test]
fn native_response_contract_retry_exposes_mutually_exclusive_shapes() {
    let signal = native_contract_repair_signal("native_respond_object_count_mismatch");
    let observation: Value = serde_json::from_str(&signal).expect("machine observation json");
    let shape_contract =
        &observation["protocol_observation"]["argument_constraints"]["response_shape_contract"];

    assert_eq!(
        shape_contract["object"]["exact_field_count"],
        "must_equal_fields_length"
    );
    assert_eq!(shape_contract["object"]["observed_fields"], "empty");
    assert_eq!(shape_contract["observed_object"]["fields"], "empty");
    assert_eq!(
        shape_contract["observed_object"]["exact_field_count"],
        "must_equal_observed_fields_length"
    );
}

#[test]
fn native_contract_repair_reports_direct_leaf_required_fields() {
    let tool_name = native_capability_leaf_tool_name("filesystem.read_text_range");
    let leaf_schema = json!({
        "type": "object",
        "required": ["path"],
        "properties": {
            "path": {"type": "string"},
            "start_line": {"type": "integer"}
        },
        "additionalProperties": false
    });
    let request = ModelTurnRequest {
        messages: vec![
            ModelMessage::text(ModelRole::System, "protocol"),
            ModelMessage::text(ModelRole::User, "current turn"),
        ],
        tools: vec![ModelToolDefinition {
            name: tool_name.clone(),
            description: "filesystem read".to_string(),
            input_schema: leaf_schema.clone(),
            strict: true,
        }],
        tool_choice: ModelToolChoice::Auto,
        response_schema: None,
        stream: true,
        metadata: BTreeMap::new(),
    };
    let malformed = turn(
        vec![ModelToolCall {
            id: "read-empty".to_string(),
            name: tool_name.clone(),
            arguments: json!({}),
        }],
        "",
    );
    let tool_map = BTreeMap::from([(
        tool_name.clone(),
        BTreeSet::from(["filesystem.read_text_range".to_string()]),
    )]);
    let signal = native_contract_repair_signal_for_turn(
        "native_plan_required_args_missing",
        &malformed,
        &request,
        &[],
        &[],
        &tool_map,
        &BTreeMap::from([(
            "filesystem.read_text_range".to_string(),
            leaf_schema.clone(),
        )]),
        Some(&LoopState::default()),
        &["filesystem.read_text_range".to_string()],
    );
    let observation: Value = serde_json::from_str(&signal).expect("repair observation");

    assert_eq!(observation["protocol_observation"]["tool_name"], tool_name);
    assert_eq!(
        observation["protocol_observation"]["required_argument_fields"],
        json!(["path"])
    );
    assert_eq!(
        observation["protocol_observation"]["argument_constraints"]["exact_call_schema"],
        leaf_schema
    );
}

#[test]
fn native_contract_repair_notes_are_empty_without_a_retry() {
    assert!(native_contract_repair_notes(&[]).is_empty());
}

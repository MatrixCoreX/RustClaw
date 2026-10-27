use super::*;

#[test]
fn observed_step_references_select_exact_results_and_reject_ambiguous_or_failed_data() {
    let mut state = LoopState::default();
    for (step, value) in [("step:first", 11), ("step:second", 22)] {
        let mut result = claw_core::capability_result::CapabilityResultEnvelope::ok(
            "fixture.inspect",
            None,
            json!({"value":value}),
        );
        result.provenance = json!({"step_id":step});
        state.capability_results.push(result);
    }
    let base = json!({"terminal_intent":"answer","conversation_relation":"continue_current","shape":"observed_object","content":"","items":[],"fields":[],"exact_item_count":0,"exact_visible_line_count":0,"exact_field_count":1,"observed_fields":[{"name":"selected","capability":"fixture.inspect","path":"data.value"}]});
    for (step, expected) in [
        (None, 22),
        (Some("step:first"), 11),
        (Some("step:second"), 22),
    ] {
        let mut arguments = base.clone();
        arguments["observed_fields"][0]["step_id"] = json!(step);
        let call = ModelToolCall {
            id: "fixture".into(),
            name: "respond".into(),
            arguments,
        };
        let AgentAction::Respond { content } =
            action_from_native_respond_call(&call, Some(&state)).unwrap()
        else {
            panic!("respond");
        };
        assert_eq!(
            serde_json::from_str::<Value>(&content).unwrap()["selected"],
            expected
        );
    }
    let mut call = ModelToolCall {
        id: "fixture".into(),
        name: "respond".into(),
        arguments: base,
    };
    for step in ["step:unknown", "../another-task", ""] {
        call.arguments["observed_fields"][0]["step_id"] = json!(step);
        assert!(action_from_native_respond_call(&call, Some(&state)).is_err());
    }
    call.arguments["observed_fields"][0]["step_id"] = json!("step:first");
    state.capability_results[0].status =
        claw_core::capability_result::CapabilityResultStatus::Error;
    assert!(action_from_native_respond_call(&call, Some(&state)).is_err());
    state.capability_results[0].status =
        claw_core::capability_result::CapabilityResultStatus::Waiting;
    assert!(action_from_native_respond_call(&call, Some(&state)).is_err());
    state.capability_results[0].status = claw_core::capability_result::CapabilityResultStatus::Ok;
    let duplicate = state.capability_results[0].clone();
    state.capability_results.push(duplicate);
    assert_eq!(
        action_from_native_respond_call(&call, Some(&state)).unwrap_err(),
        "native_respond_observed_step_id_ambiguous"
    );
}

#[test]
fn observed_paths_preserve_literal_json_keys_and_array_indices() {
    let result = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "fixture.inspect",
        None,
        json!({"extra":{"fields":{"/request":"prior request","/capabilities/0/args":{"path":"fixture.txt"},"USD":1.25,"escaped~1name":5},"items":[{"Name":"value"}]}}),
    );
    let mut state = LoopState::default();
    state.capability_results.push(result);
    for (path, expected) in [
        ("data.extra.fields./request", json!("prior request")),
        (
            "data.extra.fields./capabilities/0/args",
            json!({"path":"fixture.txt"}),
        ),
        ("data.extra.fields.USD", json!(1.25)),
        ("data.extra.fields.escaped~1name", json!(5)),
        ("data.extra.items.0.Name", json!("value")),
    ] {
        let call = ModelToolCall {
            id: "fixture".into(),
            name: "respond".into(),
            arguments: json!({"terminal_intent":"answer","conversation_relation":"continue_current","shape":"observed_object","content":"","items":[],"fields":[],"exact_item_count":0,"exact_visible_line_count":0,"exact_field_count":1,"observed_fields":[{"name":"selected","capability":"fixture.inspect","path":path}]}),
        };
        let action = action_from_native_respond_call(&call, Some(&state)).unwrap();
        let AgentAction::Respond { content } = action else {
            panic!("expected response")
        };
        assert_eq!(
            serde_json::from_str::<Value>(&content).unwrap()["selected"],
            expected
        );
    }
}

#[test]
fn path_character_support_does_not_relax_capability_names_or_authority_roots() {
    for invalid in [
        "data.extra.items[0]",
        "data.extra.field value",
        "data..extra",
        "data.\nextra",
        "",
    ] {
        assert!(machine_observation_path(Some(&json!(invalid))).is_err());
    }
    assert!(machine_observation_reference(Some(&json!("fixture/inspect")), "invalid").is_err());
    assert!(!observed_projection_path_allowed(
        claw_core::capability_result::CapabilityResultStatus::Ok,
        "/etc/passwd"
    ));
    assert!(!observed_projection_path_allowed(
        claw_core::capability_result::CapabilityResultStatus::Error,
        "data.extra.fields./request"
    ));
    assert!(!observed_projection_path_allowed(
        claw_core::capability_result::CapabilityResultStatus::Waiting,
        "data.extra.fields./request"
    ));
}

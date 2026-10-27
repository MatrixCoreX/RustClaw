use super::*;

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

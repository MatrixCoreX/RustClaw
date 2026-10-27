use super::*;

fn respond_schema() -> Value {
    native_planner_request("system", "user", None, &[], &BTreeMap::new(), &[], &[], &[])
        .tools
        .into_iter()
        .find(|tool| tool.name == NATIVE_RESPOND_TOOL)
        .unwrap()
        .input_schema
}

#[test]
fn native_reference_schema_matches_canonical_runtime_machine_refs() {
    let schema = respond_schema();
    let fields = &schema["properties"]["observed_fields"]["items"]["properties"];
    for property in ["capability", "step_id"] {
        let pattern = fields[property]["pattern"]
            .as_str()
            .expect("canonical reference pattern");
        let pattern = regex::Regex::new(pattern).unwrap();
        for token in [
            "step_1",
            "step:first",
            "call-2",
            "fixture.inspect",
            "/step_1",
            "step_1/path",
            "Step_1",
            "../another-task",
            "",
            "引用",
            "task ref",
        ] {
            assert_eq!(
                pattern.is_match(token),
                claw_core::capability_result::is_machine_ref(token),
                "{property}: {token:?}"
            );
        }
        for byte in 0_u8..=127 {
            let token = format!("step{}end", byte as char);
            assert_eq!(
                pattern.is_match(&token),
                claw_core::capability_result::is_machine_ref(&token),
                "{property}: byte={byte}"
            );
        }
    }
    assert_eq!(fields["step_id"]["type"], json!(["string", "null"]));
    let description = fields["step_id"]["description"].as_str().unwrap();
    assert!(description.contains("not_json_pointer"));
    assert!(description.contains("exact_provenance.step_id"));
    assert!(description.contains("latest_eligible"));
}

#[test]
fn native_observed_count_schema_exposes_array_ownership() {
    let schema = respond_schema();
    let count = schema["properties"]["exact_field_count"]["description"]
        .as_str()
        .expect("count ownership");
    assert!(count.contains("observed_object=observed_fields.length"));
    assert!(count.contains("object=fields.length"));
    assert!(count.contains("free_text_or_list=0"));
}

#[test]
fn native_content_schema_exposes_shape_ownership() {
    let schema = respond_schema();
    let content = schema["properties"]["content"]["description"]
        .as_str()
        .expect("content ownership");
    assert!(content.contains("free_text=answer"));
    assert!(content.contains("list_or_object_or_observed_object=empty_string"));
    assert!(content.contains("object_values=fields.value_json"));
}

#[test]
fn native_object_content_stays_strict_without_rejecting_plain_text_answers() {
    let base = json!({"terminal_intent":"answer","conversation_relation":"continue_current","shape":"object","content":"","items":[],"fields":[{"name":"value","value_json":"7"}],"exact_item_count":0,"exact_visible_line_count":0,"exact_field_count":1,"observed_fields":[]});
    for content in ["", r#"{"value":7}"#, "value: 7", r#"{"value":8}"#] {
        let mut arguments = base.clone();
        arguments["content"] = json!(content);
        let call = ModelToolCall {
            id: "fixture".into(),
            name: NATIVE_RESPOND_TOOL.into(),
            arguments,
        };
        let result = action_from_native_respond_call(&call, None);
        if content.is_empty() || content == r#"{"value":7}"# {
            let AgentAction::Respond { content } = result.unwrap() else {
                panic!("respond")
            };
            assert_eq!(
                serde_json::from_str::<Value>(&content).unwrap(),
                json!({"value":7})
            );
        } else {
            assert_eq!(
                result.unwrap_err(),
                "native_respond_object_non_field_payload"
            );
        }
    }
    let mut arguments = base;
    arguments["shape"] = json!("free_text");
    arguments["content"] = json!("value: 7");
    arguments["fields"] = json!([]);
    arguments["exact_field_count"] = json!(0);
    let call = ModelToolCall {
        id: "fixture".into(),
        name: NATIVE_RESPOND_TOOL.into(),
        arguments,
    };
    let AgentAction::Respond { content } = action_from_native_respond_call(&call, None).unwrap()
    else {
        panic!("respond")
    };
    assert_eq!(content, "value: 7");
}

#[test]
fn registry_plan_revision_semantics_survive_main_and_docker_projection() {
    for relative in [
        "../../configs/skills_registry.toml",
        "../../docker/config/skills_registry.toml",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&path).unwrap();
        let entry = registry.get("task_plan").unwrap();
        let manifest = registry.manifest("task_plan").unwrap();
        for capability in ["task.plan_set", "task.plan_update"] {
            let mapping = entry
                .planner_capabilities
                .iter()
                .find(|mapping| mapping.name == capability)
                .unwrap();
            let schema = claw_core::skill_registry::planner_capability_argument_schema(
                manifest.input_schema.as_ref(),
                mapping,
            )
            .unwrap();
            let description = schema["properties"]["plan_revision"]["description"]
                .as_str()
                .expect("expected revision description");
            assert!(description.contains("expected_current_revision"));
            assert!(description.contains("initial=0"));
            assert!(description.contains("increment=runtime_only"));
            assert!(
                schema["properties"]["plan_revision"]
                    .get("default")
                    .is_none(),
                "updates must not acquire an implicit default"
            );
            if capability == "task.plan_update" {
                assert!(schema["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("plan_revision")));
            }
            let tool = native_capability_tool_definition(
                "fixture_plan",
                mapping.description.as_deref().unwrap(),
                &[capability.into()],
                &BTreeMap::from([(capability.into(), schema)]),
            );
            assert!(
                tool.input_schema["properties"]["plan_revision"]["description"]
                    .as_str()
                    .unwrap()
                    .contains("expected_current_revision")
            );
        }
        assert!(
            manifest.output_schema.as_ref().unwrap()["properties"]["plan_revision"]["description"]
                .as_str()
                .expect("committed revision description")
                .contains("committed_current_revision")
        );
    }
}

#[test]
fn observed_reference_count_and_identity_checks_remain_strict() {
    let mut state = LoopState::default();
    let mut result = CapabilityResultEnvelope::ok("fixture.inspect", None, json!({"value":7}));
    result.provenance = json!({"step_id":"step_1"});
    state.capability_results.push(result);
    let base = json!({"terminal_intent":"answer","conversation_relation":"continue_current","shape":"observed_object","content":"","items":[],"fields":[],"exact_item_count":0,"exact_visible_line_count":0,"exact_field_count":1,"observed_fields":[{"name":"value","capability":"fixture.inspect","path":"data.value"}]});
    for step_id in [None, Some(Value::Null), Some(json!("step_1"))] {
        let mut arguments = base.clone();
        if let Some(step_id) = step_id {
            arguments["observed_fields"][0]["step_id"] = step_id;
        }
        let call = ModelToolCall {
            id: "fixture".into(),
            name: NATIVE_RESPOND_TOOL.into(),
            arguments,
        };
        let AgentAction::Respond { content } =
            action_from_native_respond_call(&call, Some(&state)).unwrap()
        else {
            panic!("respond")
        };
        assert_eq!(
            serde_json::from_str::<Value>(&content).unwrap(),
            json!({"value":7})
        );
    }
    for step_id in ["/step_1", "step_1/path", "another_task:step_1", "step_2"] {
        let mut arguments = base.clone();
        arguments["observed_fields"][0]["step_id"] = json!(step_id);
        let call = ModelToolCall {
            id: "fixture".into(),
            name: NATIVE_RESPOND_TOOL.into(),
            arguments,
        };
        assert!(action_from_native_respond_call(&call, Some(&state)).is_err());
    }
    let mut arguments = base;
    arguments["exact_field_count"] = json!(2);
    let call = ModelToolCall {
        id: "fixture".into(),
        name: NATIVE_RESPOND_TOOL.into(),
        arguments,
    };
    assert_eq!(
        action_from_native_respond_call(&call, Some(&state)).unwrap_err(),
        "native_respond_observed_object_count_mismatch"
    );
}

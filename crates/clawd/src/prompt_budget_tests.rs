use super::*;

#[test]
fn prompt_section_report_records_tokens_cacheability_provenance_and_omission() {
    let report = prompt_section_budget_report(
        "planner",
        &[
            PromptSection {
                name: "stable_protocol",
                text: "protocol",
                cacheability: "stable_prefix",
                provenance: "prompt_registry",
                omission_reason: None,
            },
            PromptSection {
                name: "skill_playbook",
                text: "",
                cacheability: "task_scoped",
                provenance: "skill_registry",
                omission_reason: Some("not_selected"),
            },
        ],
    );

    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["section_count"], 2);
    assert_eq!(report["included_section_count"], 1);
    assert_eq!(report["sections"][0]["cacheability"], "stable_prefix");
    assert_eq!(report["sections"][0]["provenance"], "prompt_registry");
    assert_eq!(report["sections"][1]["omission_reason"], "not_selected");
    assert!(report["token_safety_estimate"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn model_tool_surface_report_counts_tools_capabilities_and_schema_cost() {
    let tools = vec![
        ModelToolDefinition {
            name: "call_filesystem".to_string(),
            description: "filesystem capability group".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "capability": {
                        "type": "string",
                        "enum": ["filesystem.read_text", "filesystem.list_entries"]
                    },
                    "args": {"type": "object"}
                }
            }),
            strict: true,
        },
        ModelToolDefinition {
            name: "call_weather".to_string(),
            description: "weather capability group".to_string(),
            input_schema: json!({
                "type": "object",
                "oneOf": [
                    {
                        "type": "object",
                        "properties": {
                            "capability": {
                                "type": "string",
                                "enum": ["weather.current"]
                            },
                            "args": {
                                "type": "object",
                                "properties": {"city": {"type": "string"}}
                            }
                        }
                    }
                ]
            }),
            strict: true,
        },
        ModelToolDefinition {
            name: "respond".to_string(),
            description: "final response".to_string(),
            input_schema: json!({"type": "object"}),
            strict: true,
        },
    ];

    let report = model_tool_surface_budget_report(
        "agent_loop_planner",
        &tools,
        250,
        41,
        1,
        "scoped_playbooks",
    );

    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["tool_count"], 3);
    assert_eq!(report["callable_capability_count"], 250);
    assert_eq!(report["eager_group_count"], 41);
    assert_eq!(report["selected_group_count"], 1);
    assert_eq!(report["tools"][0]["capability_enum_count"], 2);
    assert_eq!(report["tools"][1]["capability_enum_count"], 1);
    assert_eq!(report["tools"][2]["capability_enum_count"], 0);
    assert!(report["serialized_byte_count"].as_u64().unwrap_or(0) > 0);
    assert!(report["serialized_token_estimate"].as_u64().unwrap_or(0) > 0);
}

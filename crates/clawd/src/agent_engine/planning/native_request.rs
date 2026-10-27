use super::*;

pub(super) fn native_planner_request(
    system_prompt: &str,
    user_prompt: &str,
    provider_timeout_seconds: Option<u64>,
    callable_capability_names: &[String],
    ungrouped_capability_argument_schemas: &BTreeMap<String, Value>,
    all_native_capability_groups: &[crate::capability_map::PlannerNativeCapabilityGroup],
    native_capability_groups: &[crate::capability_map::PlannerNativeCapabilityGroup],
    loadable_capability_group_names: &[String],
) -> ModelTurnRequest {
    let mut metadata = std::collections::BTreeMap::new();
    if let Some(timeout_seconds) = provider_timeout_seconds {
        metadata.insert(
            "provider_timeout_seconds".to_string(),
            Value::Number(timeout_seconds.into()),
        );
    }
    let grouped_capability_names = all_native_capability_groups
        .iter()
        .flat_map(|group| group.capability_names.iter().cloned())
        .collect::<BTreeSet<_>>();
    let ungrouped_capability_names = callable_capability_names
        .iter()
        .filter(|name| !grouped_capability_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    let mut tools = Vec::new();
    if !ungrouped_capability_names.is_empty() {
        tools.push(native_capability_tool_definition(
            NATIVE_CALL_CAPABILITY_TOOL,
            "schema:runtime_ungrouped_capability_catalog_v1",
            &ungrouped_capability_names,
            ungrouped_capability_argument_schemas,
        ));
    }
    if !loadable_capability_group_names.is_empty() {
        tools.push(native_capability_loader_tool_definition(
            loadable_capability_group_names,
        ));
    }
    for group in native_capability_groups {
        tools.extend(native_group_tool_definitions(group));
    }
    tools.push(ModelToolDefinition {
        name: NATIVE_RESPOND_TOOL.to_string(),
        description: "Submit either the final user-visible answer or one clarification for a missing required input. This tool formats responses; it does not execute or simulate runtime capabilities. Never use it alone to claim that runtime state was remembered, saved, bound, updated, paused, stopped, or otherwise mutated: first call the matching mutation capability and observe success, even when the user constrains the final reply to one literal value. Set terminal_intent=clarify and identify the schema-level missing_slot when a required capability argument is absent; do not invoke an invalid capability merely to prove that its declared input is missing. Runtime-owned provider/config/permission, domain parse/normalize/validate/preview, dry-run, artifact/job, checkpoint, diff, verification, repair, and rewind fields require a prior matching capability result. A lower-level environment observation is supporting context, not a substitute for the disclosed domain capability that owns those fields. Use free_text for prose/scalars, list for exact items, object for model-authored exact named fields, or observed_object to copy selected JSON fields from current-loop success data or structured failure fields without re-serializing their values. Each object value_json contains one complete serialized JSON value and is validated before delivery; JSON string values include their surrounding JSON quotes.".to_string(),
        input_schema: json!({
            "type": "object",
            "required": [
                "terminal_intent",
                "conversation_relation",
                "shape",
                "content",
                "items",
                "exact_item_count",
                "exact_visible_line_count",
                "fields",
                "observed_fields",
                "exact_field_count"
            ],
            "properties": {
                "terminal_intent": {
                    "type": "string",
                    "enum": ["answer", "clarify"],
                    "description": "answer=final_deliverable; clarify=one_question_for_missing_required_input"
                },
                "conversation_relation": {
                    "type": "string",
                    "enum": [
                        "continue_current",
                        "amend_current",
                        "start_followup",
                        "side_reply",
                        "clarify"
                    ],
                    "description": "semantic relation to the active task: continue_current=finish an accepted deliverable unchanged, including a terminal answer that contains both a requested side answer and the completed primary deliverable; amend_current=finish an accepted deliverable with mid-turn changes; start_followup=the initial primary deliverable when no plan/reply/effect was accepted yet, or an independent new primary goal; side_reply=a nonterminal side answer that leaves the primary task active; clarify=missing required input or an unresolved target_ref when multiple active targets remain equally plausible. Never terminate after only a side answer when the same input asks to complete the primary deliverable. Never guess, merge, or update all ambiguous targets. A conversation_input_batch is bound to the same task: merge it into an interrupted first attempt, but after any accepted plan/reply/effect classify its semantic change as amend_current or continue_current. Decide from meaning, never fixed phrases"
                },
                "clarify_reason_code": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 128,
                    "description": "stable_machine_reason_code; only_with_terminal_intent=clarify"
                },
                "missing_slot": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 128,
                    "description": "required_schema_field_or_stable_boundary_slot; required_with_terminal_intent=clarify"
                },
                "message_key": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 192,
                    "description": "optional_stable_message_key; only_with_terminal_intent=clarify"
                },
                "field_path": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 256,
                    "description": "optional_machine_field_path; only_with_terminal_intent=clarify"
                },
                "locator_kind": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 64,
                    "description": "optional_machine_locator_kind; only_with_terminal_intent=clarify"
                },
                "shape": {
                    "type": "string",
                    "enum": ["free_text", "list", "object", "observed_object"]
                },
                "content": {
                    "type": "string",
                    "description": "free_text=answer; canonical_list_or_object_or_observed_object=empty_string; object_values=fields.value_json; observed_values=observed_fields; object_prose=invalid"
                },
                "items": {
                    "type": "array",
                    "items": {"type": "string"},
                    "maxItems": MAX_NATIVE_RESPONSE_ITEMS
                },
                "exact_item_count": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_NATIVE_RESPONSE_ITEMS
                },
                "exact_visible_line_count": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_NATIVE_RESPONSE_ITEMS,
                    "description": "Exact whole-answer newline-delimited visible line count selected from the current user constraints. Use 0 when no exact whole-answer line, bullet, or numbered-entry count was requested. When nonzero, emit no blank lines, preface, heading, recap, or extra marker line. Include a required trailing marker, signature, checksum, or suffix in the final requested line; do not increase the count unless the user explicitly requests an additional line. For shape=list this must equal exact_item_count."
                },
                "fields": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["name", "value_json"],
                        "properties": {
                            "name": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 128
                            },
                            "value_json": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 65536,
                                "description": "schema:complete_serialized_json_value_v1; json_string_requires_surrounding_quotes=true; malformed_json=rejected; example_json_string=\"\\\"text\\\"\"; scalar_and_composite_encoding=standard_json"
                            }
                        },
                        "additionalProperties": false
                    },
                    "maxItems": MAX_NATIVE_RESPONSE_FIELDS
                },
                "observed_fields": {
                    "type": "array",
                    "description": "schema:current_loop_capability_result_field_references_v3; value_source=runtime_copy; success_roots=data; failure_roots=status,error; model_value_copy=forbidden; optional_step_id=exact_current_loop_provenance_step_id; omitted_or_null_step_id=latest_eligible_capability_result",
                    "items": {
                        "type": "object",
                        "required": ["name", "capability", "path"],
                        "properties": {
                            "name": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 128
                            },
                            "capability": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": MAX_NATIVE_RESPONSE_SOURCE_PATH,
                                "pattern": claw_core::capability_result::MACHINE_REFERENCE_PATTERN,
                                "description": "source=current_loop_outcome; selector=exact_capability_token"
                            },
                            "step_id": {
                                "type": ["string", "null"],
                                "minLength": 1,
                                "maxLength": MAX_NATIVE_RESPONSE_SOURCE_PATH,
                                "pattern": claw_core::capability_result::MACHINE_REFERENCE_PATTERN,
                                "description": "selector=exact_provenance.step_id_from_current_loop; token=opaque_machine_id_not_json_pointer_or_path; copy_without_prefix_or_reconstruction; unknown_or_ambiguous=rejected; omitted_or_null=latest_eligible_result"
                            },
                            "path": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": MAX_NATIVE_RESPONSE_SOURCE_PATH,
                                "description": "selector=machine_dotted_json_path; success_roots=data,data.extra,data.output; failure_roots=status,error.code,error.message_key,error.retryable,error.details; array_index=decimal_path_segment; slash=literal_object_key_character; object_key_case=preserved; examples=data.extra.items.0.name,data.extra.fields./request,error.details.structured_error.extra.error_code; bracket_notation=unsupported"
                            }
                        },
                        "additionalProperties": false
                    },
                    "maxItems": MAX_NATIVE_RESPONSE_FIELDS
                },
                "exact_field_count": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_NATIVE_RESPONSE_FIELDS,
                    "description": "object=fields.length; observed_object=observed_fields.length; observed_zero=derive_from_observed_fields; free_text_or_list=0"
                }
            },
            "additionalProperties": false
        }),
        strict: true,
    });
    ModelTurnRequest {
        messages: vec![
            ModelMessage::text(ModelRole::System, system_prompt),
            ModelMessage::text(ModelRole::User, user_prompt),
        ],
        tools,
        tool_choice: ModelToolChoice::Required,
        response_schema: None,
        stream: true,
        metadata,
    }
}

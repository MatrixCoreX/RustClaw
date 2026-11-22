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
        description: "Final answer or missing-input clarification; does not execute or simulate capabilities. For state claims, first call the matching mutation capability and observe success, even when the user constrains the final reply to one literal value. Provider/config/permission, domain parse/normalize/validate/preview, artifact/job, checkpoint, verification/repair/rewind fields require a prior matching capability result. A raw observation is not a substitute for the disclosed domain capability. Follow the protocol and typed shape/count/source fields.".to_string(),
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
                    "description": "continue_current=finish accepted goal unchanged; amend_current=apply bound changes; start_followup=initial/independent goal; side_reply=nonterminal side answer; clarify=required input/target_ref missing. conversation_input_batch: merge an interrupted first attempt; after any accepted plan/reply/effect use continue/amend. Finish requests include side and main deliverables; clarify ambiguous targets."
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
                    "enum": ["free_text", "list", "object", "observed_object"],
                    "description": "free_text preserves content including plain newline-separated identifiers. list adds ordinal numbers to every item; use it only when numbered presentation is allowed. For bare payload lines use free_text, content with the exact lines, items=[], and exact_item_count=0."
                },
                "content": {
                    "type": "string",
                    "description": "free_text=answer; canonical_list_or_object_or_observed_object=empty_string; object_values=fields.value_json; observed_values=observed_fields; object_prose=invalid"
                },
                "items": {
                    "type": "array",
                    "description": "Always a JSON array. Use [] when shape is not list; never use {}, null, or an empty string.",
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
                    "description": "For object and observed_object always use 0: these shapes deliver named JSON fields, not a scalar line. For free_text and list use the requested exact whole-answer newline-delimited line count, or 0 if unspecified. When nonzero emit no extra lines; put a required suffix in the final requested line; do not add a line unless the user explicitly requests an additional line. For list a nonzero value must equal exact_item_count."
                },
                "fields": {
                    "type": "array",
                    "description": "Always a JSON array. Use [] unless shape=object; never use {} or null.",
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
                    "description": "Always a JSON array; use [] unless shape=observed_object, never {} or null. observed_object delivers named JSON fields only; a lone scalar or date uses free_text.content. schema:current_loop_capability_result_field_references_v3; value_source=runtime_copy; success_roots=data; failure_roots=status,error; optional_step_id=exact_current_loop_provenance_step_id; omitted_or_null_step_id=latest_eligible_capability_result",
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

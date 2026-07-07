use serde_json::Value;

use super::{
    execution_recipe_value_declares_command_payload,
    execution_recipe_value_declares_health_check_observation,
    execution_recipe_value_declares_package_detect_manager_capability,
    execution_recipe_value_declares_scalar_runtime_tool_observation,
    execution_recipe_value_declares_service_status_observation,
    execution_recipe_value_declares_structured_read_observation,
    normalize_output_delivery_intent_for_schema, normalize_output_locator_kind_for_schema,
    normalize_output_response_shape_for_schema, normalize_output_semantic_kind_for_schema,
    normalize_schema_token, normalizer_object_declares_tool_action_payload,
    output_recipe_value_declares_execution, parse_output_semantic_kind,
    schema_text_declares_execution_recipe, ContractRepairReport, OutputDeliveryIntent,
    OutputLocatorKind, OutputResponseShape, OutputSemanticKind,
};

pub(super) fn parse_top_level_json_object_preserving_meaningful_duplicates(
    raw: &str,
) -> Option<Value> {
    struct MeaningfulDuplicateVisitor;

    impl<'de> serde::de::Visitor<'de> for MeaningfulDuplicateVisitor {
        type Value = Value;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a JSON object")
        }

        fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut map = serde_json::Map::new();
            while let Some(key) = access.next_key::<String>()? {
                let value = access.next_value::<Value>()?;
                match map.get(&key) {
                    Some(existing)
                        if route_duplicate_value_score(existing)
                            > route_duplicate_value_score(&value) => {}
                    _ => {
                        map.insert(key, value);
                    }
                }
            }
            Ok(Value::Object(map))
        }
    }

    let mut deserializer = serde_json::Deserializer::from_str(raw.trim());
    serde::de::Deserializer::deserialize_map(&mut deserializer, MeaningfulDuplicateVisitor).ok()
}

fn route_duplicate_value_score(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::String(raw) => {
            if raw.trim().is_empty() {
                0
            } else {
                3
            }
        }
        Value::Bool(false) => 1,
        Value::Bool(true) => 2,
        Value::Number(number) => {
            if number.as_i64() == Some(0) || number.as_u64() == Some(0) {
                1
            } else {
                2
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                0
            } else {
                3
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                0
            } else {
                4
            }
        }
    }
}

pub(super) fn contract_repair_report_from_before_after(
    before: &Value,
    after: &Value,
) -> ContractRepairReport {
    let mut report = ContractRepairReport::default();
    let before_obj = before.as_object();
    let after_obj = after.as_object();

    if before_obj.is_some_and(normalizer_object_declares_tool_action_payload) {
        report.add(
            "tool_payload",
            "normalizer_tool_payload_declared_action_boundary",
        );
    }

    let before_recipe = before_obj.and_then(|obj| obj.get("execution_recipe"));
    if execution_recipe_value_declares_command_payload(before_recipe) {
        report.add("command_payload", "execution_recipe_command_payload");
    } else if execution_recipe_value_declares_package_detect_manager_capability(before_recipe) {
        report.add(
            "structured_recipe",
            "execution_recipe_package_detect_manager_capability",
        );
    } else if execution_recipe_value_declares_scalar_runtime_tool_observation(
        before_recipe,
        before_obj.and_then(|obj| obj.get("output_contract")),
    ) {
        report.add(
            "structured_recipe",
            "execution_recipe_scalar_runtime_tool_observation",
        );
    } else if execution_recipe_value_declares_structured_read_observation(before_recipe) {
        report.add(
            "structured_recipe",
            "execution_recipe_structured_read_observation",
        );
    } else if execution_recipe_value_declares_service_status_observation(before_recipe) {
        report.add(
            "structured_recipe",
            "execution_recipe_service_status_observation",
        );
        if execution_recipe_value_declares_health_check_observation(before_recipe) {
            report.add(
                "structured_recipe",
                "execution_recipe_health_check_observation",
            );
        }
    } else if output_recipe_value_declares_execution(before_recipe) {
        report.add("enum_alias", "execution_recipe_enum");
    } else if execution_recipe_value_has_untrusted_text(before_recipe) {
        report.add(
            "conservative_none",
            "execution_recipe_untrusted_text_ignored",
        );
    }

    if schema_field_alias_or_normalization_changed(
        before_obj,
        after_obj,
        &["turn_type"],
        "turn_type",
        normalize_turn_type_schema_token_for_report,
    ) {
        report.add("enum_alias", "turn_type_enum_normalized");
    }
    if schema_field_alias_or_normalization_changed(
        before_obj,
        after_obj,
        &["target_task_policy"],
        "target_task_policy",
        normalize_target_task_policy_schema_token_for_report,
    ) {
        report.add("enum_alias", "target_task_policy_enum_normalized");
    }

    let before_contract = before_obj
        .and_then(|obj| obj.get("output_contract"))
        .and_then(Value::as_object);
    let after_contract = after_obj
        .and_then(|obj| obj.get("output_contract"))
        .and_then(Value::as_object);
    if output_contract_schema_field_changed(
        before_contract,
        after_contract,
        &[
            "response_shape",
            "shape",
            "answer_shape",
            "format",
            "response_format",
        ],
        "response_shape",
        normalize_output_response_shape_for_schema,
        "free",
    ) {
        report.add("enum_alias", "output_contract_response_shape_normalized");
    }
    if output_contract_schema_field_changed(
        before_contract,
        after_contract,
        &["locator_kind"],
        "locator_kind",
        normalize_output_locator_kind_for_schema,
        "none",
    ) {
        report.add("enum_alias", "output_contract_locator_kind_normalized");
    }
    if output_contract_schema_field_changed(
        before_contract,
        after_contract,
        &["delivery_intent"],
        "delivery_intent",
        normalize_output_delivery_intent_for_schema,
        "none",
    ) {
        report.add("enum_alias", "output_contract_delivery_intent_normalized");
    }
    if output_contract_schema_field_changed(
        before_contract,
        after_contract,
        &["contract_marker"],
        "contract_marker",
        normalize_output_semantic_kind_for_schema,
        "none",
    ) {
        report.add("enum_alias", "output_contract_marker_normalized");
    }
    if output_contract_unknown_semantic_was_ignored(before_contract, after_contract) {
        report.add(
            "conservative_none",
            "output_contract_unknown_semantic_ignored",
        );
    }
    if output_contract_unknown_scalar_was_ignored(before_obj, after_contract) {
        report.add(
            "boundary_contract",
            "executable_route_unknown_scalar_output_contract",
        );
    }
    if output_contract_requires_evidence_was_repaired(before_contract, after_contract) {
        report.add(
            "structured_contract",
            "output_contract_requires_evidence_repaired",
        );
    }
    if !output_contract_has_executable_shape(before_contract)
        && output_contract_has_executable_shape(after_contract)
    {
        report.add(
            "structured_contract",
            "execution_signal_derived_from_output_contract",
        );
    }
    if execution_recipe_schema_field_changed(
        before_recipe,
        after_obj.and_then(|obj| obj.get("execution_recipe")),
        "kind",
        |raw| Some(crate::execution_recipe::parse_execution_recipe_kind_text(raw).as_str()),
        "none",
    ) || execution_recipe_schema_field_changed(
        before_recipe,
        after_obj.and_then(|obj| obj.get("execution_recipe")),
        "profile",
        |raw| Some(crate::execution_recipe::parse_execution_recipe_profile_text(raw).as_str()),
        "none",
    ) || execution_recipe_schema_field_changed(
        before_recipe,
        after_obj.and_then(|obj| obj.get("execution_recipe")),
        "target_scope",
        |raw| Some(crate::execution_recipe::parse_execution_recipe_target_scope_text(raw).as_str()),
        "unknown",
    ) {
        report.add("enum_alias", "execution_recipe_fields_normalized");
    }

    report
}

fn normalize_turn_type_schema_token_for_report(raw: &str) -> Option<&'static str> {
    match normalize_schema_token(raw).as_str() {
        "task_request" => Some("task_request"),
        "task_append" => Some("task_append"),
        "task_replace" => Some("task_replace"),
        "task_correct" => Some("task_correct"),
        "task_scope_update" => Some("task_scope_update"),
        "run_control" => Some("run_control"),
        "approval_decision" => Some("approval_decision"),
        "status_query" => Some("status_query"),
        "feedback_or_error" => Some("feedback_or_error"),
        "preference_or_memory" => Some("preference_or_memory"),
        _ => None,
    }
}

fn normalize_target_task_policy_schema_token_for_report(raw: &str) -> Option<&'static str> {
    match normalize_schema_token(raw).as_str() {
        "reuse_active" => Some("reuse_active"),
        "replace_active" => Some("replace_active"),
        "pause_and_queue" => Some("pause_and_queue"),
        "standalone" => Some("standalone"),
        _ => None,
    }
}

fn schema_field_alias_or_normalization_changed(
    before_obj: Option<&serde_json::Map<String, Value>>,
    after_obj: Option<&serde_json::Map<String, Value>>,
    before_keys: &[&str],
    after_key: &str,
    normalize: fn(&str) -> Option<&'static str>,
) -> bool {
    let Some(after_text) = after_obj
        .and_then(|obj| obj.get(after_key))
        .and_then(scalar_json_value_text)
    else {
        return false;
    };
    let Some(after_normalized) = normalize(&after_text) else {
        return false;
    };
    before_keys.iter().any(|key| {
        let Some(before_text) = before_obj
            .and_then(|obj| obj.get(*key))
            .and_then(scalar_json_value_text)
        else {
            return false;
        };
        normalize(&before_text).is_some_and(|candidate| candidate == after_normalized)
            && normalize_schema_token(&before_text) != after_normalized
    })
}

fn output_contract_schema_field_changed(
    before_contract: Option<&serde_json::Map<String, Value>>,
    after_contract: Option<&serde_json::Map<String, Value>>,
    before_keys: &[&str],
    after_key: &str,
    normalize: fn(&str) -> &'static str,
    default: &str,
) -> bool {
    let Some(after_text) = after_contract
        .and_then(|obj| obj.get(after_key))
        .and_then(scalar_json_value_text)
    else {
        return false;
    };
    let after_normalized = normalize(&after_text);
    if after_normalized == default {
        return false;
    }
    before_keys.iter().any(|key| {
        let Some(before_text) = before_contract
            .and_then(|obj| obj.get(*key))
            .and_then(scalar_json_value_text)
        else {
            return false;
        };
        normalize(&before_text) == after_normalized
            && normalize_schema_token(&before_text) != after_normalized
    })
}

fn output_contract_unknown_semantic_was_ignored(
    before_contract: Option<&serde_json::Map<String, Value>>,
    after_contract: Option<&serde_json::Map<String, Value>>,
) -> bool {
    let before_text = before_contract
        .and_then(|obj| obj.get("contract_marker").and_then(scalar_json_value_text))
        .unwrap_or_default();
    if before_text.trim().is_empty()
        || schema_text_is_neutral_none(&before_text)
        || normalize_output_semantic_kind_for_schema(&before_text)
            != OutputSemanticKind::None.as_str()
    {
        return false;
    }
    after_contract
        .and_then(|obj| obj.get("contract_marker"))
        .and_then(scalar_json_value_text)
        .is_some_and(|text| text == OutputSemanticKind::None.as_str())
}

fn output_contract_unknown_scalar_was_ignored(
    before_obj: Option<&serde_json::Map<String, Value>>,
    after_contract: Option<&serde_json::Map<String, Value>>,
) -> bool {
    let Some(before_obj) = before_obj else {
        return false;
    };
    if !normalizer_object_declares_executable_route(before_obj) {
        return false;
    }
    let Some(raw) = before_obj
        .get("output_contract")
        .and_then(scalar_json_value_text)
    else {
        return false;
    };
    if raw.trim().is_empty()
        || schema_text_is_neutral_none(&raw)
        || output_contract_scalar_looks_like_schema_token(&raw)
    {
        return false;
    }
    let Some(after_contract) = after_contract else {
        return false;
    };
    let after_semantic_is_none = after_contract
        .get("contract_marker")
        .and_then(scalar_json_value_text)
        .is_none_or(|text| text == OutputSemanticKind::None.as_str());
    let after_shape_is_free = after_contract
        .get("response_shape")
        .and_then(scalar_json_value_text)
        .is_none_or(|text| text == OutputResponseShape::Free.as_str());
    let after_locator_is_none = after_contract
        .get("locator_kind")
        .and_then(scalar_json_value_text)
        .is_none_or(|text| text == OutputLocatorKind::None.as_str());
    let after_delivery_is_none = after_contract
        .get("delivery_intent")
        .and_then(scalar_json_value_text)
        .is_none_or(|text| text == OutputDeliveryIntent::None.as_str());
    let after_requires_evidence = after_contract
        .get("requires_content_evidence")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let after_delivery_required = after_contract
        .get("delivery_required")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    after_semantic_is_none
        && after_shape_is_free
        && after_locator_is_none
        && after_delivery_is_none
        && !after_requires_evidence
        && !after_delivery_required
}

fn output_contract_requires_evidence_was_repaired(
    before_contract: Option<&serde_json::Map<String, Value>>,
    after_contract: Option<&serde_json::Map<String, Value>>,
) -> bool {
    let before_requires = before_contract
        .and_then(|obj| obj.get("requires_content_evidence"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let after_requires = after_contract
        .and_then(|obj| obj.get("requires_content_evidence"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    !before_requires && after_requires
}

fn output_contract_has_executable_shape(contract: Option<&serde_json::Map<String, Value>>) -> bool {
    let Some(contract) = contract else {
        return false;
    };
    contract
        .get("requires_content_evidence")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || contract
            .get("delivery_required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || contract
            .get("locator_kind")
            .and_then(scalar_json_value_text)
            .is_some_and(|value| normalize_output_locator_kind_for_schema(&value) != "none")
        || contract
            .get("contract_marker")
            .and_then(scalar_json_value_text)
            .is_some_and(|value| {
                !matches!(
                    parse_output_semantic_kind(&value),
                    OutputSemanticKind::None | OutputSemanticKind::FileBasename
                )
            })
}

fn execution_recipe_schema_field_changed(
    before_recipe: Option<&Value>,
    after_recipe: Option<&Value>,
    key: &str,
    normalize: fn(&str) -> Option<&'static str>,
    default: &str,
) -> bool {
    let Some(after_text) = after_recipe
        .and_then(Value::as_object)
        .and_then(|obj| obj.get(key))
        .and_then(scalar_json_value_text)
    else {
        return false;
    };
    let Some(after_normalized) = normalize(&after_text) else {
        return false;
    };
    if after_normalized == default {
        return false;
    }
    let before_text = before_recipe
        .and_then(Value::as_object)
        .and_then(|obj| obj.get(key))
        .and_then(scalar_json_value_text)
        .or_else(|| before_recipe.and_then(scalar_json_value_text));
    before_text.is_some_and(|text| {
        normalize(&text).is_some_and(|candidate| candidate == after_normalized)
            && normalize_schema_token(&text) != after_normalized
    })
}

fn execution_recipe_value_has_untrusted_text(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(raw)) => {
            !raw.trim().is_empty()
                && !schema_text_is_neutral_none(raw)
                && !schema_text_declares_execution_recipe(raw)
        }
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_has_untrusted_text(Some(value))),
        Some(Value::Object(map)) => map.iter().any(|(key, value)| {
            if matches!(
                key.as_str(),
                "kind"
                    | "profile"
                    | "target_scope"
                    | "turn_type"
                    | "target_task_policy"
                    | "should_interrupt_active_run"
                    | "state_patch"
                    | "attachment_processing_required"
            ) {
                return false;
            }
            execution_recipe_value_has_untrusted_text(Some(value))
        }),
        Some(other) => scalar_json_value_text(other).is_some_and(|text| {
            !text.trim().is_empty()
                && !schema_text_is_neutral_none(&text)
                && !schema_text_declares_execution_recipe(&text)
        }),
        None => false,
    }
}

fn normalizer_object_declares_executable_route(obj: &serde_json::Map<String, Value>) -> bool {
    obj.get("execution_recipe")
        .is_some_and(|value| output_recipe_value_declares_execution(Some(value)))
}

fn schema_text_is_neutral_none(raw: &str) -> bool {
    matches!(
        normalize_schema_token(raw).as_str(),
        "" | "none" | "null" | "no" | "false"
    )
}

pub(super) fn answer_like_normalizer_payload_text(
    obj: &serde_json::Map<String, Value>,
) -> Option<String> {
    for key in [
        "response_text",
        "response",
        "reply",
        "answer",
        "content",
        "summary",
    ] {
        if let Some(text) = obj.get(key).and_then(scalar_json_value_text) {
            return Some(text);
        }
    }
    if let Some(contract) = obj
        .get("output_contract")
        .and_then(|value| value.as_object())
    {
        for key in [
            "content",
            "scalar_content",
            "scalar_output",
            "answer",
            "response_text",
        ] {
            if let Some(text) = contract.get(key).and_then(scalar_json_value_text) {
                return Some(text);
            }
        }
    }

    const ROUTE_KEYS: &[&str] = &[
        "resolved_user_intent",
        "answer_candidate",
        "resume_behavior",
        "schedule_kind",
        "schedule_intent",
        "wants_file_delivery",
        "should_refresh_long_term_memory",
        "agent_display_name_hint",
        "needs_clarify",
        "clarify_question",
        "reason",
        "confidence",
        "decision",
        "output_contract",
        "execution_recipe",
        "turn_type",
        "target_task_policy",
        "should_interrupt_active_run",
        "state_patch",
        "attachment_processing_required",
    ];
    if obj.keys().any(|key| ROUTE_KEYS.contains(&key.as_str())) {
        return None;
    }
    let mut values = obj
        .values()
        .filter(|value| !value.is_null())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    if values.len() == 1 {
        return scalar_json_value_text(values.pop()?).or_else(|| {
            serde_json::to_string(&Value::Object(obj.clone()))
                .ok()
                .filter(|text| !text.trim().is_empty())
        });
    }
    serde_json::to_string(&Value::Object(obj.clone()))
        .ok()
        .filter(|text| !text.trim().is_empty())
}

pub(super) fn scalar_json_value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.trim().to_string()).filter(|text| !text.is_empty()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn output_contract_scalar_looks_like_schema_token(raw: &str) -> bool {
    let token = normalize_schema_token(raw);
    if token.is_empty() {
        return true;
    }
    matches!(
        token.as_str(),
        "free"
            | "text"
            | "plain_text"
            | "string"
            | "message"
            | "answer"
            | "response"
            | "clarification"
            | "json"
            | "json_object"
            | "raw_json"
            | "structured"
            | "structured_data"
            | "text_plain"
            | "text/plain"
            | "text_markdown"
            | "text/markdown"
            | "application_json"
            | "application/json"
            | "application_xml"
            | "application/xml"
            | "text_csv"
            | "text/csv"
            | "text_html"
            | "text/html"
    ) || matches!(
        normalize_output_response_shape_for_schema(&token),
        "one_sentence" | "strict" | "scalar" | "file_token"
    ) || !matches!(normalize_output_locator_kind_for_schema(&token), "none")
        || !matches!(normalize_output_delivery_intent_for_schema(&token), "none")
        || !matches!(normalize_output_semantic_kind_for_schema(&token), "none")
}

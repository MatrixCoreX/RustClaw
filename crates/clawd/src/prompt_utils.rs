use std::sync::OnceLock;

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::AppState;

pub(crate) fn render_prompt_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut rendered = template.to_string();
    for (key, value) in replacements {
        rendered = rendered.replace(key, value);
    }
    rendered
}

pub(crate) fn log_prompt_render(
    state: &AppState,
    task_id: &str,
    prompt_name: &str,
    prompt_source: &str,
    round: Option<usize>,
) {
    log_prompt_render_with_version(state, task_id, prompt_name, prompt_source, None, round);
}

/// §3.5a: 带 prompt_version 字段的版本，给已迁移到 with_meta API 的关键审计点用。
///
/// `prompt_version` 缺失时记 `prompt_version=none`；存在时记 `prompt_version=...`。
/// 该字段会进 model_io / task_journal payload，为 prompt 改动追溯提供锚点。
pub(crate) fn log_prompt_render_with_version(
    state: &AppState,
    task_id: &str,
    prompt_name: &str,
    prompt_source: &str,
    prompt_version: Option<&str>,
    round: Option<usize>,
) {
    if !state.policy.routing.debug_log_prompt {
        return;
    }
    let version = prompt_version.unwrap_or("none");
    match round {
        Some(round) => {
            tracing::info!(
                "{} prompt_invocation task_id={} prompt_name={} prompt_source={} prompt_version={} prompt_dynamic=true note=dynamic_built_prompt round={}",
                crate::highlight_tag("prompt"),
                task_id,
                prompt_name,
                prompt_source,
                version,
                round
            );
        }
        None => {
            tracing::info!(
                "{} prompt_invocation task_id={} prompt_name={} prompt_source={} prompt_version={} prompt_dynamic=true note=dynamic_built_prompt",
                crate::highlight_tag("prompt"),
                task_id,
                prompt_name,
                prompt_source,
                version
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptSchemaId {
    IntentNormalizer,
    ContractRepairJudge,
    DirectAnswerGate,
    AnswerVerifier,
    UserResponseContractValidator,
    PlanResult,
    FinalizerOut,
    DeliveryTextClassifier,
    ScheduleIntent,
    LongTermSummary,
    #[allow(dead_code)]
    MemoryIntent,
    RunCmdSuggestion,
}

impl PromptSchemaId {
    fn as_str(self) -> &'static str {
        match self {
            Self::IntentNormalizer => "intent_normalizer",
            Self::ContractRepairJudge => "contract_repair_judge",
            Self::DirectAnswerGate => "direct_answer_gate",
            Self::AnswerVerifier => "answer_verifier",
            Self::UserResponseContractValidator => "user_response_contract_validator",
            Self::PlanResult => "plan_result",
            Self::FinalizerOut => "finalizer_out",
            Self::DeliveryTextClassifier => "delivery_text_classifier",
            Self::ScheduleIntent => "schedule_intent",
            Self::LongTermSummary => "long_term_summary",
            Self::MemoryIntent => "memory_intent",
            Self::RunCmdSuggestion => "run_cmd_suggestion",
        }
    }

    fn schema_value(self) -> &'static Value {
        fn parse_schema(raw: &str) -> Value {
            serde_json::from_str(raw).expect("prompt schema must be valid JSON")
        }

        static INTENT_NORMALIZER: OnceLock<Value> = OnceLock::new();
        static CONTRACT_REPAIR_JUDGE: OnceLock<Value> = OnceLock::new();
        static DIRECT_ANSWER_GATE: OnceLock<Value> = OnceLock::new();
        static ANSWER_VERIFIER: OnceLock<Value> = OnceLock::new();
        static USER_RESPONSE_CONTRACT_VALIDATOR: OnceLock<Value> = OnceLock::new();
        static PLAN_RESULT: OnceLock<Value> = OnceLock::new();
        static FINALIZER_OUT: OnceLock<Value> = OnceLock::new();
        static DELIVERY_TEXT_CLASSIFIER: OnceLock<Value> = OnceLock::new();
        static SCHEDULE_INTENT: OnceLock<Value> = OnceLock::new();
        static LONG_TERM_SUMMARY: OnceLock<Value> = OnceLock::new();
        static MEMORY_INTENT: OnceLock<Value> = OnceLock::new();
        static RUN_CMD_SUGGESTION: OnceLock<Value> = OnceLock::new();

        match self {
            Self::IntentNormalizer => INTENT_NORMALIZER.get_or_init(|| {
                parse_schema(include_str!(
                    "../../../prompts/schemas/intent_normalizer.schema.json"
                ))
            }),
            Self::ContractRepairJudge => CONTRACT_REPAIR_JUDGE.get_or_init(|| {
                parse_schema(include_str!(
                    "../../../prompts/schemas/contract_repair_judge.schema.json"
                ))
            }),
            Self::DirectAnswerGate => DIRECT_ANSWER_GATE.get_or_init(|| {
                parse_schema(include_str!(
                    "../../../prompts/schemas/direct_answer_gate.schema.json"
                ))
            }),
            Self::AnswerVerifier => ANSWER_VERIFIER.get_or_init(|| {
                parse_schema(include_str!(
                    "../../../prompts/schemas/answer_verifier.schema.json"
                ))
            }),
            Self::UserResponseContractValidator => {
                USER_RESPONSE_CONTRACT_VALIDATOR.get_or_init(|| {
                    parse_schema(include_str!(
                        "../../../prompts/schemas/user_response_contract_validator.schema.json"
                    ))
                })
            }
            Self::PlanResult => PLAN_RESULT.get_or_init(|| {
                parse_schema(include_str!(
                    "../../../prompts/schemas/plan_result.schema.json"
                ))
            }),
            Self::FinalizerOut => FINALIZER_OUT.get_or_init(|| {
                parse_schema(include_str!(
                    "../../../prompts/schemas/finalizer_out.schema.json"
                ))
            }),
            Self::DeliveryTextClassifier => DELIVERY_TEXT_CLASSIFIER.get_or_init(|| {
                parse_schema(include_str!(
                    "../../../prompts/schemas/delivery_text_classifier.schema.json"
                ))
            }),
            Self::ScheduleIntent => SCHEDULE_INTENT.get_or_init(|| {
                parse_schema(include_str!(
                    "../../../prompts/schemas/schedule_intent.schema.json"
                ))
            }),
            Self::LongTermSummary => LONG_TERM_SUMMARY.get_or_init(|| {
                parse_schema(include_str!(
                    "../../../prompts/schemas/long_term_summary.schema.json"
                ))
            }),
            Self::MemoryIntent => MEMORY_INTENT.get_or_init(|| {
                parse_schema(include_str!(
                    "../../../prompts/schemas/memory_intent.schema.json"
                ))
            }),
            Self::RunCmdSuggestion => RUN_CMD_SUGGESTION.get_or_init(|| {
                parse_schema(include_str!(
                    "../../../prompts/schemas/run_cmd_suggestion.schema.json"
                ))
            }),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ValidatedSchemaJson<T> {
    pub(crate) value: T,
    pub(crate) raw_parse_ok: bool,
    pub(crate) schema_normalized: bool,
}

#[derive(Debug)]
pub(crate) struct SchemaValidationError {
    schema_id: PromptSchemaId,
    stage: &'static str,
    details: Vec<String>,
}

impl std::fmt::Display for SchemaValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.details.is_empty() {
            write!(
                f,
                "schema_validation_failed schema={} stage={}",
                self.schema_id.as_str(),
                self.stage
            )
        } else {
            write!(
                f,
                "schema_validation_failed schema={} stage={} details={}",
                self.schema_id.as_str(),
                self.stage,
                self.details.join(" | ")
            )
        }
    }
}

impl std::error::Error for SchemaValidationError {}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn schema_type_matches(value: &Value, expected: &str) -> bool {
    match expected {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "number" => value.is_number(),
        "integer" => value
            .as_f64()
            .map(|n| n.fract().abs() < f64::EPSILON)
            .unwrap_or(false),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    }
}

fn schema_declared_type_matches(value: &Value, schema: &Value) -> bool {
    match schema.get("type") {
        Some(Value::String(kind)) => schema_type_matches(value, kind),
        Some(Value::Array(kinds)) => kinds
            .iter()
            .filter_map(|kind| kind.as_str())
            .any(|kind| schema_type_matches(value, kind)),
        Some(_) => false,
        None => true,
    }
}

fn schema_expected_types(schema: &Value) -> Option<String> {
    match schema.get("type") {
        Some(Value::String(kind)) => Some(kind.clone()),
        Some(Value::Array(kinds)) => Some(
            kinds
                .iter()
                .filter_map(|kind| kind.as_str())
                .collect::<Vec<_>>()
                .join("|"),
        ),
        _ => None,
    }
}

fn schema_ref_target<'a>(schema_root: &'a Value, raw_ref: &str) -> Option<&'a Value> {
    let pointer = raw_ref.strip_prefix('#')?;
    schema_root.pointer(pointer)
}

fn schema_path_key(path: &str, key: &str) -> String {
    if path == "$" {
        format!("$.{key}")
    } else {
        format!("{path}.{key}")
    }
}

fn schema_path_index(path: &str, index: usize) -> String {
    format!("{path}[{index}]")
}

fn validate_schema_value(
    schema_root: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
    errors: &mut Vec<String>,
) {
    if let Some(raw_ref) = schema.get("$ref").and_then(|v| v.as_str()) {
        match schema_ref_target(schema_root, raw_ref) {
            Some(target) => validate_schema_value(schema_root, target, value, path, errors),
            None => errors.push(format!("{path}: unresolved schema ref `{raw_ref}`")),
        }
        return;
    }

    if let Some(branches) = schema.get("oneOf").and_then(|v| v.as_array()) {
        let mut matched = false;
        for branch in branches {
            let mut branch_errors = Vec::new();
            validate_schema_value(schema_root, branch, value, path, &mut branch_errors);
            if branch_errors.is_empty() {
                matched = true;
                break;
            }
        }
        if !matched {
            errors.push(format!(
                "{path}: does not match any allowed schema variant (got {})",
                value_type_name(value)
            ));
        }
        return;
    }

    if !schema_declared_type_matches(value, schema) {
        if let Some(expected) = schema_expected_types(schema) {
            errors.push(format!(
                "{path}: expected type {expected}, got {}",
                value_type_name(value)
            ));
        }
        return;
    }

    if let Some(enum_values) = schema.get("enum").and_then(|v| v.as_array()) {
        if !enum_values.iter().any(|allowed| allowed == value) {
            let allowed = enum_values
                .iter()
                .map(|candidate| candidate.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            errors.push(format!(
                "{path}: expected one of [{allowed}], got {}",
                value
            ));
            return;
        }
    }

    if let Some(const_value) = schema.get("const") {
        if const_value != value {
            errors.push(format!(
                "{path}: expected const {}, got {}",
                const_value, value
            ));
            return;
        }
    }

    if let Some(minimum) = schema.get("minimum").and_then(|v| v.as_f64()) {
        if value.as_f64().map(|n| n < minimum).unwrap_or(false) {
            errors.push(format!("{path}: expected >= {minimum}, got {value}"));
        }
    }
    if let Some(maximum) = schema.get("maximum").and_then(|v| v.as_f64()) {
        if value.as_f64().map(|n| n > maximum).unwrap_or(false) {
            errors.push(format!("{path}: expected <= {maximum}, got {value}"));
        }
    }

    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        if let Some(obj) = value.as_object() {
            for field in required.iter().filter_map(|v| v.as_str()) {
                if !obj.contains_key(field) {
                    errors.push(format!("{path}: missing required field `{field}`"));
                }
            }
        }
    }

    if let Some(obj) = value.as_object() {
        let properties = schema.get("properties").and_then(|v| v.as_object());
        let additional = schema.get("additionalProperties");
        for (key, field_value) in obj {
            if let Some(field_schema) = properties.and_then(|props| props.get(key)) {
                validate_schema_value(
                    schema_root,
                    field_schema,
                    field_value,
                    &schema_path_key(path, key),
                    errors,
                );
                continue;
            }
            match additional {
                Some(Value::Bool(false)) => {
                    errors.push(format!("{}: unexpected property `{}`", path, key));
                }
                Some(extra_schema @ Value::Object(_)) => validate_schema_value(
                    schema_root,
                    extra_schema,
                    field_value,
                    &schema_path_key(path, key),
                    errors,
                ),
                _ => {}
            }
        }
    }

    if let Some(arr) = value.as_array() {
        if let Some(items_schema) = schema.get("items") {
            for (index, item) in arr.iter().enumerate() {
                validate_schema_value(
                    schema_root,
                    items_schema,
                    item,
                    &schema_path_index(path, index),
                    errors,
                );
            }
        }
    }
}

fn canonicalize_plan_action_step_value(value: Value) -> (Value, bool) {
    let Value::Object(mut map) = value else {
        return (value, false);
    };
    let Some(kind) = map
        .get("type")
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
    else {
        return (Value::Object(map), false);
    };
    let allowed_keys: &[&str] = match kind.as_str() {
        "think" => &["type", "content"],
        "call_skill" => &["type", "skill", "args"],
        "call_tool" => &["type", "tool", "args"],
        "call_capability" => &["type", "capability", "args"],
        "synthesize_answer" => &["type", "evidence_refs"],
        "respond" => &["type", "content"],
        _ => return (Value::Object(map), false),
    };
    let original_len = map.len();
    map.retain(|key, _| allowed_keys.contains(&key.as_str()));
    let mut normalized = map.len() != original_len;
    if map.get("type").and_then(|value| value.as_str()) != Some(kind.as_str()) {
        map.insert("type".to_string(), Value::String(kind));
        normalized = true;
    }
    (Value::Object(map), normalized)
}

fn canonicalize_plan_steps_value(value: Value) -> (Value, bool) {
    match value {
        Value::Array(steps) => {
            let mut normalized = false;
            let steps = steps
                .into_iter()
                .map(|step| {
                    let (step, step_normalized) = canonicalize_plan_action_step_value(step);
                    normalized |= step_normalized;
                    step
                })
                .collect::<Vec<_>>();
            (Value::Array(steps), normalized)
        }
        Value::Object(_) => {
            let (step, _) = canonicalize_plan_action_step_value(value);
            (json!([step]), true)
        }
        other => (other, false),
    }
}

fn canonicalize_plan_result_object(mut map: serde_json::Map<String, Value>) -> (Value, bool) {
    let mut normalized = false;
    let steps = if let Some(steps) = map.remove("steps") {
        steps
    } else {
        let mut alias_steps = None;
        for alias in ["actions", "plan", "tool_calls"] {
            if let Some(steps) = map.remove(alias) {
                alias_steps = Some(steps);
                normalized = true;
                break;
            }
        }
        match alias_steps {
            Some(steps) => steps,
            None => {
                let (step, _) = canonicalize_plan_action_step_value(Value::Object(map));
                return (json!({ "steps": [step] }), true);
            }
        }
    };
    let (steps, steps_normalized) = canonicalize_plan_steps_value(steps);
    normalized |= steps_normalized;

    let planner_notes = map.remove("planner_notes");
    normalized |= !map.is_empty();

    let mut out = serde_json::Map::new();
    out.insert("steps".to_string(), steps);
    if let Some(planner_notes) = planner_notes {
        out.insert("planner_notes".to_string(), planner_notes);
    }
    (Value::Object(out), normalized)
}

fn default_direct_answer_gate_contract() -> Value {
    json!({
        "response_shape": "free",
        "exact_sentence_count": null,
        "requires_content_evidence": false,
        "delivery_required": false,
        "locator_kind": "none",
        "delivery_intent": "none",
        "semantic_kind": "none",
        "locator_hint": "",
        "self_extension": {
            "mode": "none",
            "trigger": "none",
            "execute_now": false
        }
    })
}

fn default_direct_answer_gate_reference_resolution() -> Value {
    json!({ "target": "none" })
}

fn normalize_schema_token_for_gate(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .trim_matches('_')
        .to_string()
}

fn gate_locator_hint_is_path_like(hint: &str) -> bool {
    let hint = hint.trim();
    hint.starts_with('/')
        || hint.starts_with("./")
        || hint.starts_with("../")
        || hint.starts_with("~/")
        || hint.contains('/')
        || hint.contains('\\')
}

fn normalize_direct_answer_gate_locator_kind(raw: &str, locator_hint: &str) -> &'static str {
    match normalize_schema_token_for_gate(raw).as_str() {
        "path" | "file_path" | "directory" | "directory_path" | "dir" => "path",
        "file" | "file_locator" => {
            if gate_locator_hint_is_path_like(locator_hint) {
                "path"
            } else {
                "filename"
            }
        }
        "current_workspace" | "workspace" | "repo" | "repository" => "current_workspace",
        "url" | "uri" | "link" => "url",
        "filename" | "file_name" | "basename" => "filename",
        _ => "none",
    }
}

fn normalize_direct_answer_gate_response_shape(raw: &str) -> &'static str {
    match normalize_schema_token_for_gate(raw).as_str() {
        "one_sentence" | "single_sentence" | "sentence" | "short_sentence" => "one_sentence",
        "strict" | "exact" | "exact_text" | "strict_text" | "exact_format" | "one_line"
        | "single_line" | "line_only" | "list" | "array" | "string_list" => "strict",
        "scalar" | "value" | "value_only" | "single_value" | "field_value" => "scalar",
        "file_token" | "delivery_token" => "file_token",
        _ => "free",
    }
}

fn direct_answer_gate_semantic_token_requests_scalar_shape(raw: &str) -> bool {
    matches!(
        normalize_schema_token_for_gate(raw).as_str(),
        "scalar"
            | "scalar_value"
            | "scalar_only"
            | "value"
            | "value_only"
            | "single_value"
            | "field_value"
            | "file_field_value"
    )
}

fn normalize_direct_answer_gate_delivery_intent(raw: &str) -> &'static str {
    match normalize_schema_token_for_gate(raw).as_str() {
        "file_single" | "single_file" | "file" | "deliver_file" | "file_delivery" => "file_single",
        "directory_lookup" | "dir_lookup" | "directory" | "list_directory" => "directory_lookup",
        "directory_batch_files" | "batch_directory_delivery" | "dir_batch" => {
            "directory_batch_files"
        }
        _ => "none",
    }
}

fn normalize_direct_answer_gate_semantic_kind(raw: &str) -> &'static str {
    match normalize_schema_token_for_gate(raw).as_str() {
        "none" => "none",
        "raw" | "raw_output" | "command_output" | "shell_output" | "terminal_output" => {
            "raw_command_output"
        }
        "raw_command_output" => "raw_command_output",
        "service_status"
        | "service_state"
        | "service_running_status"
        | "process_status"
        | "process_state"
        | "process_running_status"
        | "daemon_status"
        | "daemon_state" => "service_status",
        "hidden_files"
        | "hidden_entries"
        | "hidden_file_check"
        | "hidden_files_check"
        | "hidden_entry_check"
        | "hidden_entries_check" => "hidden_entries_check",
        "file_names"
        | "file_names_only"
        | "file_name_only"
        | "files_listing"
        | "files_list"
        | "names_only"
        | "entry_names"
        | "directory_entry_names"
        | "file_listing"
        | "file_list"
        | "filename_listing"
        | "filename_list"
        | "filename_only"
        | "filenames_list"
        | "filenames_only"
        | "list_filenames"
        | "list_file_names" => "file_names",
        "directory_names"
        | "directory_names_only"
        | "directory_name_only"
        | "dir_names"
        | "dir_names_only"
        | "folder_names"
        | "folder_names_only"
        | "folders_only" => "directory_names",
        "directory_entry_groups"
        | "directory_file_groups"
        | "file_directory_groups"
        | "entry_kind_groups"
        | "entries_by_kind"
        | "grouped_entries"
        | "grouped_entry_names" => "directory_entry_groups",
        "file_paths" | "file_paths_only" | "path_list" | "paths_list" | "file_path_list" => {
            "file_paths"
        }
        "directory_purpose_summary" | "listing_purpose_summary" | "directory_listing_summary" => {
            "directory_purpose_summary"
        }
        "content_excerpt" | "content_excerpt_summary" | "file_excerpt" | "tail_lines" => {
            "content_excerpt_summary"
        }
        "content_excerpt_with_summary"
        | "excerpt_with_summary"
        | "raw_excerpt_with_summary"
        | "bounded_excerpt_with_summary" => "content_excerpt_with_summary",
        "content_presence_check"
        | "content_contains_check"
        | "content_match_check"
        | "identifier_presence_check"
        | "field_presence_check"
        | "text_presence_check" => "content_presence_check",
        "excerpt_kind_judgment" | "content_excerpt_judgment" | "log_vs_checklist" => {
            "excerpt_kind_judgment"
        }
        "recent_artifacts_judgment" | "artifact_style_classification" => {
            "recent_artifacts_judgment"
        }
        "workspace_summary" | "workspace_project_summary" => "workspace_project_summary",
        "scalar_count" | "count" => "scalar_count",
        "quantity_comparison" | "comparison" => "quantity_comparison",
        "failed_step" | "failed_command_step" | "execution_failure_step" => "execution_failed_step",
        "new_file_delivery" | "created_file_delivery" | "write_then_send_file" => {
            "generated_file_delivery"
        }
        "scalar_path_only" => "scalar_path_only",
        "existence_with_path" | "exists_with_path" => "existence_with_path",
        "existence_with_path_summary" => "existence_with_path_summary",
        "recent_scalar_equality_check" | "one_line_comparison" | "single_line_comparison" => {
            "recent_scalar_equality_check"
        }
        "git_commit_subject" | "git_commit_title" | "commit_subject" | "commit_title" => {
            "git_commit_subject"
        }
        "structured_keys" => "structured_keys",
        "config_validation" | "structured_config_validation" => "config_validation",
        "config_mutation" | "config_write" | "config_set" | "structured_config_mutation" => {
            "config_mutation"
        }
        "config_risk_assessment" | "config_risk" | "structured_config_risk" | "config_guard" => {
            "config_risk_assessment"
        }
        "package_manager_detection" | "package_manager_detect" | "package_detect_manager" => {
            "package_manager_detection"
        }
        "sqlite_table_listing" => "sqlite_table_listing",
        "sqlite_table_names_only" => "sqlite_table_names_only",
        "sqlite_database_kind_judgment" => "sqlite_database_kind_judgment",
        "sqlite_schema_version" => "sqlite_schema_version",
        "archive_list" => "archive_list",
        "archive_pack" => "archive_pack",
        "archive_unpack" => "archive_unpack",
        "docker_ps" => "docker_ps",
        "docker_images" => "docker_images",
        "docker_logs" => "docker_logs",
        "docker_container_lifecycle" => "docker_container_lifecycle",
        _ => "none",
    }
}

fn normalize_direct_answer_gate_reference_target(raw: &str) -> &'static str {
    match normalize_schema_token_for_gate(raw).as_str() {
        "current_action_result" => "current_action_result",
        "current_turn_locator" => "current_turn_locator",
        "comparison_result" => "comparison_result",
        "unresolved_prior_object" => "unresolved_prior_object",
        "missing_locator" => "missing_locator",
        "ambiguous_locator" => "ambiguous_locator",
        _ => "none",
    }
}

fn canonicalize_direct_answer_gate_reference_resolution(value: Value) -> (Value, bool) {
    let Value::Object(mut map) = value else {
        return (default_direct_answer_gate_reference_resolution(), true);
    };
    let original_len = map.len();
    map.retain(|key, _| key == "target");
    let mut normalized = map.len() != original_len;
    match map.get("target").and_then(Value::as_str) {
        Some(raw) => {
            let canonical = normalize_direct_answer_gate_reference_target(raw);
            if canonical != raw {
                map.insert("target".to_string(), Value::String(canonical.to_string()));
                normalized = true;
            }
        }
        None => {
            map.insert("target".to_string(), Value::String("none".to_string()));
            normalized = true;
        }
    }
    (Value::Object(map), normalized)
}

fn canonicalize_direct_answer_gate_contract(value: Value) -> (Value, bool) {
    let Value::Object(mut map) = value else {
        return (default_direct_answer_gate_contract(), true);
    };
    let original_len = map.len();
    let allowed_keys = [
        "response_shape",
        "exact_sentence_count",
        "requires_content_evidence",
        "delivery_required",
        "locator_kind",
        "delivery_intent",
        "semantic_kind",
        "locator_hint",
        "self_extension",
    ];
    map.retain(|key, _| allowed_keys.contains(&key.as_str()));
    let mut normalized = map.len() != original_len;
    let defaults = default_direct_answer_gate_contract();
    let default_obj = defaults
        .as_object()
        .expect("default direct answer gate contract is object");
    for key in allowed_keys {
        if !map.contains_key(key) {
            if let Some(default_value) = default_obj.get(key) {
                map.insert(key.to_string(), default_value.clone());
                normalized = true;
            }
        }
    }
    let locator_hint = map
        .get("locator_hint")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    if let Some(Value::String(raw)) = map.get("response_shape").cloned() {
        let canonical = normalize_direct_answer_gate_response_shape(&raw);
        if canonical != raw {
            map.insert(
                "response_shape".to_string(),
                Value::String(canonical.to_string()),
            );
            normalized = true;
        }
    }
    if let Some(Value::String(raw)) = map.get("locator_kind").cloned() {
        let canonical = normalize_direct_answer_gate_locator_kind(&raw, &locator_hint);
        if canonical != raw {
            map.insert(
                "locator_kind".to_string(),
                Value::String(canonical.to_string()),
            );
            normalized = true;
        }
    }
    if let Some(Value::String(raw)) = map.get("delivery_intent").cloned() {
        let canonical = normalize_direct_answer_gate_delivery_intent(&raw);
        if canonical != raw {
            map.insert(
                "delivery_intent".to_string(),
                Value::String(canonical.to_string()),
            );
            normalized = true;
        }
    }
    let mut semantic_token_requests_scalar_shape = false;
    if let Some(Value::String(raw)) = map.get("semantic_kind").cloned() {
        semantic_token_requests_scalar_shape =
            direct_answer_gate_semantic_token_requests_scalar_shape(&raw);
        let canonical = normalize_direct_answer_gate_semantic_kind(&raw);
        if canonical != raw {
            map.insert(
                "semantic_kind".to_string(),
                Value::String(canonical.to_string()),
            );
            normalized = true;
        }
    }
    if semantic_token_requests_scalar_shape
        && map
            .get("response_shape")
            .and_then(Value::as_str)
            .is_some_and(|shape| shape != "scalar" && shape != "file_token")
    {
        map.insert(
            "response_shape".to_string(),
            Value::String("scalar".to_string()),
        );
        normalized = true;
    }
    let self_extension = map
        .remove("self_extension")
        .unwrap_or_else(|| default_obj["self_extension"].clone());
    let self_extension = match self_extension {
        Value::Object(mut extension) => {
            let original_len = extension.len();
            let allowed_extension_keys = ["mode", "trigger", "execute_now"];
            extension.retain(|key, _| allowed_extension_keys.contains(&key.as_str()));
            normalized |= extension.len() != original_len;
            let default_extension = default_obj["self_extension"]
                .as_object()
                .expect("default self_extension is object");
            for key in allowed_extension_keys {
                if !extension.contains_key(key) {
                    if let Some(default_value) = default_extension.get(key) {
                        extension.insert(key.to_string(), default_value.clone());
                        normalized = true;
                    }
                }
            }
            Value::Object(extension)
        }
        _ => {
            normalized = true;
            default_obj["self_extension"].clone()
        }
    };
    map.insert("self_extension".to_string(), self_extension);
    (Value::Object(map), normalized)
}

fn canonicalize_direct_answer_gate_object(
    mut map: serde_json::Map<String, Value>,
) -> (Value, bool) {
    let original_len = map.len();
    let allowed_keys = [
        "decision",
        "reason",
        "confidence",
        "clarify_question",
        "resolved_user_intent",
        "reference_resolution",
        "output_contract",
    ];
    map.retain(|key, _| allowed_keys.contains(&key.as_str()));
    let mut normalized = map.len() != original_len;
    if let Some(output_contract) = map.remove("output_contract") {
        let (output_contract, contract_normalized) =
            canonicalize_direct_answer_gate_contract(output_contract);
        normalized |= contract_normalized;
        map.insert("output_contract".to_string(), output_contract);
    } else {
        map.insert(
            "output_contract".to_string(),
            default_direct_answer_gate_contract(),
        );
        normalized = true;
    }
    if let Some(reference_resolution) = map.remove("reference_resolution") {
        let (reference_resolution, reference_normalized) =
            canonicalize_direct_answer_gate_reference_resolution(reference_resolution);
        normalized |= reference_normalized;
        map.insert("reference_resolution".to_string(), reference_resolution);
    } else {
        map.insert(
            "reference_resolution".to_string(),
            default_direct_answer_gate_reference_resolution(),
        );
        normalized = true;
    }
    (Value::Object(map), normalized)
}

fn canonicalize_contract_repair_judge_execution_recipe(value: Value) -> (Value, bool) {
    let Value::Object(mut recipe) = value else {
        return (
            json!({
                "kind": "none",
                "profile": "none",
                "target_scope": "unknown"
            }),
            true,
        );
    };
    let original_len = recipe.len();
    let allowed_keys = ["kind", "profile", "target_scope"];
    recipe.retain(|key, _| allowed_keys.contains(&key.as_str()));
    let mut normalized = recipe.len() != original_len;

    for (key, default) in [
        ("kind", "none"),
        ("profile", "none"),
        ("target_scope", "unknown"),
    ] {
        if !recipe.contains_key(key) {
            recipe.insert(key.to_string(), Value::String(default.to_string()));
            normalized = true;
        }
        if !recipe.get(key).is_some_and(Value::is_string) {
            recipe.insert(key.to_string(), Value::String(default.to_string()));
            normalized = true;
        }
    }

    if let Some(raw) = recipe.get("kind").and_then(Value::as_str) {
        let canonical = crate::execution_recipe::parse_execution_recipe_kind_text(raw).as_str();
        if canonical != raw {
            recipe.insert("kind".to_string(), Value::String(canonical.to_string()));
            normalized = true;
        }
    }
    if let Some(raw) = recipe.get("profile").and_then(Value::as_str) {
        let canonical = crate::execution_recipe::parse_execution_recipe_profile_text(raw).as_str();
        if canonical != raw {
            recipe.insert("profile".to_string(), Value::String(canonical.to_string()));
            normalized = true;
        }
    }
    if let Some(raw) = recipe.get("target_scope").and_then(Value::as_str) {
        let canonical =
            crate::execution_recipe::parse_execution_recipe_target_scope_text(raw).as_str();
        if canonical != raw {
            recipe.insert(
                "target_scope".to_string(),
                Value::String(canonical.to_string()),
            );
            normalized = true;
        }
    }

    (Value::Object(recipe), normalized)
}

fn canonicalize_contract_repair_judge_turn_type(value: Option<Value>) -> (Value, bool) {
    let Some(Value::String(raw)) = value else {
        return (Value::String(String::new()), true);
    };
    let normalized = normalize_schema_token_for_gate(&raw);
    let canonical = match normalized.as_str() {
        "" | "none" | "null" => "",
        "task_request" => "task_request",
        "task_append" => "task_append",
        "task_replace" => "task_replace",
        "task_correct" => "task_correct",
        "task_scope_update" => "task_scope_update",
        "run_control" => "run_control",
        "approval_decision" => "approval_decision",
        "status_query" => "status_query",
        "feedback_or_error" => "feedback_or_error",
        "preference_or_memory" => "preference_or_memory",
        _ => "",
    };
    (
        Value::String(canonical.to_string()),
        canonical != raw.trim(),
    )
}

fn canonicalize_contract_repair_judge_target_task_policy(value: Option<Value>) -> (Value, bool) {
    let Some(Value::String(raw)) = value else {
        return (Value::String(String::new()), true);
    };
    let normalized = normalize_schema_token_for_gate(&raw);
    let canonical = match normalized.as_str() {
        "" | "none" | "null" => "",
        "reuse_active" => "reuse_active",
        "replace_active" => "replace_active",
        "pause_and_queue" => "pause_and_queue",
        "standalone" => "standalone",
        _ => "",
    };
    (
        Value::String(canonical.to_string()),
        canonical != raw.trim(),
    )
}

fn infer_contract_repair_judge_apply(map: &serde_json::Map<String, Value>) -> bool {
    let decision = map
        .get("decision")
        .and_then(Value::as_str)
        .map(normalize_schema_token_for_gate)
        .unwrap_or_default();
    if decision == "planner_execute"
        || map.get("needs_clarify").and_then(Value::as_bool) == Some(true)
    {
        return true;
    }
    let Some(contract) = map.get("output_contract").and_then(Value::as_object) else {
        return false;
    };
    contract
        .get("requires_content_evidence")
        .and_then(Value::as_bool)
        == Some(true)
        || contract.get("delivery_required").and_then(Value::as_bool) == Some(true)
        || contract
            .get("semantic_kind")
            .and_then(Value::as_str)
            .is_some_and(|raw| normalize_direct_answer_gate_semantic_kind(raw) != "none")
        || contract
            .get("locator_kind")
            .and_then(Value::as_str)
            .is_some_and(|raw| normalize_direct_answer_gate_locator_kind(raw, "") != "none")
        || contract
            .get("delivery_intent")
            .and_then(Value::as_str)
            .is_some_and(|raw| normalize_direct_answer_gate_delivery_intent(raw) != "none")
}

fn canonicalize_contract_repair_judge_object(
    mut map: serde_json::Map<String, Value>,
) -> (Value, bool) {
    let original_len = map.len();
    let allowed_keys = [
        "apply",
        "reason",
        "confidence",
        "decision",
        "needs_clarify",
        "clarify_question",
        "resolved_user_intent",
        "output_contract",
        "execution_recipe",
        "turn_type",
        "target_task_policy",
        "state_patch",
    ];
    map.retain(|key, _| allowed_keys.contains(&key.as_str()));
    let mut normalized = map.len() != original_len;

    if let Some(output_contract) = map.remove("output_contract") {
        let (output_contract, contract_normalized) =
            canonicalize_direct_answer_gate_contract(output_contract);
        normalized |= contract_normalized;
        map.insert("output_contract".to_string(), output_contract);
    }
    if let Some(execution_recipe) = map.remove("execution_recipe") {
        let (execution_recipe, recipe_normalized) =
            canonicalize_contract_repair_judge_execution_recipe(execution_recipe);
        normalized |= recipe_normalized;
        map.insert("execution_recipe".to_string(), execution_recipe);
    }
    let (turn_type, turn_type_normalized) =
        canonicalize_contract_repair_judge_turn_type(map.remove("turn_type"));
    normalized |= turn_type_normalized;
    map.insert("turn_type".to_string(), turn_type);
    let (target_task_policy, target_policy_normalized) =
        canonicalize_contract_repair_judge_target_task_policy(map.remove("target_task_policy"));
    normalized |= target_policy_normalized;
    map.insert("target_task_policy".to_string(), target_task_policy);
    if !map.contains_key("apply") {
        let inferred_apply = infer_contract_repair_judge_apply(&map);
        map.insert("apply".to_string(), Value::Bool(inferred_apply));
        normalized = true;
    }

    (Value::Object(map), normalized)
}

fn canonicalize_schema_input(schema_id: PromptSchemaId, value: Value) -> (Value, bool) {
    match (schema_id, value) {
        (PromptSchemaId::IntentNormalizer, Value::Object(mut map)) => {
            let mut normalized = false;
            let mut execution_recipe_locator_hint: Option<Value> = None;
            let mut execution_recipe_self_extension: Option<Value> = None;
            if let Some(Value::Object(execution_recipe)) = map.get_mut("execution_recipe") {
                let allowed_keys = ["kind", "profile", "target_scope"];
                let mut stray_fields = Vec::new();
                for key in execution_recipe.keys().cloned().collect::<Vec<_>>() {
                    if allowed_keys.contains(&key.as_str()) {
                        continue;
                    }
                    if let Some(value) = execution_recipe.remove(&key) {
                        stray_fields.push((key, value));
                        normalized = true;
                    }
                }
                for (field, value) in stray_fields {
                    if field.contains("locator_hint") {
                        execution_recipe_locator_hint.get_or_insert(value);
                        continue;
                    }
                    if field.contains("self_extension") {
                        execution_recipe_self_extension.get_or_insert(value);
                        continue;
                    }
                }
            }
            if execution_recipe_locator_hint.is_some() || execution_recipe_self_extension.is_some()
            {
                match map.get_mut("output_contract") {
                    Some(Value::Object(output_contract)) => {
                        if let Some(locator_hint) = execution_recipe_locator_hint {
                            let needs_locator_hint = output_contract
                                .get("locator_hint")
                                .and_then(|v| v.as_str())
                                .map(str::trim)
                                .map(str::is_empty)
                                .unwrap_or(true);
                            if needs_locator_hint {
                                output_contract.insert("locator_hint".to_string(), locator_hint);
                            }
                        }
                        if let Some(self_extension) = execution_recipe_self_extension {
                            output_contract
                                .entry("self_extension".to_string())
                                .or_insert(self_extension);
                        }
                    }
                    Some(_) => {}
                    None => {
                        let mut output_contract = serde_json::Map::new();
                        if let Some(locator_hint) = execution_recipe_locator_hint {
                            output_contract.insert("locator_hint".to_string(), locator_hint);
                        }
                        if let Some(self_extension) = execution_recipe_self_extension {
                            output_contract.insert("self_extension".to_string(), self_extension);
                        }
                        map.insert(
                            "output_contract".to_string(),
                            Value::Object(output_contract),
                        );
                    }
                }
            }
            (Value::Object(map), normalized)
        }
        (PromptSchemaId::PlanResult, Value::Array(steps)) => {
            let (steps, _) = canonicalize_plan_steps_value(Value::Array(steps));
            (json!({ "steps": steps }), true)
        }
        (PromptSchemaId::PlanResult, Value::Object(map)) => canonicalize_plan_result_object(map),
        (PromptSchemaId::DirectAnswerGate, Value::Object(map)) => {
            canonicalize_direct_answer_gate_object(map)
        }
        (PromptSchemaId::ContractRepairJudge, Value::Object(map)) => {
            canonicalize_contract_repair_judge_object(map)
        }
        (_, value) => (value, false),
    }
}

pub(crate) fn validate_against_schema<T: DeserializeOwned>(
    raw: &str,
    schema_id: PromptSchemaId,
) -> Result<ValidatedSchemaJson<T>, SchemaValidationError> {
    let raw_parse_ok = serde_json::from_str::<T>(raw.trim()).is_ok();
    let parsed_value = parse_llm_json_raw_or_any_with_repair::<Value>(raw).ok_or_else(|| {
        SchemaValidationError {
            schema_id,
            stage: "parse_repair",
            details: vec!["unable to parse repaired JSON candidate".to_string()],
        }
    })?;
    let (schema_value, schema_normalized) = canonicalize_schema_input(schema_id, parsed_value);
    let schema_root = schema_id.schema_value();
    let mut validation_errors = Vec::new();
    validate_schema_value(
        schema_root,
        schema_root,
        &schema_value,
        "$",
        &mut validation_errors,
    );
    if !validation_errors.is_empty() {
        return Err(SchemaValidationError {
            schema_id,
            stage: "schema",
            details: validation_errors,
        });
    }
    let value = serde_json::from_value::<T>(schema_value).map_err(|err| SchemaValidationError {
        schema_id,
        stage: "deserialize",
        details: vec![err.to_string()],
    })?;
    Ok(ValidatedSchemaJson {
        value,
        raw_parse_ok,
        schema_normalized,
    })
}

pub(crate) fn parse_llm_json_extract_or_any<T: DeserializeOwned>(raw: &str) -> Option<T> {
    extract_json_object(raw)
        .or_else(|| extract_first_json_object_any(raw))
        .and_then(|s| serde_json::from_str::<T>(&s).ok())
}

pub(crate) fn parse_llm_json_raw_or_any<T: DeserializeOwned>(raw: &str) -> Option<T> {
    serde_json::from_str::<T>(raw.trim()).ok().or_else(|| {
        extract_first_json_object_any(raw).and_then(|s| serde_json::from_str::<T>(&s).ok())
    })
}

pub(crate) fn parse_llm_json_raw_or_any_with_repair<T: DeserializeOwned>(raw: &str) -> Option<T> {
    // F11: minimax / 部分模型偏好把 JSON plan 包在 ```json ... ``` 代码围栏里，
    // 围栏前/后还会带 prose（"根据上下文：..."、"需要先读取..."）。原生
    // `extract_first_json_object_any` 是 byte-level brace balancer，少量 raw
    // 在含中文宽括号 / 引号 / `\n` 转义 + `{{template}}` 占位时会过早终止，
    // 抓出的 candidate 不完整 → 解析后 step_count < 真实 step 数（典型现象：
    // plan 实际 4 步 [read, read, chat, respond] 被解析成只剩 [read, read]，
    // 后续 chat/respond 全丢，执行落入 observed_answer_fallback）。
    // 这里先用 codefence 显式提取一遍，命中则跳过 prose 干扰，保证 brace
    // balancer 从 envelope 第一个真正的 `{` 起走，否则回退原行为不破坏其它
    // 已经直接吐 JSON 的路径。
    if let Some(stripped) = strip_first_json_codefence(raw) {
        if let Some(value) = parse_json_with_repair::<T>(stripped.trim()) {
            return Some(value);
        }
        if let Some(value) =
            extract_first_json_object_any(&stripped).and_then(|s| parse_json_with_repair::<T>(&s))
        {
            return Some(value);
        }
    }
    parse_json_with_repair(raw.trim()).or_else(|| {
        extract_first_json_object_any(raw).and_then(|s| parse_json_with_repair::<T>(&s))
    })
}

/// 提取 raw 里第一个 ```json``` / ``` 代码围栏的内容；命中返回 fence 内文本，
/// 未命中返回 None。围栏类型容忍：` ```json `, ` ```JSON `, ` ``` ` 三种。
fn strip_first_json_codefence(raw: &str) -> Option<String> {
    let trimmed = raw.trim_start();
    // 找开 fence
    let fence_start = trimmed.find("```")?;
    let after_fence = &trimmed[fence_start + 3..];
    // 跳过可选语言标签 (json / JSON / 任意非换行串) + 一个换行
    let lang_end = after_fence.find('\n')?;
    let body_start = lang_end + 1;
    let body_and_rest = &after_fence[body_start..];
    // Prefer the first complete JSON value inside the fence. A planner may put
    // markdown fences inside a JSON string field such as `respond.content`; a
    // plain `find("```")` would treat that inner content as the closing fence
    // and truncate the plan.
    if let Some(json) = extract_first_json_value_any(body_and_rest) {
        return Some(json);
    }
    // 找闭 fence
    let close = body_and_rest.find("```")?;
    Some(body_and_rest[..close].to_string())
}

pub(crate) fn extract_first_json_value_any(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let opener = bytes[i];
        if opener != b'{' && opener != b'[' {
            i += 1;
            continue;
        }
        let start = i;
        let mut stack = vec![opener];
        let mut in_string = false;
        let mut escaped = false;
        let mut j = i + 1;
        while j < bytes.len() {
            let c = bytes[j];
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == b'\\' {
                    escaped = true;
                } else if c == b'"' {
                    in_string = false;
                }
                j += 1;
                continue;
            }
            match c {
                b'"' => in_string = true,
                b'{' | b'[' => stack.push(c),
                b'}' | b']' => {
                    let Some(last) = stack.pop() else {
                        break;
                    };
                    let matched = matches!((last, c), (b'{', b'}') | (b'[', b']'));
                    if !matched {
                        break;
                    }
                    if stack.is_empty() {
                        let candidate = &text[start..=j];
                        if serde_json::from_str::<Value>(candidate).is_ok() {
                            return Some(candidate.to_string());
                        }
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        i = start + 1;
    }
    None
}

pub(crate) fn extract_first_json_object_any(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i;
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escaped = false;
            let mut j = i;
            while j < bytes.len() {
                let c = bytes[j];
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == b'"' {
                        in_string = false;
                    }
                } else if c == b'"' {
                    in_string = true;
                } else if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    if depth == 0 {
                        return Some(text[start..=j].to_string());
                    }
                }
                j += 1;
            }
            i = j;
        }
        i += 1;
    }
    None
}

pub(crate) fn extract_json_object(text: &str) -> Option<String> {
    extract_agent_action_objects(text).into_iter().next()
}

pub(crate) fn extract_agent_action_objects(text: &str) -> Vec<String> {
    fn push_candidate_if_action(out: &mut Vec<String>, candidate: String) {
        if is_agent_action_candidate(&candidate) {
            out.push(candidate);
        }
    }

    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i;
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escaped = false;
            let mut j = i;
            let mut closed = false;

            while j < bytes.len() {
                let c = bytes[j];
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == b'"' {
                        in_string = false;
                    }
                } else if c == b'"' {
                    in_string = true;
                } else if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    if depth == 0 {
                        closed = true;
                        push_candidate_if_action(&mut out, text[start..=j].to_string());
                        break;
                    }
                } else if c == b']' && depth == 1 {
                    // Recover the trailing inner object when a wrapper array closes before the
                    // final action object emitted its own `}`.
                    let mut repaired = text[start..j].to_string();
                    repaired.push('}');
                    if serde_json::from_str::<Value>(&repaired).is_ok() {
                        closed = true;
                        push_candidate_if_action(&mut out, repaired);
                        break;
                    }
                }
                j += 1;
            }
            if closed {
                i = j;
            } else {
                i = start;
            }
        }
        i += 1;
    }
    out
}

pub(crate) fn parse_agent_action_json_with_repair(
    raw: &str,
    state: &AppState,
) -> Result<Value, String> {
    let parsed = match serde_json::from_str::<Value>(raw) {
        Ok(v) => Ok(v),
        Err(first_err) => {
            let repaired = repair_invalid_json_escapes(raw);
            match serde_json::from_str::<Value>(&repaired) {
                Ok(v) => Ok(v),
                Err(second_err) => Err(format!(
                    "initial parse error: {first_err}; repaired parse error: {second_err}"
                )),
            }
        }
    }?;
    Ok(normalize_agent_action_shape(parsed, state))
}

fn is_agent_action_candidate(candidate: &str) -> bool {
    if let Ok(v) = serde_json::from_str::<Value>(candidate) {
        return v.get("type").is_some()
            || v.get("action").is_some()
            || v.get("tool").is_some()
            || v.get("skill").is_some()
            || v.get("capability").is_some();
    }
    candidate.contains("\"type\"")
        || candidate.contains("\"action\"")
        || candidate.contains("\"tool\"")
        || candidate.contains("\"skill\"")
        || candidate.contains("\"capability\"")
}

fn repair_invalid_json_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    let mut in_string = false;
    let mut escaped = false;

    for ch in raw.chars() {
        if !in_string {
            if ch == '"' {
                in_string = true;
            }
            out.push(ch);
            continue;
        }

        if escaped {
            let valid = matches!(ch, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u');
            if !valid {
                out.push('\\');
            }
            out.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' => {
                out.push(ch);
                escaped = true;
            }
            '"' => {
                out.push(ch);
                in_string = false;
            }
            _ => out.push(ch),
        }
    }

    out
}

fn repair_stray_quote_after_primitive(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if ch == '"' {
            let prev = out.chars().rev().find(|c| !c.is_whitespace());
            let next = chars
                .iter()
                .skip(i + 1)
                .copied()
                .find(|c| !c.is_whitespace());
            if prev.is_some_and(|c| c.is_ascii_alphanumeric())
                && next.is_some_and(|c| matches!(c, ',' | '}' | ']'))
            {
                i += 1;
                continue;
            }
            in_string = true;
        }

        out.push(ch);
        i += 1;
    }

    out
}

fn repair_unescaped_inner_quotes(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len() + 16);
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];
        if !in_string {
            if ch == '"' {
                in_string = true;
            }
            out.push(ch);
            i += 1;
            continue;
        }

        if escaped {
            out.push(ch);
            escaped = false;
            i += 1;
            continue;
        }

        match ch {
            '\\' => {
                out.push(ch);
                escaped = true;
            }
            '"' => {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                let looks_like_string_end =
                    j >= chars.len() || matches!(chars[j], ',' | '}' | ']' | ':');
                if looks_like_string_end {
                    out.push(ch);
                    in_string = false;
                } else {
                    out.push('\\');
                    out.push('"');
                }
            }
            _ => out.push(ch),
        }
        i += 1;
    }

    out
}

/// 把任意 JSON 文本里的对象重复键去重为「last-wins」。
///
/// 背景：minimax 这类模型偶尔会输出包含重复键的 JSON（例如
/// `{"target_scope":"system","target_scope":"system"}`）。serde_json 自身把
/// `Value::Object` 实现为 BTreeMap/Map（last-wins，不会报错），但
/// `serde::Deserialize` 派生的 struct 反序列化时会触发
/// `Error("duplicate field ...")`，导致整个 JSON 解析失败。
///
/// 这里先把字符串 round-trip 一次：解析为 `Value` 时已经隐式去重，再
/// 序列化回字符串即可作为后续 struct deserialize 的喂入。
fn dedupe_json_object_keys(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    serde_json::to_string(&value).ok()
}

fn parse_json_with_repair<T: DeserializeOwned>(raw: &str) -> Option<T> {
    serde_json::from_str::<T>(raw)
        .ok()
        .or_else(|| {
            let repaired = repair_invalid_json_escapes(raw);
            serde_json::from_str::<T>(&repaired).ok()
        })
        .or_else(|| {
            let repaired = repair_stray_quote_after_primitive(raw);
            serde_json::from_str::<T>(&repaired).ok()
        })
        .or_else(|| {
            let repaired = repair_stray_quote_after_primitive(&repair_invalid_json_escapes(raw));
            serde_json::from_str::<T>(&repaired).ok()
        })
        .or_else(|| {
            let repaired = repair_unescaped_inner_quotes(raw);
            serde_json::from_str::<T>(&repaired).ok()
        })
        .or_else(|| {
            let repaired = repair_unescaped_inner_quotes(&repair_invalid_json_escapes(raw));
            serde_json::from_str::<T>(&repaired).ok()
        })
        // 最后再尝试一次「对象重复键去重」回退路径：
        // 处理 minimax / 部分模型偶发输出 `{"x":1,"x":1}` 之类 duplicate-field
        // 的合法 JSON 但派生 Deserialize 失败的 case（详见 dedupe_json_object_keys 注释）。
        // 仍然套一层 escape/quote repair，覆盖「重复键 + 转义异常」的复合场景。
        .or_else(|| {
            let deduped = dedupe_json_object_keys(raw)?;
            serde_json::from_str::<T>(&deduped).ok()
        })
        .or_else(|| {
            let deduped = dedupe_json_object_keys(&repair_invalid_json_escapes(raw))?;
            serde_json::from_str::<T>(&deduped).ok()
        })
        .or_else(|| {
            let deduped =
                dedupe_json_object_keys(&repair_unescaped_inner_quotes(raw)).or_else(|| {
                    dedupe_json_object_keys(&repair_unescaped_inner_quotes(
                        &repair_invalid_json_escapes(raw),
                    ))
                })?;
            serde_json::from_str::<T>(&deduped).ok()
        })
        // §F3-a：补齐截断 JSON 末尾未闭合的 `{`/`[`。
        // adv12 复现：MiniMax 偶发把 envelope 末尾 `}` 漏掉 + 把废弃字段误嵌入
        // `execution_recipe` 内部，导致 normalizer 解析失败 → 走 clarify 兜底，
        // 永远到不了 planner。补齐括号后 serde 用 `#[serde(default)]` 拿到字段的
        // 默认值，路由路径恢复。
        .or_else(|| {
            let balanced = balance_unclosed_brackets(raw)?;
            serde_json::from_str::<T>(&balanced).ok()
        })
        .or_else(|| {
            let balanced = balance_unclosed_brackets(&repair_invalid_json_escapes(raw))?;
            serde_json::from_str::<T>(&balanced).ok()
        })
        .or_else(|| {
            let balanced = balance_unclosed_brackets(&repair_unescaped_inner_quotes(raw))?;
            serde_json::from_str::<T>(&balanced).ok()
        })
        .or_else(|| {
            let balanced = balance_unclosed_brackets(&repair_unescaped_inner_quotes(
                &repair_invalid_json_escapes(raw),
            ))?;
            serde_json::from_str::<T>(&balanced).ok()
        })
}

/// §F3-a：在 raw 末尾按未闭合栈顺序补齐 `]` / `}`。
///
/// 实现要点：
/// - 全程感知 JSON 字符串语法（含 `\\` / `\"` 等转义），不会把字面量里的
///   括号当成结构标记；
/// - 只追加，不删除任何字符，保持已有内容字节级稳定，避免破坏其它 repair
///   路径；
/// - 字符串里如果末尾仍未闭合，先补一个 `"` 再补结构括号；
/// - 如果一路扫到末尾 `stack` 已经空了（即已经是平衡 JSON），返回 None
///   表示「无需追加」，让上游继续走原路径，而不是返回一个完全相同的字符串
///   再做一次 `from_str` 浪费一次 CPU。
fn balance_unclosed_brackets(raw: &str) -> Option<String> {
    let trimmed = raw.trim_end();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let mut stack: Vec<u8> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for &c in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_string = false;
            }
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                if stack.last() == Some(&c) {
                    stack.pop();
                }
            }
            _ => {}
        }
    }
    if !in_string && stack.is_empty() {
        return None;
    }
    let mut out = trimmed.to_string();
    if in_string {
        out.push('"');
    }
    while let Some(closer) = stack.pop() {
        out.push(closer as char);
    }
    Some(out)
}

fn normalize_agent_action_shape(value: Value, state: &AppState) -> Value {
    let Some(obj) = value.as_object() else {
        return value;
    };
    let Some(raw_type) = obj.get("type").and_then(|v| v.as_str()) else {
        if let Some(capability) = obj.get("capability").and_then(|v| v.as_str()) {
            let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
            return json!({
                "type": "call_capability",
                "capability": capability.trim(),
                "args": args,
            });
        }
        if let Some(skill) = obj.get("skill").and_then(|v| v.as_str()) {
            let normalized_skill = state.resolve_canonical_skill_name(skill.trim());
            if state.is_builtin_skill(&normalized_skill) {
                let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
                return json!({
                    "type": "call_skill",
                    "skill": normalized_skill,
                    "args": args,
                });
            }
        }
        if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
            let normalized_tool = state.resolve_canonical_skill_name(tool.trim());
            let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
            return json!({
                "type": "call_skill",
                "skill": normalized_tool,
                "args": args,
            });
        }
        if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
            return json!({
                "type": "respond",
                "content": content,
            });
        }
        if let Some(raw_action) = obj.get("action").and_then(|v| v.as_str()) {
            let action = raw_action.trim().to_ascii_lowercase();
            let args = collect_bare_action_alias_args(obj);
            if matches!(action.as_str(), "respond" | "reply" | "answer" | "final") {
                let content = args
                    .get("content")
                    .or_else(|| args.get("text"))
                    .or_else(|| args.get("message"))
                    .or_else(|| args.get("body"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                return json!({
                    "type": "respond",
                    "content": content,
                });
            }
            if action == "run_cmd" {
                return normalize_run_cmd_call(obj, args.as_object());
            }
            let normalized_skill = state.resolve_canonical_skill_name(&action);
            if state.is_builtin_skill(&normalized_skill) {
                return json!({
                    "type": "call_skill",
                    "skill": normalized_skill,
                    "args": args,
                });
            }
        }
        return value;
    };
    let step_type = raw_type.trim().to_ascii_lowercase();
    if matches!(
        step_type.as_str(),
        "think" | "call_tool" | "call_skill" | "call_capability" | "respond"
    ) {
        if step_type == "call_capability" {
            if let Some(capability) = obj.get("capability").and_then(|v| v.as_str()) {
                let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
                return json!({
                    "type": "call_capability",
                    "capability": capability.trim(),
                    "args": args,
                });
            }
        }
        if step_type == "call_tool" {
            if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
                let normalized_tool = state.resolve_canonical_skill_name(tool.trim());
                if normalized_tool == "run_cmd" {
                    return normalize_run_cmd_call(
                        obj,
                        obj.get("args").and_then(|v| v.as_object()),
                    );
                }
                let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
                return json!({
                    "type": "call_skill",
                    "skill": normalized_tool,
                    "args": args,
                });
            }
        }
        // F17: 兼容 LLM（典型 minimax）把多 step 合并到一个对象时，后写的
        // `"skill":"respond"` 字段覆盖前面的，导致 step 变成 call_skill(respond)。
        // executor 看到 skill="respond" 直接报"技能未开启 respond"。这里检测
        // call_skill+skill∈{respond,reply,answer} 时降级为顶层 respond，content
        // 取 args.content / args.text / content / text 中第一个有值的字符串。
        if step_type == "call_skill" {
            if let Some(skill) = obj.get("skill").and_then(|v| v.as_str()) {
                let canon = skill.trim().to_ascii_lowercase();
                if matches!(canon.as_str(), "respond" | "reply" | "answer" | "final") {
                    let args = obj.get("args").and_then(|v| v.as_object());
                    let pick = |k: &str| -> Option<String> {
                        let from_args = args
                            .and_then(|m| m.get(k))
                            .and_then(|v| v.as_str())
                            .map(str::to_string);
                        let from_top = obj.get(k).and_then(|v| v.as_str()).map(str::to_string);
                        from_args.or(from_top)
                    };
                    let content = pick("content")
                        .or_else(|| pick("text"))
                        .or_else(|| pick("message"))
                        .or_else(|| pick("body"))
                        .unwrap_or_default();
                    return json!({
                        "type": "respond",
                        "content": content,
                    });
                }
                let normalized_skill = state.resolve_canonical_skill_name(skill.trim());
                if normalized_skill == "run_cmd" {
                    return normalize_run_cmd_call(
                        obj,
                        obj.get("args").and_then(|v| v.as_object()),
                    );
                }
                if normalized_skill == "system_basic" {
                    if let Some(args) = obj.get("args").and_then(|v| v.as_object()) {
                        if let Some(base_skill) = normalize_system_basic_base_skill_alias(args) {
                            return base_skill;
                        }
                        if args.get("action").and_then(|v| v.as_str()) == Some("run_cmd") {
                            return normalize_run_cmd_call(obj, Some(args));
                        }
                    }
                }
            }
        }
        return value;
    }

    if step_type == "run_cmd" {
        return normalize_run_cmd_call(obj, obj.get("args").and_then(|v| v.as_object()));
    }

    let args = collect_bare_action_args(obj);
    if state.is_builtin_skill(&step_type) {
        return json!({
            "type": "call_skill",
            "skill": step_type,
            "args": args,
        });
    }

    let normalized_skill = state.resolve_canonical_skill_name(&step_type);
    if state.is_builtin_skill(&normalized_skill) {
        return json!({
            "type": "call_skill",
            "skill": normalized_skill,
            "args": args,
        });
    }

    value
}

fn normalize_run_cmd_call(
    obj: &serde_json::Map<String, Value>,
    raw_args: Option<&serde_json::Map<String, Value>>,
) -> Value {
    let value_for = |primary: &str, aliases: &[&str]| -> Option<Value> {
        raw_args
            .and_then(|args| args.get(primary).cloned())
            .or_else(|| obj.get(primary).cloned())
            .or_else(|| {
                aliases.iter().find_map(|alias| {
                    raw_args
                        .and_then(|args| args.get(*alias).cloned())
                        .or_else(|| obj.get(*alias).cloned())
                })
            })
    };

    let mut args = serde_json::Map::new();
    if let Some(command) = value_for("command", &["cmd"]) {
        args.insert("command".to_string(), command);
    }
    if let Some(cwd) = value_for("cwd", &["workdir"]) {
        args.insert("cwd".to_string(), cwd);
    }
    if let Some(timeout) = value_for("timeout_seconds", &[]) {
        args.insert("timeout_seconds".to_string(), timeout);
    } else if let Some(timeout_ms) = value_for("timeout_ms", &[]).and_then(|v| v.as_u64()) {
        args.insert(
            "timeout_seconds".to_string(),
            json!(((timeout_ms + 999) / 1000).max(1)),
        );
    }
    if let Some(request_text) = value_for("request_text", &[]) {
        args.insert("request_text".to_string(), request_text);
    }
    if let Some(suggested_params) = value_for("suggested_params", &[]) {
        args.insert("suggested_params".to_string(), suggested_params);
    }
    if let Some(suggest_once) = value_for("suggest_once", &[]) {
        args.insert("suggest_once".to_string(), suggest_once);
    }
    if let Some(llm_suggest_once) = value_for("llm_suggest_once", &[]) {
        args.insert("llm_suggest_once".to_string(), llm_suggest_once);
    }
    preserve_internal_execution_args(&mut args, raw_args);
    preserve_internal_execution_args(&mut args, Some(obj));
    json!({
        "type": "call_skill",
        "skill": "run_cmd",
        "args": Value::Object(args),
    })
}

fn preserve_internal_execution_args(
    args: &mut serde_json::Map<String, Value>,
    source: Option<&serde_json::Map<String, Value>>,
) {
    let Some(source) = source else {
        return;
    };
    for (key, value) in source {
        if key.starts_with("_clawd_") {
            args.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}

fn normalize_system_basic_base_skill_alias(args: &serde_json::Map<String, Value>) -> Option<Value> {
    let action = args.get("action").and_then(|v| v.as_str())?;
    let path_value = args
        .get("path")
        .cloned()
        .or_else(|| args.get("dir").cloned())
        .or_else(|| args.get("target").cloned());
    match action {
        "list_dir" => {
            if system_basic_list_dir_requires_inventory_dir(args) {
                let mut inventory_args = serde_json::Map::new();
                inventory_args.insert("action".to_string(), json!("inventory_dir"));
                inventory_args.insert("path".to_string(), path_value?);
                for key in [
                    "names_only",
                    "include_hidden",
                    "files_only",
                    "dirs_only",
                    "ext_filter",
                ] {
                    if let Some(value) = args.get(key).cloned() {
                        inventory_args.insert(key.to_string(), value);
                    }
                }
                if let Some(limit) = args
                    .get("max_entries")
                    .cloned()
                    .or_else(|| args.get("limit").cloned())
                {
                    inventory_args.insert("max_entries".to_string(), limit);
                }
                if let Some(sort_by) = normalize_inventory_dir_sort_by(args) {
                    inventory_args.insert("sort_by".to_string(), json!(sort_by));
                }
                Some(json!({
                    "type": "call_skill",
                    "skill": "system_basic",
                    "args": Value::Object(inventory_args),
                }))
            } else {
                Some(json!({
                    "type": "call_skill",
                    "skill": "list_dir",
                    "args": {
                        "path": path_value?,
                        "names_only": args
                            .get("names_only")
                            .cloned()
                            .unwrap_or_else(|| json!(false))
                    },
                }))
            }
        }
        "read_file" => Some(json!({
            "type": "call_skill",
            "skill": "read_file",
            "args": {
                "path": path_value?
            },
        })),
        "make_dir" => Some(json!({
            "type": "call_skill",
            "skill": "make_dir",
            "args": {
                "path": path_value?
            },
        })),
        "remove_file" => Some(json!({
            "type": "call_skill",
            "skill": "remove_file",
            "args": {
                "path": path_value?
            },
        })),
        "write_file" => Some(json!({
            "type": "call_skill",
            "skill": "write_file",
            "args": {
                "path": path_value?,
                "content": args.get("content").cloned()?
            },
        })),
        _ => None,
    }
}

fn system_basic_list_dir_requires_inventory_dir(args: &serde_json::Map<String, Value>) -> bool {
    args.keys().any(|key| {
        matches!(
            key.as_str(),
            "limit"
                | "max_entries"
                | "sort_by"
                | "order"
                | "include_hidden"
                | "files_only"
                | "dirs_only"
                | "ext_filter"
                | "options"
        )
    })
}

fn normalize_inventory_dir_sort_by(args: &serde_json::Map<String, Value>) -> Option<String> {
    let sort_by = args
        .get("sort_by")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_ascii_lowercase();
    let descending = args
        .get("order")
        .and_then(|v| v.as_str())
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !matches!(value.as_str(), "asc" | "ascending")
        })
        .unwrap_or(true);
    match sort_by.as_str() {
        "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc" | "name" => Some(sort_by),
        "mtime" | "modified" | "modified_ts" | "modified_time" => Some(if descending {
            "mtime_desc".to_string()
        } else {
            "mtime_asc".to_string()
        }),
        "size" | "size_bytes" | "bytes" => Some(if descending {
            "size_desc".to_string()
        } else {
            "size_asc".to_string()
        }),
        _ => None,
    }
}

fn collect_bare_action_args(obj: &serde_json::Map<String, Value>) -> Value {
    let mut args = obj
        .get("args")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    for (key, value) in obj {
        if matches!(
            key.as_str(),
            "type" | "args" | "tool" | "skill" | "capability"
        ) {
            continue;
        }
        args.insert(key.clone(), value.clone());
    }
    Value::Object(args)
}

fn collect_bare_action_alias_args(obj: &serde_json::Map<String, Value>) -> Value {
    let mut args = collect_bare_action_args(obj);
    if let Value::Object(map) = &mut args {
        map.remove("action");
    }
    args
}

#[cfg(test)]
#[path = "prompt_utils_tests.rs"]
mod tests;

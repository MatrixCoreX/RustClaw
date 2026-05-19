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
    if let Some(Value::String(raw)) = map.get("semantic_kind").cloned() {
        let canonical = normalize_direct_answer_gate_semantic_kind(&raw);
        if canonical != raw {
            map.insert(
                "semantic_kind".to_string(),
                Value::String(canonical.to_string()),
            );
            normalized = true;
        }
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
mod tests {
    use serde_json::{json, Value};

    #[test]
    fn validate_against_schema_rejects_unknown_intent_decision() {
        let raw = r#"{
            "resolved_user_intent":"check logs",
            "decision":"oops_status",
            "needs_clarify":false,
            "reason":"r",
            "confidence":0.9
        }"#;
        let err =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
                .expect_err("unknown decision should fail schema validation");
        assert!(err.to_string().contains("$.decision"));
        assert!(err.to_string().contains("oops_status"));
    }

    #[test]
    fn validate_against_schema_rejects_out_of_range_finalizer_confidence() {
        let raw = r#"{
            "answer":"done",
            "qualified":true,
            "needs_clarify":false,
            "is_meta_instruction":false,
            "publishable":true,
            "confidence":1.5
        }"#;
        let err = super::validate_against_schema::<Value>(raw, super::PromptSchemaId::FinalizerOut)
            .expect_err("confidence > 1 should fail schema validation");
        assert!(err.to_string().contains("$.confidence"));
    }

    #[test]
    fn validate_against_schema_canonicalizes_bare_plan_array() {
        let raw = r#"[{"type":"respond","content":"done"}]"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::PlanResult)
                .expect("bare array should canonicalize to steps envelope");
        assert!(validated.schema_normalized);
        assert_eq!(
            validated
                .value
                .pointer("/steps/0/type")
                .and_then(|v| v.as_str()),
            Some("respond")
        );
    }

    #[test]
    fn validate_against_schema_canonicalizes_plan_action_alias_array() {
        let raw = r#"{"actions":[{"type":"respond","content":"done"}]}"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::PlanResult)
                .expect("actions alias should canonicalize to steps envelope");
        assert!(validated.schema_normalized);
        assert_eq!(
            validated
                .value
                .pointer("/steps/0/content")
                .and_then(|v| v.as_str()),
            Some("done")
        );
        assert!(validated.value.get("actions").is_none());
    }

    #[test]
    fn validate_against_schema_strips_plan_result_noise_fields() {
        let raw = r#"{
            "goal":"ignored envelope noise",
            "planner_notes":"kept",
            "steps":[
                {
                    "type":"RESPOND",
                    "content":"done",
                    "id":"step_1",
                    "description":"ignored action noise"
                }
            ]
        }"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::PlanResult)
                .expect("plan_result noise fields should be stripped before schema validation");
        assert!(validated.schema_normalized);
        assert_eq!(
            validated
                .value
                .pointer("/steps/0/type")
                .and_then(|v| v.as_str()),
            Some("respond")
        );
        assert_eq!(
            validated
                .value
                .pointer("/steps/0/content")
                .and_then(|v| v.as_str()),
            Some("done")
        );
        assert_eq!(
            validated
                .value
                .get("planner_notes")
                .and_then(|v| v.as_str()),
            Some("kept")
        );
        assert!(validated.value.get("goal").is_none());
        assert!(validated.value.pointer("/steps/0/id").is_none());
        assert!(validated.value.pointer("/steps/0/description").is_none());
    }

    #[test]
    fn validate_against_schema_projects_normalizer_shaped_direct_answer_gate_output() {
        let raw = r#"{
            "resolved_user_intent": "List current listening ports and highlight notable ports",
            "resume_behavior": "none",
            "schedule_kind": "none",
            "schedule_intent": null,
            "wants_file_delivery": false,
            "should_refresh_long_term_memory": false,
            "agent_display_name_hint": "",
            "needs_clarify": false,
            "clarify_question": "",
            "reason": "Fresh system state observation is required.",
            "confidence": 0.94,
            "decision": "planner_execute",
            "reference_resolution": {"target": "none"},
            "output_contract": {
                "response_shape": "free",
                "requires_content_evidence": true,
                "delivery_required": false,
                "locator_kind": "none",
                "delivery_intent": "none",
                "semantic_kind": "none",
                "locator_hint": "",
                "self_extension": {
                    "mode": "none",
                    "trigger": "none",
                    "execute_now": false,
                    "ignored": true
                },
                "ignored_contract_field": "drop"
            },
            "execution_recipe": {"kind": "none", "profile": "none", "target_scope": "none"},
            "turn_type": "task_request",
            "target_task_policy": "standalone",
            "should_interrupt_active_run": false,
            "state_patch": null,
            "attachment_processing_required": false
        }"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::DirectAnswerGate)
                .expect("normalizer-shaped gate output should project to gate schema");
        assert!(validated.schema_normalized);
        assert_eq!(
            validated.value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            validated
                .value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(validated.value.get("execution_recipe").is_none());
        assert!(validated
            .value
            .pointer("/output_contract/ignored_contract_field")
            .is_none());
        assert!(validated
            .value
            .pointer("/output_contract/self_extension/ignored")
            .is_none());
    }

    #[test]
    fn validate_against_schema_defaults_missing_direct_answer_gate_reference_resolution() {
        let raw = r#"{
            "decision": "planner_execute",
            "reason": "Fresh file content is required.",
            "confidence": 0.93,
            "clarify_question": "",
            "resolved_user_intent": "Inspect the current workspace schema for a target enum",
            "output_contract": {
                "response_shape": "free",
                "requires_content_evidence": true,
                "delivery_required": false,
                "locator_kind": "current_workspace",
                "delivery_intent": "none",
                "semantic_kind": "content_presence_check",
                "locator_hint": "",
                "self_extension": {
                    "mode": "none",
                    "trigger": "none",
                    "execute_now": false
                }
            }
        }"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::DirectAnswerGate)
                .expect("missing reference_resolution should be normalized to none");

        assert!(validated.schema_normalized);
        assert_eq!(
            validated
                .value
                .pointer("/reference_resolution/target")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert_eq!(
            validated.value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
    }

    #[test]
    fn validate_against_schema_normalizes_direct_answer_gate_file_locator_alias() {
        let raw = r#"{
            "decision": "planner_execute",
            "reason": "fresh file content is required",
            "confidence": 0.95,
            "clarify_question": "",
            "resolved_user_intent": "Read the last lines from /tmp/clawd.log",
            "reference_resolution": {"target": "none"},
            "output_contract": {
                "response_shape": "free",
                "requires_content_evidence": true,
                "delivery_required": false,
                "locator_kind": "file",
                "delivery_intent": "none",
                "semantic_kind": "tail_lines",
                "locator_hint": "/tmp/clawd.log",
                "self_extension": {
                    "mode": "none",
                    "trigger": "none",
                    "execute_now": false
                }
            }
        }"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::DirectAnswerGate)
                .expect("gate file locator alias should be normalized");

        assert!(validated.schema_normalized);
        assert_eq!(
            validated
                .value
                .pointer("/output_contract/locator_kind")
                .and_then(|v| v.as_str()),
            Some("path")
        );
        assert_eq!(
            validated
                .value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("content_excerpt_summary")
        );
    }

    #[test]
    fn validate_against_schema_preserves_direct_answer_gate_semantic_enums() {
        let semantic_kinds = [
            "service_status",
            "directory_entry_groups",
            "directory_purpose_summary",
            "excerpt_kind_judgment",
            "recent_artifacts_judgment",
        ];

        for semantic_kind in semantic_kinds {
            let raw = json!({
                "decision": "planner_execute",
                "reason": "fresh observation is required",
                "confidence": 0.95,
                "clarify_question": "",
                "resolved_user_intent": "Inspect a concrete workspace target",
                "reference_resolution": {"target": "none"},
                "output_contract": {
                    "response_shape": "strict",
                    "requires_content_evidence": true,
                    "delivery_required": false,
                    "locator_kind": "path",
                    "delivery_intent": "none",
                    "semantic_kind": semantic_kind,
                    "locator_hint": "logs",
                    "self_extension": {
                        "mode": "none",
                        "trigger": "none",
                        "execute_now": false
                    }
                }
            })
            .to_string();
            let validated = super::validate_against_schema::<Value>(
                &raw,
                super::PromptSchemaId::DirectAnswerGate,
            )
            .expect("canonical semantic kind should pass gate schema");

            assert_eq!(
                validated
                    .value
                    .pointer("/output_contract/semantic_kind")
                    .and_then(|v| v.as_str()),
                Some(semantic_kind)
            );
        }
    }

    #[test]
    fn validate_against_schema_canonicalizes_single_plan_step_object() {
        let raw = r#"{"steps":{"type":"respond","content":"done"}}"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::PlanResult)
                .expect("single object steps should canonicalize to steps array");
        assert!(validated.schema_normalized);
        assert_eq!(
            validated
                .value
                .pointer("/steps/0/type")
                .and_then(|v| v.as_str()),
            Some("respond")
        );
    }

    #[test]
    fn fenced_plan_parser_keeps_inner_markdown_fence_in_respond_content() {
        let raw = "模型说明。\n\n```json\n{\"steps\":[{\"type\":\"respond\",\"content\":\"前 15 行：\\n```\\n#!/usr/bin/env bash\\nset -euo pipefail\\n```\\n\\n这是一个重启 clawd 服务的脚本。\"}]}\n```\n";
        let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
            .expect("fenced plan with nested markdown fence should parse");
        let content = parsed
            .pointer("/steps/0/content")
            .and_then(|v| v.as_str())
            .expect("respond content should be preserved");
        assert!(content.contains("#!/usr/bin/env bash"));
        assert!(content.contains("这是一个重启 clawd 服务的脚本"));
    }

    #[test]
    fn validate_against_schema_drops_execution_recipe_stray_fields() {
        let raw = r#"{
            "resolved_user_intent":"列出 document 目录下前 5 个文件名",
            "decision":"planner_execute",
            "output_contract":{
                "response_shape":"free",
                "requires_content_evidence":true,
                "delivery_required":false,
                "locator_kind":"filename",
                "delivery_intent":"none",
                "semantic_kind":"none"
            },
            "needs_clarify":false,
            "reason":"r",
            "confidence":0.92,
            "execution_recipe":{
                "kind":"none",
                "profile":"none",
                "target_scope":"current_repo",
                "unknown_extra_text":"wrong place",
                "unknown_extra_score":0.61
            }
        }"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
                .expect("model-noise execution_recipe stray fields should be dropped");
        assert!(validated.schema_normalized);
        assert!(validated
            .value
            .get("execution_recipe")
            .and_then(|v| v.get("unknown_extra_text"))
            .is_none());
    }

    #[test]
    fn validate_against_schema_normalizes_contract_repair_judge_payload_noise() {
        let raw = r#"{
            "apply": true,
            "reason": "semantic repair",
            "confidence": 0.92,
            "decision":"planner_execute",
            "needs_clarify": false,
            "clarify_question": "",
            "resolved_user_intent": "find README candidates",
            "agent_display_name_hint": "extra field from model",
            "output_contract": {
                "response_shape": "list",
                "requires_content_evidence": true,
                "delivery_required": false,
                "locator_kind": "file",
                "delivery_intent": "none",
                "semantic_kind": "path_list",
                "locator_hint": "README",
                "unused": "drop me",
                "self_extension": {
                    "mode": "none",
                    "trigger": "none",
                    "execute_now": false
                }
            },
            "execution_recipe": {
                "kind": "none",
                "profile": "none",
                "target_scope": "none",
                "unexpected": "drop me"
            }
        }"#;

        let validated = super::validate_against_schema::<Value>(
            raw,
            super::PromptSchemaId::ContractRepairJudge,
        )
        .expect("contract repair judge output should tolerate harmless model noise");

        assert!(validated.schema_normalized);
        assert!(validated.value.get("agent_display_name_hint").is_none());
        assert_eq!(
            validated
                .value
                .pointer("/output_contract/response_shape")
                .and_then(Value::as_str),
            Some("strict")
        );
        assert_eq!(
            validated
                .value
                .pointer("/output_contract/semantic_kind")
                .and_then(Value::as_str),
            Some("file_paths")
        );
        assert_eq!(
            validated
                .value
                .pointer("/execution_recipe/target_scope")
                .and_then(Value::as_str),
            Some("unknown")
        );
        assert!(validated
            .value
            .pointer("/execution_recipe/unexpected")
            .is_none());
    }

    #[test]
    fn validate_against_schema_repairs_execution_recipe_locator_hint() {
        let raw = r#"{
            "resolved_user_intent":"列出 document 目录下前 5 个文件名",
            "decision":"planner_execute",
            "needs_clarify":false,
            "reason":"r",
            "confidence":0.95,
            "output_contract":{
                "response_shape":"free",
                "requires_content_evidence":false,
                "delivery_required":false,
                "locator_kind":"current_workspace",
                "delivery_intent":"none",
                "semantic_kind":"none"
            },
            "execution_recipe":{
                "kind":"none",
                "profile":"none",
                "target_scope":"current_repo",
                "locator_hint":"document"
            }
        }"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
                .expect("execution_recipe.locator_hint should be canonicalized");
        assert!(validated.schema_normalized);
        assert_eq!(
            validated
                .value
                .pointer("/output_contract/locator_hint")
                .and_then(|v| v.as_str()),
            Some("document")
        );
        assert!(validated
            .value
            .pointer("/execution_recipe/locator_hint")
            .is_none());
    }

    #[test]
    fn validate_against_schema_repairs_execution_recipe_self_extension() {
        let raw = r#"{
            "resolved_user_intent":"检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径",
            "decision":"planner_execute",
            "needs_clarify":false,
            "reason":"r",
            "confidence":0.95,
            "output_contract":{
                "response_shape":"scalar",
                "requires_content_evidence":false,
                "delivery_required":false,
                "locator_kind":"current_workspace",
                "delivery_intent":"none",
                "semantic_kind":"existence_with_path",
                "locator_hint":"rustclaw.service"
            },
            "execution_recipe":{
                "kind":"none",
                "profile":"none",
                "target_scope":"current_repo",
                "self_extension":{"mode":"none","trigger":"none","execute_now":false}
            }
        }"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
                .expect("execution_recipe.self_extension should be canonicalized");
        assert!(validated.schema_normalized);
        assert_eq!(
            validated
                .value
                .pointer("/output_contract/self_extension/mode")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert!(validated
            .value
            .pointer("/execution_recipe/self_extension")
            .is_none());
    }

    #[test]
    fn validate_against_schema_repairs_execution_recipe_reason() {
        let raw = r#"{
            "resolved_user_intent":"列出 logs 目录下前 5 个文件名（按顺序编号）",
            "decision":"planner_execute",
            "needs_clarify":false,
            "reason":"r",
            "confidence":0.92,
            "output_contract":{
                "response_shape":"free",
                "requires_content_evidence":true,
                "delivery_required":false,
                "locator_kind":"current_workspace",
                "delivery_intent":"none",
                "semantic_kind":"none",
                "locator_hint":"logs"
            },
            "execution_recipe":{
                "kind":"none",
                "profile":"none",
                "target_scope":"current_repo",
                "reason":"scope is clear"
            }
        }"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
                .expect("execution_recipe.reason should be canonicalized away");
        assert!(validated.schema_normalized);
        assert_eq!(
            validated
                .value
                .pointer("/execution_recipe/kind")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert!(validated
            .value
            .pointer("/execution_recipe/reason")
            .is_none());
    }

    #[test]
    fn validate_against_schema_repairs_execution_recipe_qualifier() {
        let raw = r#"{
            "resolved_user_intent":"执行基础健康检查，只列最重要的结论",
            "decision":"planner_execute",
            "needs_clarify":false,
            "reason":"r",
            "confidence":0.92,
            "output_contract":{
                "response_shape":"one_sentence",
                "requires_content_evidence":true,
                "delivery_required":false,
                "locator_kind":"none",
                "delivery_intent":"none",
                "semantic_kind":"service_status"
            },
            "execution_recipe":{
                "kind":"none",
                "profile":"ops_service",
                "target_scope":"system",
                "qualifier":""
            }
        }"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
                .expect("execution_recipe.qualifier should be dropped");
        assert!(validated.schema_normalized);
        assert_eq!(
            validated
                .value
                .pointer("/execution_recipe/profile")
                .and_then(|v| v.as_str()),
            Some("ops_service")
        );
        assert!(validated
            .value
            .pointer("/execution_recipe/qualifier")
            .is_none());
    }

    #[test]
    fn validate_against_schema_repairs_malformed_execution_recipe_boundary_field() {
        let raw = r#"{
            "resolved_user_intent":"查看当前 git 分支名称，只输出分支名",
            "decision":"planner_execute",
            "needs_clarify":false,
            "reason":"r",
            "confidence":0.95,
            "output_contract":{
                "response_shape":"scalar",
                "requires_content_evidence":false,
                "delivery_required":false,
                "locator_kind":"current_workspace",
                "delivery_intent":"none",
                "semantic_kind":"scalar_path_only"
            },
            "execution_recipe":{
                "kind":"none",
                "profile":"none",
                "target_scope":"current_repo",
                "},\"unknown_extra_text":""
            },
            "unknown_extra_score":0.0
        }"#;
        let validated =
            super::validate_against_schema::<Value>(raw, super::PromptSchemaId::IntentNormalizer)
                .expect("malformed execution_recipe boundary field should be dropped");
        assert!(validated.schema_normalized);
        assert_eq!(
            validated
                .value
                .pointer("/execution_recipe/target_scope")
                .and_then(|v| v.as_str()),
            Some("current_repo")
        );
        assert!(validated
            .value
            .pointer("/execution_recipe/},\\\"unknown_extra_text")
            .is_none());
    }

    #[test]
    fn parse_llm_json_raw_or_any_with_repair_handles_unescaped_quotes() {
        let raw = r#"{"resolved_user_intent":"记住："那玩意README"指向 /home/guagua/test/README.md","reason":"用户定义了"那玩意README"映射","confidence":1.0}"#;
        let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
            .expect("should parse repaired json");
        assert_eq!(
            parsed
                .get("resolved_user_intent")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            "记住：\"那玩意README\"指向 /home/guagua/test/README.md"
        );
    }

    #[test]
    fn parse_llm_json_raw_or_any_with_repair_dedupes_object_keys_for_struct() {
        use serde::Deserialize;
        #[derive(Debug, Deserialize, PartialEq, Eq)]
        struct ExecutionRecipeProbe {
            kind: String,
            target_scope: String,
        }
        let raw = r#"{"kind":"none","target_scope":"system","target_scope":"system"}"#;
        // Sanity check: 直接 derive Deserialize 在 duplicate field 上会失败。
        assert!(serde_json::from_str::<ExecutionRecipeProbe>(raw).is_err());
        let parsed = super::parse_llm_json_raw_or_any_with_repair::<ExecutionRecipeProbe>(raw)
            .expect("dedup pass should recover duplicate-key object");
        assert_eq!(
            parsed,
            ExecutionRecipeProbe {
                kind: "none".to_string(),
                target_scope: "system".to_string(),
            }
        );
    }

    #[test]
    fn parse_llm_json_raw_or_any_with_repair_dedupes_nested_duplicate_keys() {
        let raw = r#"{"decision":"planner_execute","execution_recipe":{"kind":"none","profile":"ops_service","target_scope":"system","target_scope":"system"}}"#;
        let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
            .expect("nested duplicate keys should be repaired");
        assert_eq!(
            parsed
                .pointer("/execution_recipe/target_scope")
                .and_then(|v| v.as_str()),
            Some("system")
        );
        assert_eq!(
            parsed.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
    }

    /// §F3-a：补齐缺失尾括号 + 测试 adv12 真实 MiniMax 输出。
    #[test]
    fn balance_unclosed_brackets_recovers_truncated_object() {
        // 完整对象本身已平衡，应返回 None（不重复劳动）。
        assert!(super::balance_unclosed_brackets(r#"{"a":1}"#).is_none());
        // 简单缺一个 `}`。
        assert_eq!(
            super::balance_unclosed_brackets(r#"{"a":1"#).as_deref(),
            Some(r#"{"a":1}"#)
        );
        // 嵌套缺多个 `}`。
        assert_eq!(
            super::balance_unclosed_brackets(r#"{"a":{"b":{"c":1"#).as_deref(),
            Some(r#"{"a":{"b":{"c":1}}}"#)
        );
        // 字符串里出现 `{` / `}` 不应当成结构标记。
        assert!(super::balance_unclosed_brackets(r#"{"text":"{x}"}"#).is_none());
        // 数组也兼容。
        assert_eq!(
            super::balance_unclosed_brackets(r#"[1,[2,3"#).as_deref(),
            Some(r#"[1,[2,3]]"#)
        );
        // 字符串未闭合 + 缺 `}`：先补 `"`，再补 `}`。
        assert_eq!(
            super::balance_unclosed_brackets(r#"{"a":"hello"#).as_deref(),
            Some(r#"{"a":"hello"}"#)
        );
    }

    /// §F3-a：adv12 复现的 MiniMax 输出模式（结尾少一个 `}` +
    /// 废弃/未知字段误嵌入 `execution_recipe`）必须能被 repair 成可解析。
    #[test]
    fn parse_llm_json_raw_or_any_with_repair_recovers_adv12_minimax_envelope() {
        // 注意：原始 JSON 末尾少了 envelope 自己的最后一个 `}`，
        // 且废弃字段错误地嵌入 `execution_recipe`。repair 后应能解析并保留
        // envelope 顶层字段。
        let raw = r#"{"resolved_user_intent":"x","resume_behavior":"none","schedule_kind":"none","schedule_intent":null,"wants_file_delivery":false,"should_refresh_long_term_memory":false,"agent_display_name_hint":"","needs_clarify":false,"clarify_question":"","reason":"r","confidence":0.95,"decision":"planner_execute","output_contract":{"response_shape":"free","requires_content_evidence":false,"delivery_required":false,"locator_kind":"filename","delivery_intent":"none","semantic_kind":"existence_with_path","locator_hint":"AGENTS.md","self_extension":{"mode":"none","trigger":"none","execute_now":false}},"execution_recipe":{"kind":"none","profile":"none","target_scope":"current_repo","unknown_extra_text":"","unknown_extra_score":0.0}"#;
        // 直接 from_str 必失败：少最后一个 `}`。
        assert!(serde_json::from_str::<serde_json::Value>(raw).is_err());
        let parsed = super::parse_llm_json_raw_or_any_with_repair::<serde_json::Value>(raw)
            .expect("balance pass should recover truncated MiniMax envelope");
        assert_eq!(
            parsed.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute"),
            "envelope decision field must survive repair"
        );
        assert_eq!(
            parsed.get("needs_clarify").and_then(|v| v.as_bool()),
            Some(false),
            "envelope needs_clarify must survive repair"
        );
        assert_eq!(
            parsed
                .pointer("/output_contract/locator_kind")
                .and_then(|v| v.as_str()),
            Some("filename")
        );
    }

    #[test]
    fn parse_llm_json_raw_or_any_with_repair_keeps_valid_json() {
        let raw = r#"{"decision":"direct_answer","confidence":0.9}"#;
        let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
            .expect("valid json should parse");
        assert_eq!(
            parsed
                .get("decision")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            "direct_answer"
        );
    }

    #[test]
    fn parse_llm_json_raw_or_any_with_repair_removes_stray_quote_after_bool() {
        let raw = r#"{"decision":"planner_execute","needs_clarify":false","confidence":0.9}"#;
        assert!(serde_json::from_str::<Value>(raw).is_err());
        let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
            .expect("stray quote after primitive should repair");
        assert_eq!(
            parsed.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            parsed.get("needs_clarify").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    /// §D1：dedupe_json_object_keys 幂等性。任意 JSON dedup 一次和二次结果必须一致。
    /// 防止未来引入「dedup 自身搬动了 key 顺序导致再 dedup 又改」这种回归。
    #[test]
    fn dedupe_json_object_keys_is_idempotent() {
        let corpus = [
            r#"{"a":1}"#,
            r#"{"a":1,"a":2}"#,
            r#"{"a":1,"a":2,"a":3,"a":4}"#,
            r#"{"a":{"b":1,"b":2},"a":{"b":3,"b":4}}"#,
            r#"[{"x":1,"x":2},{"x":3,"x":4}]"#,
            r#"{"decision":"planner_execute","execution_recipe":{"kind":"none","profile":"ops_service","target_scope":"system","target_scope":"system"}}"#,
            r#"{"a":[1,2,3],"a":[4,5,6]}"#,
            r#"{}"#,
            r#"[]"#,
            r#""hi""#,
            r#"42"#,
            r#"true"#,
            r#"null"#,
        ];
        for raw in corpus {
            let once =
                super::dedupe_json_object_keys(raw).expect("first dedup pass should succeed");
            let twice =
                super::dedupe_json_object_keys(&once).expect("second dedup pass should succeed");
            assert_eq!(
                once, twice,
                "dedupe_json_object_keys not idempotent on input {}",
                raw
            );
        }
    }

    /// §D1：N-fold 重复键 last-wins 规则覆盖。覆盖兼容模型偶发把同一字段
    /// 重复 2/3/5/10 次的全部观测形态。
    #[test]
    fn dedupe_json_object_keys_last_wins_for_n_fold_duplicates() {
        for n in [2usize, 3, 5, 10] {
            let mut payload = String::from("{");
            for i in 0..n {
                if i > 0 {
                    payload.push(',');
                }
                payload.push_str(&format!(r#""x":"v{}""#, i));
            }
            payload.push('}');
            let deduped = super::dedupe_json_object_keys(&payload)
                .expect("n-fold duplicate input should round-trip through Value");
            let parsed: Value =
                serde_json::from_str(&deduped).expect("dedup output should still parse as Value");
            assert_eq!(
                parsed.get("x").and_then(|v| v.as_str()),
                Some(format!("v{}", n - 1).as_str()),
                "expected last-wins for n={}, got: {}",
                n,
                deduped
            );
        }
    }

    /// §D1：minimax 实际观测的「病态 JSON 语料库」全部能跑通解析回路 —— 含
    /// duplicate keys / 嵌套 duplicate / 数组里的 object-with-duplicates / 数值与
    /// bool 重复 / null 与字符串混合重复。任何一条 panic 都视为回归。
    ///
    /// 这里**不**断言每一条都能 dedup 成功；只断言不 panic 且能 round-trip：
    /// `parse_llm_json_raw_or_any_with_repair::<Value>(...)` 拿到结果后再 to_string
    /// 然后再 dedup 仍然能 parse。
    #[test]
    fn parse_llm_json_raw_or_any_with_repair_survives_minimax_pathological_corpus() {
        let corpus = [
            // duplicate top-level keys
            r#"{"target_scope":"system","target_scope":"system"}"#,
            // duplicate top + duplicate nested
            r#"{"a":"x","a":"y","b":{"c":1,"c":2,"c":3}}"#,
            // duplicate inside array element
            r#"{"items":[{"k":1,"k":2},{"k":3,"k":4,"k":5}]}"#,
            // duplicate boolean / null mixed
            r#"{"flag":true,"flag":false,"missing":null,"missing":"present"}"#,
            // duplicate keys with mixed value types (str -> obj)
            r#"{"contract":"loose","contract":{"shape":"strict"}}"#,
            // realistic minimax normalizer payload: duplicate target_scope inside
            // execution_recipe nested in IntentNormalizerOut-style envelope.
            r#"{"resolved_user_intent":"check service","decision":"planner_execute","needs_clarify":false,"reason":"r","confidence":0.8,"execution_recipe":{"kind":"ops_closed_loop","profile":"ops_service","target_scope":"system","target_scope":"system"}}"#,
        ];
        for raw in corpus {
            let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
                .unwrap_or_else(|| panic!("failed to repair-and-parse: {}", raw));
            let reserialized =
                serde_json::to_string(&parsed).expect("repaired Value should re-serialize");
            let again = super::parse_llm_json_raw_or_any_with_repair::<Value>(&reserialized)
                .unwrap_or_else(|| panic!("re-parse of normalized form failed: {}", reserialized));
            assert!(
                again.is_object()
                    || again.is_array()
                    || again.is_string()
                    || again.is_number()
                    || again.is_boolean()
                    || again.is_null()
            );
        }
    }

    #[test]
    fn extract_agent_action_objects_recovers_inner_actions_from_malformed_wrapper() {
        let raw = r#"{"steps":[{"type":"call_skill","skill":"read_file","args":{"path":"README.md"}},{"type":"call_skill","skill":"system_basic","args":{"action":"info"}]}"#;
        let extracted = super::extract_agent_action_objects(raw);
        assert_eq!(extracted.len(), 2);
        let parsed: Value =
            serde_json::from_str(&extracted[0]).expect("first inner action should parse");
        assert_eq!(
            parsed.get("skill").and_then(|v| v.as_str()),
            Some("read_file")
        );
        let parsed_second: Value =
            serde_json::from_str(&extracted[1]).expect("second inner action should parse");
        assert_eq!(
            parsed_second.get("skill").and_then(|v| v.as_str()),
            Some("system_basic")
        );
    }

    #[test]
    fn normalize_agent_action_shape_rewrites_bare_run_cmd_aliases() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let normalized = super::parse_agent_action_json_with_repair(
            r#"{"type":"run_cmd","cmd":"pwd","workdir":"/tmp","timeout_ms":2500}"#,
            &state,
        )
        .expect("bare run_cmd should normalize");
        assert_eq!(
            normalized,
            json!({
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "pwd",
                    "cwd": "/tmp",
                    "timeout_seconds": 3
                }
            })
        );
    }

    #[test]
    fn normalize_agent_action_shape_preserves_bare_run_cmd_args_object() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let normalized = super::parse_agent_action_json_with_repair(
            r#"{"type":"run_cmd","args":{"command":"git branch --show-current","cwd":"/tmp/repo"}}"#,
            &state,
        )
        .expect("bare run_cmd args object should normalize");
        assert_eq!(
            normalized,
            json!({
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "git branch --show-current",
                    "cwd": "/tmp/repo"
                }
            })
        );
    }

    #[test]
    fn normalize_agent_action_shape_preserves_internal_run_cmd_metadata() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let normalized = super::parse_agent_action_json_with_repair(
            r#"{"type":"call_skill","skill":"run_cmd","args":{"command":"bash /tmp/check.sh","cwd":"/tmp","_clawd_validation":{"profile":"code_change","validator_type":"runtime_probe","validated_target":"/tmp/check.sh"}}}"#,
            &state,
        )
        .expect("run_cmd should normalize");
        assert_eq!(
            normalized,
            json!({
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "bash /tmp/check.sh",
                    "cwd": "/tmp",
                    "_clawd_validation": {
                        "profile": "code_change",
                        "validator_type": "runtime_probe",
                        "validated_target": "/tmp/check.sh"
                    }
                }
            })
        );
    }

    #[test]
    fn normalize_agent_action_shape_preserves_top_level_internal_run_cmd_metadata() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let normalized = super::parse_agent_action_json_with_repair(
            r#"{"type":"run_cmd","cmd":"pwd","_clawd_continue_on_error":true}"#,
            &state,
        )
        .expect("bare run_cmd should normalize");
        assert_eq!(
            normalized,
            json!({
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "pwd",
                    "_clawd_continue_on_error": true
                }
            })
        );
    }

    #[test]
    fn normalize_agent_action_shape_rewrites_action_run_cmd_alias() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let normalized = super::parse_agent_action_json_with_repair(
            r#"{"action":"run_cmd","cmd":"pwd","workdir":"/tmp"}"#,
            &state,
        )
        .expect("action run_cmd should normalize");
        assert_eq!(
            normalized,
            json!({
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "pwd",
                    "cwd": "/tmp"
                }
            })
        );
    }

    #[test]
    fn normalize_agent_action_shape_rewrites_action_builtin_skill_alias() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let normalized = super::parse_agent_action_json_with_repair(
            r#"{"action":"list_dir","path":"logs","limit":2}"#,
            &state,
        )
        .expect("action builtin skill should normalize");
        assert_eq!(
            normalized,
            json!({
                "type": "call_skill",
                "skill": "list_dir",
                "args": {
                    "path": "logs",
                    "limit": 2
                }
            })
        );
    }

    #[test]
    fn normalize_agent_action_shape_rewrites_system_basic_run_cmd_to_run_cmd_skill() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let normalized = super::parse_agent_action_json_with_repair(
            r#"{"type":"call_skill","skill":"system_basic","args":{"action":"run_cmd","command":"git branch --show-current","description":"获取当前git分支名称"}}"#,
            &state,
        )
        .expect("system_basic run_cmd should normalize");
        assert_eq!(
            normalized,
            json!({
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "git branch --show-current"
                }
            })
        );
    }

    #[test]
    fn normalize_agent_action_shape_rewrites_call_tool_run_cmd_aliases() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let normalized = super::parse_agent_action_json_with_repair(
            r#"{"type":"call_tool","tool":"run_cmd","args":{"cmd":"whoami","timeout_ms":1}}"#,
            &state,
        )
        .expect("call_tool run_cmd should normalize");
        assert_eq!(
            normalized,
            json!({
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "whoami",
                    "timeout_seconds": 1
                }
            })
        );
    }

    #[test]
    fn normalize_agent_action_shape_rewrites_system_basic_list_dir_to_base_skill() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let normalized = super::parse_agent_action_json_with_repair(
            r#"{"type":"call_skill","skill":"system_basic","args":{"action":"list_dir","path":"scripts","names_only":true}}"#,
            &state,
        )
        .expect("system_basic list_dir should normalize");
        assert_eq!(
            normalized,
            json!({
                "type": "call_skill",
                "skill": "list_dir",
                "args": {
                    "path": "scripts",
                    "names_only": true
                }
            })
        );
    }

    #[test]
    fn normalize_agent_action_shape_rewrites_rich_system_basic_list_dir_to_inventory_dir() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let normalized = super::parse_agent_action_json_with_repair(
            r#"{"type":"call_skill","skill":"system_basic","args":{"action":"list_dir","path":"logs","sort_by":"mtime","limit":2,"names_only":true,"options":{"show_timestamps":true}}}"#,
            &state,
        )
        .expect("rich system_basic list_dir should normalize");
        assert_eq!(
            normalized,
            json!({
                "type": "call_skill",
                "skill": "system_basic",
                "args": {
                    "action": "inventory_dir",
                    "path": "logs",
                    "sort_by": "mtime_desc",
                    "max_entries": 2,
                    "names_only": true
                }
            })
        );
    }
}

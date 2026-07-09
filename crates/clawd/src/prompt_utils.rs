use std::sync::OnceLock;

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::AppState;

#[cfg(test)]
#[path = "prompt_utils_contract_repair_judge.rs"]
mod contract_repair_judge;
#[cfg(test)]
#[path = "prompt_utils_output_contract.rs"]
mod output_contract;
#[path = "prompt_utils_schema.rs"]
mod schema_validation;
#[cfg(test)]
use contract_repair_judge::canonicalize_contract_repair_judge_object;
#[cfg(test)]
use output_contract::{
    canonicalize_output_contract, normalize_output_contract_delivery_intent,
    normalize_output_contract_locator_kind, normalize_output_contract_semantic_kind,
    normalize_schema_token_for_contract,
};
use schema_validation::validate_schema_value;

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
    #[cfg(test)]
    ContractRepairJudge,
    AnswerVerifier,
    UserResponseContractValidator,
    PlanResult,
    FinalizerOut,
    DeliveryTextClassifier,
    ScheduleIntent,
    LongTermSummary,
    MemoryIntent,
    RunCmdSuggestion,
}

impl PromptSchemaId {
    fn as_str(self) -> &'static str {
        match self {
            Self::IntentNormalizer => "intent_normalizer",
            #[cfg(test)]
            Self::ContractRepairJudge => "contract_repair_judge",
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
        #[cfg(test)]
        static CONTRACT_REPAIR_JUDGE: OnceLock<Value> = OnceLock::new();
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
            #[cfg(test)]
            Self::ContractRepairJudge => CONTRACT_REPAIR_JUDGE.get_or_init(|| {
                parse_schema(include_str!(
                    "../../../prompts/schemas/contract_repair_judge.schema.json"
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
        "respond" => &[
            "type",
            "content",
            "terminal_intent",
            "clarify_reason_code",
            "missing_slot",
            "message_key",
            "field_path",
            "locator_kind",
        ],
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

fn canonicalize_schedule_intent_schema_object(
    mut map: serde_json::Map<String, Value>,
) -> (Value, bool) {
    let mut normalized = false;
    if let Some(Value::Object(mut intent)) = map.remove("schedule_intent") {
        for (outer_key, inner_key) in [
            ("schedule_kind", "kind"),
            ("timezone", "timezone"),
            ("raw", "raw"),
            ("reason", "reason"),
            ("needs_clarify", "needs_clarify"),
            ("clarify_question", "clarify_question"),
            ("confidence", "confidence"),
        ] {
            if !intent.contains_key(inner_key) {
                if let Some(value) = map.remove(outer_key) {
                    intent.insert(inner_key.to_string(), value);
                }
            }
        }
        if !intent.contains_key("raw") {
            if let Some(value) = map.remove("resolved_user_intent") {
                intent.insert("raw".to_string(), value);
            }
        }
        map = intent;
        normalized = true;
    }
    canonicalize_schedule_intent_fields(map, normalized)
}

fn canonicalize_schedule_intent_fields(
    mut map: serde_json::Map<String, Value>,
    mut normalized: bool,
) -> (Value, bool) {
    for field in [
        "kind",
        "timezone",
        "target_job_id",
        "raw",
        "mode",
        "reason",
        "clarify_question",
    ] {
        let default = if field == "mode" { "execute" } else { "" };
        normalized |= canonicalize_string_field(&mut map, field, default);
    }
    if !map.contains_key("needs_clarify") {
        map.insert("needs_clarify".to_string(), Value::Bool(false));
        normalized = true;
    }
    if !map.get("needs_clarify").is_some_and(Value::is_boolean) {
        let value = map
            .get("needs_clarify")
            .and_then(Value::as_str)
            .map(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "yes"))
            .unwrap_or(false);
        map.insert("needs_clarify".to_string(), Value::Bool(value));
        normalized = true;
    }
    normalized |= canonicalize_number_field(&mut map, "confidence", 0.0);
    normalized |= canonicalize_schedule_value(&mut map);
    normalized |= canonicalize_schedule_task_value(&mut map);
    (Value::Object(map), normalized)
}

fn canonicalize_string_field(
    map: &mut serde_json::Map<String, Value>,
    field: &str,
    default: &str,
) -> bool {
    match map.get_mut(field) {
        Some(Value::String(_)) => false,
        Some(Value::Null) | None => {
            map.insert(field.to_string(), Value::String(default.to_string()));
            true
        }
        Some(slot) => {
            *slot = Value::String(schema_scalar_text(slot));
            true
        }
    }
}

fn canonicalize_number_field(
    map: &mut serde_json::Map<String, Value>,
    field: &str,
    default: f64,
) -> bool {
    let value = map.get(field).and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
    });
    if let Some(value) = value.filter(|value| value.is_finite()) {
        map.insert(field.to_string(), Value::from(value.clamp(0.0, 1.0)));
        return true;
    }
    if !map.contains_key(field) {
        map.insert(field.to_string(), Value::from(default));
        return true;
    }
    false
}

fn canonicalize_schedule_value(map: &mut serde_json::Map<String, Value>) -> bool {
    let mut normalized = false;
    let schedule = map
        .entry("schedule".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Value::String(raw) = schedule {
        *schedule = serde_json::from_str::<Value>(raw)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        normalized = true;
    }
    if !schedule.is_object() {
        *schedule = Value::Object(serde_json::Map::new());
        normalized = true;
    }
    let Some(schedule) = schedule.as_object_mut() else {
        return normalized;
    };
    for field in ["type", "run_at", "time", "cron"] {
        normalized |= canonicalize_string_field(schedule, field, "");
    }
    for field in ["weekday", "every_minutes"] {
        let value = schedule
            .get(field)
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
            })
            .unwrap_or(0);
        schedule.insert(field.to_string(), Value::from(value));
    }
    normalized
}

fn canonicalize_schedule_task_value(map: &mut serde_json::Map<String, Value>) -> bool {
    let message = map
        .remove("message")
        .map(|value| schema_scalar_text(&value));
    let mut normalized = message.is_some();
    let task = map
        .entry("task".to_string())
        .or_insert_with(|| schedule_task_value_from_message(message.as_deref().unwrap_or("")));
    if let Value::String(raw) = task {
        *task = schedule_task_value_from_message(raw);
        return true;
    }
    if !task.is_object() {
        *task = schedule_task_value_from_message(message.as_deref().unwrap_or(""));
        return true;
    }
    let Some(task) = task.as_object_mut() else {
        return normalized;
    };
    let has_payload = task.get("payload").is_some();
    if !task.get("kind").is_some_and(Value::is_string) {
        task.insert(
            "kind".to_string(),
            Value::String(if has_payload || message.is_some() {
                "ask".to_string()
            } else {
                String::new()
            }),
        );
        normalized = true;
    }
    match task.get_mut("payload") {
        Some(Value::Object(_)) => {}
        Some(Value::String(raw)) => {
            let mut payload = serde_json::Map::new();
            payload.insert("message".to_string(), Value::String(raw.trim().to_string()));
            task.insert("payload".to_string(), Value::Object(payload));
            normalized = true;
        }
        Some(Value::Null) | None => {
            let mut payload = serde_json::Map::new();
            if let Some(message) = message.as_deref().filter(|value| !value.is_empty()) {
                payload.insert("message".to_string(), Value::String(message.to_string()));
            }
            task.insert("payload".to_string(), Value::Object(payload));
            normalized = true;
        }
        Some(slot) => {
            let mut payload = serde_json::Map::new();
            payload.insert(
                "message".to_string(),
                Value::String(schema_scalar_text(slot)),
            );
            task.insert("payload".to_string(), Value::Object(payload));
            normalized = true;
        }
    }
    normalized
}

fn schedule_task_value_from_message(message: &str) -> Value {
    let mut payload = serde_json::Map::new();
    if !message.trim().is_empty() {
        payload.insert(
            "message".to_string(),
            Value::String(message.trim().to_string()),
        );
    }
    let mut task = serde_json::Map::new();
    task.insert("kind".to_string(), Value::String("ask".to_string()));
    task.insert("payload".to_string(), Value::Object(payload));
    Value::Object(task)
}

fn schema_scalar_text(value: &Value) -> String {
    value
        .as_str()
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| value.to_string())
}

fn canonicalize_schema_input(schema_id: PromptSchemaId, value: Value) -> (Value, bool) {
    match (schema_id, value) {
        (PromptSchemaId::IntentNormalizer, Value::Object(mut map)) => {
            let mut normalized = false;
            if map.remove("decision").is_some() {
                normalized = true;
            }
            let allowed_top_level_keys = [
                "boundary_envelope",
                "resolved_user_intent",
                "resume_behavior",
                "schedule_kind",
                "wants_file_delivery",
                "should_refresh_long_term_memory",
                "agent_display_name_hint",
                "needs_clarify",
                "clarify_question",
                "reason",
                "confidence",
                "schedule_intent",
                "output_contract",
                "execution_recipe",
                "turn_type",
                "target_task_policy",
                "should_interrupt_active_run",
                "state_patch",
                "attachment_processing_required",
            ];
            for key in map.keys().cloned().collect::<Vec<_>>() {
                if !allowed_top_level_keys.contains(&key.as_str()) {
                    map.remove(&key);
                    normalized = true;
                }
            }
            if !matches!(map.get("boundary_envelope"), Some(Value::Object(_))) {
                map.insert(
                    "boundary_envelope".to_string(),
                    json!({
                        "schema_version": crate::intent_router::BOUNDARY_ENVELOPE_SCHEMA_VERSION,
                        "raw_chars": 0,
                        "language_hint": null,
                        "schedule_intent": null,
                        "attachment_refs": [],
                        "explicit_locators": [],
                        "active_task_reference": null,
                        "session_binding": null,
                        "safety_budget_hint": null,
                    }),
                );
                normalized = true;
            }
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
        (PromptSchemaId::ScheduleIntent, Value::Object(map)) => {
            canonicalize_schedule_intent_schema_object(map)
        }
        #[cfg(test)]
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
            if normalized_tool == "run_cmd" {
                return normalize_run_cmd_call(obj, args.as_object());
            }
            return json!({
                "type": "call_tool",
                "tool": normalized_tool,
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
                    "type": "call_tool",
                    "tool": normalized_tool,
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
    preserve_run_cmd_execution_args(&mut args, raw_args);
    preserve_run_cmd_execution_args(&mut args, Some(obj));
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
    complete_run_cmd_async_start_contract(&mut args);
    json!({
        "type": "call_skill",
        "skill": "run_cmd",
        "args": Value::Object(args),
    })
}

fn preserve_run_cmd_execution_args(
    args: &mut serde_json::Map<String, Value>,
    source: Option<&serde_json::Map<String, Value>>,
) {
    let Some(source) = source else {
        return;
    };
    for key in [
        "idle_timeout_seconds",
        "max_output_bytes",
        "async_start",
        "poll_after_seconds",
        "expires_in_seconds",
    ] {
        if let Some(value) = source.get(key) {
            args.entry(key.to_string()).or_insert_with(|| value.clone());
        }
    }
}

fn complete_run_cmd_async_start_contract(args: &mut serde_json::Map<String, Value>) {
    if args.get("async_start").and_then(Value::as_bool) != Some(true) {
        return;
    }
    args.entry("poll_after_seconds".to_string())
        .or_insert_with(|| Value::from(2));
    args.entry("expires_in_seconds".to_string())
        .or_insert_with(|| Value::from(600));
    args.entry(crate::agent_engine::CLAWD_RUNTIME_ASYNC_JOB_START_ARG.to_string())
        .or_insert_with(|| Value::String("async_job_protocol".to_string()));
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

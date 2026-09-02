use serde_json::Value;

use super::*;

pub(super) async fn execute_schedule_workflow_for_task(
    state: &AppState,
    task: &ClaimedTask,
    map: &serde_json::Map<String, Value>,
    args: &Value,
    action: &str,
) -> Result<String, String> {
    ensure_only_keys(
        map,
        &[
            "action",
            "text",
            "raw",
            "intent",
            "intent_json",
            "kind",
            "timezone",
            "schedule",
            "task",
            "target_job_id",
            "match_task_kind",
            "match_skill_name",
            "match_task_action",
            "match_platforms",
            "mode",
            "dry_run",
            "preview_only",
            "create_real",
            "reason",
            "needs_clarify",
            "clarify_question",
            "confidence",
        ],
    )?;
    if action == "delete_matching" {
        return crate::schedule_service::delete_matching_skill_schedules(state, task, args);
    }
    let prompt = schedule_workflow_prompt_for_task(task, map, args, action);
    let mut intent = explicit_schedule_intent_from_args(args, action, &prompt)?;
    if intent.is_none() {
        intent = crate::schedule_service::parse_schedule_intent(state, task, &prompt).await;
    }
    if let Some(intent) = intent.as_mut() {
        normalize_schedule_workflow_intent(intent, action, &prompt);
    }
    let intent =
        intent.ok_or_else(|| schedule_workflow_error("schedule_intent_not_detected", None))?;
    if intent.needs_clarify {
        return Err(schedule_replan_error(&intent));
    }
    Box::pin(crate::schedule_service::try_handle_schedule_request(
        state,
        task,
        &prompt,
        Some(&intent),
    ))
    .await?
    .ok_or_else(|| schedule_workflow_error("schedule_intent_not_detected", None))
}

pub(super) fn schedule_workflow_prompt_for_task(
    task: &ClaimedTask,
    map: &serde_json::Map<String, Value>,
    args: &Value,
    action: &str,
) -> String {
    if matches!(action, "preview" | "dry_run" | "create") {
        if let Some(original) = crate::language_policy::task_original_user_text(task) {
            return original;
        }
    }
    schedule_workflow_prompt(map, args)
}

pub(super) fn schedule_workflow_prompt(
    map: &serde_json::Map<String, Value>,
    args: &Value,
) -> String {
    optional_string(map, "text")
        .or_else(|| optional_string(map, "raw"))
        .or_else(|| args.get("intent").and_then(Value::as_str))
        .or_else(|| {
            args.get("intent")
                .and_then(|value| value.get("raw"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("schedule workflow request")
        .to_string()
}

pub(super) fn explicit_schedule_intent_from_args(
    args: &Value,
    action: &str,
    prompt: &str,
) -> Result<Option<crate::ScheduleIntentOutput>, String> {
    if let Some(intent_json) = args.get("intent_json").and_then(Value::as_str) {
        return serde_json::from_str::<crate::ScheduleIntentOutput>(intent_json)
            .map(Some)
            .map_err(|err| {
                schedule_workflow_error(
                    "invalid_schedule_intent",
                    Some(serde_json::json!({ "detail": err.to_string() })),
                )
            });
    }
    if let Some(intent) = args.get("intent").filter(|value| value.is_object()) {
        return serde_json::from_value::<crate::ScheduleIntentOutput>(intent.clone())
            .map(Some)
            .map_err(|err| {
                schedule_workflow_error(
                    "invalid_schedule_intent",
                    Some(serde_json::json!({ "detail": err.to_string() })),
                )
            });
    }
    if !schedule_args_contain_structured_intent(args) && action != "list" {
        return Ok(None);
    }
    let mut obj = serde_json::Map::new();
    for key in [
        "kind",
        "timezone",
        "schedule",
        "task",
        "target_job_id",
        "raw",
        "mode",
        "dry_run",
        "preview_only",
        "create_real",
        "reason",
        "needs_clarify",
        "clarify_question",
        "confidence",
    ] {
        if let Some(value) = args.get(key) {
            obj.insert(key.to_string(), value.clone());
        }
    }
    obj.entry("kind".to_string())
        .or_insert_with(|| Value::String(schedule_kind_for_action(action).to_string()));
    if !prompt.trim().is_empty() {
        obj.entry("raw".to_string())
            .or_insert_with(|| Value::String(prompt.trim().to_string()));
    }
    serde_json::from_value::<crate::ScheduleIntentOutput>(Value::Object(obj))
        .map(Some)
        .map_err(|err| {
            schedule_workflow_error(
                "invalid_schedule_intent",
                Some(serde_json::json!({ "detail": err.to_string() })),
            )
        })
}

fn schedule_workflow_error(error_kind: &'static str, extra: Option<Value>) -> String {
    builtin_error("schedule", error_kind, error_kind, None, None, extra)
}

pub(super) fn schedule_replan_error(intent: &crate::ScheduleIntentOutput) -> String {
    let detail = [intent.clarify_question.trim(), intent.reason.trim()]
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or("schedule input is incomplete");
    builtin_error(
        "schedule",
        "schedule_needs_more_info",
        detail,
        None,
        None,
        Some(serde_json::json!({
            "retryable": true,
            "failure_phase": "pre_dispatch",
            "side_effect_applied": false,
            "recovery_action": "replan_arguments",
            "required_input": ["time", "content"],
        })),
    )
}

pub(super) fn schedule_args_contain_structured_intent(args: &Value) -> bool {
    ["intent_json", "kind", "schedule", "task", "target_job_id"]
        .iter()
        .any(|key| args.get(*key).is_some())
}

pub(super) fn schedule_kind_for_action(action: &str) -> &'static str {
    match action {
        "list" | "query" => "list",
        "delete" => "delete",
        "pause" => "pause",
        "resume" => "resume",
        "preview" | "dry_run" | "create" | "create_structured" => "create",
        _ => "",
    }
}

fn normalize_schedule_workflow_intent(
    intent: &mut crate::ScheduleIntentOutput,
    action: &str,
    prompt: &str,
) {
    if intent.kind.trim().is_empty() {
        intent.kind = schedule_kind_for_action(action).to_string();
    }
    if intent.raw.trim().is_empty() && !prompt.trim().is_empty() {
        intent.raw = prompt.trim().to_string();
    }
    if matches!(action, "preview" | "dry_run") {
        intent.mode = "compile_only".to_string();
        intent.dry_run = true;
        intent.preview_only = true;
        intent.create_real = Some(false);
        if intent.kind.trim().is_empty() {
            intent.kind = "create".to_string();
        }
    }
}

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
            "kind",
            "timezone",
            "schedule",
            "task",
            "target_job_id",
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
    let prompt = schedule_workflow_prompt(map, args);
    let mut intent = explicit_schedule_intent_from_args(args, action, &prompt)?;
    if intent.is_none() {
        intent = crate::schedule_service::parse_schedule_intent(state, task, &prompt).await;
    }
    if let Some(intent) = intent.as_mut() {
        normalize_schedule_workflow_intent(intent, action, &prompt);
    }
    let intent =
        intent.ok_or_else(|| schedule_workflow_error("schedule_intent_not_detected", None))?;
    Box::pin(crate::schedule_service::try_handle_schedule_request(
        state,
        task,
        &prompt,
        Some(&intent),
    ))
    .await?
    .ok_or_else(|| schedule_workflow_error("schedule_intent_not_detected", None))
}

fn schedule_workflow_prompt(map: &serde_json::Map<String, Value>, args: &Value) -> String {
    optional_string(map, "text")
        .or_else(|| optional_string(map, "raw"))
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

fn explicit_schedule_intent_from_args(
    args: &Value,
    action: &str,
    prompt: &str,
) -> Result<Option<crate::ScheduleIntentOutput>, String> {
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
    if !schedule_args_contain_structured_intent(args) {
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

fn schedule_args_contain_structured_intent(args: &Value) -> bool {
    [
        "kind",
        "timezone",
        "schedule",
        "task",
        "target_job_id",
        "mode",
        "dry_run",
        "preview_only",
        "create_real",
    ]
    .iter()
    .any(|key| args.get(*key).is_some())
}

fn schedule_kind_for_action(action: &str) -> &'static str {
    match action {
        "list" | "query" => "list",
        "delete" => "delete",
        "pause" => "pause",
        "resume" => "resume",
        "preview" | "dry_run" | "create" => "create",
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

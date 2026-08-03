use super::*;

pub(crate) async fn run_pinned_async_poll_skill_with_runner(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: Value,
    execution_binding: &Value,
) -> Result<Value, String> {
    let adapter_id = state.resolve_canonical_skill_name(skill_name);
    let pinned_adapter_id = execution_binding
        .get("skill_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "async poll execution binding skill is missing".to_string())?;
    if adapter_id != pinned_adapter_id {
        return Err("async poll execution binding skill mismatch".to_string());
    }
    if args.get("action").and_then(Value::as_str) != Some("poll")
        || args
            .get("job_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_none_or(str::is_empty)
    {
        return Err("async poll runner arguments are invalid".to_string());
    }
    if state.skill_kind_for_dispatch(&adapter_id) == SkillKind::Builtin {
        return Err("async poll runner cannot dispatch a builtin skill".to_string());
    }

    let timeout = resolve_skill_timeout(state, &adapter_id, &args);
    let serialization_key = skill_dispatch_serialization_key(state, &adapter_id, &args);
    let _dispatch_permits = acquire_skill_dispatch_permits_with_serialization(
        &state.skill_rt.skill_concurrency_gates,
        &state.skill_rt.skill_semaphore,
        &task.task_id,
        &adapter_id,
        state.skill_max_concurrency_for_dispatch(&adapter_id),
        serialization_key.as_deref(),
    )
    .await?;
    let source = match task_runtime_channel(state, task) {
        RuntimeChannel::Whatsapp => "whatsapp",
        RuntimeChannel::Telegram => "telegram",
        RuntimeChannel::Wechat => "wechat",
        RuntimeChannel::Feishu => "feishu",
        RuntimeChannel::Lark => "lark",
    };
    let value = runner::run_skill_with_runner_once_pinned(
        state,
        task,
        &adapter_id,
        &args,
        source,
        timeout.seconds,
        None,
        None,
        Some(execution_binding),
    )
    .await?;
    if value.get("status").and_then(Value::as_str) != Some("ok") {
        return Err(structured_skill_error_string(&adapter_id, &value));
    }
    Ok(value)
}

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

    let resource_request = state.skill_resource_request_for_dispatch(&adapter_id, Some("poll"));
    let resource_estimate_key = super::resource_estimate_key(state, &adapter_id, Some("poll"));
    let resource_lease = state
        .skill_rt
        .skill_concurrency_gates
        .resource_broker()
        .try_acquire_background_for_estimate_key(
            resource_request.as_ref(),
            state
                .skill_max_concurrency_for_dispatch(&adapter_id)
                .unwrap_or(state.skill_rt.skill_global_max_concurrency),
            Some(&resource_estimate_key),
        )
        .map_err(|grant| {
            structured_skill_error_from_parts(
                &adapter_id,
                "resource_admission_unavailable",
                "resource_admission_unavailable",
                Some(std::env::consts::OS),
                Some(json!({
                    "message_key": "clawd.execution.resource_admission_unavailable",
                    "retryable": true,
                    "wait_reason": grant.wait_reason,
                    "resource_grant": grant.projection,
                })),
            )
        })?;
    let resource_grant = resource_lease.grant().clone();

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
        Some(&resource_grant.projection),
        None,
        None,
        Some(execution_binding),
        Some(&resource_estimate_key),
    )
    .await?;
    if value.get("status").and_then(Value::as_str) != Some("ok") {
        return Err(structured_skill_error_string(&adapter_id, &value));
    }
    Ok(value)
}

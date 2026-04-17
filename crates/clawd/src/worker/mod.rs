use std::time::Duration;

use crate::{repo, AppState};
use anyhow::anyhow;
use serde_json::Value;
use tracing::{debug, error, info, info_span, warn, Instrument};

mod ask_finalize;
mod ask_pipeline;
mod ask_prepare;
mod channels;
mod locator;
mod run_skill_finalize;
mod runtime_support;

pub(super) use ask_finalize::{
    finalize_ask_direct_success, finalize_ask_result, run_classifier_direct_reply,
    try_finalize_schedule_direct_success,
};
use ask_prepare::{maybe_finalize_schedule_direct_text_success, prepare_run_skill_input};
use ask_prepare::{prepare_ask_execution_context, prepare_ask_input, prepare_ask_routing};
pub(crate) use channels::{
    runtime_channel_from_payload, send_task_channel_message, task_external_chat_id,
    task_payload_value, task_runtime_channel,
};
pub(super) use locator::{
    has_concrete_locator_hint, has_explicit_path_or_url_locator_hint,
    try_resolve_implicit_locator_path, LocatorAutoResolution,
};
pub(super) use run_skill_finalize::finalize_run_skill_result;
use runtime_support::spawn_long_term_summary_refresh;
pub(crate) use runtime_support::{
    maybe_recover_stale_running_tasks_runtime, recover_stale_running_tasks_on_startup,
    spawn_cleanup_worker, spawn_schedule_worker, spawn_worker, start_task_heartbeat,
};

pub(crate) fn is_resume_continue_source(raw: &str) -> bool {
    let source = raw.trim().to_ascii_lowercase();
    crate::RESUME_CONTINUE_SOURCES
        .iter()
        .any(|value| *value == source)
}

pub(crate) async fn worker_once(state: &AppState) -> anyhow::Result<()> {
    maybe_recover_stale_running_tasks_runtime(state)?;

    let Some(task) = repo::claim_next_task(state)? else {
        debug!("worker_once: no queued tasks, idle tick");
        return Ok(());
    };

    let call_id = task.task_id.clone();
    let call_span = info_span!(
        "task_call",
        call_id = %call_id,
        task_id = %task.task_id,
        user_id = task.user_id,
        chat_id = task.chat_id,
        kind = %task.kind,
        channel = %task.channel
    );
    async {
        info!(
            "worker_once: picked task_id={} user_id={} chat_id={} kind={}",
            task.task_id, task.user_id, task.chat_id, task.kind
        );
        info!("{}", crate::LOG_CALL_WRAP);
        info!(
            "task_call_begin call_id={} task_id={} kind={} user_id={} chat_id={}",
            call_id, task.task_id, task.kind, task.user_id, task.chat_id
        );
        info!("{}", crate::LOG_CALL_WRAP);

        let mut payload = serde_json::from_str::<Value>(&task.payload_json)
            .map_err(|err| anyhow!("invalid payload_json for task {}: {err}", task.task_id))?;

        let task_kind_for_timeout_log = task.kind.clone();
        let worker_timeout_secs = state.worker.worker_task_timeout_seconds.max(1);
        let heartbeat_stop = start_task_heartbeat(state.clone(), task.task_id.clone());
        let task_result = tokio::time::timeout(Duration::from_secs(worker_timeout_secs), async {
            process_claimed_task_by_kind(state, &task, &mut payload).await?;
            Ok::<(), anyhow::Error>(())
        })
        .await;
        let _ = heartbeat_stop.send(());

        match task_result {
            Ok(inner) => inner?,
            Err(_) => {
                finalize_worker_timeout(
                    state,
                    &task,
                    &payload,
                    worker_timeout_secs,
                    &task_kind_for_timeout_log,
                )
                .await?
            }
        }
        Ok(())
    }
    .instrument(call_span)
    .await
}

async fn process_claimed_task_by_kind(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &mut Value,
) -> anyhow::Result<()> {
    match task.kind.as_str() {
        "ask" => {
            process_ask_task(state, task, payload).await?;
        }
        "run_skill" => {
            process_run_skill_task(state, task, payload).await?;
        }
        other => {
            let err = format!("Unsupported task kind: {other}");
            error!(
                "worker_once: unsupported task kind for task_id={}: {}",
                task.task_id, other
            );
            repo::update_task_failure(state, &task.task_id, &err)?;
            info!("{}", crate::LOG_CALL_WRAP);
            info!(
                "task_call_end task_id={} kind={} status=failed error={}",
                task.task_id,
                other,
                crate::truncate_for_log(&err)
            );
            info!("{}", crate::LOG_CALL_WRAP);
        }
    }
    Ok(())
}

async fn finalize_worker_timeout(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    worker_timeout_secs: u64,
    task_kind_for_timeout_log: &str,
) -> anyhow::Result<()> {
    let timeout_err = format!(
        "worker timeout after {}s while processing kind={}",
        worker_timeout_secs, task_kind_for_timeout_log
    );
    error!(
        "worker_once timeout: task_id={} kind={} timeout_seconds={}",
        task.task_id, task_kind_for_timeout_log, worker_timeout_secs
    );
    crate::update_task_timeout(state, &task.task_id, &timeout_err)?;
    maybe_notify_schedule_result(state, task, payload, false, &timeout_err).await;
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind={} status=timeout error={}",
        task.task_id,
        task_kind_for_timeout_log,
        crate::truncate_for_log(&timeout_err)
    );
    info!("{}", crate::LOG_CALL_WRAP);
    Ok(())
}

pub(crate) async fn maybe_notify_schedule_result(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    success: bool,
    text: &str,
) {
    let is_scheduled = payload
        .get("schedule_triggered")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !is_scheduled {
        return;
    }
    let Some(job_id) = payload.get("schedule_job_id").and_then(|v| v.as_str()) else {
        return;
    };
    let prefix = if success {
        crate::i18n_t_with_default(
            state,
            "clawd.msg.schedule_run_success_prefix",
            "Scheduled job executed successfully",
        )
    } else {
        crate::i18n_t_with_default(
            state,
            "clawd.msg.schedule_run_failed_prefix",
            "Scheduled job execution failed",
        )
    };
    let job_id_label =
        crate::i18n_t_with_default(state, "clawd.msg.schedule_run_job_id_label", "Job ID");
    let status_block = format!("{prefix}\n{job_id_label}: {job_id}");
    let text_trimmed = text.trim();
    let message = if text_trimmed.is_empty() {
        status_block
    } else {
        format!("{text_trimmed}\n\n{status_block}")
    };
    let runtime_ch = runtime_channel_from_payload(state, payload);
    let channel_str = task.channel.trim();
    info!(
        "schedule notify push: task_id={} channel={} runtime_channel={:?}",
        task.task_id, channel_str, runtime_ch
    );
    match send_task_channel_message(state, task, payload, &message).await {
        Ok(()) => {
            info!(
                "schedule notify success: task_id={} channel={} runtime_channel={:?}",
                task.task_id, channel_str, runtime_ch
            );
        }
        Err(err) => {
            warn!(
                "schedule notify failed: task_id={} channel={} runtime_channel={:?} err={}",
                task.task_id, channel_str, runtime_ch, err
            );
        }
    }
}

pub(crate) async fn process_ask_task(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &mut Value,
) -> anyhow::Result<()> {
    let prepared_input = prepare_ask_input(state, task, payload).await;
    let prompt = prepared_input.prompt;
    let source = prepared_input.source;
    if maybe_finalize_schedule_direct_text_success(state, task, payload, &prompt).await? {
        return Ok(());
    }

    let prepared_flow =
        ask_pipeline::prepare_ask_flow(state, task, payload, &prompt, &source).await?;
    let agent_run_context = Some(crate::agent_engine::AgentRunContext {
        route_result: Some(prepared_flow.route_result.clone()),
        execution_recipe_hint: prepared_flow.execution_recipe_hint,
        context_bundle_summary: Some(prepared_flow.context_bundle_summary.clone()),
        auto_locator_path: prepared_flow.auto_locator_path.clone(),
        user_request: Some(prompt.clone()),
    });

    let Some(result) = ask_pipeline::execute_ask_dispatch(
        state,
        task,
        payload,
        &prompt,
        &prepared_flow.recent_execution_context,
        &prepared_flow.resolved_prompt_for_execution,
        &prepared_flow.prompt_with_memory_for_execution,
        &prepared_flow.chat_prompt_context,
        &prepared_flow.route_result,
        prepared_flow.agent_mode,
        &prepared_flow.clarify_reason,
        prepared_flow.clarify_reason_kind,
        &prepared_flow.fuzzy_locator_suggestions,
        &prepared_flow.ask_mode,
        prepared_flow.should_route_schedule_direct,
        agent_run_context,
    )
    .await?
    else {
        return Ok(());
    };

    finalize_ask_result(
        state,
        task,
        payload,
        &prompt,
        &prepared_flow.context_bundle_summary,
        &prepared_flow.resolved_prompt_for_execution,
        &prepared_flow.route_result,
        result,
    )
    .await
}

pub(crate) async fn process_run_skill_task(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
) -> anyhow::Result<()> {
    let prepared_input = prepare_run_skill_input(payload);

    info!(
        "worker_once: processing run_skill task_id={} user_id={} chat_id={} skill_name={} args={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        prepared_input.skill_name,
        crate::truncate_for_log(&prepared_input.args.to_string())
    );

    finalize_run_skill_result(
        state,
        task,
        payload,
        &prepared_input.skill_name,
        crate::run_skill_with_runner_outcome(
            state,
            task,
            &prepared_input.skill_name,
            prepared_input.args,
        )
        .await,
    )
    .await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn wechat_payload_shape_keeps_context_token_available() {
        let payload = json!({
            "channel": "wechat",
            "external_chat_id": "wx-user-1",
            "context_token": "ctx-123"
        });
        assert_eq!(
            payload.get("channel").and_then(|v| v.as_str()),
            Some("wechat")
        );
        assert_eq!(
            payload.get("context_token").and_then(|v| v.as_str()),
            Some("ctx-123")
        );
    }
}

use std::time::Duration;

use crate::{repo, AppState};
use anyhow::anyhow;
use serde_json::{json, Value};
use tracing::{debug, error, info, info_span, warn, Instrument};

mod ask_execution_context;
mod ask_input;
mod ask_planner_frontdoor;
mod ask_runtime;
mod async_poll_executor;
mod channels;
mod locator;
mod resume_replay_executor;
pub(crate) mod run_capability;
mod run_skill_finalize;
mod run_skill_permission;
mod runtime_support;

// Phase 3.3 Stage 2.2：ask_finalize.rs 已物理搬移到 `crate::finalize::task`，
// 调用面统一通过 `crate::finalize::*` facade 访问。
use ask_execution_context::prepare_ask_execution_context;
use ask_input::{
    maybe_finalize_schedule_direct_text_success, prepare_ask_input, prepare_run_skill_input,
};
use ask_planner_frontdoor::prepare_planner_owned_ask_routing;
pub(crate) use channels::{
    runtime_channel_from_payload, send_task_channel_message, task_external_chat_id,
    task_payload_value, task_runtime_channel,
};
pub(super) use locator::{has_concrete_locator_hint, has_explicit_path_or_url_locator_hint};
use run_skill_finalize::{finalize_run_skill_confirmation_required, finalize_run_skill_result};
pub(crate) use runtime_support::spawn_long_term_summary_refresh;
pub(crate) use runtime_support::{
    maybe_recover_stale_running_tasks_runtime, recover_stale_running_tasks_on_startup,
    spawn_cleanup_worker, spawn_schedule_worker, spawn_worker, start_task_heartbeat,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduleNotifyOutcome {
    pub(crate) job_id: String,
    pub(crate) channel: String,
    pub(crate) runtime_channel: String,
    pub(crate) task_success: bool,
    pub(crate) delivered: bool,
    pub(crate) error_text: Option<String>,
}

fn runtime_channel_label(channel: crate::RuntimeChannel) -> &'static str {
    match channel {
        crate::RuntimeChannel::Telegram => "telegram",
        crate::RuntimeChannel::Whatsapp => "whatsapp",
        crate::RuntimeChannel::Wechat => "wechat",
        crate::RuntimeChannel::Feishu => "feishu",
        crate::RuntimeChannel::Lark => "lark",
    }
}

pub(crate) fn schedule_notify_observation(outcome: &ScheduleNotifyOutcome) -> Value {
    let mut value = json!({
        "source": "schedule_notify",
        "execution_surface": "schedule_notify",
        "execution_surface_owner": "delivery_boundary",
        "job_id": outcome.job_id,
        "channel": outcome.channel,
        "runtime_channel": outcome.runtime_channel,
        "task_success": outcome.task_success,
        "status": if outcome.delivered { "ok" } else { "error" },
    });
    if !outcome.delivered {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("error_kind".to_string(), json!("channel_send_failed"));
            obj.insert(
                "failure_attribution".to_string(),
                json!(crate::evidence_policy::FailureAttribution::DeliveryError.as_str()),
            );
            if let Some(error_text) = outcome.error_text.as_deref() {
                obj.insert(
                    "error_text".to_string(),
                    json!(crate::truncate_for_log(error_text)),
                );
            }
        }
    }
    value
}

pub(crate) fn record_schedule_notify_outcome(
    journal: &mut crate::task_journal::TaskJournal,
    outcome: Option<ScheduleNotifyOutcome>,
) {
    if let Some(outcome) = outcome {
        journal.push_task_observation(schedule_notify_observation(&outcome));
    }
}

pub(crate) async fn worker_once(state: &AppState) -> anyhow::Result<()> {
    maybe_recover_stale_running_tasks_runtime(state).await?;

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
            "worker_once: worker_id={} picked task_id={} user_id={} chat_id={} kind={}",
            state.worker.worker_id, task.task_id, task.user_id, task.chat_id, task.kind
        );
        info!("{}", crate::LOG_CALL_WRAP);
        info!(
            "task_call_begin worker_id={} call_id={} task_id={} kind={} user_id={} chat_id={}",
            state.worker.worker_id, call_id, task.task_id, task.kind, task.user_id, task.chat_id
        );
        info!("{}", crate::LOG_CALL_WRAP);

        let mut payload = match serde_json::from_str::<Value>(&task.payload_json) {
            Ok(payload) => payload,
            Err(error) => {
                let error = anyhow!("invalid payload_json for task {}: {error}", task.task_id);
                finalize_worker_runtime_error(state, &task, None, &error)?;
                crate::task_event_transport::publish_task_status_projection(state, &task.task_id);
                return Ok(());
            }
        };

        let task_kind_for_timeout_log = task.kind.clone();
        let worker_timeout_secs = state.worker.worker_task_timeout_seconds.max(1);
        let _task_cancellation = state.worker.register_active_task(&task.task_id);
        let heartbeat_stop = start_task_heartbeat(state.clone(), task.task_id.clone());
        let task_result = tokio::time::timeout(Duration::from_secs(worker_timeout_secs), async {
            process_claimed_task_by_kind(state, &task, &mut payload).await?;
            Ok::<(), anyhow::Error>(())
        })
        .await;
        let _ = heartbeat_stop.send(());
        state.worker.unregister_active_task(&task.task_id);

        match task_result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                finalize_worker_runtime_error(state, &task, Some(&payload), &error)?;
            }
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
        crate::task_event_transport::publish_task_status_projection(state, &task.task_id);
        Ok(())
    }
    .instrument(call_span)
    .await
}

fn finalize_worker_runtime_error(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: Option<&Value>,
    error: &anyhow::Error,
) -> anyhow::Result<()> {
    let error_text = error.to_string();
    error!(
        "worker_once runtime error: worker_id={} task_id={} kind={} error={}",
        state.worker.worker_id,
        task.task_id,
        task.kind,
        crate::truncate_for_log(&error_text)
    );
    repo::update_task_failure(state, &task.task_id, &error_text)?;
    if payload.is_some_and(repo::child_tasks::is_child_subagent_payload) {
        repo::child_tasks::record_child_task_terminal_projection(
            state,
            &task.task_id,
            payload.expect("checked child payload"),
        )?;
    }
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind={} status=failed error={}",
        task.task_id,
        task.kind,
        crate::truncate_for_log(&error_text)
    );
    info!("{}", crate::LOG_CALL_WRAP);
    Ok(())
}

async fn process_claimed_task_by_kind(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &mut Value,
) -> anyhow::Result<()> {
    match task.kind.as_str() {
        "ask" => {
            process_ask_task(state, task, payload).await?;
            if repo::child_tasks::is_child_subagent_payload(payload) {
                repo::child_tasks::record_child_task_terminal_projection(
                    state,
                    &task.task_id,
                    payload,
                )?;
            }
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
    crate::task_event_transport::publish_persisted_task_events(state, &task.task_id);
    Ok(())
}

#[cfg(test)]
#[path = "worker_error_finalization_tests.rs"]
mod worker_error_finalization_tests;

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
        "worker_once timeout: worker_id={} task_id={} kind={} timeout_seconds={}",
        state.worker.worker_id, task.task_id, task_kind_for_timeout_log, worker_timeout_secs
    );
    let terminal_timeout = crate::update_task_timeout(state, &task.task_id, &timeout_err)?;
    if terminal_timeout {
        let _ = maybe_notify_schedule_result(state, task, payload, false, &timeout_err).await;
    }
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind={} status={} error={}",
        task.task_id,
        task_kind_for_timeout_log,
        if terminal_timeout {
            "timeout"
        } else {
            "checkpoint_preserved"
        },
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
) -> Option<ScheduleNotifyOutcome> {
    let is_scheduled = payload
        .get("schedule_triggered")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !is_scheduled {
        return None;
    }
    let Some(job_id) = payload.get("schedule_job_id").and_then(|v| v.as_str()) else {
        return None;
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
    let runtime_channel = runtime_channel_label(runtime_ch).to_string();
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
            record_schedule_run_history(
                state,
                task,
                payload,
                job_id,
                success,
                &json!({
                    "delivered": true,
                    "runtime_channel": runtime_channel.clone(),
                }),
            );
            Some(ScheduleNotifyOutcome {
                job_id: job_id.to_string(),
                channel: channel_str.to_string(),
                runtime_channel,
                task_success: success,
                delivered: true,
                error_text: None,
            })
        }
        Err(err) => {
            warn!(
                "schedule notify failed: task_id={} channel={} runtime_channel={:?} err={}",
                task.task_id, channel_str, runtime_ch, err
            );
            record_schedule_run_history(
                state,
                task,
                payload,
                job_id,
                success,
                &json!({
                    "delivered": false,
                    "runtime_channel": runtime_channel.clone(),
                    "error_code": "channel_send_failed",
                }),
            );
            Some(ScheduleNotifyOutcome {
                job_id: job_id.to_string(),
                channel: channel_str.to_string(),
                runtime_channel,
                task_success: success,
                delivered: false,
                error_text: Some(err),
            })
        }
    }
}

fn record_schedule_run_history(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    job_id: &str,
    success: bool,
    notification: &Value,
) {
    let terminal_status = if success { "succeeded" } else { "failed" };
    let terminal_result = crate::scheduled_run_contract::scheduled_run_terminal_result(
        success,
        payload,
        Some(notification),
    );
    if let Ok(db) = state.core.db.get() {
        if let Err(err) = crate::scheduled_run_contract::update_scheduled_run_terminal(
            &db,
            job_id,
            &task.task_id,
            terminal_status,
            &crate::now_ts(),
            &terminal_result,
        ) {
            warn!(
                "schedule run history update failed: task_id={} job_id={} err={}",
                task.task_id, job_id, err
            );
        }
    }
}

pub(crate) async fn process_ask_task(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &mut Value,
) -> anyhow::Result<()> {
    if run_capability::is_direct_capability_payload(payload) {
        return run_capability::process_run_capability_task(state, task, payload).await;
    }
    crate::log_ask_transition(
        state,
        &task.task_id,
        None,
        crate::AskState::Received,
        "ask_task_claimed",
        None,
    );
    let prepared_input = prepare_ask_input(state, task, payload).await;
    let prompt = prepared_input.prompt;
    let source = prepared_input.source;
    if maybe_finalize_schedule_direct_text_success(state, task, payload, &prompt).await? {
        return Ok(());
    }

    crate::log_ask_transition(
        state,
        &task.task_id,
        Some(crate::AskState::Received),
        crate::AskState::Routing,
        "prepare_ask_flow",
        None,
    );
    let prepared_flow =
        ask_runtime::prepare_ask_flow(state, task, payload, &prompt, &source).await?;
    let result = ask_runtime::execute_ask_dispatch(state, task, &prepared_flow).await?;

    crate::finalize::finalize_ask_result(
        state,
        task,
        payload,
        &prompt,
        &prepared_flow.context_bundle_summary,
        prepared_flow.memory_trace.as_ref(),
        &prepared_flow.resolved_prompt_for_execution,
        None,
        &[],
        None,
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
        crate::truncate_for_log(&crate::visible_text::sanitize_user_visible_text(
            &prepared_input.args.to_string()
        ))
    );

    let verification = run_skill_permission::verify_direct_run_skill(
        state,
        task,
        &prepared_input.skill_name,
        prepared_input.args.clone(),
    );
    if verification.needs_confirmation() {
        return finalize_run_skill_confirmation_required(
            state,
            task,
            payload,
            &prepared_input.skill_name,
            &verification,
        )
        .await;
    }
    let result = if verification.allowed() {
        crate::run_skill_with_runner_outcome(
            state,
            task,
            &prepared_input.skill_name,
            prepared_input.args,
        )
        .await
    } else {
        Err(verification.denial_error(&prepared_input.skill_name))
    };

    finalize_run_skill_result(
        state,
        task,
        payload,
        &prepared_input.skill_name,
        &verification,
        result,
    )
    .await
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

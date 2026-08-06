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
#[cfg(test)]
#[path = "child_approval_tests.rs"]
mod child_approval_tests;
mod child_task_execution_scope;
pub(crate) mod conversation_compaction;
mod locator;
mod resume_replay_executor;
pub(crate) mod run_capability;
mod run_skill_finalize;
mod run_skill_mutation;
mod run_skill_permission;
mod runtime_support;
mod workspace_instructions;

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
    pub(crate) accepted: bool,
    pub(crate) delivered: bool,
    pub(crate) delivery_status: String,
    pub(crate) delivery_id: Option<String>,
    pub(crate) diagnostic_id: Option<String>,
    pub(crate) error_code: Option<String>,
    pub(crate) message_key: Option<String>,
    pub(crate) retryable: bool,
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
    let pending = matches!(
        outcome.delivery_status.as_str(),
        "in_progress" | "query_required"
    );
    let mut value = json!({
        "source": "schedule_notify",
        "execution_surface": "schedule_notify",
        "execution_surface_owner": "delivery_boundary",
        "job_id": outcome.job_id,
        "channel": outcome.channel,
        "runtime_channel": outcome.runtime_channel,
        "task_success": outcome.task_success,
        "accepted": outcome.accepted,
        "delivered": outcome.delivered,
        "delivery_status": outcome.delivery_status,
        "status": if outcome.accepted { "ok" } else if pending { "pending" } else { "error" },
    });
    if let Some(obj) = value.as_object_mut() {
        if let Some(delivery_id) = outcome.delivery_id.as_deref() {
            obj.insert("delivery_id".to_string(), json!(delivery_id));
        }
        if let Some(diagnostic_id) = outcome.diagnostic_id.as_deref() {
            obj.insert("diagnostic_id".to_string(), json!(diagnostic_id));
        }
    }
    if !outcome.accepted {
        if let Some(obj) = value.as_object_mut() {
            let error_code = match outcome.delivery_status.as_str() {
                "in_progress" => Some("channel_delivery_in_progress"),
                "query_required" => Some("channel_delivery_receipt_query_required"),
                _ => outcome.error_code.as_deref(),
            }
            .unwrap_or("channel_send_failed");
            obj.insert("error_code".to_string(), json!(error_code));
            if let Some(message_key) = outcome.message_key.as_deref() {
                obj.insert("message_key".to_string(), json!(message_key));
            }
            obj.insert("retryable".to_string(), json!(outcome.retryable));
            if !pending {
                obj.insert(
                    "failure_attribution".to_string(),
                    json!(crate::evidence_policy::FailureAttribution::DeliveryError.as_str()),
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

        let _task_cancellation = state.worker.register_active_task(&task.task_id);
        let heartbeat_stop =
            start_task_heartbeat(state.clone(), task.task_id.clone(), task.claim_attempt);
        // A claimed task has no implicit global wall-clock deadline. Durable
        // jobs hand off through checkpoints, while explicit deadlines,
        // cancellation, adapter/tool timeouts and stale-lease recovery remain
        // independently enforceable.
        let runtime_deadline = child_runtime_deadline(&payload);
        let task_result = if let Some(deadline) = runtime_deadline {
            match tokio::time::timeout(
                std::time::Duration::from_millis(deadline.duration_ms),
                process_claimed_task_by_kind(state, &task, &mut payload),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    state.worker.cancel_active_task(&task.task_id);
                    let result = repo::update_task_runtime_timeout(
                        state,
                        &task.task_id,
                        task.claim_attempt,
                        deadline.duration_ms,
                        deadline.source,
                    );
                    if result.is_ok() && repo::child_tasks::is_child_subagent_payload(&payload) {
                        repo::child_tasks::record_child_task_terminal_projection(
                            state,
                            &task.task_id,
                            &payload,
                        )
                        .map(|_| ())
                    } else {
                        result
                    }
                }
            }
        } else {
            process_claimed_task_by_kind(state, &task, &mut payload).await
        };
        let _ = heartbeat_stop.send(());
        state.worker.unregister_active_task(&task.task_id);

        match task_result {
            Ok(()) => {}
            Err(error) => {
                if let Some(rejection) = error.downcast_ref::<repo::WorkerTaskWriteRejected>() {
                    warn!(
                        "worker_write_rejected status_code={} lease_lost={} operation={} task_id={} expected_claim_attempt={} task_status={} lease_owner={} active_claim_attempt={}",
                        rejection.status_code,
                        rejection.status_code == repo::WORKER_LEASE_LOST_STATUS_CODE,
                        rejection.operation,
                        rejection.task_id,
                        rejection.expected_claim_attempt,
                        rejection.task_status.as_deref().unwrap_or("missing"),
                        rejection.lease_owner.as_deref().unwrap_or("none"),
                        rejection
                            .active_claim_attempt
                            .map(|value| value.to_string())
                            .as_deref()
                            .unwrap_or("none")
                    );
                } else {
                    finalize_worker_runtime_error(state, &task, Some(&payload), &error)?;
                }
            }
        }
        crate::task_event_transport::publish_task_status_projection(state, &task.task_id);
        Ok(())
    }
    .instrument(call_span)
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChildRuntimeDeadline {
    duration_ms: u64,
    source: &'static str,
}

fn child_runtime_deadline(payload: &Value) -> Option<ChildRuntimeDeadline> {
    if payload.get("task_role").and_then(Value::as_str) != Some("subagent_child") {
        return None;
    }
    let contract = payload.get("child_task_contract")?;
    let schema_version = contract
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let budget = contract.get("budget")?;
    if let Some(duration_ms) = budget.get("runtime_deadline_ms").and_then(Value::as_u64) {
        return Some(ChildRuntimeDeadline {
            duration_ms: duration_ms.max(1_000),
            source: "explicit_runtime_deadline",
        });
    }
    (schema_version == 1)
        .then(|| budget.get("timeout_ms").and_then(Value::as_u64))
        .flatten()
        .map(|duration_ms| ChildRuntimeDeadline {
            duration_ms: duration_ms.max(1_000),
            source: "v1_budget_timeout_ms",
        })
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
    repo::update_task_failure(state, &task.task_id, task.claim_attempt, &error_text)?;
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
            let mut child_scope =
                child_task_execution_scope::ChildTaskExecutionScope::prepare(state, task, payload)?;
            let process_result = process_ask_task(child_scope.state(state), task, payload).await;
            if process_result.is_ok() && repo::child_tasks::is_child_subagent_payload(payload) {
                if child_requires_noninteractive_approval_failure(payload) {
                    let _ = repo::fail_noninteractive_child_approval(
                        state,
                        &task.task_id,
                        task.claim_attempt,
                    )?;
                }
                let mut parent_owned_patch = false;
                if let Some(projection) = child_scope.projection(state) {
                    parent_owned_patch = projection
                        .pointer("/patch_artifact/status")
                        .and_then(Value::as_str)
                        .is_some_and(|status| matches!(status, "ready" | "empty"));
                    repo::child_tasks::record_child_task_execution_scope(
                        state,
                        &task.task_id,
                        &projection,
                    )?;
                }
                repo::child_tasks::record_child_task_terminal_projection(
                    state,
                    &task.task_id,
                    payload,
                )?;
                if parent_owned_patch {
                    child_scope.retain_for_parent_decision();
                }
            }
            process_result?;
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
            repo::update_task_failure(state, &task.task_id, task.claim_attempt, &err)?;
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

fn child_requires_noninteractive_approval_failure(payload: &Value) -> bool {
    payload
        .pointer("/child_execution/interactive_approval_available")
        .and_then(Value::as_bool)
        == Some(false)
}

#[cfg(test)]
#[path = "worker_error_finalization_tests.rs"]
mod worker_error_finalization_tests;

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
    let delivery_result = match crate::delivery_service::build_scheduled_delivery_envelope(
        state, task, payload, &message,
    ) {
        Ok(envelope) => {
            crate::delivery_service::deliver_task_envelope(state, task, payload, &envelope).await
        }
        Err(err) => Err(err),
    };
    let result = match delivery_result {
        Ok(result) => result,
        Err(_err) => crate::delivery_service::ChannelDeliveryServiceResult {
            status: crate::delivery_service::ChannelDeliveryServiceStatus::Failed,
            receipt: None,
            error_code: Some("channel.delivery.internal".to_string()),
            message_key: Some("channel.error.delivery_failed".to_string()),
            retryable: false,
        },
    };
    let accepted = result.accepted();
    let delivered = result.delivered();
    let delivery_status = result.status_token().to_string();
    let delivery_id = result
        .receipt
        .as_ref()
        .map(|receipt| receipt.delivery_id.clone());
    let diagnostic_id = result
        .receipt
        .as_ref()
        .and_then(|receipt| receipt.diagnostic_id.clone());
    let error_code = match result.status {
        crate::delivery_service::ChannelDeliveryServiceStatus::InProgress => {
            Some("channel_delivery_in_progress".to_string())
        }
        crate::delivery_service::ChannelDeliveryServiceStatus::QueryRequired => {
            Some("channel_delivery_receipt_query_required".to_string())
        }
        crate::delivery_service::ChannelDeliveryServiceStatus::Failed => result
            .error_code
            .clone()
            .or_else(|| Some("channel_send_failed".to_string())),
        _ => None,
    };
    if accepted {
        info!(
            "schedule notify accepted: task_id={} channel={} runtime_channel={:?} delivery_status={}",
            task.task_id, channel_str, runtime_ch, delivery_status
        );
    } else {
        warn!(
            "schedule notify not accepted: task_id={} channel={} runtime_channel={:?} delivery_status={} error_code={} retryable={}",
            task.task_id,
            channel_str,
            runtime_ch,
            delivery_status,
            error_code.as_deref().unwrap_or("none"),
            result.retryable
        );
    }
    let mut notification = json!({
        "accepted": accepted,
        "delivered": delivered,
        "delivery_status": delivery_status,
        "runtime_channel": runtime_channel.clone(),
    });
    if let Some(obj) = notification.as_object_mut() {
        if let Some(delivery_id) = delivery_id.as_deref() {
            obj.insert("delivery_id".to_string(), json!(delivery_id));
        }
        if let Some(diagnostic_id) = diagnostic_id.as_deref() {
            obj.insert("diagnostic_id".to_string(), json!(diagnostic_id));
        }
        if let Some(error_code) = error_code.as_deref() {
            obj.insert("error_code".to_string(), json!(error_code));
        }
        if let Some(message_key) = result.message_key.as_deref() {
            obj.insert("message_key".to_string(), json!(message_key));
        }
        if !accepted {
            obj.insert("retryable".to_string(), json!(result.retryable));
        }
    }
    record_schedule_run_history(state, task, payload, job_id, success, &notification);
    Some(ScheduleNotifyOutcome {
        job_id: job_id.to_string(),
        channel: channel_str.to_string(),
        runtime_channel,
        task_success: success,
        accepted,
        delivered,
        delivery_status,
        delivery_id,
        diagnostic_id,
        error_code: result.error_code,
        message_key: result.message_key,
        retryable: result.retryable,
    })
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
    if conversation_compaction::is_conversation_compaction_payload(payload) {
        return conversation_compaction::process_conversation_compaction_task(state, task, payload)
            .await;
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
    let mutation_guard = if verification.allowed() {
        run_skill_mutation::prepare_direct_run_skill_mutation(
            state,
            task,
            &prepared_input.skill_name,
            &prepared_input.args,
        )
        .map_err(anyhow::Error::msg)?
    } else {
        run_skill_mutation::DirectRunSkillMutationGuard::NotRequired
    };
    if let run_skill_mutation::DirectRunSkillMutationGuard::ReconciliationRequired(record) =
        &mutation_guard
    {
        return run_skill_mutation::finalize_direct_run_skill_reconciliation(
            state,
            task,
            &prepared_input.skill_name,
            &record.action_ref,
            &record.fingerprint_hash,
        );
    }
    let mut result = if verification.allowed() {
        match &mutation_guard {
            run_skill_mutation::DirectRunSkillMutationGuard::ReplaySuppressed(record) => Ok(
                run_skill_mutation::replay_suppressed_run_skill_outcome(record),
            ),
            _ => {
                let execution_context = mutation_guard.execution_context();
                crate::skills::run_skill_with_runner_outcome_with_context(
                    state,
                    task,
                    &prepared_input.skill_name,
                    prepared_input.args.clone(),
                    execution_context.as_ref(),
                )
                .await
            }
        }
    } else {
        Err(verification.denial_error(&prepared_input.skill_name))
    };
    if !run_skill_mutation::persist_direct_run_skill_mutation_result(
        state,
        &mutation_guard,
        &result,
    ) {
        if let run_skill_mutation::DirectRunSkillMutationGuard::Acquired(lease) = &mutation_guard {
            match crate::agent_engine::mutation_ledger::reconcile_uncertain_mutation_from_registry(
                state,
                task,
                lease,
                &prepared_input.skill_name,
                &prepared_input.args,
            )
            .await
            {
                Ok(crate::agent_engine::mutation_ledger::AutomaticMutationReconciliation::Applied(record)) => {
                    result = Ok(run_skill_mutation::replay_suppressed_run_skill_outcome(&record));
                }
                Ok(crate::agent_engine::mutation_ledger::AutomaticMutationReconciliation::NotApplied(record)) => {
                    return run_skill_mutation::finalize_direct_run_skill_reconciliation(
                        state,
                        task,
                        &prepared_input.skill_name,
                        &record.action_ref,
                        &record.fingerprint_hash,
                    );
                }
                Ok(crate::agent_engine::mutation_ledger::AutomaticMutationReconciliation::StillUnknown(record)) => {
                    return run_skill_mutation::finalize_direct_run_skill_reconciliation(
                        state,
                        task,
                        &prepared_input.skill_name,
                        &record.action_ref,
                        &record.fingerprint_hash,
                    );
                }
                Ok(crate::agent_engine::mutation_ledger::AutomaticMutationReconciliation::NotDeclared)
                | Err(_) => {
                    return run_skill_mutation::finalize_direct_run_skill_reconciliation(
                        state,
                        task,
                        &prepared_input.skill_name,
                        &lease.record.action_ref,
                        &lease.record.fingerprint_hash,
                    );
                }
            }
        }
    }

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

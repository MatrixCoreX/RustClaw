use std::time::Duration;

use anyhow::anyhow;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{now_ts, now_ts_u64, repo, schedule_service, AppState, ScheduledJobDue};

const CHANNEL_TERMINAL_DELIVERY_LEASE_SECONDS: u64 = 300;
const CHANNEL_TERMINAL_DELIVERY_MAX_ATTEMPTS: u32 = 96;
const CHANNEL_TERMINAL_DELIVERY_MAX_RETRY_DELAY_SECONDS: u64 = 300;
const MAINTENANCE_DELETE_BATCH_SIZE: i64 = 512;

fn channel_terminal_delivery_retry_delay(base_seconds: u64, attempt_count: u32) -> u64 {
    let exponent = attempt_count.saturating_sub(1).min(6);
    base_seconds
        .max(1)
        .saturating_mul(1_u64 << exponent)
        .min(CHANNEL_TERMINAL_DELIVERY_MAX_RETRY_DELAY_SECONDS)
}

fn channel_terminal_delivery_retry_budget_exhausted(attempt_count: u32) -> bool {
    attempt_count > CHANNEL_TERMINAL_DELIVERY_MAX_ATTEMPTS
}

pub(crate) fn start_task_heartbeat(
    state: AppState,
    task_id: String,
    claim_attempt: i64,
) -> oneshot::Sender<()> {
    let interval_secs = state.worker.worker_task_heartbeat_seconds.max(5);
    let worker_id = state.worker.worker_id.clone();
    let monitor = super::worker_supervision::current_monitor();
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {
                    match task_heartbeat_tick(state.clone(), task_id.clone(), claim_attempt).await {
                        Ok(true) => {
                            if let Some(monitor) = &monitor { monitor.task_heartbeat(&task_id); }
                        }
                        Ok(false) => break,
                        Err(err) => warn!(
                            "task heartbeat update failed: worker_id={} task_id={} interval_secs={} err={}",
                            worker_id, task_id, interval_secs, err
                        ),
                    }
                }
                _ = &mut stop_rx => {
                    break;
                }
            }
        }
    });
    stop_tx
}

pub(super) fn execution_watchdog_graces(state: &AppState) -> (Duration, Duration) {
    let task = Duration::from_secs(
        state
            .worker
            .worker_task_heartbeat_seconds
            .max(5)
            .saturating_mul(4)
            .max(300),
    );
    // A valid resume lease must get a chance to renew before a tick is declared stale.
    let tick = Duration::from_secs(
        state
            .worker
            .worker_running_no_progress_timeout_seconds
            .max(60),
    )
    .max(task);
    (task, tick)
}

pub(crate) fn spawn_worker(state: AppState, poll_interval_ms: u64, concurrency: usize) {
    let worker_count = concurrency.max(1);
    info!(
        "spawn_worker: worker_id={} starting {} worker loop(s), poll_interval_ms={}",
        state.worker.worker_id,
        worker_count,
        poll_interval_ms.max(10)
    );
    let monitors = (0..worker_count)
        .map(|index| {
            crate::runtime::worker_health::WorkerMonitor::named(format!("task_worker:{index}"))
        })
        .collect::<Vec<_>>();
    let recovery =
        crate::runtime::worker_health::WorkerMonitor::named("stale_task_recovery".into());
    state
        .worker
        .supervision
        .configure(monitors.clone(), recovery.clone());
    let pulse = Duration::from_secs(state.worker.worker_task_heartbeat_seconds.max(5));
    let (task_grace, tick_grace) = execution_watchdog_graces(&state);
    for monitor in monitors {
        let state_cloned = state.clone();
        super::worker_supervision::spawn_managed_loop(
            monitor,
            pulse,
            task_grace,
            tick_grace,
            Duration::from_millis(poll_interval_ms.max(10)),
            move || {
                let state = state_cloned.clone();
                async move { super::super::worker_once(&state).await }
            },
        );
    }
    let recovery_state = state.clone();
    super::worker_supervision::spawn_managed_loop(
        recovery,
        pulse,
        task_grace,
        tick_grace,
        Duration::from_secs(
            state
                .worker
                .worker_running_recovery_check_interval_seconds
                .max(10),
        ),
        move || {
            let state = recovery_state.clone();
            async move { super::recovery_flow::maybe_recover_stale_running_tasks_only(&state).await }
        },
    );
}

pub(crate) fn spawn_cleanup_worker(state: AppState) {
    let idle = Duration::from_secs(state.policy.maintenance.cleanup_interval_seconds.max(30));
    let worker_state = state.clone();
    super::worker_supervision::spawn_background_loop(&state, "cleanup".into(), idle, move || {
        let state = worker_state.clone();
        async move {
            if state
                .skill_rt
                .skill_concurrency_gates
                .resource_broker()
                .pressure_state()
                == claw_core::host_resources::ResourcePressureState::Critical
            {
                state.clear_prompt_template_cache();
            } else {
                cleanup_tick(state).await?;
            }
            Ok(false)
        }
    });
}

pub(crate) fn spawn_schedule_worker(state: AppState) {
    let worker_state = state.clone();
    super::worker_supervision::spawn_background_loop(
        &state,
        "scheduler".into(),
        Duration::from_secs(1),
        move || {
            let state = worker_state.clone();
            async move {
                schedule_tick(state).await?;
                Ok(false)
            }
        },
    );
}

pub(crate) fn spawn_channel_terminal_delivery_worker(state: AppState) {
    let worker_state = state.clone();
    super::worker_supervision::spawn_background_loop(
        &state,
        "terminal_delivery".into(),
        Duration::from_millis(500),
        move || {
            let state = worker_state.clone();
            async move { channel_terminal_delivery_once(&state).await }
        },
    );
}

async fn cleanup_tick(state: AppState) -> anyhow::Result<()> {
    run_blocking_tick("cleanup", move || cleanup_once(&state)).await
}

async fn schedule_tick(state: AppState) -> anyhow::Result<()> {
    run_blocking_tick("scheduled_job_tick", move || schedule_once(&state)).await
}

async fn task_heartbeat_tick(
    state: AppState,
    task_id: String,
    claim_attempt: i64,
) -> anyhow::Result<bool> {
    run_blocking_tick("task_heartbeat", move || {
        repo::touch_running_task(&state, &task_id, claim_attempt)
    })
    .await
}

pub(super) async fn run_blocking_tick<T: Send + 'static>(
    stage: &'static str,
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    // Each caller awaits completion before scheduling its next tick. Dropping the
    // waiter cannot undo a started DB operation; repository claim/CAS fencing stays authoritative.
    let queued_at = std::time::Instant::now();
    let blocking_operation = super::worker_supervision::BlockingOperation::start();
    let (result, queue_ms, execution_ms) = tokio::task::spawn_blocking(move || {
        let _blocking_operation = blocking_operation;
        let started_at = std::time::Instant::now();
        let queue_ms = queued_at.elapsed().as_millis();
        let result = work();
        (result, queue_ms, started_at.elapsed().as_millis())
    })
    .await
    .map_err(|error| anyhow!("background_tick_join_failed stage={stage} error={error}"))?;
    if let Err(error) = &result {
        let detail = super::worker_supervision::tick_error_detail(error);
        tracing::warn!(
            event = "background_tick_failed",
            stage,
            queue_ms,
            execution_ms,
            error_code = detail.error_code,
            sqlite_extended_code = detail.sqlite_extended_code,
            "background operation failed"
        );
    } else if queue_ms >= 1_000 || execution_ms >= 1_000 {
        tracing::warn!(
            event = "background_tick_slow",
            stage,
            queue_ms,
            execution_ms,
            "background operation exceeded latency threshold"
        );
    } else {
        tracing::debug!(stage, queue_ms, execution_ms, "background_tick_completed");
    }
    result
}

pub(crate) fn spawn_conversation_reply_delivery_worker(state: AppState) {
    let worker_state = state.clone();
    super::worker_supervision::spawn_background_loop(
        &state,
        "conversation_delivery".into(),
        Duration::from_millis(500),
        move || {
            let state = worker_state.clone();
            async move { conversation_reply_delivery_once(&state).await }
        },
    );
}

async fn conversation_reply_delivery_once(state: &AppState) -> anyhow::Result<bool> {
    if repo::reconcile_cancelled_task_settlements(state, 64)? > 0 {
        return Ok(true);
    }
    let Some(claim) = repo::claim_due_conversation_reply_delivery(
        &state.core.db,
        now_ts_u64(),
        CHANNEL_TERMINAL_DELIVERY_LEASE_SECONDS,
    )?
    else {
        return Ok(false);
    };
    if channel_terminal_delivery_retry_budget_exhausted(claim.attempt_count) {
        repo::finish_conversation_reply_delivery(
            &state.core.db,
            &claim,
            false,
            None,
            Some("channel_delivery_retry_budget_exhausted"),
            now_ts_u64(),
        )?;
        return Ok(true);
    }
    let Some(item) = repo::get_conversation_reply_item(&state.core.db, &claim.reply_id)? else {
        repo::finish_conversation_reply_delivery(
            &state.core.db,
            &claim,
            false,
            None,
            Some("conversation_reply_not_found"),
            now_ts_u64(),
        )?;
        return Ok(true);
    };
    match crate::http::task_delivery::deliver_loaded_conversation_reply(state, &item).await {
        Ok(response) if response.accepted() => repo::finish_conversation_reply_delivery(
            &state.core.db,
            &claim,
            true,
            None,
            None,
            now_ts_u64(),
        )?,
        Ok(response) if !response.retryable => repo::finish_conversation_reply_delivery(
            &state.core.db,
            &claim,
            false,
            None,
            response.error_code.as_deref(),
            now_ts_u64(),
        )?,
        Ok(response) => repo::finish_conversation_reply_delivery(
            &state.core.db,
            &claim,
            false,
            Some(channel_terminal_delivery_retry_delay(
                5,
                claim.attempt_count,
            )),
            response.error_code.as_deref(),
            now_ts_u64(),
        )?,
        Err(error) => {
            let error_code = error.to_string();
            let retry_after = (!matches!(
                error_code.as_str(),
                "channel_delivery_authorization_revoked"
                    | "channel_delivery_owner_missing"
                    | "channel_delivery_owner_mismatch"
                    | "channel_delivery_ingress_missing"
                    | "channel_delivery_ingress_invalid"
                    | "channel_delivery_scope_mismatch"
                    | "channel_delivery_reply_target_missing"
                    | "channel_delivery_reply_target_mismatch"
                    | "task_not_found"
            ))
            .then(|| channel_terminal_delivery_retry_delay(1, claim.attempt_count));
            repo::finish_conversation_reply_delivery(
                &state.core.db,
                &claim,
                false,
                retry_after,
                Some(&error_code),
                now_ts_u64(),
            )?;
        }
    }
    Ok(true)
}

async fn channel_terminal_delivery_once(state: &AppState) -> anyhow::Result<bool> {
    let Some(claim) = repo::claim_due_channel_terminal_delivery(
        &state.core.db,
        now_ts_u64(),
        CHANNEL_TERMINAL_DELIVERY_LEASE_SECONDS,
    )?
    else {
        return Ok(false);
    };
    if channel_terminal_delivery_retry_budget_exhausted(claim.attempt_count) {
        warn!(
            task_id = claim.task_id,
            attempt_count = claim.attempt_count,
            "channel terminal delivery retry budget exhausted"
        );
        repo::finish_channel_terminal_delivery(
            &state.core.db,
            &claim,
            false,
            None,
            Some("channel_delivery_retry_budget_exhausted"),
            now_ts_u64(),
        )?;
        return Ok(true);
    }
    if repo::has_unsettled_conversation_reply_delivery(&state.core.db, &claim.task_id)? {
        repo::finish_channel_terminal_delivery(
            &state.core.db,
            &claim,
            false,
            Some(1),
            Some("conversation_reply_delivery_pending"),
            now_ts_u64(),
        )?;
        return Ok(true);
    }
    let record = match repo::get_task_delivery_record(state, &claim.task_id)? {
        Some(record) => record,
        None => {
            repo::finish_channel_terminal_delivery(
                &state.core.db,
                &claim,
                false,
                None,
                Some("task_not_found"),
                now_ts_u64(),
            )?;
            return Ok(true);
        }
    };
    if crate::task_lifecycle::cancellation_settlement_pending(
        &record.status,
        record.result_json.as_ref(),
    ) {
        repo::finish_channel_terminal_delivery(
            &state.core.db,
            &claim,
            false,
            Some(1),
            Some("task_cancellation_settlement_pending"),
            now_ts_u64(),
        )?;
        return Ok(true);
    }
    let request = claw_core::channel_delivery::ChannelTaskDeliveryRequest::daemon(
        claw_core::channel_delivery::ChannelDeliverySource::BackgroundCompletion,
    );
    match crate::http::task_delivery::deliver_loaded_terminal_task(state, &record, &request).await {
        Ok(response) if response.accepted => repo::finish_channel_terminal_delivery(
            &state.core.db,
            &claim,
            true,
            None,
            None,
            now_ts_u64(),
        )?,
        Ok(response)
            if response.status
                == claw_core::channel_delivery::ChannelTaskDeliveryStatus::QueryRequired
                && !response.retryable =>
        {
            // Stop automatic attempts, retaining the unknown receipt for explicit reconciliation.
            repo::finish_channel_terminal_delivery(
                &state.core.db,
                &claim,
                false,
                None,
                Some("channel_delivery_confirmation_required"),
                now_ts_u64(),
            )?
        }
        Ok(response)
            if response.status
                == claw_core::channel_delivery::ChannelTaskDeliveryStatus::Failed
                && !response.retryable =>
        {
            repo::finish_channel_terminal_delivery(
                &state.core.db,
                &claim,
                false,
                None,
                response.error_code.as_deref(),
                now_ts_u64(),
            )?
        }
        Ok(response) => {
            let base_delay = if response.status
                == claw_core::channel_delivery::ChannelTaskDeliveryStatus::QueryRequired
            {
                30
            } else {
                5
            };
            let delay = channel_terminal_delivery_retry_delay(base_delay, claim.attempt_count);
            repo::finish_channel_terminal_delivery(
                &state.core.db,
                &claim,
                false,
                Some(delay),
                response.error_code.as_deref(),
                now_ts_u64(),
            )?;
        }
        Err(error) => {
            let error_code = error.to_string();
            let retry_after = (!matches!(
                error_code.as_str(),
                "channel_delivery_authorization_revoked"
                    | "channel_delivery_owner_missing"
                    | "channel_delivery_ingress_missing"
                    | "channel_delivery_ingress_invalid"
                    | "channel_delivery_scope_mismatch"
                    | "channel_delivery_reply_target_missing"
                    | "channel_delivery_reply_target_mismatch"
            ))
            .then(|| channel_terminal_delivery_retry_delay(1, claim.attempt_count));
            repo::finish_channel_terminal_delivery(
                &state.core.db,
                &claim,
                false,
                retry_after,
                Some(&error_code),
                now_ts_u64(),
            )?;
        }
    }
    Ok(true)
}

fn stamp_current_admin_policy_for_legacy_scheduled_job(
    db: &rusqlite::Connection,
    user_key: Option<&str>,
    payload: &mut Value,
) -> anyhow::Result<()> {
    if !payload.is_object()
        || crate::task_execution_policy::policy_payload(payload).is_some()
        || user_key.is_none()
    {
        return Ok(());
    }
    let identity = crate::repo::auth::resolve_auth_identity_on_connection(db, user_key.unwrap())?;
    crate::task_execution_policy::stamp_authenticated_submission_policy(
        payload,
        identity.as_ref(),
        None,
        None,
    )
    .map_err(|error| anyhow!(error.as_token()))
}

include!("scheduled_enqueue.rs");

fn schedule_once(state: &AppState) -> anyhow::Result<()> {
    let now = now_ts_u64() as i64;
    let mut due_jobs: Vec<ScheduledJobDue> = Vec::new();

    {
        let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
        let mut stmt = db.prepare(&format!(
            "{SCHEDULE_JOB_SELECT}
             WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1
             ORDER BY next_run_at ASC
             LIMIT 16"
        ))?;
        let rows = stmt.query_map(rusqlite::params![now], scheduled_job_row)?;
        for row in rows {
            due_jobs.push(row?);
        }
    }

    if due_jobs.is_empty() {
        return Ok(());
    }

    for job in due_jobs {
        let next_run = schedule_service::compute_next_run_for_schedule(
            &job.schedule_type,
            job.time_of_day.as_deref(),
            job.weekday,
            job.every_minutes,
            &job.timezone,
            now,
        );

        if coalesce_active_scheduled_thread_wakeup(state, &job, now)? {
            let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
            advance_scheduled_job(
                &db,
                &job,
                next_run,
                now,
                job.last_thread_task_id.as_deref().unwrap_or_default(),
            )?;
            continue;
        }

        let run_id = format!("run_{}", Uuid::new_v4().simple());
        let mut db = state.core.db.get()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // A manual submission can commit after the due-job scan. Check again
        // under the same write lock used by the manual endpoint.
        let task_id = match active_scheduled_run(&tx, &job.job_id)? {
            Some(active_id) => active_id,
            None => enqueue_scheduled_run(&tx, &job, &run_id)?,
        };
        if !advance_scheduled_job(&tx, &job, next_run, now, &task_id)? {
            continue;
        }
        tx.commit()?;
    }

    Ok(())
}

fn coalesce_active_scheduled_thread_wakeup(
    state: &AppState,
    job: &ScheduledJobDue,
    now_ts: i64,
) -> anyhow::Result<bool> {
    if !job.thread_resume_enabled {
        return Ok(false);
    }
    let Some(task_id) = job
        .last_thread_task_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(false);
    };
    let status = {
        let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
        db.query_row(
            "SELECT status FROM tasks WHERE task_id = ?1 LIMIT 1",
            rusqlite::params![task_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    };
    let Some(status) = status else {
        return Ok(false);
    };
    if !matches!(status.as_str(), "queued" | "running") {
        return Ok(false);
    }
    if status == "running" {
        let resume_trigger = crate::task_lifecycle::ResumeTrigger::ScheduledWakeup;
        let _ = repo::resume_task_with_input(
            state,
            repo::TaskResumeControlInput {
                task_id: task_id.to_string(),
                checkpoint_id: None,
                resume_trigger,
                resume_reason: Some(resume_trigger.status_code().to_string()),
                user_message: None,
                new_constraints: Some(json!({
                    "schedule_job_id": job.job_id,
                    "thread_ref": crate::scheduled_run_contract::scheduled_run_thread_ref(&job.job_id),
                    "wake_requested_at": now_ts,
                })),
            },
        )?;
    }
    Ok(true)
}

fn advance_scheduled_job(
    db: &rusqlite::Connection,
    job: &ScheduledJobDue,
    next_run: Option<i64>,
    now_ts: i64,
    task_id: &str,
) -> anyhow::Result<bool> {
    let changed = match next_run {
        Some(next_run_at) => db.execute(
            "UPDATE scheduled_jobs
             SET last_run_at = ?2, next_run_at = ?3, last_thread_task_id = ?5, updated_at = ?2
             WHERE job_id = ?1 AND enabled = 1 AND next_run_at = ?4",
            rusqlite::params![
                job.job_id,
                now_ts.to_string(),
                next_run_at,
                job.next_run_at,
                task_id
            ],
        )?,
        None => db.execute(
            "UPDATE scheduled_jobs
             SET enabled = 0, last_run_at = ?2, next_run_at = NULL, last_thread_task_id = ?4, updated_at = ?2
             WHERE job_id = ?1 AND enabled = 1 AND next_run_at = ?3",
            rusqlite::params![job.job_id, now_ts.to_string(), job.next_run_at, task_id],
        )?,
    };
    Ok(changed > 0)
}

fn cleanup_once(state: &AppState) -> anyhow::Result<()> {
    let mut db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;

    let now = now_ts_u64() as i64;

    // Retention is not cancellation: active execution and durable consumers own
    // their task rows until settled, even when an age or row limit is exceeded.
    let edition_refs = crate::http::ui_routes::edition_retention_refs(state, &db)?;
    let eligible = "kind != 'ask'
        AND NOT EXISTS (SELECT 1 FROM json_each(?3) AS retained
            WHERE tasks.kind='run_skill' AND CASE WHEN json_valid(tasks.payload_json) THEN
                retained.value IN (json_extract(tasks.payload_json,'$.skill_name'),
                    json_extract(tasks.payload_json,'$.skill_name') || '.' || json_extract(tasks.payload_json,'$.args.action'))
            ELSE 0 END)
        AND status IN ('succeeded', 'failed', 'canceled', 'timeout')
        AND (status != 'canceled' OR COALESCE(
            CASE WHEN json_valid(result_json)
                 THEN json_extract(result_json, '$.task_lifecycle.state') END, ''
        ) != 'cancel_requested')
        AND NOT EXISTS (
            SELECT 1 FROM channel_terminal_delivery_outbox outbox
            WHERE outbox.task_id = tasks.task_id AND outbox.state IN ('pending', 'dispatching')
        )
        AND NOT EXISTS (
            SELECT 1 FROM conversation_reply_items item
            JOIN conversation_reply_delivery_outbox outbox ON outbox.reply_id = item.reply_id
            WHERE item.task_id = tasks.task_id AND outbox.state IN ('pending', 'dispatching')
        )
        AND task_id NOT IN (
            SELECT last_thread_task_id FROM scheduled_jobs
            WHERE thread_resume_enabled = 1 AND last_thread_task_id IS NOT NULL
        )
        AND NOT EXISTS (SELECT 1 FROM child_task_graphs graph WHERE graph.parent_task_id = tasks.task_id)
        AND NOT EXISTS (SELECT 1 FROM child_task_graph_nodes node WHERE node.child_task_id = tasks.task_id)";
    let task_cutoff = crate::retention_policy::cutoff_timestamp(
        now,
        state.policy.maintenance.tasks_retention_days,
    );
    db.execute(
        &format!(
            "DELETE FROM tasks WHERE task_id IN (
             SELECT task_id FROM tasks
             WHERE {eligible}
               AND CAST(created_at AS INTEGER) < ?1
             ORDER BY CAST(created_at AS INTEGER) ASC, task_id ASC
             LIMIT ?2
         )"
        ),
        rusqlite::params![task_cutoff, MAINTENANCE_DELETE_BATCH_SIZE, edition_refs],
    )?;

    db.execute(
        &format!(
            "DELETE FROM tasks WHERE task_id IN (
             SELECT task_id FROM tasks
             WHERE {eligible}
             ORDER BY CAST(created_at AS INTEGER) DESC, task_id DESC
             LIMIT ?2 OFFSET ?1
         )"
        ),
        rusqlite::params![
            state.policy.maintenance.tasks_max_rows as i64,
            MAINTENANCE_DELETE_BATCH_SIZE, edition_refs
        ],
    )?;
    db.execute(
        "DELETE FROM task_mutation_ledger WHERE rowid IN (
             SELECT task_mutation_ledger.rowid FROM task_mutation_ledger
             WHERE NOT EXISTS (
                 SELECT 1 FROM tasks
                 WHERE tasks.task_id = task_mutation_ledger.task_id
             )
             LIMIT ?1
         )",
        [MAINTENANCE_DELETE_BATCH_SIZE],
    )?;
    db.execute(
        "DELETE FROM llm_cost_ledger WHERE rowid IN (
             SELECT llm_cost_ledger.rowid FROM llm_cost_ledger
             WHERE NOT EXISTS (
                 SELECT 1 FROM tasks
                 WHERE tasks.task_id = llm_cost_ledger.task_id
             )
             LIMIT ?1
         )",
        [MAINTENANCE_DELETE_BATCH_SIZE],
    )?;
    crate::task_event_archive::delete_orphaned_records(
        &db,
        MAINTENANCE_DELETE_BATCH_SIZE as usize,
    )?;
    if let Err(error) = crate::task_artifacts::cleanup_orphaned_delivery_artifacts(
        &state.skill_rt.workspace_root,
        &db,
    ) {
        warn!("cleanup task delivery artifacts failed: {error}");
    }

    // Phase 2.2 Stage 2: audit_logs 已经搬到独立 audit pool（见 db_init::init_audit_db）。
    // 这里清理也走 audit_db，避免在主库 writer 锁上和任务回收争抢。
    {
        let audit_db = state
            .core
            .audit_db
            .get()
            .map_err(|e| anyhow!("audit db pool: {e}"))?;
        let audit_cutoff = crate::retention_policy::cutoff_timestamp(
            now,
            state.policy.maintenance.audit_retention_days,
        );
        audit_db.execute(
            "DELETE FROM audit_logs WHERE id IN (
                 SELECT id FROM audit_logs
                 WHERE CAST(ts AS INTEGER) < ?1
                 ORDER BY id ASC
                 LIMIT ?2
             )",
            rusqlite::params![audit_cutoff, MAINTENANCE_DELETE_BATCH_SIZE],
        )?;

        audit_db.execute(
            "DELETE FROM audit_logs WHERE id IN (
                 SELECT id FROM audit_logs
                 ORDER BY id DESC
                 LIMIT ?2 OFFSET ?1
             )",
            rusqlite::params![
                state.policy.maintenance.audit_max_rows as i64,
                MAINTENANCE_DELETE_BATCH_SIZE
            ],
        )?;
    }

    if let Err(error) = crate::task_result_objects::compact_batch(&mut db, 64) {
        warn!(error = %error, "task_result_compaction_failed");
    }
    if crate::task_journal_archive::enabled(&db)? {
        if let Err(error) = crate::task_journal_archive::compact_batch(
            &mut db,
            32,
            now.saturating_sub(86_400).saturating_mul(1000),
        ) {
            warn!(error = %error, "task_journal_archive_compaction_failed");
        }
    }
    match crate::task_event_storage_maintenance::compact_batch(&mut db, 128) {
        Ok(report) if report.rows > 0 => info!(
            rows = report.rows,
            compressed_rows = report.compressed_rows,
            logical_bytes = report.logical_bytes,
            stored_bytes = report.stored_bytes,
            "event_storage_compaction_completed"
        ),
        Ok(_) => {}
        Err(error) => warn!(%error, "event_storage_compaction_failed"),
    }
    if crate::event_json_chunks::maintenance_enabled(&db)? {
        match crate::event_json_chunks::compact_batch(&mut db, 128) {
            Ok(rows) if rows > 0 => info!(rows, "event_cold_chunks_compacted"),
            Ok(_) => {}
            Err(error) => warn!(%error,"event_cold_chunk_compaction_failed"),
        }
    }
    match crate::task_event_storage_maintenance::prune_hot_batch(
        &mut db,
        now.saturating_sub(86_400).saturating_mul(1000),
        128,
    ) {
        Ok(rows) if rows > 0 => info!(rows, "event_hot_cache_pruned"),
        Ok(_) => {}
        Err(error) => warn!(%error, "event_hot_cache_prune_failed"),
    }
    let memory_cleanup =
        crate::memory::retention::cleanup_memory_data(&db, &state.policy.memory, now)?;
    crate::memory::ux::scrub_expired_deletion_grace(&db, now)?;
    let principal_ids = {
        let mut statement = db.prepare(
            "SELECT DISTINCT principal_id FROM (
                SELECT principal_id FROM memory_revisions
                UNION ALL SELECT principal_id FROM memory_jobs
                UNION ALL SELECT principal_id FROM memory_retrieval_index
             ) WHERE principal_id IS NOT NULL AND TRIM(principal_id) != ''",
        )?;
        let principal_ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        principal_ids
    };
    for principal_id in principal_ids {
        let report = crate::memory::ux::check_memory_consistency(&db, &principal_id, true, now)?;
        if report.repaired_rows > 0 {
            warn!(
                repaired_rows = report.repaired_rows,
                orphan_retrieval_rows = report.orphan_retrieval_rows,
                stale_jobs = report.stale_jobs,
                "memory_consistency_sweeper_repaired_residual_state"
            );
        }
    }
    if memory_cleanup.storage_pressure_state != "normal" {
        warn!(
            state = memory_cleanup.storage_pressure_state,
            observed_bytes = memory_cleanup.observed_bytes,
            "memory_storage_pressure_active"
        );
    }
    drop(db);

    // model_io.log：不再每次 append 后做全量 prune（会 O(N²) 磁盘）。
    // 改由这里按 cleanup 节拍把跨天的行迁到 `model_io.log.YYYY-MM-DD` 归档，
    // 主文件只保留当天；同时清理超过 keep_days 的旧归档。
    let model_io_path = state
        .skill_rt
        .workspace_root
        .join("logs")
        .join("model_io.log");
    if let Err(err) = crate::providers::rotate_model_io_log_daily(
        &model_io_path,
        crate::providers::MODEL_IO_LOG_KEEP_DAYS,
    ) {
        tracing::warn!("rotate model io log failed: {err}");
    }

    Ok(())
}

#[cfg(test)]
#[path = "background_workers_tests.rs"]
mod tests;

use std::time::Duration;

use anyhow::anyhow;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{now_ts, now_ts_u64, repo, schedule_service, AppState, ScheduledJobDue};

pub(crate) fn start_task_heartbeat(state: AppState, task_id: String) -> oneshot::Sender<()> {
    let interval_secs = state.worker.worker_task_heartbeat_seconds.max(5);
    let worker_id = state.worker.worker_id.clone();
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {
                    if let Err(err) = repo::touch_running_task(&state, &task_id) {
                        warn!(
                            "task heartbeat update failed: worker_id={} task_id={} interval_secs={} err={}",
                            worker_id, task_id, interval_secs, err
                        );
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

pub(crate) fn spawn_long_term_summary_refresh(
    state: AppState,
    task: crate::ClaimedTask,
    force_refresh: bool,
) {
    tokio::spawn(async move {
        if let Err(err) =
            crate::memory::service::maybe_refresh_long_term_summary(&state, &task, force_refresh)
                .await
        {
            warn!("refresh long-term memory summary failed: {err}");
        }
    });
}

pub(crate) fn spawn_worker(state: AppState, poll_interval_ms: u64, concurrency: usize) {
    let worker_count = concurrency.max(1);
    info!(
        "spawn_worker: worker_id={} starting {} worker loop(s), poll_interval_ms={}",
        state.worker.worker_id,
        worker_count,
        poll_interval_ms.max(10)
    );
    for worker_idx in 0..worker_count {
        let state_cloned = state.clone();
        tokio::spawn(async move {
            loop {
                if let Err(err) = super::super::worker_once(&state_cloned).await {
                    error!(
                        "Worker tick failed (worker_id={} worker_idx={}): {}",
                        state_cloned.worker.worker_id, worker_idx, err
                    );
                }
                tokio::time::sleep(Duration::from_millis(poll_interval_ms.max(10))).await;
            }
        });
    }
}

pub(crate) fn spawn_cleanup_worker(state: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(
                state.policy.maintenance.cleanup_interval_seconds.max(30),
            ))
            .await;

            if let Err(err) = cleanup_once(&state) {
                error!("Cleanup task failed: {}", err);
            }
        }
    });
}

pub(crate) fn spawn_schedule_worker(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(err) = schedule_once(&state) {
                error!("Schedule worker tick failed: {}", err);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

fn schedule_once(state: &AppState) -> anyhow::Result<()> {
    let now = now_ts_u64() as i64;
    let mut due_jobs: Vec<ScheduledJobDue> = Vec::new();

    {
        let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
        let mut stmt = db.prepare(
            "SELECT job_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, task_kind, task_payload_json, next_run_at,
                    schedule_type, time_of_day, weekday, every_minutes, timezone, isolation_profile, permission_policy_json,
                    thread_resume_enabled, last_thread_task_id
             FROM scheduled_jobs
             WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1
             ORDER BY next_run_at ASC
             LIMIT 16",
        )?;
        let rows = stmt.query_map(rusqlite::params![now], |row| {
            Ok(ScheduledJobDue {
                job_id: row.get(0)?,
                user_id: row.get(1)?,
                chat_id: row.get(2)?,
                user_key: row.get(3)?,
                channel: row.get(4)?,
                external_user_id: row.get(5)?,
                external_chat_id: row.get(6)?,
                task_kind: row.get(7)?,
                task_payload_json: row.get(8)?,
                next_run_at: row.get(9)?,
                schedule_type: row.get(10)?,
                time_of_day: row.get(11)?,
                weekday: row.get(12)?,
                every_minutes: row.get(13)?,
                timezone: row.get(14)?,
                isolation_profile: row.get(15)?,
                permission_policy_json: row.get(16)?,
                thread_resume_enabled: row.get::<_, i64>(17)? != 0,
                last_thread_task_id: row.get(18)?,
            })
        })?;
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

        let mut payload =
            serde_json::from_str::<Value>(&job.task_payload_json).unwrap_or_else(|_| json!({}));
        if let Value::Object(map) = &mut payload {
            map.insert("channel".to_string(), Value::String(job.channel.clone()));
            if let Some(v) = job.external_user_id.as_ref() {
                map.insert("external_user_id".to_string(), Value::String(v.clone()));
            }
            if let Some(v) = job.external_chat_id.as_ref() {
                map.insert("external_chat_id".to_string(), Value::String(v.clone()));
            }
            for (k, v) in crate::scheduled_run_contract::scheduled_run_policy_metadata(
                &job.isolation_profile,
                &job.permission_policy_json,
            ) {
                map.insert(k, v);
            }
            for (k, v) in crate::scheduled_run_contract::scheduled_run_thread_resume_metadata(
                job.thread_resume_enabled,
                job.last_thread_task_id.as_deref(),
            ) {
                map.insert(k, v);
            }
        }

        let task_id = Uuid::new_v4().to_string();
        let run_id = format!("run_{}", Uuid::new_v4().simple());
        let thread_ref = crate::scheduled_run_contract::scheduled_run_thread_ref(&job.job_id);
        let now_text = now_ts();
        if let Value::Object(map) = &mut payload {
            for (k, v) in schedule_service::schedule_invocation_metadata(&job.job_id, &run_id) {
                map.insert(k, v);
            }
        }
        let mut db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO tasks (task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, message_id, kind, payload_json, status, result_json, error_text, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, 'queued', NULL, NULL, ?10, ?10)",
            rusqlite::params![
                task_id,
                job.user_id,
                job.chat_id,
                job.user_key,
                job.channel,
                job.external_user_id,
                job.external_chat_id,
                job.task_kind,
                payload.to_string(),
                now_text
            ],
        )?;
        crate::scheduled_run_contract::insert_scheduled_run_enqueued(
            &tx,
            &crate::scheduled_run_contract::ScheduledRunEnqueued {
                run_id: &run_id,
                job_id: &job.job_id,
                task_id: &task_id,
                thread_ref: &thread_ref,
                started_at: &now_text,
            },
        )?;
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
             WHERE job_id = ?1 AND next_run_at = ?4",
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
             WHERE job_id = ?1 AND next_run_at = ?3",
            rusqlite::params![job.job_id, now_ts.to_string(), job.next_run_at, task_id],
        )?,
    };
    Ok(changed > 0)
}

fn cleanup_once(state: &AppState) -> anyhow::Result<()> {
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;

    let now = now_ts_u64() as i64;

    let task_cutoff = now - (state.policy.maintenance.tasks_retention_days as i64 * 86400);
    db.execute(
        "DELETE FROM tasks WHERE CAST(created_at AS INTEGER) < ?1",
        rusqlite::params![task_cutoff],
    )?;

    db.execute(
        "DELETE FROM tasks WHERE task_id IN (
             SELECT task_id FROM tasks
             ORDER BY CAST(created_at AS INTEGER) DESC
             LIMIT -1 OFFSET ?1
         )",
        rusqlite::params![state.policy.maintenance.tasks_max_rows as i64],
    )?;
    db.execute(
        "DELETE FROM task_mutation_ledger
         WHERE NOT EXISTS (
             SELECT 1 FROM tasks
             WHERE tasks.task_id = task_mutation_ledger.task_id
         )",
        [],
    )?;
    db.execute(
        "DELETE FROM llm_cost_ledger
         WHERE NOT EXISTS (
             SELECT 1 FROM tasks
             WHERE tasks.task_id = llm_cost_ledger.task_id
         )",
        [],
    )?;

    // Phase 2.2 Stage 2: audit_logs 已经搬到独立 audit pool（见 db_init::init_audit_db）。
    // 这里清理也走 audit_db，避免在主库 writer 锁上和任务回收争抢。
    {
        let audit_db = state
            .core
            .audit_db
            .get()
            .map_err(|e| anyhow!("audit db pool: {e}"))?;
        let audit_cutoff = now - (state.policy.maintenance.audit_retention_days as i64 * 86400);
        audit_db.execute(
            "DELETE FROM audit_logs WHERE CAST(ts AS INTEGER) < ?1",
            rusqlite::params![audit_cutoff],
        )?;

        audit_db.execute(
            "DELETE FROM audit_logs WHERE id IN (
                 SELECT id FROM audit_logs
                 ORDER BY id DESC
                 LIMIT -1 OFFSET ?1
             )",
            rusqlite::params![state.policy.maintenance.audit_max_rows as i64],
        )?;
    }

    let memory_cutoff = now - (state.policy.memory.retention_days as i64 * 86400);
    db.execute(
        "DELETE FROM memories
         WHERE COALESCE(created_at_ts, CAST(created_at AS INTEGER)) < ?1",
        rusqlite::params![memory_cutoff],
    )?;

    db.execute(
        "DELETE FROM memories WHERE id IN (
             SELECT id FROM memories
             ORDER BY id DESC
             LIMIT -1 OFFSET ?1
         )",
        rusqlite::params![state.policy.memory.max_rows as i64],
    )?;
    if state.policy.memory.hybrid_recall_enabled {
        let index_max_rows = state.policy.memory.max_rows.saturating_mul(3).max(2000);
        crate::memory::indexing::cleanup_retrieval_index(&db, memory_cutoff, index_max_rows)?;
    }

    let long_term_cutoff = now - (state.policy.memory.long_term_retention_days as i64 * 86400);
    db.execute(
        "DELETE FROM long_term_memories
         WHERE COALESCE(updated_at_ts, CAST(updated_at AS INTEGER)) < ?1",
        rusqlite::params![long_term_cutoff],
    )?;

    db.execute(
        "DELETE FROM long_term_memories WHERE id IN (
             SELECT id FROM long_term_memories
             ORDER BY id DESC
             LIMIT -1 OFFSET ?1
         )",
        rusqlite::params![state.policy.memory.long_term_max_rows as i64],
    )?;
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

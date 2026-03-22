use std::time::Duration;

use anyhow::anyhow;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tracing::{debug, error, info, info_span, warn, Instrument};
use uuid::Uuid;

use crate::{now_ts, now_ts_u64, repo, schedule_service, AppState, ScheduledJobDue};

fn recover_stale_running_tasks_by_no_progress(state: &AppState) -> anyhow::Result<Vec<String>> {
    let timeout_secs = state.worker_running_no_progress_timeout_seconds.max(60);
    let now = now_ts_u64() as i64;
    let stale_before = now.saturating_sub(timeout_secs as i64);
    let stale_note = format!(
        "auto timeout: no progress heartbeat for {}s while status=running",
        timeout_secs
    );
    let db = state.db.lock().map_err(|_| anyhow!("db lock poisoned"))?;

    let mut task_ids = Vec::new();
    {
        let mut stmt = db.prepare(
            "SELECT task_id
             FROM tasks
             WHERE status = 'running'
               AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?1
             ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![stale_before.to_string()], |row| {
            row.get::<_, String>(0)
        })?;
        for row in rows {
            task_ids.push(row?);
        }
    }

    if task_ids.is_empty() {
        return Ok(task_ids);
    }

    let changed = db.execute(
        "UPDATE tasks
         SET status = 'timeout',
             error_text = CASE
                 WHEN error_text IS NULL OR TRIM(error_text) = '' THEN ?2
                 ELSE error_text
             END,
             updated_at = ?3
         WHERE status = 'running'
           AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?1",
        rusqlite::params![stale_before.to_string(), stale_note, now_ts()],
    )?;
    if changed != task_ids.len() {
        warn!(
            "runtime stale-running recovery count mismatch: selected={} updated={}",
            task_ids.len(),
            changed
        );
    }
    Ok(task_ids)
}

pub(crate) fn maybe_recover_stale_running_tasks_runtime(state: &AppState) -> anyhow::Result<()> {
    let now = now_ts_u64();
    let interval = state.worker_running_recovery_check_interval_seconds.max(10);
    {
        let mut guard = state
            .last_running_recovery_check_ts
            .lock()
            .map_err(|_| anyhow!("running recovery lock poisoned"))?;
        if now.saturating_sub(*guard) < interval {
            return Ok(());
        }
        *guard = now;
    }
    let recovered = recover_stale_running_tasks_by_no_progress(state)?;
    if !recovered.is_empty() {
        warn!(
            "runtime stale-running recovery applied: converted {} running tasks to timeout (no_progress_timeout={}s)",
            recovered.len(),
            state.worker_running_no_progress_timeout_seconds
        );
    }
    Ok(())
}

pub(crate) fn start_task_heartbeat(state: AppState, task_id: String) -> oneshot::Sender<()> {
    let interval_secs = state.worker_task_heartbeat_seconds.max(5);
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {
                    if let Err(err) = repo::touch_running_task(&state, &task_id) {
                        warn!(
                            "task heartbeat update failed: task_id={} interval_secs={} err={}",
                            task_id, interval_secs, err
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

fn spawn_long_term_summary_refresh(state: AppState, task: crate::ClaimedTask) {
    tokio::spawn(async move {
        if let Err(err) =
            crate::memory::service::maybe_refresh_long_term_summary(&state, &task).await
        {
            warn!("refresh long-term memory summary failed: {err}");
        }
    });
}

pub(crate) fn spawn_worker(state: AppState, poll_interval_ms: u64, concurrency: usize) {
    let worker_count = concurrency.max(1);
    info!(
        "spawn_worker: starting {} worker loop(s), poll_interval_ms={}",
        worker_count,
        poll_interval_ms.max(10)
    );
    for worker_idx in 0..worker_count {
        let state_cloned = state.clone();
        tokio::spawn(async move {
            loop {
                if let Err(err) = worker_once(&state_cloned).await {
                    error!("Worker tick failed (worker_idx={}): {}", worker_idx, err);
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
                state.maintenance.cleanup_interval_seconds.max(30),
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
        let db = state.db.lock().map_err(|_| anyhow!("db lock poisoned"))?;
        let mut stmt = db.prepare(
            "SELECT job_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, task_kind, task_payload_json, next_run_at,
                    schedule_type, time_of_day, weekday, every_minutes, timezone
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
            })
        })?;
        for row in rows {
            due_jobs.push(row?);
        }
    }

    if due_jobs.is_empty() {
        return Ok(());
    }

    let db = state.db.lock().map_err(|_| anyhow!("db lock poisoned"))?;

    for job in due_jobs {
        let next_run = schedule_service::compute_next_run_for_schedule(
            &job.schedule_type,
            job.time_of_day.as_deref(),
            job.weekday,
            job.every_minutes,
            &job.timezone,
            now,
        );

        let mut payload =
            serde_json::from_str::<Value>(&job.task_payload_json).unwrap_or_else(|_| json!({}));
        if let Value::Object(map) = &mut payload {
            for (k, v) in schedule_service::schedule_invocation_metadata(&job.job_id) {
                map.insert(k, v);
            }
            map.insert("channel".to_string(), Value::String(job.channel.clone()));
            if let Some(v) = job.external_user_id.as_ref() {
                map.insert("external_user_id".to_string(), Value::String(v.clone()));
            }
            if let Some(v) = job.external_chat_id.as_ref() {
                map.insert("external_chat_id".to_string(), Value::String(v.clone()));
            }
        }

        let task_id = Uuid::new_v4().to_string();
        let now_text = now_ts();
        db.execute(
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

        match next_run {
            Some(ts) => {
                db.execute(
                    "UPDATE scheduled_jobs
                     SET last_run_at = ?2, next_run_at = ?3, updated_at = ?2
                     WHERE job_id = ?1 AND next_run_at = ?4",
                    rusqlite::params![job.job_id, now.to_string(), ts, job.next_run_at],
                )?;
            }
            None => {
                db.execute(
                    "UPDATE scheduled_jobs
                     SET enabled = 0, last_run_at = ?2, next_run_at = NULL, updated_at = ?2
                     WHERE job_id = ?1 AND next_run_at = ?3",
                    rusqlite::params![job.job_id, now.to_string(), job.next_run_at],
                )?;
            }
        }
    }

    Ok(())
}

pub(crate) fn runtime_channel_from_payload(
    state: &AppState,
    payload: &Value,
) -> crate::RuntimeChannel {
    let ch = payload
        .get("channel")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if is_whatsapp_channel_value(crate::main_flow_rules(state), &ch) {
        return crate::RuntimeChannel::Whatsapp;
    }
    if ch == "wechat" {
        return crate::RuntimeChannel::Wechat;
    }
    if ch == "feishu" {
        return crate::RuntimeChannel::Feishu;
    }
    if ch == "lark" {
        return crate::RuntimeChannel::Lark;
    }
    crate::RuntimeChannel::Telegram
}

pub(crate) fn is_whatsapp_channel_value(rules: &crate::MainFlowRules, raw: &str) -> bool {
    let channel = raw.trim().to_ascii_lowercase();
    rules
        .runtime_whatsapp_channel_aliases
        .iter()
        .any(|v| v == &channel)
}

pub(crate) fn is_resume_continue_source(rules: &crate::MainFlowRules, raw: &str) -> bool {
    let source = raw.trim().to_ascii_lowercase();
    rules.resume_continue_sources.iter().any(|v| v == &source)
}

pub(crate) fn task_payload_value(task: &crate::ClaimedTask) -> Option<Value> {
    serde_json::from_str::<Value>(&task.payload_json).ok()
}

pub(crate) fn task_runtime_channel(
    state: &AppState,
    task: &crate::ClaimedTask,
) -> crate::RuntimeChannel {
    let ch = task.channel.trim().to_ascii_lowercase();
    if is_whatsapp_channel_value(crate::main_flow_rules(state), &ch) {
        return crate::RuntimeChannel::Whatsapp;
    }
    if ch == "wechat" {
        return crate::RuntimeChannel::Wechat;
    }
    if ch == "feishu" {
        return crate::RuntimeChannel::Feishu;
    }
    if ch == "lark" {
        return crate::RuntimeChannel::Lark;
    }
    let Some(payload) = task_payload_value(task) else {
        return crate::RuntimeChannel::Telegram;
    };
    runtime_channel_from_payload(state, &payload)
}

pub(crate) fn task_external_chat_id(task: &crate::ClaimedTask) -> Option<String> {
    if let Some(v) = task
        .external_chat_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(v);
    }
    let payload = task_payload_value(task)?;
    payload
        .get("external_chat_id")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub(crate) fn resolve_whatsapp_delivery_route(
    state: &AppState,
    payload: &Value,
) -> crate::WhatsappDeliveryRoute {
    let rules = crate::main_flow_rules(state);
    let adapter = payload
        .get("adapter")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if rules.whatsapp_web_adapters.iter().any(|a| a == &adapter) {
        return crate::WhatsappDeliveryRoute::WebBridge;
    }
    if rules.whatsapp_cloud_adapters.iter().any(|a| a == &adapter) {
        return crate::WhatsappDeliveryRoute::Cloud;
    }
    if state.whatsapp_web_enabled && !state.whatsapp_cloud_enabled {
        return crate::WhatsappDeliveryRoute::WebBridge;
    }
    crate::WhatsappDeliveryRoute::Cloud
}

pub(crate) async fn maybe_bind_recent_failed_resume_context(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &mut Value,
    user_text: &str,
) -> Option<()> {
    if payload.get("resume_context").is_some() {
        return None;
    }
    let source = payload
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if is_resume_continue_source(crate::main_flow_rules(state), source) {
        return None;
    }
    let candidate = crate::find_recent_failed_resume_context(state, task.user_id, task.chat_id)?;
    if candidate.has_newer_successful_ask_after_failed_task {
        return None;
    }
    let binding_context_json = json!({
        "source": "recent_failed_resume_context_candidate",
        "failed_resume_context_ts": candidate.failed_ts,
        "has_newer_successful_ask_after_failed_task": candidate.has_newer_successful_ask_after_failed_task,
    });
    let followup_intent = crate::intent_router::classify_resume_followup_intent(
        state,
        task,
        user_text,
        &candidate.resume_context,
        &binding_context_json,
    )
    .await;
    if !followup_intent.bind_resume_context
        || followup_intent.decision == crate::intent_router::ResumeFollowupDecision::Abandon
    {
        return None;
    }
    let obj = payload.as_object_mut()?;
    let resume_source = crate::main_flow_rules(state)
        .resume_continue_sources
        .first()
        .cloned()
        .unwrap_or_else(|| "resume_continue_execute".to_string());
    obj.insert("source".to_string(), Value::String(resume_source));
    obj.insert(
        "resume_user_text".to_string(),
        Value::String(user_text.to_string()),
    );
    obj.insert(
        "failed_resume_context_ts".to_string(),
        Value::from(candidate.failed_ts),
    );
    obj.insert(
        "has_newer_successful_ask_after_failed_task".to_string(),
        Value::Bool(candidate.has_newer_successful_ask_after_failed_task),
    );
    obj.insert(
        "resume_followup_decision".to_string(),
        Value::String(
            match followup_intent.decision {
                crate::intent_router::ResumeFollowupDecision::Resume => "resume",
                crate::intent_router::ResumeFollowupDecision::Abandon => "abandon",
                crate::intent_router::ResumeFollowupDecision::Defer => "defer",
            }
            .to_string(),
        ),
    );
    obj.insert("resume_context".to_string(), candidate.resume_context);
    Some(())
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
        let worker_timeout_secs = state.worker_task_timeout_seconds.max(1);
        let heartbeat_stop = start_task_heartbeat(state.clone(), task.task_id.clone());
        let task_result = tokio::time::timeout(Duration::from_secs(worker_timeout_secs), async {
            match task.kind.as_str() {
                "ask" => {
                    process_ask_task(state, &task, &mut payload).await?;
                }
                "run_skill" => {
                    process_run_skill_task(state, &task, &payload).await?;
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
            Ok::<(), anyhow::Error>(())
        })
        .await;
        let _ = heartbeat_stop.send(());

        match task_result {
            Ok(inner) => inner?,
            Err(_) => {
                let timeout_err = format!(
                    "worker timeout after {}s while processing kind={}",
                    worker_timeout_secs, task_kind_for_timeout_log
                );
                error!(
                    "worker_once timeout: task_id={} kind={} timeout_seconds={}",
                    task.task_id, task_kind_for_timeout_log, worker_timeout_secs
                );
                crate::update_task_timeout(state, &task.task_id, &timeout_err)?;
                maybe_notify_schedule_result(state, &task, &payload, false, &timeout_err).await;
                info!("{}", crate::LOG_CALL_WRAP);
                info!(
                    "task_call_end task_id={} kind={} status=timeout error={}",
                    task.task_id,
                    task_kind_for_timeout_log,
                    crate::truncate_for_log(&timeout_err)
                );
                info!("{}", crate::LOG_CALL_WRAP);
            }
        }
        Ok(())
    }
    .instrument(call_span)
    .await
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
    let original_prompt = payload
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let _ = maybe_bind_recent_failed_resume_context(state, task, payload, &original_prompt).await;
    let prompt = payload
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let source = payload.get("source").and_then(|v| v.as_str()).unwrap_or("");
    let is_schedule_triggered = payload
        .get("schedule_triggered")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let schedule_task_mode = payload
        .get("schedule_task_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let schedule_force_agent = payload
        .get("schedule_force_agent")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let schedule_direct_text_mode = is_schedule_triggered
        && !schedule_force_agent
        && (schedule_task_mode.is_empty() || schedule_task_mode == "direct_text");
    if schedule_direct_text_mode {
        let direct_text = prompt.trim();
        if !direct_text.is_empty() {
            let answer_text = crate::intercept_response_text_for_delivery(direct_text);
            let result = json!({ "text": answer_text.clone() });
            repo::update_task_success(state, &task.task_id, &result.to_string())?;
            maybe_notify_schedule_result(state, task, payload, true, &answer_text).await;
            let _ = crate::memory::service::insert_memory(
                state,
                task.user_id,
                task.chat_id,
                task.user_key.as_deref(),
                &task.channel,
                task.external_chat_id.as_deref(),
                "user",
                prompt,
                state.memory.item_max_chars.max(256),
            );
            let _ = crate::memory::service::insert_memory(
                state,
                task.user_id,
                task.chat_id,
                task.user_key.as_deref(),
                &task.channel,
                task.external_chat_id.as_deref(),
                "assistant",
                &answer_text,
                state.memory.item_max_chars.max(256),
            );
            info!("{}", crate::LOG_CALL_WRAP);
            info!(
                "task_call_end task_id={} kind=ask status=success path=schedule_direct_text result={}",
                task.task_id,
                crate::truncate_for_log(&answer_text)
            );
            info!("{}", crate::LOG_CALL_WRAP);
            return Ok(());
        }
    }

    let main_rules = crate::main_flow_rules(state);
    let is_resume_continue = is_resume_continue_source(main_rules, source);
    let (now_iso, timezone_str, schedule_rules) =
        schedule_service::schedule_context_for_normalizer(state);
    let resume_context_opt = if is_resume_continue {
        payload.get("resume_context").cloned()
    } else {
        None
    };
    let binding_context_json = json!({
        "source": "resume_continue_source",
        "failed_resume_context_ts": Value::Null,
        "has_newer_successful_ask_after_failed_task": false,
    });
    let normalizer_out = crate::intent_router::run_intent_normalizer(
        state,
        task,
        prompt,
        resume_context_opt.as_ref(),
        Some(&binding_context_json),
        &now_iso,
        &timezone_str,
        &schedule_rules,
    )
    .await;
    let resume_should_apply_context = is_resume_continue
        && normalizer_out.resume_behavior == crate::intent_router::ResumeBehavior::ResumeExecute;
    let resume_should_discuss_context = is_resume_continue
        && normalizer_out.resume_behavior == crate::intent_router::ResumeBehavior::ResumeDiscuss;
    info!(
        "worker_once: ask raw_message task_id={} user_id={} chat_id={} text={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        crate::truncate_for_log(prompt)
    );
    let runtime_prompt = if resume_should_apply_context {
        crate::build_resume_continue_execute_prompt(state, payload, prompt)
    } else if resume_should_discuss_context {
        crate::build_resume_followup_discussion_prompt(state, payload, prompt)
    } else {
        normalizer_out.resolved_user_intent.clone()
    };
    info!(
        "worker_once: ask received_message task_id={} user_id={} chat_id={} text={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        crate::truncate_for_log(&runtime_prompt)
    );
    let agent_mode = payload
        .get("agent_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let direct_resume_execution = is_resume_continue && resume_should_apply_context;
    let direct_resume_discussion = is_resume_continue && resume_should_discuss_context;
    let context_resolution = crate::intent_router::ContextResolution {
        resolved_user_intent: runtime_prompt.clone(),
        needs_clarify: normalizer_out.needs_clarify,
        confidence: Some(normalizer_out.confidence),
        reason: normalizer_out.reason.clone(),
    };
    let resolved_prompt = context_resolution.resolved_user_intent.clone();
    info!(
        "{} worker_once: ask resolved_message task_id={} needs_clarify={} confidence={} reason={} resolved_text={}",
        crate::highlight_tag("routing"),
        task.task_id,
        context_resolution.needs_clarify,
        context_resolution.confidence.unwrap_or(-1.0),
        crate::truncate_for_log(&context_resolution.reason),
        crate::truncate_for_log(&resolved_prompt)
    );
    let chat_memory_budget_chars =
        crate::dynamic_chat_memory_budget_chars(state, task, &resolved_prompt);
    let memory_ctx = crate::memory::service::prepare_prompt_with_memory(
        state,
        task,
        &resolved_prompt,
        chat_memory_budget_chars,
    );
    let long_term_summary = memory_ctx.long_term_summary;
    let preferences = memory_ctx.preferences;
    let recalled = memory_ctx.recalled;
    let similar_triggers = memory_ctx.similar_triggers;
    let relevant_facts = memory_ctx.relevant_facts;
    let recent_related_events = memory_ctx.recent_related_events;
    let prompt_with_memory = memory_ctx.prompt_with_memory;
    let mut chat_prompt_context = memory_ctx.chat_prompt_context;
    let mut resolved_prompt_for_execution = resolved_prompt.clone();
    let mut prompt_with_memory_for_execution = prompt_with_memory.clone();
    let last_turn_full_context = crate::memory::build_last_turn_full_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        1200,
        2400,
    );
    if last_turn_full_context != "<none>" {
        prompt_with_memory_for_execution.push_str("\n\n");
        prompt_with_memory_for_execution.push_str(&last_turn_full_context);
        chat_prompt_context.push_str("\n\n");
        chat_prompt_context.push_str(&last_turn_full_context);
    }
    let recent_execution_anchor_context =
        crate::routing_context::build_recent_execution_anchor_context(state, task);
    if recent_execution_anchor_context != "<none>" {
        prompt_with_memory_for_execution.push_str(
            "\n\n### RECENT_EXECUTION_CONTEXT\n\
Use this block as the primary anchor for short follow-up requests. If the current request does not explicitly name a new target, continue from this latest successful subject/domain instead of switching to older memory.\n",
        );
        prompt_with_memory_for_execution.push_str(&recent_execution_anchor_context);
    }
    if let Some(image_context) =
        crate::analyze_attached_images_for_ask(state, task, payload, &resolved_prompt).await?
    {
        let trimmed_image_context = image_context.trim();
        if !trimmed_image_context.is_empty() {
            let image_context_block = format!(
                "\n\nAttached image analysis context:\n{}",
                trimmed_image_context
            );
            resolved_prompt_for_execution.push_str(&image_context_block);
            prompt_with_memory_for_execution.push_str(&image_context_block);
        }
    }
    let long_term_log = long_term_summary
        .as_deref()
        .map(crate::truncate_for_log)
        .unwrap_or_else(|| "<none>".to_string());
    let recalled_log = if recalled.is_empty() {
        "<none>".to_string()
    } else {
        let merged = recalled
            .iter()
            .map(|(role, content)| format!("{role}:{content}"))
            .collect::<Vec<_>>()
            .join(" | ");
        crate::truncate_for_log(&merged)
    };
    let preferences_log = if preferences.is_empty() {
        "<none>".to_string()
    } else {
        let merged = preferences
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" | ");
        crate::truncate_for_log(&merged)
    };
    let trigger_log = if similar_triggers.is_empty() {
        "<none>".to_string()
    } else {
        crate::truncate_for_log(
            &similar_triggers
                .iter()
                .map(|v| v.text.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        )
    };
    let fact_log = if relevant_facts.is_empty() {
        "<none>".to_string()
    } else {
        crate::truncate_for_log(
            &relevant_facts
                .iter()
                .map(|v| v.text.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        )
    };
    let related_log = if recent_related_events.is_empty() {
        "<none>".to_string()
    } else {
        crate::truncate_for_log(
            &recent_related_events
                .iter()
                .map(|v| v.text.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        )
    };
    info!(
        "worker_once: ask memory task_id={} memory.long_term_summary={} memory.preferences={} memory.similar_triggers={} memory.relevant_facts={} memory.related_events={} memory.recalled_recent_count={} memory.recalled_recent={}",
        task.task_id,
        long_term_log,
        preferences_log,
        trigger_log,
        fact_log,
        related_log,
        recalled.len(),
        recalled_log,
    );

    let classifier_direct_mode = crate::main_flow_rules(state)
        .classifier_direct_sources
        .iter()
        .any(|s| s == &source.to_ascii_lowercase());
    let force_clarify = context_resolution.needs_clarify;
    let has_schedule_intent =
        normalizer_out.schedule_kind != crate::intent_router::ScheduleKind::None;
    let should_route_schedule_direct =
        has_schedule_intent && !direct_resume_execution && !direct_resume_discussion;

    let result = if force_clarify {
        let clarify = crate::intent_router::generate_clarify_question(
            state,
            task,
            &resolved_prompt_for_execution,
            &context_resolution.reason,
        )
        .await;
        Ok(crate::AskReply::non_llm(clarify))
    } else if direct_resume_discussion {
        let resume_prompt_file = crate::resolve_prompt_rel_path_for_vendor(
            &state.workspace_root,
            &crate::active_prompt_vendor_name(state),
            crate::RESUME_FOLLOWUP_DISCUSSION_PROMPT_PATH,
        );
        crate::log_prompt_render(
            &task.task_id,
            "resume_followup_discussion_prompt",
            &resume_prompt_file,
            None,
        );
        crate::llm_gateway::run_with_fallback_with_prompt_file(
            state,
            task,
            &resolved_prompt_for_execution,
            &resume_prompt_file,
        )
        .await
        .map(|s| crate::AskReply::llm(s.trim().to_string()))
    } else if direct_resume_execution {
        crate::agent_engine::run_agent_with_tools(
            state,
            task,
            &prompt_with_memory_for_execution,
            &resolved_prompt_for_execution,
        )
        .await
    } else if should_route_schedule_direct {
        if let Ok(Some(schedule_reply)) = crate::intent_router::try_handle_schedule_request(
            state,
            task,
            &resolved_prompt_for_execution,
        )
        .await
        {
            let schedule_reply = crate::intercept_response_text_for_delivery(&schedule_reply);
            let result = json!({ "text": schedule_reply.clone() });
            repo::update_task_success(state, &task.task_id, &result.to_string())?;
            let _ = crate::memory::service::insert_memory(
                state,
                task.user_id,
                task.chat_id,
                task.user_key.as_deref(),
                &task.channel,
                task.external_chat_id.as_deref(),
                "user",
                prompt,
                state.memory.item_max_chars.max(256),
            );
            let _ = crate::memory::service::insert_memory(
                state,
                task.user_id,
                task.chat_id,
                task.user_key.as_deref(),
                &task.channel,
                task.external_chat_id.as_deref(),
                "assistant",
                &schedule_reply,
                state.memory.item_max_chars.max(256),
            );
            info!("{}", crate::LOG_CALL_WRAP);
            info!(
                "task_call_end task_id={} kind=ask status=success path=schedule_direct result={}",
                task.task_id,
                crate::truncate_for_log(&schedule_reply)
            );
            info!("{}", crate::LOG_CALL_WRAP);
            return Ok(());
        }
        if classifier_direct_mode && !resume_should_discuss_context {
            crate::log_prompt_render(
                &task.task_id,
                "classifier_direct",
                "prompts/classifier_direct.md",
                None,
            );
            crate::llm_gateway::run_with_fallback_with_prompt_file(
                state,
                task,
                &resolved_prompt_for_execution,
                "prompts/classifier_direct.md",
            )
            .await
            .map(|s| crate::AskReply::llm(s.trim().to_string()))
            .map_err(|e| e.to_string())
        } else {
            crate::execute_ask_routed(
                state,
                task,
                &chat_prompt_context,
                &prompt_with_memory_for_execution,
                &resolved_prompt_for_execution,
                agent_mode,
                resume_should_discuss_context,
                Some(normalizer_out.routed_mode),
            )
            .await
        }
    } else if classifier_direct_mode {
        crate::log_prompt_render(
            &task.task_id,
            "classifier_direct",
            "prompts/classifier_direct.md",
            None,
        );
        crate::llm_gateway::run_with_fallback_with_prompt_file(
            state,
            task,
            &resolved_prompt_for_execution,
            "prompts/classifier_direct.md",
        )
        .await
        .map(|s| crate::AskReply::llm(s.trim().to_string()))
        .map_err(|e| e.to_string())
    } else {
        crate::execute_ask_routed(
            state,
            task,
            &chat_prompt_context,
            &prompt_with_memory_for_execution,
            &resolved_prompt_for_execution,
            agent_mode,
            false,
            Some(normalizer_out.routed_mode),
        )
        .await
    };

    match result {
        Ok(answer) => {
            if !repo::is_task_still_running(state, &task.task_id)? {
                info!(
                    "task_call_end task_id={} kind=ask status=canceled path=normal",
                    task.task_id
                );
                return Ok(());
            }
            let (answer_text, answer_messages) =
                crate::intercept_response_payload_for_delivery(state, answer.text, answer.messages);
            let result = if answer_messages.is_empty() {
                json!({ "text": answer_text.clone() })
            } else {
                json!({ "text": answer_text.clone(), "messages": answer_messages })
            };
            repo::update_task_success(state, &task.task_id, &result.to_string())?;
            maybe_notify_schedule_result(state, task, payload, true, &answer_text).await;
            let _ = crate::memory::service::insert_memory(
                state,
                task.user_id,
                task.chat_id,
                task.user_key.as_deref(),
                &task.channel,
                task.external_chat_id.as_deref(),
                "user",
                prompt,
                state.memory.item_max_chars.max(256),
            );
            let assistant_memory_text =
                if answer.is_llm_reply && state.memory.mark_llm_reply_in_short_term {
                    format!(
                        "{}{}",
                        crate::memory::LLM_SHORT_TERM_MEMORY_PREFIX,
                        answer_text
                    )
                } else {
                    answer_text.clone()
                };
            let _ = crate::memory::service::insert_memory(
                state,
                task.user_id,
                task.chat_id,
                task.user_key.as_deref(),
                &task.channel,
                task.external_chat_id.as_deref(),
                "assistant",
                &assistant_memory_text,
                state.memory.item_max_chars.max(256),
            );
            spawn_long_term_summary_refresh(state.clone(), task.clone());
            info!("{}", crate::LOG_CALL_WRAP);
            info!(
                "task_call_end task_id={} kind=ask status=success path=normal result={}",
                task.task_id,
                crate::truncate_for_log(&answer_text)
            );
            info!("{}", crate::LOG_CALL_WRAP);
        }
        Err(err_text) => {
            if err_text == crate::agent_engine::TASK_CANCELED_ERR
                || !repo::is_task_still_running(state, &task.task_id)?
            {
                info!(
                    "task_call_end task_id={} kind=ask status=canceled path=normal",
                    task.task_id
                );
                return Ok(());
            }
            if let Some((user_error, resume_ctx)) = crate::parse_resume_context_error(&err_text) {
                let resume_payload = resume_ctx
                    .get("resume_context")
                    .cloned()
                    .unwrap_or(resume_ctx);
                let result = json!({
                    "text": user_error.clone(),
                    "resume_context": resume_payload,
                });
                repo::update_task_failure_with_result(
                    state,
                    &task.task_id,
                    &result.to_string(),
                    &user_error,
                )?;
                maybe_notify_schedule_result(state, task, payload, false, &user_error).await;
                info!("{}", crate::LOG_CALL_WRAP);
                info!(
                    "task_call_end task_id={} kind=ask status=failed path=normal error={} resume_context=true",
                    task.task_id,
                    crate::truncate_for_log(&user_error)
                );
                info!("{}", crate::LOG_CALL_WRAP);
                return Ok(());
            }
            error!(
                "worker_once: ask task_id={} failed: {}",
                task.task_id, err_text
            );
            repo::update_task_failure(state, &task.task_id, &err_text)?;
            maybe_notify_schedule_result(state, task, payload, false, &err_text).await;
            info!("{}", crate::LOG_CALL_WRAP);
            info!(
                "task_call_end task_id={} kind=ask status=failed path=normal error={}",
                task.task_id,
                crate::truncate_for_log(&err_text)
            );
            info!("{}", crate::LOG_CALL_WRAP);
        }
    }

    Ok(())
}

pub(crate) async fn process_run_skill_task(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
) -> anyhow::Result<()> {
    let skill_name = payload
        .get("skill_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let args = payload.get("args").cloned().unwrap_or_else(|| json!(""));

    info!(
        "worker_once: processing run_skill task_id={} user_id={} chat_id={} skill_name={} args={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        skill_name,
        crate::truncate_for_log(&args.to_string())
    );

    match crate::run_skill_with_runner_outcome(state, task, skill_name, args).await {
        Ok(outcome) => {
            if !repo::is_task_still_running(state, &task.task_id)? {
                info!(
                    "task_call_end task_id={} kind=run_skill status=canceled skill={}",
                    task.task_id, skill_name
                );
                return Ok(());
            }
            let clean_text = crate::intercept_response_text_for_delivery(&outcome.text);
            let result = json!({
                "text": clean_text,
                "delivery_meta": {
                    "mode": "single_step_skill",
                    "label": "step",
                    "skill_name": skill_name
                }
            });
            repo::update_task_success(state, &task.task_id, &result.to_string())?;
            if outcome.notify.unwrap_or(true) {
                maybe_notify_schedule_result(state, task, payload, true, &clean_text).await;
            }
            let _ = crate::memory::service::insert_memory(
                state,
                task.user_id,
                task.chat_id,
                task.user_key.as_deref(),
                &task.channel,
                task.external_chat_id.as_deref(),
                "assistant",
                &clean_text,
                state.memory.item_max_chars.max(256),
            );
            let _ = repo::insert_audit_log(
                state,
                Some(task.user_id),
                "run_skill",
                Some(
                    &json!({
                        "task_id": task.task_id,
                        "chat_id": task.chat_id,
                        "skill_name": skill_name,
                        "status": "ok"
                    })
                    .to_string(),
                ),
                None,
            );
            info!("{}", crate::LOG_CALL_WRAP);
            info!(
                "task_call_end task_id={} kind=run_skill status=success skill={} result={}",
                task.task_id,
                skill_name,
                crate::truncate_for_log(&clean_text)
            );
            info!("{}", crate::LOG_CALL_WRAP);
        }
        Err(err_text) => {
            if !repo::is_task_still_running(state, &task.task_id)? {
                info!(
                    "task_call_end task_id={} kind=run_skill status=canceled skill={}",
                    task.task_id, skill_name
                );
                return Ok(());
            }
            error!(
                "worker_once: run_skill task_id={} skill={} failed: {}",
                task.task_id, skill_name, err_text
            );
            repo::update_task_failure(state, &task.task_id, &err_text)?;
            maybe_notify_schedule_result(state, task, payload, false, &err_text).await;
            let action = if err_text.contains("timeout") {
                "timeout"
            } else {
                "run_skill"
            };
            let _ = repo::insert_audit_log(
                state,
                Some(task.user_id),
                action,
                Some(
                    &json!({
                        "task_id": task.task_id,
                        "chat_id": task.chat_id,
                        "skill_name": skill_name,
                        "status": "failed"
                    })
                    .to_string(),
                ),
                Some(&err_text),
            );
            info!("{}", crate::LOG_CALL_WRAP);
            info!(
                "task_call_end task_id={} kind=run_skill status=failed skill={} error={}",
                task.task_id,
                skill_name,
                crate::truncate_for_log(&err_text)
            );
            info!("{}", crate::LOG_CALL_WRAP);
        }
    }

    Ok(())
}

async fn send_task_channel_message(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    text: &str,
) -> Result<(), String> {
    match runtime_channel_from_payload(state, payload) {
        crate::RuntimeChannel::Telegram => {
            let target_chat_id = task_external_chat_id(task)
                .or_else(|| {
                    payload
                        .get("external_chat_id")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                })
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(task.chat_id);
            crate::channel_send::send_telegram_message(state, target_chat_id, text).await
        }
        crate::RuntimeChannel::Whatsapp => {
            let to = task_external_chat_id(task)
                .or_else(|| {
                    payload
                        .get("external_chat_id")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                })
                .ok_or_else(|| "missing external_chat_id for whatsapp task".to_string())?;
            match resolve_whatsapp_delivery_route(state, payload) {
                crate::WhatsappDeliveryRoute::WebBridge => {
                    crate::channel_send::send_whatsapp_web_bridge_text_message(state, &to, text)
                        .await
                }
                crate::WhatsappDeliveryRoute::Cloud => {
                    crate::channel_send::send_whatsapp_cloud_text_message(state, &to, text).await
                }
            }
        }
        crate::RuntimeChannel::Wechat => {
            let to_user_id = task_external_chat_id(task)
                .or_else(|| {
                    payload
                        .get("external_chat_id")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                })
                .ok_or_else(|| "missing external_chat_id for wechat task".to_string())?;
            let context_token = payload
                .get("context_token")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty());
            crate::channel_send::send_wechat_text_message(state, &to_user_id, context_token, text)
                .await
        }
        crate::RuntimeChannel::Feishu => {
            let receive_id = task_external_chat_id(task)
                .or_else(|| {
                    payload
                        .get("external_chat_id")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                })
                .ok_or_else(|| "missing external_chat_id for feishu task".to_string())?;
            crate::channel_send::send_feishu_text_message(state, &receive_id, text).await
        }
        crate::RuntimeChannel::Lark => {
            let receive_id = task_external_chat_id(task)
                .or_else(|| {
                    payload
                        .get("external_chat_id")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                })
                .ok_or_else(|| "missing external_chat_id for lark task".to_string())?;
            crate::channel_send::send_lark_text_message(state, &receive_id, text).await
        }
    }
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
        assert_eq!(payload.get("channel").and_then(|v| v.as_str()), Some("wechat"));
        assert_eq!(
            payload.get("context_token").and_then(|v| v.as_str()),
            Some("ctx-123")
        );
    }
}

fn cleanup_once(state: &AppState) -> anyhow::Result<()> {
    let db = state.db.lock().map_err(|_| anyhow!("db lock poisoned"))?;

    let now = now_ts_u64() as i64;

    let task_cutoff = now - (state.maintenance.tasks_retention_days as i64 * 86400);
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
        rusqlite::params![state.maintenance.tasks_max_rows as i64],
    )?;

    let audit_cutoff = now - (state.maintenance.audit_retention_days as i64 * 86400);
    db.execute(
        "DELETE FROM audit_logs WHERE CAST(ts AS INTEGER) < ?1",
        rusqlite::params![audit_cutoff],
    )?;

    db.execute(
        "DELETE FROM audit_logs WHERE id IN (
             SELECT id FROM audit_logs
             ORDER BY id DESC
             LIMIT -1 OFFSET ?1
         )",
        rusqlite::params![state.maintenance.audit_max_rows as i64],
    )?;

    let memory_cutoff = now - (state.memory.retention_days as i64 * 86400);
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
        rusqlite::params![state.memory.max_rows as i64],
    )?;
    if state.memory.hybrid_recall_enabled {
        let index_max_rows = state.memory.max_rows.saturating_mul(3).max(2000);
        crate::memory::indexing::cleanup_retrieval_index(&db, memory_cutoff, index_max_rows)?;
    }

    let long_term_cutoff = now - (state.memory.long_term_retention_days as i64 * 86400);
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
        rusqlite::params![state.memory.long_term_max_rows as i64],
    )?;

    Ok(())
}

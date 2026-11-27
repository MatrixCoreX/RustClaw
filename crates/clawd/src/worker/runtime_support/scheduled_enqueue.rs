const SCHEDULE_JOB_SELECT: &str = "SELECT job_id, user_id, chat_id, user_key, principal_id, channel, external_user_id, external_chat_id, task_kind, task_payload_json, COALESCE(next_run_at,0),
                    schedule_type, time_of_day, weekday, every_minutes, timezone, isolation_profile, permission_policy_json,
                    thread_resume_enabled, last_thread_task_id FROM scheduled_jobs";

fn scheduled_job_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledJobDue> {
    Ok(ScheduledJobDue {
        job_id: row.get(0)?,
        user_id: row.get(1)?,
        chat_id: row.get(2)?,
        user_key: row.get(3)?,
        principal_id: row.get(4)?,
        channel: row.get(5)?,
        external_user_id: row.get(6)?,
        external_chat_id: row.get(7)?,
        task_kind: row.get(8)?,
        task_payload_json: row.get(9)?,
        next_run_at: row.get(10)?,
        schedule_type: row.get(11)?,
        time_of_day: row.get(12)?,
        weekday: row.get(13)?,
        every_minutes: row.get(14)?,
        timezone: row.get(15)?,
        isolation_profile: row.get(16)?,
        permission_policy_json: row.get(17)?,
        thread_resume_enabled: row.get::<_, i64>(18)? != 0,
        last_thread_task_id: row.get(19)?,
    })
}

fn enqueue_scheduled_run(
    tx: &rusqlite::Transaction<'_>,
    job: &ScheduledJobDue,
    run_id: &str,
) -> anyhow::Result<String> {
    let mut payload =
        serde_json::from_str::<Value>(&job.task_payload_json).unwrap_or_else(|_| json!({}));
    // Jobs created before execution-policy inheritance was introduced have
    // no policy stamp. Re-resolve the current key on every wakeup so an
    // enabled admin keeps admin authority while a revoked key cannot.
    stamp_current_admin_policy_for_legacy_scheduled_job(tx, job.user_key.as_deref(), &mut payload)?;
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
    let thread_ref = crate::scheduled_run_contract::scheduled_run_thread_ref(&job.job_id);
    let now_text = now_ts();
    if let Value::Object(map) = &mut payload {
        for (k, v) in schedule_service::schedule_invocation_metadata(&job.job_id, run_id) {
            map.insert(k, v);
        }
    }
    tx.execute(
        "INSERT INTO tasks (task_id, user_id, chat_id, user_key, principal_id, channel, external_user_id, external_chat_id, message_id, kind, payload_json, status, result_json, error_text, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10, 'queued', NULL, NULL, ?11, ?11)",
        rusqlite::params![
            task_id,
            job.user_id,
            job.chat_id,
            job.user_key,
            job.principal_id,
            job.channel,
            job.external_user_id,
            job.external_chat_id,
            job.task_kind,
            payload.to_string(),
            now_text
        ],
    )?;
    crate::scheduled_run_contract::insert_scheduled_run_enqueued(
        tx,
        &crate::scheduled_run_contract::ScheduledRunEnqueued {
            run_id,
            job_id: &job.job_id,
            task_id: &task_id,
            thread_ref: &thread_ref,
            started_at: &now_text,
        },
    )?;
    Ok(task_id)
}

fn active_scheduled_run(db: &rusqlite::Connection, job_id: &str) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT r.task_id FROM scheduled_job_runs r JOIN tasks t ON t.task_id=r.task_id
         WHERE r.job_id=?1 AND t.status IN ('queued','running') ORDER BY r.id DESC LIMIT 1",
        [job_id],
        |row| row.get(0),
    )
    .optional()
}

pub(crate) enum ManualScheduledRun {
    Queued {
        task_id: String,
        run_id: String,
        replayed: bool,
    },
    NotFound,
    Active,
}

pub(crate) fn trigger_scheduled_run(
    state: &AppState,
    identity: &claw_core::types::AuthIdentity,
    job_id: &str,
    request_id: Uuid,
) -> anyhow::Result<ManualScheduledRun> {
    use sha2::{Digest, Sha256};
    let mut db = state.core.db.get()?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let job = tx
        .query_row(
            &format!(
                "{SCHEDULE_JOB_SELECT} WHERE job_id=?1 AND
        (principal_id=?2 OR (principal_id IS NULL AND user_key=?3))"
            ),
            rusqlite::params![job_id, identity.principal_id, identity.user_key],
            scheduled_job_row,
        )
        .optional()?;
    let Some(job) = job else {
        return Ok(ManualScheduledRun::NotFound);
    };
    let scope = serde_json::to_vec(&(&identity.principal_id, job_id, request_id.to_string()))?;
    let run_id = format!("manual_{:x}", Sha256::digest(scope));
    if let Some(task_id) = tx
        .query_row(
            "SELECT task_id FROM scheduled_job_runs WHERE run_id=?1 AND job_id=?2",
            rusqlite::params![run_id, job_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(ManualScheduledRun::Queued {
            task_id,
            run_id,
            replayed: true,
        });
    }
    if active_scheduled_run(&tx, job_id)?.is_some() {
        return Ok(ManualScheduledRun::Active);
    }
    let task_id = enqueue_scheduled_run(&tx, &job, &run_id)?;
    tx.commit()?;
    Ok(ManualScheduledRun::Queued {
        task_id,
        run_id,
        replayed: false,
    })
}

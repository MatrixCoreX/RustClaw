//! Repair only the legacy unavailable-model enqueue bug, never an uncertain executed job.
use super::*;

pub(crate) fn list(
    state: &crate::AppState,
    principal: &str,
) -> anyhow::Result<Vec<MemoryJobSnapshot>> {
    let db = state.core.db.get()?;
    let ids = {
        let mut stmt = db.prepare("SELECT job_id FROM memory_jobs WHERE principal_id = ?1 ORDER BY updated_at_ts DESC, job_id LIMIT 100")?;
        let rows = stmt
            .query_map([principal], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    ids.into_iter()
        .map(|id| {
            load_job(&db, &id)?.ok_or_else(|| permanent("memory_job_source_task_unavailable"))
        })
        .collect()
}

pub(crate) fn retry_unavailable(
    state: &crate::AppState,
    principal: &str,
) -> anyhow::Result<serde_json::Value> {
    let mut requeued = 0;
    let mut blocked = 0;
    let jobs = {
        let db = state.core.db.get()?;
        let mut stmt = db.prepare("SELECT job_id FROM memory_jobs WHERE principal_id = ?1 AND status = 'failed'
            AND ((provider_name = 'unavailable' AND provider_type = 'unavailable' AND model_name = 'unavailable')
              OR (error_code = 'memory_job_provider_unavailable'
                AND json_extract(checkpoint_json, '$.legacy_repair.reason_code') = 'legacy_empty_model_binding'))
            AND progress_current = 0 AND cancel_requested = 0 ORDER BY created_at_ts, job_id LIMIT 100")?;
        let ids = stmt
            .query_map([principal], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                load_job(&db, &id)?.ok_or_else(|| permanent("memory_job_source_task_unavailable"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?
    };
    for mut job in jobs {
        if job.status != "failed" || job.cancel_requested {
            continue;
        }
        let db = state.core.db.get()?;
        let progress: i64 = db.query_row(
            "SELECT progress_current FROM memory_jobs WHERE job_id = ?1",
            [&job.job_id],
            |row| row.get(0),
        )?;
        drop(db);
        if progress != 0 {
            continue;
        }
        // Reconstruct the authentic source payload; retain any original explicit model choice.
        job.provider_type.clear();
        job.model_name.clear();
        let repair = (|| -> anyhow::Result<_> {
            let task = load_source_task(state, &job)?;
            let source_db = state.core.db.get()?;
            sources::verify_legacy_job(&source_db, &task, &job)?;
            drop(source_db);
            let settings = super::super::settings::resolve_task_memory_settings(state, &task)?
                .ok_or_else(|| permanent("memory_job_settings_unavailable"))?;
            anyhow::ensure!(
                settings.target_principal_id == principal,
                permanent("memory_job_source_principal_mismatch")
            );
            anyhow::ensure!(
                settings.generate_memory,
                permanent("memory_job_generation_revoked")
            );
            anyhow::ensure!(
                settings.revision == job.settings_revision
                    && settings.policy_digest == job.policy_digest,
                permanent("memory_job_settings_snapshot_changed")
            );
            let db = state.core.db.get()?;
            super::super::retention::ensure_principal_quota(
                &db,
                &state.policy.memory,
                principal,
                crate::now_ts_u64() as i64,
            )?;
            anyhow::ensure!(
                super::super::retention::automatic_generation_allowed(&db, principal)?,
                permanent("memory_job_quota_blocked")
            );
            drop(db);
            anyhow::ensure!(
                !load_user_source_bundle(state, &job)?.trim().is_empty(),
                permanent("memory_job_source_range_missing")
            );
            let snapshot = provider_snapshot_for_job(state, &task, &settings, &job.job_kind)?;
            let mut pinned = job.clone();
            pinned.provider_name = snapshot.0.clone();
            pinned.provider_type = snapshot.1.clone();
            pinned.model_name = snapshot.2.clone();
            pinned.model_capability_digest = snapshot.3.clone();
            let pinned_task = load_source_task(state, &pinned)?;
            validate_pinned_model(state, &pinned_task, &pinned)?;
            Ok(snapshot)
        })();
        let db = state.core.db.get()?;
        let now = crate::now_ts_u64() as i64;
        match repair {
            Ok((name, provider, model, digest)) => {
                requeued += db.execute("UPDATE memory_jobs SET provider_name = ?2, provider_type = ?3,
                    model_name = ?4, model_capability_digest = ?5, status = 'queued', attempt = 0,
                    error_code = NULL, retryable = 1, finished_at_ts = NULL, not_before_ts = ?6,
                    updated_at_ts = ?6, checkpoint_json = json_set(checkpoint_json,
                        '$.legacy_repair', json(COALESCE(json_extract(checkpoint_json, '$.legacy_repair'), ?7)),
                        '$.model_binding_repair', json(?7))
                    WHERE job_id = ?1 AND principal_id = ?8 AND status = 'failed' AND cancel_requested = 0
                    AND progress_current = 0
                    AND ((provider_name = 'unavailable' AND provider_type = 'unavailable' AND model_name = 'unavailable')
                      OR (error_code = 'memory_job_provider_unavailable'
                        AND json_extract(checkpoint_json, '$.legacy_repair.reason_code') = 'legacy_empty_model_binding'))",
                    params![job.job_id, name, provider, model, digest, now,
                        json!({"reason_code":if job.provider_name == "unavailable" { "legacy_empty_model_binding" } else { "provider_protocol_binding_corrected" }, "previous_attempts":job.attempt,
                            "previous_error_code":job.error_code, "repaired_at_ts":now}).to_string(), principal])?;
            }
            Err(error) => {
                let failure = JobFailure::from_error(&error);
                blocked += 1;
                db.execute("UPDATE memory_jobs SET error_code = ?2, retryable = 0,
                    checkpoint_json = json_set(checkpoint_json, '$.failure', json(?3)), updated_at_ts = ?4
                    WHERE job_id = ?1 AND principal_id = ?5 AND status = 'failed' AND cancel_requested = 0",
                    params![job.job_id, failure.error_code, serde_json::to_string(&failure)?, now, principal])?;
            }
        }
    }
    Ok(json!({"schema_version":1,"requeued_jobs":requeued,"blocked_jobs":blocked}))
}

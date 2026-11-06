//! Durable response staging for background memory generation. An interrupted
//! provider request is uncertain, not permission to repeat the model call.
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::job_completion::{phase, GenerationLease, JobPhase};

fn digest(text: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(text.as_bytes()))
}

pub(super) async fn run(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    prompt_source: &str,
    generation: Option<GenerationLease<'_>>,
) -> anyhow::Result<String> {
    let Some(lease) = generation else {
        return crate::llm_gateway::run_with_fallback_with_prompt_source(
            state,
            task,
            prompt,
            prompt_source,
        )
        .await
        .map_err(anyhow::Error::msg);
    };
    let prompt_digest = digest(&serde_json::to_string(&(prompt_source, prompt))?);
    let resource;
    {
        let mut db = state.core.db.get()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let checkpoint: String = tx.query_row(
            "SELECT checkpoint_json FROM memory_jobs
            WHERE job_id = ?1 AND principal_id = ?2 AND status = 'running'
              AND lease_owner = ?3 AND cancel_requested = 0",
            params![lease.job_id, lease.principal_id, lease.worker_id],
            |row| row.get(0),
        )?;
        let cached: Option<(String, String, String)> = tx.query_row(
            "SELECT response_text, response_digest, prompt_digest FROM memory_generation_outputs
             WHERE job_id = ?1 AND principal_id = ?2",
            params![lease.job_id, lease.principal_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
        if let Some((response, response_digest, original_prompt)) = cached {
            anyhow::ensure!(
                phase(&checkpoint) == JobPhase::ResponsePersisted,
                "memory_job_response_checkpoint_mismatch"
            );
            anyhow::ensure!(
                response_digest == digest(&response),
                "memory_job_response_digest_mismatch"
            );
            anyhow::ensure!(
                original_prompt == prompt_digest,
                "memory_job_response_prompt_changed"
            );
            return Ok(response);
        }
        anyhow::ensure!(
            matches!(
                phase(&checkpoint),
                JobPhase::Unstarted | JobPhase::SourceVerified | JobPhase::ResourceWait
            ),
            "memory_job_provider_result_unavailable"
        );
        resource = match crate::llm_gateway::acquire_llm_resource_lease(state, task) {
            Ok(resource) => resource,
            Err(error) => {
                let changed = tx.execute(
                    "UPDATE memory_jobs SET checkpoint_json = ?1,
                    updated_at_ts = ?2 WHERE job_id = ?3 AND principal_id = ?4
                    AND status = 'running' AND lease_owner = ?5 AND cancel_requested = 0",
                    params![
                        json!({"schema_version":1,"phase":"resource_wait"}).to_string(),
                        crate::now_ts_u64() as i64,
                        lease.job_id,
                        lease.principal_id,
                        lease.worker_id
                    ],
                )?;
                anyhow::ensure!(changed == 1, "memory_job_resource_wait_lease_lost");
                tx.commit()?;
                return Err(anyhow::Error::msg(error));
            }
        };
        let changed = tx.execute(
            "UPDATE memory_jobs SET checkpoint_json = ?1, updated_at_ts = ?2
            WHERE job_id = ?3 AND principal_id = ?4 AND status = 'running'
              AND lease_owner = ?5 AND cancel_requested = 0",
            params![
                json!({"schema_version":1,"phase":"provider_request_started",
                "prompt_digest":prompt_digest})
                .to_string(),
                crate::now_ts_u64() as i64,
                lease.job_id,
                lease.principal_id,
                lease.worker_id
            ],
        )?;
        anyhow::ensure!(changed == 1, "memory_job_provider_lease_lost");
        tx.commit()?;
    }
    // No database connection, transaction or wall-clock task timer is held
    // across the provider call. The worker continues renewing its own lease.
    let response = crate::llm_gateway::run_with_preacquired_resource(
        state,
        task,
        prompt,
        prompt_source,
        resource,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    let (safe_response, _) =
        crate::skill_output_artifact::sensitivity_aware_text_model_view(&response);
    {
        let mut db = state.core.db.get()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO memory_generation_outputs(job_id, principal_id,
            response_text, response_digest, prompt_digest, created_at_ts)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                lease.job_id,
                lease.principal_id,
                safe_response,
                digest(&safe_response),
                prompt_digest,
                crate::now_ts_u64() as i64
            ],
        )?;
        let changed = tx.execute(
            "UPDATE memory_jobs SET checkpoint_json = ?1, updated_at_ts = ?2
            WHERE job_id = ?3 AND principal_id = ?4 AND status = 'running'
              AND lease_owner = ?5 AND cancel_requested = 0",
            params![
                json!({"schema_version":1,"phase":"provider_response_persisted",
                "prompt_digest":prompt_digest, "response_digest":digest(&safe_response)})
                .to_string(),
                crate::now_ts_u64() as i64,
                lease.job_id,
                lease.principal_id,
                lease.worker_id
            ],
        )?;
        anyhow::ensure!(changed == 1, "memory_job_provider_lease_lost");
        tx.commit()?;
    }
    Ok(safe_response)
}

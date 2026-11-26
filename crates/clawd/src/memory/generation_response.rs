//! Durable response staging for background memory generation. An interrupted
//! provider request is uncertain, not permission to repeat the model call.
use std::sync::Arc;

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::job_completion::{phase, GenerationLease, JobPhase};

pub(super) struct GenerationCall<'a> {
    pub(super) lease: GenerationLease<'a>,
    pub(super) provider: Arc<crate::LlmProviderRuntime>,
}

fn digest(text: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(text.as_bytes()))
}

pub(super) async fn run<T>(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    prompt_source: &str,
    generation: Option<&GenerationCall<'_>>,
    parse: impl Fn(&str) -> Result<T, super::job_failure::JobFailure>,
) -> anyhow::Result<T> {
    let Some(generation) = generation else {
        let (_, parsed) = generate(state, task, prompt, prompt_source, None, None, &parse).await?;
        return Ok(parsed);
    };
    let lease = generation.lease;
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
            if serde_json::from_str::<serde_json::Value>(&checkpoint)?["finish_reason"] != "stop" {
                return Err(super::job_failure::JobFailure::new(
                    "memory_job_finish_unverified",
                    false,
                    "response_restore",
                )
                .into());
            }
            return Ok(parse(&response)?);
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
                    "UPDATE memory_jobs SET checkpoint_json = json_patch(checkpoint_json, ?1),
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
            "UPDATE memory_jobs SET checkpoint_json = json_patch(checkpoint_json, ?1), updated_at_ts = ?2
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
    let (safe_response, parsed) = generate(
        state,
        task,
        prompt,
        prompt_source,
        Some(generation),
        Some(&resource),
        &parse,
    )
    .await?;
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
            "UPDATE memory_jobs SET checkpoint_json = json_patch(checkpoint_json, ?1), updated_at_ts = ?2
            WHERE job_id = ?3 AND principal_id = ?4 AND status = 'running'
              AND lease_owner = ?5 AND cancel_requested = 0",
            params![
                json!({"schema_version":1,"phase":"provider_response_persisted","finish_reason":"stop",
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
    Ok(parsed)
}

async fn generate<T>(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    source: &str,
    generation: Option<&GenerationCall<'_>>,
    resource: Option<&crate::resource_scheduler::ResourceLease>,
    parse: impl Fn(&str) -> Result<T, super::job_failure::JobFailure>,
) -> anyhow::Result<(String, T)> {
    use super::job_failure::JobFailure;
    use claw_core::model_turn::ModelFinishReason;
    let policy =
        crate::task_budget_contract::load_task_budget_policy(&state.skill_rt.workspace_root);
    let mut budget = crate::task_budget_contract::model_output::GenerationBudget::new(
        policy.model_output,
        crate::llm_gateway::classify_prompt_source(source),
    );
    let mut corrected_prompt = None;
    let providers = match generation {
        Some(call) => vec![call.provider.clone()],
        None => state.task_llm_providers(task),
    };
    loop {
        if let Some(call) = generation {
            let lease = call.lease;
            let active: bool = state.core.db.get()?.query_row(
                "SELECT EXISTS(SELECT 1 FROM memory_jobs WHERE job_id=?1 AND principal_id=?2 AND status='running' AND lease_owner=?3 AND cancel_requested=0 AND lease_expires_at_ts>=?4)",
                params![lease.job_id, lease.principal_id, lease.worker_id, crate::now_ts_u64() as i64], |row| row.get(0))?;
            anyhow::ensure!(active, "memory_job_provider_lease_lost");
        }
        let mut hints = budget.hints();
        crate::task_model_selection::apply_task_model_policy_hints(task, &mut hints);
        let (text, finish) = crate::llm_gateway::run_with_provider_resource(
            state,
            task,
            corrected_prompt.as_deref().unwrap_or(prompt),
            source,
            hints,
            providers.clone(),
            resource,
        )
        .await
        .map_err(|error| {
            if let Some(code) = serde_json::from_str::<serde_json::Value>(&error)
                .ok()
                .and_then(|v| v["error_code"].as_str().map(str::to_owned))
            {
                JobFailure::new(&code, false, "provider_admission")
            } else {
                JobFailure::provider(source)
            }
        })?;
        match finish {
            ModelFinishReason::Length if budget.grow() => continue,
            ModelFinishReason::Length => {
                return Err(JobFailure::new(
                    "memory_job_output_limit_exhausted",
                    false,
                    "generation",
                )
                .after_response_rejection()
                .into())
            }
            ModelFinishReason::Stop => (),
            _ => {
                return Err(
                    JobFailure::new("memory_job_finish_unverified", false, "generation")
                        .after_response_rejection()
                        .into(),
                )
            }
        }
        let (safe, _) = crate::skill_output_artifact::sensitivity_aware_text_model_view(&text);
        match parse(&safe) {
            Ok(parsed) => return Ok((safe, parsed)),
            Err(error) if corrected_prompt.is_none() && budget.retry_at_current_limit() => {
                corrected_prompt = Some(format!(
                    "{prompt}\n\n{}",
                    json!({"schema_version":1,"previous_response_rejected":error.error_code,"regenerate_complete_object":true})
                ));
            }
            Err(error) => return Err(error.after_response_rejection().into()),
        }
    }
}

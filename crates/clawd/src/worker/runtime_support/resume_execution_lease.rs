use std::{future::Future, time::Duration};

use crate::{now_ts_u64, repo, AppState};
use tracing::warn;

pub(crate) enum RenewableResumeExecution<T> {
    Completed(T),
    LeaseLost,
}

pub(crate) async fn run_with_renewable_resume_execution_lease<F, T>(
    state: &AppState,
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    lease_seconds: i64,
    execution: F,
) -> anyhow::Result<RenewableResumeExecution<T>>
where
    F: Future<Output = T>,
{
    let lease_seconds = lease_seconds.max(1);
    let mut confirmed_expires_at = claimed
        .lease_expires_at
        .min(claimed.handoff_claim_expires_at)
        .min(claimed.dispatch_claim_expires_at);
    let initial_now = now_ts_u64() as i64;
    match renew_resume_execution_lease(state, claimed, initial_now, lease_seconds).await {
        Ok(true) => confirmed_expires_at = initial_now.saturating_add(lease_seconds),
        Ok(false) => return Ok(RenewableResumeExecution::LeaseLost),
        Err(error) if confirmed_expires_at > initial_now => warn!(
            task_id = %claimed.task_id,
            checkpoint_id = %claimed.checkpoint_id,
            confirmed_expires_at,
            error = %error,
            "resume execution lease initial renewal deferred after transient storage failure"
        ),
        Err(error) => return Err(error),
    }

    let heartbeat_seconds = state.worker.worker_task_heartbeat_seconds.max(5) as i64;
    let interval_seconds = heartbeat_seconds.min((lease_seconds / 3).max(1)) as u64;
    let heartbeat = tokio::time::sleep(Duration::from_secs(interval_seconds));
    tokio::pin!(heartbeat);
    tokio::pin!(execution);

    loop {
        tokio::select! {
            result = &mut execution => {
                return Ok(RenewableResumeExecution::Completed(result));
            }
            _ = &mut heartbeat => {
                let heartbeat_at = now_ts_u64() as i64;
                let next_interval_seconds = match renew_resume_execution_lease(
                    state,
                    claimed,
                    heartbeat_at,
                    lease_seconds,
                ).await {
                    Ok(true) => {
                        confirmed_expires_at = heartbeat_at.saturating_add(lease_seconds);
                        interval_seconds
                    }
                    Ok(false) => return Ok(RenewableResumeExecution::LeaseLost),
                    Err(error) if confirmed_expires_at > heartbeat_at => {
                        warn!(
                            task_id = %claimed.task_id,
                            checkpoint_id = %claimed.checkpoint_id,
                            confirmed_expires_at,
                            error = %error,
                            "resume execution lease renewal deferred after transient storage failure"
                        );
                        interval_seconds.min(2).max(1)
                    }
                    Err(error) => return Err(error),
                };
                heartbeat.as_mut().reset(
                    tokio::time::Instant::now()
                        + Duration::from_secs(next_interval_seconds)
                );
            }
        }
    }
}

async fn renew_resume_execution_lease(
    state: &AppState,
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    now_ts: i64,
    lease_seconds: i64,
) -> anyhow::Result<bool> {
    let mut last_error = None;
    for attempt in 0..3 {
        match repo::renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal(
            state,
            claimed,
            now_ts,
            lease_seconds,
        ) {
            Ok(true) => return Ok(true),
            Ok(false) => last_error = None,
            Err(error) => last_error = Some(error),
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    match last_error {
        Some(error) => Err(error),
        None => Ok(false),
    }
}

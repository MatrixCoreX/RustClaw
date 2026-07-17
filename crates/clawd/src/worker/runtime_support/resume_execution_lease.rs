use std::{future::Future, time::Duration};

use crate::{now_ts_u64, repo, AppState};

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
    if !repo::renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal(
        state,
        claimed,
        now_ts_u64() as i64,
        lease_seconds,
    )? {
        return Ok(RenewableResumeExecution::LeaseLost);
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
                if !repo::renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal(
                    state,
                    claimed,
                    now_ts_u64() as i64,
                    lease_seconds,
                )? {
                    return Ok(RenewableResumeExecution::LeaseLost);
                }
                heartbeat.as_mut().reset(
                    tokio::time::Instant::now() + Duration::from_secs(interval_seconds)
                );
            }
        }
    }
}

use super::*;
use crate::local_process_job::observation_test_support::LiveJobFixture;

fn claimed(
    fixture: &LiveJobFixture,
) -> crate::repo::ClaimedDispatchedPausedCheckpointResumeExecution {
    let mut claimed = async_poll_claimed_dispatch(None);
    let job_id = "local_process:cancellation-observation";
    claimed.execution_plan["job_id"] = json!(job_id);
    claimed.dispatch_payload["job_id"] = json!(job_id);
    let job = claimed.task_checkpoint.pending_async_job.as_mut().unwrap();
    job.job_id = job_id.to_owned();
    job.cancel_ref = format!("local_process:{}", fixture.path().display());
    job.expires_at = 1001;
    fixture.request_cancel();
    fixture.write("retention_seconds", "120");
    claimed
}

fn pending_until_observed_exit(early_exit: bool, missing_pid: bool) {
    let mut fixture = LiveJobFixture::new();
    let claimed = claimed(&fixture);
    if early_exit {
        fixture.write("exit_code", "0");
    }
    if missing_pid {
        std::fs::remove_file(fixture.path().join("pid")).unwrap();
    }
    let payload = super::super::execute_async_poll_dispatch_result(&claimed, 1000, 30).unwrap();
    fixture.finish();
    assert_eq!(payload["adapter_status"], "running");
    assert!(payload.get("cancellation_result_json").is_none());
    assert_eq!(
        payload["process_observation"]["cancellation_requested"],
        true
    );
    assert!(payload["expires_at"].as_i64().unwrap() >= 1120);
    if missing_pid {
        assert_eq!(
            payload["process_observation"]["process_identity_state"],
            "unknown"
        );
    } else {
        assert_eq!(
            payload["process_observation"]["process_identity_state"],
            "alive_verified"
        );
        fixture.write("exit_code", "143");
        let terminal =
            super::super::execute_async_poll_dispatch_result(&claimed, 1001, 30).unwrap();
        assert_eq!(terminal["adapter_status"], "cancelled");
        assert!(terminal.get("cancellation_result_json").is_some());
    }
}

#[test]
fn cancellation_observation_live_process_remains_pending() {
    pending_until_observed_exit(false, false);
}

#[test]
fn cancellation_observation_early_exit_record_does_not_override_liveness() {
    pending_until_observed_exit(true, false);
}

#[test]
fn cancellation_observation_missing_pid_does_not_prove_completion() {
    pending_until_observed_exit(true, true);
}

#[test]
fn cancellation_observation_preserves_output_cursors_until_the_process_exits() {
    let mut fixture = LiveJobFixture::new();
    let claimed = claimed(&fixture);
    fixture.write("stdout", "first\n");
    let first = super::super::execute_async_poll_dispatch_result(&claimed, 1000, 30).unwrap();
    fixture.write("stdout", "first\nsecond\n");
    let second = super::super::execute_async_poll_dispatch_result(&claimed, 1001, 30).unwrap();
    fixture.finish();
    fixture.write("exit_code", "143");
    let terminal = super::super::execute_async_poll_dispatch_result(&claimed, 1002, 30).unwrap();
    assert_eq!(first["executor_result_status"], "async_poll_rescheduled");
    assert_eq!(first["process_observation"]["stdout"], "first\n");
    assert_eq!(first["process_observation"]["stdout_cursor"], 6);
    assert_eq!(second["adapter_status"], "running");
    assert_eq!(second["process_observation"]["stdout"], "second\n");
    assert_eq!(second["process_observation"]["stdout_start_cursor"], 6);
    assert_eq!(second["process_observation"]["stdout_cursor"], 13);
    assert_eq!(terminal["adapter_status"], "cancelled");
    assert_eq!(terminal["cancellation_result_json"]["stdout"], "");
    assert_eq!(terminal["cancellation_result_json"]["stdout_cursor"], 13);
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("stdout")).unwrap(),
        "first\nsecond\n"
    );
}

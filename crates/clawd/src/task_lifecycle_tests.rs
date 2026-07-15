use serde_json::json;

use claw_core::types::TaskExecutionState;

use super::{
    checkpoint_resume_directive, paused_checkpoint_recovery_status,
    paused_checkpoint_resume_readiness, task_execution_state_from_lifecycle,
    task_query_lifecycle_projection, AsyncJobRef, AsyncJobStatus, CheckpointBudgetCounters,
    CheckpointResumeDirective, PausedCheckpointRecoveryStatus, PausedCheckpointResumeReadiness,
    ResumeEntrypoint, ResumeTrigger, TaskCheckpoint, TerminalFailureReason,
};

#[test]
fn lifecycle_projection_marks_paused_checkpoint_resume_due_from_machine_time() {
    let overdue = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "resume_reason": "agent_loop_max_rounds",
            "next_check_after": 1,
            "checkpoint_id": "ckpt-due"
        }
    });
    let due = task_query_lifecycle_projection("running", Some(&overdue), Some(10));
    assert_eq!(due["resume_due"], true);
    assert_eq!(due["resume_wait_seconds"], 0);
    assert_eq!(due["last_heartbeat_ts"], 10);

    let future_ts = crate::now_ts_u64() as i64 + 3600;
    let future = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "async_job_poll",
            "next_check_after": future_ts,
            "checkpoint_id": "ckpt-future"
        }
    });
    let waiting = task_query_lifecycle_projection("running", Some(&future), None);
    assert_eq!(waiting["resume_due"], false);
    assert!(waiting["resume_wait_seconds"].as_i64().unwrap_or_default() > 0);
}

#[test]
fn paused_checkpoint_recovery_status_classifies_machine_checkpoint_states() {
    assert_eq!(
        paused_checkpoint_recovery_status(&json!({"task_lifecycle": {"state": "running"}}), 100,),
        PausedCheckpointRecoveryStatus::NotPaused
    );
    assert_eq!(
        paused_checkpoint_recovery_status(
            &json!({"task_lifecycle": {"state": "waiting", "next_check_after": 200}}),
            100,
        ),
        PausedCheckpointRecoveryStatus::InvalidPausedCheckpoint
    );
    assert_eq!(
        paused_checkpoint_recovery_status(
            &json!({
                "task_lifecycle": {
                    "state": "waiting",
                    "next_check_after": 200,
                    "checkpoint_id": "ckpt-future"
                }
            }),
            100,
        ),
        PausedCheckpointRecoveryStatus::Waiting {
            state: "waiting".to_string(),
            checkpoint_id: "ckpt-future".to_string(),
            resume_due: false,
            resume_wait_seconds: 100,
        }
    );
    assert_eq!(
        paused_checkpoint_recovery_status(
            &json!({
                "task_journal": {
                    "summary": {
                        "task_lifecycle": {
                            "state": "background",
                            "next_check_after": 90,
                            "checkpoint_id": "ckpt-due"
                        }
                    }
                }
            }),
            100,
        ),
        PausedCheckpointRecoveryStatus::Waiting {
            state: "background".to_string(),
            checkpoint_id: "ckpt-due".to_string(),
            resume_due: true,
            resume_wait_seconds: 0,
        }
    );
    assert_eq!(
        paused_checkpoint_recovery_status(
            &json!({
                "task_lifecycle": {
                    "state": "needs_user",
                    "checkpoint_id": "ckpt-user"
                }
            }),
            100,
        ),
        PausedCheckpointRecoveryStatus::Waiting {
            state: "needs_user".to_string(),
            checkpoint_id: "ckpt-user".to_string(),
            resume_due: true,
            resume_wait_seconds: 0,
        }
    );
}

fn checkpoint_value(
    checkpoint_id: &str,
    completed_side_effect_refs: Vec<&str>,
) -> serde_json::Value {
    json!({
        "schema_version": 1,
        "checkpoint_id": checkpoint_id,
        "boundary_context": {"route_gate_kind": "execute"},
        "observations": [],
        "evidence_refs": [],
        "artifact_refs": [],
        "completed_side_effect_refs": completed_side_effect_refs,
        "budget": {
            "round": 1,
            "step": 2,
            "llm_calls": 3,
            "tool_calls": 4,
            "elapsed_ms": 500
        },
        "resume_entrypoint": "next_planner_round"
    })
}

fn checkpoint_value_with_entrypoint(
    checkpoint_id: &str,
    resume_entrypoint: &str,
    pending_async_job: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut value = checkpoint_value(checkpoint_id, vec![]);
    value["resume_entrypoint"] = json!(resume_entrypoint);
    if let Some(job) = pending_async_job {
        value["pending_async_job"] = job;
    }
    value
}

#[test]
fn paused_checkpoint_resume_readiness_requires_due_matching_checkpoint() {
    assert_eq!(
        paused_checkpoint_resume_readiness(
            &json!({
                "task_lifecycle": {
                    "state": "waiting",
                    "next_check_after": 200,
                    "checkpoint_id": "ckpt-future"
                },
                "task_checkpoint": checkpoint_value("ckpt-future", vec![])
            }),
            100,
        ),
        PausedCheckpointResumeReadiness::WaitingNotDue {
            state: "waiting".to_string(),
            checkpoint_id: "ckpt-future".to_string(),
            resume_wait_seconds: 100,
        }
    );
    assert_eq!(
        paused_checkpoint_resume_readiness(
            &json!({
                "task_lifecycle": {
                    "state": "waiting",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-missing"
                }
            }),
            100,
        ),
        PausedCheckpointResumeReadiness::MissingTaskCheckpoint {
            state: "waiting".to_string(),
            checkpoint_id: "ckpt-missing".to_string(),
        }
    );
    assert_eq!(
        paused_checkpoint_resume_readiness(
            &json!({
                "task_lifecycle": {
                    "state": "waiting",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-invalid"
                },
                "task_checkpoint": {
                    "schema_version": 1,
                    "checkpoint_id": "ckpt-invalid"
                }
            }),
            100,
        ),
        PausedCheckpointResumeReadiness::InvalidTaskCheckpoint {
            state: "waiting".to_string(),
            checkpoint_id: "ckpt-invalid".to_string(),
        }
    );
    assert_eq!(
        paused_checkpoint_resume_readiness(
            &json!({
                "task_lifecycle": {
                    "state": "background",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-lifecycle"
                },
                "task_checkpoint": checkpoint_value("ckpt-other", vec![])
            }),
            100,
        ),
        PausedCheckpointResumeReadiness::CheckpointMismatch {
            state: "background".to_string(),
            lifecycle_checkpoint_id: "ckpt-lifecycle".to_string(),
            task_checkpoint_id: "ckpt-other".to_string(),
        }
    );
    assert_eq!(
        paused_checkpoint_resume_readiness(
            &json!({
                "task_lifecycle": {
                    "state": "background",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-ready"
                },
                "task_checkpoint": checkpoint_value("ckpt-ready", vec!["write_file:tmp/a.txt"])
            }),
            100,
        ),
        PausedCheckpointResumeReadiness::Ready {
            state: "background".to_string(),
            checkpoint_id: "ckpt-ready".to_string(),
            resume_entrypoint: ResumeEntrypoint::NextPlannerRound,
            completed_side_effect_count: 1,
            requires_idempotency_guard: true,
        }
    );
    assert_eq!(
        paused_checkpoint_resume_readiness(
            &json!({
                "task_lifecycle": {
                    "state": "waiting",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-claimed",
                    "resume_claim": {
                        "schema_version": 1,
                        "owner": "worker_recovery",
                        "checkpoint_id": "ckpt-claimed",
                        "claimed_at": 95,
                        "expires_at": 130
                    }
                },
                "task_checkpoint": checkpoint_value("ckpt-claimed", vec![])
            }),
            100,
        ),
        PausedCheckpointResumeReadiness::ActiveResumeLease {
            state: "waiting".to_string(),
            checkpoint_id: "ckpt-claimed".to_string(),
            lease_expires_at: 130,
            resume_wait_seconds: 30,
        }
    );
}

#[test]
fn checkpoint_resume_directive_is_closed_machine_instruction() {
    assert_eq!(
        checkpoint_resume_directive(
            &json!({
                "task_lifecycle": {
                    "state": "waiting",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-directive"
                },
                "task_checkpoint": checkpoint_value("ckpt-directive", vec!["write_file:tmp/a.txt"])
            }),
            100,
        ),
        CheckpointResumeDirective::RunNextPlannerRound {
            checkpoint_id: "ckpt-directive".to_string(),
            completed_side_effect_count: 1,
            requires_idempotency_guard: true,
        }
    );
    assert_eq!(
        checkpoint_resume_directive(
            &json!({
                "task_lifecycle": {
                    "state": "background",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-job-policy",
                    "async_timeout_policy": {
                        "schema_version": 1,
                        "policy_source": "async_job_contract",
                        "adapter_kind": "media_job_poll",
                        "deadline_ts": 2_000,
                        "effective_deadline_ts": 150
                    }
                },
                "task_checkpoint": checkpoint_value_with_entrypoint(
                    "ckpt-job-policy",
                    "poll_async_job",
                    Some(json!({
                        "job_id": "job-policy",
                        "status": "running",
                        "poll_after_seconds": 5,
                        "expires_at": 2_000,
                        "cancel_ref": "cancel:job-policy",
                        "message_key": "tool.msg.job.running"
                    }))
                )
            }),
            100,
        ),
        CheckpointResumeDirective::PollAsyncJob {
            checkpoint_id: "ckpt-job-policy".to_string(),
            job_id: "job-policy".to_string(),
            adapter_kind: "media_job_poll".to_string(),
            poll_after_seconds: 5,
            expires_at: 150,
            cancel_ref: "cancel:job-policy".to_string(),
            message_key: "tool.msg.job.running".to_string(),
        }
    );
    assert_eq!(
        checkpoint_resume_directive(
            &json!({
                "task_lifecycle": {
                    "state": "background",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-job-policy-expired",
                    "async_timeout_policy": {
                        "schema_version": 1,
                        "policy_source": "async_job_contract",
                        "adapter_kind": "http_job_poll",
                        "deadline_ts": 2_000,
                        "effective_deadline_ts": 100
                    }
                },
                "task_checkpoint": checkpoint_value_with_entrypoint(
                    "ckpt-job-policy-expired",
                    "poll_async_job",
                    Some(json!({
                        "job_id": "job-policy-expired",
                        "status": "running",
                        "poll_after_seconds": 5,
                        "expires_at": 2_000,
                        "cancel_ref": "cancel:job-policy-expired",
                        "message_key": "tool.msg.job.running"
                    }))
                )
            }),
            100,
        ),
        CheckpointResumeDirective::PollAsyncJob {
            checkpoint_id: "ckpt-job-policy-expired".to_string(),
            job_id: "job-policy-expired".to_string(),
            adapter_kind: "http_job_poll".to_string(),
            poll_after_seconds: 5,
            expires_at: 100,
            cancel_ref: "cancel:job-policy-expired".to_string(),
            message_key: "tool.msg.job.running".to_string(),
        }
    );
    assert_eq!(
        checkpoint_resume_directive(
            &json!({
                "task_lifecycle": {
                    "state": "waiting",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-verify"
                },
                "task_checkpoint": checkpoint_value_with_entrypoint(
                    "ckpt-verify",
                    "verify_and_finalize",
                    None
                )
            }),
            100,
        ),
        CheckpointResumeDirective::VerifyAndFinalize {
            checkpoint_id: "ckpt-verify".to_string(),
            completed_side_effect_count: 0,
        }
    );
    assert_eq!(
        checkpoint_resume_directive(
            &json!({
                "task_lifecycle": {
                    "state": "waiting",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-user"
                },
                "task_checkpoint": checkpoint_value_with_entrypoint(
                    "ckpt-user",
                    "await_user_input",
                    None
                )
            }),
            100,
        ),
        CheckpointResumeDirective::AwaitUserInput {
            checkpoint_id: "ckpt-user".to_string(),
        }
    );
    assert_eq!(
        checkpoint_resume_directive(
            &json!({
                "task_lifecycle": {
                    "state": "background",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-job"
                },
                "task_checkpoint": checkpoint_value_with_entrypoint(
                    "ckpt-job",
                    "poll_async_job",
                    Some(json!({
                        "job_id": "job-1",
                        "status": "running",
                        "poll_after_seconds": 5,
                        "expires_at": 200,
                        "cancel_ref": "cancel:job-1",
                        "message_key": "tool.msg.job.running"
                    }))
                )
            }),
            100,
        ),
        CheckpointResumeDirective::PollAsyncJob {
            checkpoint_id: "ckpt-job".to_string(),
            job_id: "job-1".to_string(),
            adapter_kind: "unspecified_poll".to_string(),
            poll_after_seconds: 5,
            expires_at: 200,
            cancel_ref: "cancel:job-1".to_string(),
            message_key: "tool.msg.job.running".to_string(),
        }
    );
    assert_eq!(
        checkpoint_resume_directive(
            &json!({
                "task_lifecycle": {
                    "state": "background",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-job-missing"
                },
                "task_checkpoint": checkpoint_value_with_entrypoint(
                    "ckpt-job-missing",
                    "poll_async_job",
                    None
                )
            }),
            100,
        ),
        CheckpointResumeDirective::NotReady {
            status_code: "missing_pending_async_job",
        }
    );
    for status in ["accepted", "running", "expired"] {
        assert_eq!(
            checkpoint_resume_directive(
                &json!({
                    "task_lifecycle": {
                        "state": "background",
                        "next_check_after": 90,
                        "checkpoint_id": format!("ckpt-job-{status}")
                    },
                    "task_checkpoint": checkpoint_value_with_entrypoint(
                        &format!("ckpt-job-{status}"),
                        "poll_async_job",
                        Some(json!({
                            "job_id": format!("job-{status}"),
                            "status": status,
                            "poll_after_seconds": 5,
                            "expires_at": 100,
                            "cancel_ref": format!("cancel:job-{status}"),
                            "message_key": "tool.msg.job.running"
                        }))
                    )
                }),
                100,
            ),
            CheckpointResumeDirective::PollAsyncJob {
                checkpoint_id: format!("ckpt-job-{status}"),
                job_id: format!("job-{status}"),
                adapter_kind: "unspecified_poll".to_string(),
                poll_after_seconds: 5,
                expires_at: 100,
                cancel_ref: format!("cancel:job-{status}"),
                message_key: "tool.msg.job.running".to_string(),
            }
        );
    }
    for (status, expires_at, expected_status_code) in [
        ("failed", 200, "async_job_failed"),
        ("succeeded", 200, "async_job_observation_required"),
    ] {
        assert_eq!(
            checkpoint_resume_directive(
                &json!({
                    "task_lifecycle": {
                        "state": "background",
                        "next_check_after": 90,
                        "checkpoint_id": format!("ckpt-job-{status}")
                    },
                    "task_checkpoint": checkpoint_value_with_entrypoint(
                        &format!("ckpt-job-{status}"),
                        "poll_async_job",
                        Some(json!({
                            "job_id": format!("job-{status}"),
                            "status": status,
                            "poll_after_seconds": 5,
                            "expires_at": expires_at,
                            "cancel_ref": format!("cancel:job-{status}"),
                            "message_key": "tool.msg.job.running"
                        }))
                    )
                }),
                100,
            ),
            CheckpointResumeDirective::NotReady {
                status_code: expected_status_code,
            }
        );
    }
    let mut succeeded_with_observation = checkpoint_value_with_entrypoint(
        "ckpt-job-succeeded-observed",
        "poll_async_job",
        Some(json!({
            "job_id": "job-succeeded-observed",
            "status": "succeeded",
            "poll_after_seconds": 5,
            "expires_at": 200,
            "cancel_ref": "cancel:job-succeeded-observed",
            "message_key": "tool.msg.job.succeeded"
        })),
    );
    succeeded_with_observation["observations"] = json!([{
        "source": "async_job",
        "task_journal": {
            "summary": {
                "final_status": "success",
                "final_answer": "observed async completion"
            }
        }
    }]);
    assert_eq!(
        checkpoint_resume_directive(
            &json!({
                "task_lifecycle": {
                    "state": "background",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-job-succeeded-observed"
                },
                "task_checkpoint": succeeded_with_observation
            }),
            100,
        ),
        CheckpointResumeDirective::VerifyAndFinalize {
            checkpoint_id: "ckpt-job-succeeded-observed".to_string(),
            completed_side_effect_count: 0,
        }
    );
    assert_eq!(
        checkpoint_resume_directive(
            &json!({
                "task_lifecycle": {
                    "state": "background",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-job-invalid"
                },
                "task_checkpoint": checkpoint_value_with_entrypoint(
                    "ckpt-job-invalid",
                    "poll_async_job",
                    Some(json!({
                        "job_id": "",
                        "status": "running",
                        "poll_after_seconds": 0,
                        "expires_at": 200,
                        "cancel_ref": "cancel:job-invalid",
                        "message_key": "tool.msg.job.running"
                    }))
                )
            }),
            100,
        ),
        CheckpointResumeDirective::NotReady {
            status_code: "invalid_pending_async_job",
        }
    );
    assert_eq!(
        checkpoint_resume_directive(
            &json!({
                "task_lifecycle": {
                    "state": "waiting",
                    "next_check_after": 90,
                    "checkpoint_id": "ckpt-lease",
                    "resume_claim": {
                        "checkpoint_id": "ckpt-lease",
                        "expires_at": 120
                    }
                },
                "task_checkpoint": checkpoint_value("ckpt-lease", vec![])
            }),
            100,
        ),
        CheckpointResumeDirective::WaitForActiveLease {
            checkpoint_id: "ckpt-lease".to_string(),
            lease_expires_at: 120,
            resume_wait_seconds: 20,
        }
    );
}

#[test]
fn checkpoint_schema_records_resume_entrypoint_and_budget() {
    let checkpoint = TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: "ckpt-1".to_string(),
        boundary_context: json!({"route_gate_kind": "execute"}),
        last_successful_round: Some(2),
        last_successful_step: Some("step_3".to_string()),
        pending_action: Some(json!({"kind": "call_capability", "capability": "fs_basic"})),
        observations: vec![json!({"source": "fs_basic", "status": "ok"})],
        evidence_refs: vec!["step_3:evidence:1".to_string()],
        artifact_refs: vec!["artifact:file:README.md".to_string()],
        completed_side_effect_refs: vec!["write_file:document/report.txt".to_string()],
        budget: CheckpointBudgetCounters {
            round: 2,
            step: 3,
            llm_calls: 5,
            tool_calls: 2,
            elapsed_ms: 1234,
            llm_elapsed_ms: 1234,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: Some(json!({"status_code": "missing_required_evidence"})),
        resume_entrypoint: ResumeEntrypoint::NextPlannerRound,
    };

    let json = checkpoint.to_machine_json();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["checkpoint_id"], "ckpt-1");
    assert_eq!(json["resume_entrypoint"], "next_planner_round");
    assert_eq!(json["budget"]["llm_calls"], 5);
    assert_eq!(json["budget"]["llm_elapsed_ms"], 1234);
    assert_eq!(json["budget"]["tool_elapsed_ms"], 0);
}

#[test]
fn resume_triggers_are_checkpoint_based_machine_tokens() {
    let serialized = serde_json::to_value(ResumeTrigger::AsyncJobPoll).expect("serialize trigger");
    assert_eq!(serialized, "async_job_poll");
}

#[test]
fn terminal_failure_reasons_are_closed_machine_status_codes() {
    let reasons = [
        (TerminalFailureReason::WorkerLeaseLost, "worker_lease_lost"),
        (
            TerminalFailureReason::ToolTimeoutWithoutAsyncResume,
            "tool_timeout_without_async_resume",
        ),
        (TerminalFailureReason::UserCancelled, "user_cancelled"),
        (
            TerminalFailureReason::ConfirmationTimeout,
            "confirmation_timeout",
        ),
        (
            TerminalFailureReason::ProviderWindowExhausted,
            "provider_window_exhausted",
        ),
        (
            TerminalFailureReason::VerifierUnrecoverable,
            "verifier_unrecoverable",
        ),
    ];

    for (reason, expected_code) in reasons {
        assert_eq!(reason.status_code(), expected_code);
    }
}

#[test]
fn async_job_contract_requires_machine_poll_fields() {
    let job = AsyncJobRef {
        job_id: String::new(),
        status: AsyncJobStatus::Accepted,
        poll_after_seconds: 0,
        expires_at: 0,
        cancel_ref: String::new(),
        message_key: String::new(),
    };

    assert_eq!(
        job.missing_required_fields(),
        vec![
            "job_id",
            "poll_after_seconds",
            "expires_at",
            "cancel_ref",
            "message_key"
        ]
    );
}

#[test]
fn task_query_lifecycle_projects_db_status_when_progress_has_no_lifecycle() {
    let lifecycle = task_query_lifecycle_projection("running", Some(&json!({})), Some(1234));

    assert_eq!(lifecycle["schema_version"], 1);
    assert_eq!(lifecycle["state"], "running");
    assert_eq!(lifecycle["execution_state"], "running");
    assert_eq!(
        task_execution_state_from_lifecycle(&lifecycle),
        TaskExecutionState::Running
    );
    assert_eq!(lifecycle["db_status"], "running");
    assert_eq!(lifecycle["source"], "db_status_projection");
    assert_eq!(lifecycle["reason_code"], "running");
    assert_eq!(lifecycle["can_poll"], true);
    assert_eq!(lifecycle["can_cancel"], true);
    assert_eq!(lifecycle["last_heartbeat_ts"], 1234);
    assert_eq!(lifecycle["heartbeat_at"], 1234);
}

#[test]
fn task_query_lifecycle_preserves_checkpoint_waiting_fields() {
    let result = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "resume_reason": "provider_gap_retry_window",
            "next_check_after": 1781800300,
            "checkpoint_id": "ckpt-1",
            "pending_job_ref": "job-1"
        }
    });

    let lifecycle = task_query_lifecycle_projection("running", Some(&result), Some(1234));

    assert_eq!(lifecycle["state"], "waiting");
    assert_eq!(lifecycle["execution_state"], "waiting");
    assert_eq!(lifecycle["db_status"], "running");
    assert_eq!(lifecycle["state_source"], "task_lifecycle_payload");
    assert_eq!(lifecycle["resume_reason"], "provider_gap_retry_window");
    assert_eq!(lifecycle["reason_code"], "provider_gap_retry_window");
    assert_eq!(lifecycle["next_check_after"], 1781800300);
    assert_eq!(lifecycle["checkpoint_id"], "ckpt-1");
    assert_eq!(lifecycle["pending_job_ref"], "job-1");
    assert_eq!(lifecycle["next_action_kind"], "resume_checkpoint");
    assert_eq!(lifecycle["next_action_ref"], "ckpt-1");
    assert_eq!(
        lifecycle["recommended_user_action_kind"],
        "wait_for_worker_resume"
    );
    assert_eq!(lifecycle["can_cancel"], true);
    assert_eq!(lifecycle["last_heartbeat_ts"], 1234);
}

#[test]
fn task_query_lifecycle_projects_checkpoint_product_contract_fields() {
    let mut checkpoint = checkpoint_value("ckpt-product", vec!["write_file:tmp/a.txt"]);
    checkpoint["last_successful_step"] = json!("step_2");
    checkpoint["last_successful_round"] = json!(2);
    checkpoint["evidence_refs"] = json!(["step_2:evidence:1"]);
    checkpoint["artifact_refs"] = json!(["changed_file:tmp/a.txt"]);
    checkpoint["resume_entrypoint"] = json!("poll_async_job");
    checkpoint["pending_async_job"] = json!({
        "job_id": "job-product",
        "status": "running",
        "poll_after_seconds": 9,
        "expires_at": 1781800800,
        "cancel_ref": "cancel:job-product",
        "message_key": "tool.job.running"
    });
    let result = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "async_job_poll",
            "next_check_after": 1781800400,
            "checkpoint_id": "ckpt-product",
            "resume_executor_claim": {
                "owner": "worker_recovery_resume_executor"
            }
        },
        "task_checkpoint": checkpoint
    });

    let lifecycle = task_query_lifecycle_projection("running", Some(&result), Some(4567));

    assert_eq!(lifecycle["waiting_reason_code"], "async_job_poll");
    assert_eq!(lifecycle["next_poll_after"], 1781800400);
    assert_eq!(lifecycle["resume_owner"], "worker_recovery_resume_executor");
    assert_eq!(lifecycle["last_stable_checkpoint_id"], "ckpt-product");
    assert_eq!(lifecycle["resume_entrypoint"], "poll_async_job");
    assert_eq!(lifecycle["last_stable_resume_entrypoint"], "poll_async_job");
    assert_eq!(lifecycle["last_successful_round"], 2);
    assert_eq!(lifecycle["completed_side_effect_count"], 1);
    assert_eq!(lifecycle["requires_idempotency_guard"], true);
    assert_eq!(
        lifecycle["completed_side_effect_refs"],
        json!(["write_file:tmp/a.txt"])
    );
    assert_eq!(lifecycle["completed_side_effect_refs_truncated"], false);
    assert_eq!(lifecycle["last_safe_step_id"], "step_2");
    assert_eq!(lifecycle["evidence_ref_count"], 1);
    assert_eq!(lifecycle["artifact_ref_count"], 1);
    assert_eq!(
        lifecycle["artifact_refs"],
        json!(["changed_file:tmp/a.txt"])
    );
    assert_eq!(lifecycle["artifact_refs_truncated"], false);
    assert_eq!(
        lifecycle["last_successful_evidence_ref"],
        "step_2:evidence:1"
    );
    assert_eq!(lifecycle["poll_ref"], "job-product");
    assert_eq!(lifecycle["cancel_ref"], "cancel:job-product");
    assert_eq!(lifecycle["next_action_kind"], "poll_async_job");
    assert_eq!(lifecycle["next_action_ref"], "job-product");
    assert_eq!(
        lifecycle["recommended_user_action_kind"],
        "wait_for_async_poll"
    );
    assert_eq!(lifecycle["poll_after_seconds"], 9);
    assert_eq!(lifecycle["async_job_expires_at"], 1781800800);
    assert_eq!(lifecycle["async_job_message_key"], "tool.job.running");
}

#[test]
fn task_query_lifecycle_projects_provider_blocker_machine_fields() {
    let mut checkpoint = checkpoint_value("ckpt-provider", vec![]);
    checkpoint["attempt_ledger"] = json!([
        {
            "attempt_id": "a1",
            "action_ref": "image_generate",
            "tool_or_skill": "image_generate",
            "recovery_action": "wait_background",
            "repair_signal": {
                "source": "executor",
                "status_code": "provider_retryable_response",
                "reason_code": "executor_step_failed",
                "next_recovery_kind": "wait_background",
                "provider_status": {
                    "provider": "minimax",
                    "status_code": "rate_limited",
                    "provider_error_class": "rate_limited",
                    "message_key": "provider.rate_limited",
                    "external_provider_blocked": true,
                    "retry_after_seconds": 60,
                    "provider_supported": true
                }
            }
        }
    ]);
    let result = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "provider_blocker_wait_background",
            "next_check_after": 1781800460,
            "checkpoint_id": "ckpt-provider"
        },
        "task_checkpoint": checkpoint
    });

    let lifecycle = task_query_lifecycle_projection("running", Some(&result), Some(4567));

    assert_eq!(lifecycle["state"], "background");
    assert_eq!(
        lifecycle["waiting_reason_code"],
        "provider_blocker_wait_background"
    );
    assert_eq!(lifecycle["provider_blocker_active"], true);
    assert_eq!(lifecycle["provider_blocker_provider"], "minimax");
    assert_eq!(lifecycle["provider_blocker_status_code"], "rate_limited");
    assert_eq!(lifecycle["provider_blocker_external_blocked"], true);
    assert_eq!(lifecycle["provider_blocker_retry_after_seconds"], 60);
    assert_eq!(lifecycle["provider_blocker_provider_supported"], true);
    assert_eq!(
        lifecycle["provider_blocker_next_recovery_kind"],
        "wait_background"
    );
    assert_eq!(lifecycle["provider_blocker_signal_source"], "executor");
    assert_eq!(
        lifecycle["provider_blocker_reason_code"],
        "executor_step_failed"
    );
    assert_eq!(lifecycle["provider_blocker_action_ref"], "image_generate");
    assert_eq!(
        lifecycle["provider_blocker_tool_or_skill"],
        "image_generate"
    );
    assert_eq!(
        lifecycle["provider_blocker_recovery_action"],
        "wait_background"
    );
    assert_eq!(lifecycle["next_action_kind"], "resume_checkpoint");
    assert_eq!(lifecycle["next_action_ref"], "ckpt-provider");
    assert_eq!(
        lifecycle["recommended_user_action_kind"],
        "wait_for_worker_resume"
    );
    assert_eq!(lifecycle["open_issue_count"], 1);
    assert_eq!(
        lifecycle["open_issue_status_code"],
        "provider_retryable_response"
    );
    assert_eq!(
        lifecycle["open_issue_next_recovery_kind"],
        "wait_background"
    );
    assert_eq!(lifecycle["open_issue_action_ref"], "image_generate");
}

#[test]
fn task_query_lifecycle_projects_open_issue_machine_fields() {
    let mut checkpoint = checkpoint_value("ckpt-open-issue", vec![]);
    checkpoint["repair_signal"] = json!({
        "source": "answer_verifier",
        "status_code": "missing_required_evidence",
        "reason_code": "answer_verifier_missing_evidence_repair",
        "next_recovery_kind": "replan",
        "retryable": true,
        "missing_fields": ["content_excerpt", "path"],
        "repair_envelope": {
            "issue_codes": ["answer_verifier_missing_evidence_repair"],
            "missing_evidence": ["content_excerpt", "path"],
            "next_recovery_kind": "replan"
        }
    });
    let result = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "resume_reason": "agent_loop_no_progress_limit",
            "next_check_after": 1781800460,
            "checkpoint_id": "ckpt-open-issue"
        },
        "task_checkpoint": checkpoint
    });

    let lifecycle = task_query_lifecycle_projection("running", Some(&result), Some(4567));

    assert_eq!(lifecycle["last_stable_checkpoint_id"], "ckpt-open-issue");
    assert_eq!(lifecycle["open_issue_count"], 1);
    assert_eq!(
        lifecycle["open_issue_codes"],
        json!(["answer_verifier_missing_evidence_repair"])
    );
    assert_eq!(
        lifecycle["open_issue_missing_fields"],
        json!(["content_excerpt", "path"])
    );
    assert_eq!(lifecycle["open_issue_source"], "answer_verifier");
    assert_eq!(
        lifecycle["open_issue_status_code"],
        "missing_required_evidence"
    );
    assert_eq!(
        lifecycle["open_issue_reason_code"],
        "answer_verifier_missing_evidence_repair"
    );
    assert_eq!(lifecycle["open_issue_next_recovery_kind"], "replan");
    assert_eq!(lifecycle["open_issue_retryable"], true);
}

#[test]
fn task_query_lifecycle_reads_journal_summary_payload() {
    let result = json!({
        "task_journal": {
            "summary": {
                "task_lifecycle": {
                    "state": "background",
                    "resume_reason": "async_job_poll",
                    "checkpoint_id": "ckpt-journal",
                    "next_check_after": 1781800400
                }
            }
        }
    });

    let lifecycle = task_query_lifecycle_projection("running", Some(&result), Some(4567));

    assert_eq!(lifecycle["schema_version"], 1);
    assert_eq!(lifecycle["state"], "background");
    assert_eq!(lifecycle["state_source"], "task_journal_summary");
    assert_eq!(lifecycle["resume_reason"], "async_job_poll");
    assert_eq!(lifecycle["checkpoint_id"], "ckpt-journal");
    assert_eq!(lifecycle["next_action_kind"], "resume_checkpoint");
    assert_eq!(lifecycle["next_action_ref"], "ckpt-journal");
    assert_eq!(lifecycle["last_heartbeat_ts"], 4567);
}

#[test]
fn task_query_lifecycle_maps_timeout_to_failed_machine_state() {
    let lifecycle = task_query_lifecycle_projection("timeout", None, None);

    assert_eq!(lifecycle["state"], "failed");
    assert_eq!(lifecycle["execution_state"], "failed");
    assert_eq!(lifecycle["db_status"], "timeout");
    assert_eq!(lifecycle["state_source"], "db_status_projection");
    assert_eq!(lifecycle["terminal_reason"], "worker_task_timeout");
    assert_eq!(lifecycle["reason_code"], "worker_task_timeout");
    assert_eq!(lifecycle["next_action_kind"], "inspect_result");
    assert_eq!(lifecycle["next_action_ref"], "timeout");
    assert_eq!(lifecycle["recommended_user_action_kind"], "inspect_result");
    assert_eq!(lifecycle["can_cancel"], false);
}

#[test]
fn task_query_lifecycle_exposes_poll_and_cancel_machine_flags_by_state() {
    let queued = task_query_lifecycle_projection("queued", None, None);
    assert_eq!(queued["state"], "queued");
    assert_eq!(queued["execution_state"], "queued");
    assert_eq!(queued["can_poll"], true);
    assert_eq!(queued["can_cancel"], true);
    assert_eq!(queued["next_action_kind"], "poll_task");
    assert_eq!(queued["next_action_ref"], "queued");
    assert_eq!(queued["recommended_user_action_kind"], "poll_task_status");

    let needs_user = task_query_lifecycle_projection(
        "running",
        Some(&json!({
            "task_lifecycle": {
                "state": "needs_user",
                "checkpoint_id": "ckpt-user",
                "resume_reason": "confirmation_required"
            }
        })),
        Some(321),
    );
    assert_eq!(needs_user["state"], "needs_user");
    assert_eq!(needs_user["execution_state"], "needs_confirmation");
    assert_eq!(needs_user["db_status"], "running");
    assert_eq!(needs_user["heartbeat_at"], 321);
    assert_eq!(needs_user["can_poll"], true);
    assert_eq!(needs_user["can_cancel"], true);
    assert_eq!(needs_user["next_action_kind"], "await_user_input");
    assert_eq!(needs_user["next_action_ref"], "ckpt-user");
    assert_eq!(
        needs_user["recommended_user_action_kind"],
        "provide_required_input"
    );
    assert_eq!(needs_user["last_heartbeat_ts"], 321);

    let succeeded = task_query_lifecycle_projection("succeeded", None, None);
    assert_eq!(succeeded["state"], "succeeded");
    assert_eq!(succeeded["execution_state"], "completed");
    assert_eq!(succeeded["can_poll"], true);
    assert_eq!(succeeded["can_cancel"], false);
    assert_eq!(succeeded["recommended_user_action_kind"], "inspect_result");

    let cancelled = task_query_lifecycle_projection("canceled", None, None);
    assert_eq!(cancelled["state"], "cancelled");
    assert_eq!(cancelled["execution_state"], "cancelled");
    assert_eq!(cancelled["can_poll"], true);
    assert_eq!(cancelled["can_cancel"], false);
    assert_eq!(cancelled["recommended_user_action_kind"], "inspect_result");
}

use super::super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[derive(Clone, Copy)]
enum Scenario {
    Accepted,
    InvalidAfterRewrite,
    ExecutionGap,
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn fixture(
    scenario: Scenario,
) -> (
    Server,
    Arc<AtomicUsize>,
    AppState,
    ClaimedTask,
    crate::task_lifecycle::TaskCheckpoint,
    LoopState,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app = axum::Router::new().fallback(move |axum::Json(body): axum::Json<Value>| {
        let counter = counter.clone();
        async move {
            assert!(body.get("tools").is_none(), "answer recovery must not dispatch tools");
            let call = counter.fetch_add(1, Ordering::SeqCst);
            let mut verdict = json!({"operation_checks":[{
                "requested_operation":"delegate the bounded write",
                "required_dispatches":[{"action_type":"call_capability","action_ref":"agent.subagent_persistent"}],
                "evidence_step_ids":["step_3"],"method_observed":true,"result_observed":true
            }],"output_field_checks":[],"pass":call==2,
                "missing_evidence_fields":["unsupported_claims"],
                "answer_incomplete_reason":"unsupported_claims","should_retry":call!=2,
                "retry_instruction":"Remove unrequested metadata; retain the attributed result.","confidence":0.99});
            let content = if call==1 {
                let prompt = body["messages"].as_array().unwrap().iter()
                    .filter_map(|message|message["content"].as_str()).collect::<Vec<_>>().join("\n");
                assert!(prompt.contains("Rejected answer:\nlatest candidate"));
                assert!(prompt.contains("Remove unrequested metadata"));
                "corrected candidate".to_string()
            } else {
                if matches!(scenario, Scenario::ExecutionGap) {
                    verdict["missing_evidence_fields"] = json!(["required_dispatch:call_capability:fixture.verify"]);
                } else if matches!(scenario, Scenario::InvalidAfterRewrite) && call==2 {
                    verdict["unexpected_field"] = json!(true);
                } else if call==2 {
                    verdict["missing_evidence_fields"] = json!([]);
                    verdict["answer_incomplete_reason"] = json!("");
                }
                verdict.to_string()
            };
            axum::Json(json!({"choices":[{"message":{"role":"assistant","content":content},
                "finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = Server(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap()
    }));
    let (state, mut task, _) = super::bound();
    let mut state = state.with_prompt_layers_installed();
    let mut provider = crate::providers::fixture_replay::build_fixture_replay_runtime(
        "verification-answer-fixture",
    );
    let p = Arc::get_mut(&mut provider).unwrap();
    p.config.provider_type = "openai_compat".into();
    p.config.base_url = base;
    p.client = reqwest::Client::builder().no_proxy().build().unwrap();
    state.core.llm_providers = vec![provider];
    state.core.active_provider_type = Some("openai_compat".into());
    state.seed_test_auth_identity("verification-owner", "admin");
    let identity = crate::resolve_auth_identity_by_key(&state, "verification-owner")
        .unwrap()
        .unwrap();
    task.user_key = Some(identity.user_key.clone());
    state.core.db.get().unwrap().execute("INSERT INTO tasks(task_id,user_id,chat_id,user_key,principal_id,channel,kind,payload_json,status,created_at,updated_at,lease_owner,claim_attempt) VALUES (?1,1,2,?2,?3,'ui','ask',?4,'running','1','1',?5,1)",rusqlite::params![task.task_id,identity.user_key,identity.principal_id,task.payload_json,state.worker.worker_id]).unwrap();
    let mut prior = LoopState::default();
    prior.tool_calls_total = 3;
    prior.total_steps_executed = 3;
    prior
        .successful_action_fingerprints
        .insert("completed-delegation".into(), 1);
    prior.last_user_visible_respond = Some("obsolete draft".into());
    for n in 1..=3 {
        prior
            .executed_step_results
            .push(crate::executor::StepExecutionResult {
                step_id: format!("step_{n}"),
                skill: "subagent".into(),
                status: if n == 3 {
                    crate::executor::StepExecutionStatus::Ok
                } else {
                    crate::executor::StepExecutionStatus::Error
                },
                output: (n == 3).then(|| {
                    json!({"extra":{"child_task_id":"fixture-child","status":"completed"}})
                        .to_string()
                }),
                error: (n != 3).then(|| "fixture_pre_admission_rejection".into()),
                started_at: n,
                finished_at: n,
            });
        prior.task_observations.push(json!({"schema_version":1,"owner_layer":"task_journal",
            "observation_kind":"checkpoint_step_provenance","step_id":format!("step_{n}"),
            "requested_action_type":"call_capability","requested_capability":"agent.subagent_persistent",
            "resolved_capability":"agent.subagent_persistent","resolved_tool_or_skill":"subagent",
            "dispatch_executed":n==3}));
    }
    let mut journal = crate::finalize::build_from_loop_state(
        &task,
        "fixture",
        &prior,
        None,
        None,
        true,
        "latest candidate",
        TaskJournalFinalStatus::Success,
    );
    journal.record_answer_verification(
        AnswerVerificationOutcome::Unavailable {
            error_code: "answer_verifier_response_invalid",
            retryable: true,
        },
        "latest candidate",
    );
    let mut value = verification_checkpoint_payload(&task, &prior)["task_checkpoint"].clone();
    bind_checkpoint(&state, &task, &journal, &mut value);
    (
        server,
        calls,
        state,
        task,
        serde_json::from_value(value).unwrap(),
        prior,
    )
}

#[tokio::test]
async fn rejected_resumed_candidate_gets_one_answer_only_rewrite_and_fresh_verification() {
    let (_server, calls, state, task, cp, mut prior) = fixture(Scenario::Accepted).await;
    let reply = try_resume(&state, &task, "fixture", None, &cp, &mut prior)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply.text, "corrected candidate");
    assert!(!reply.should_fail_task);
    let journal = reply.task_journal.unwrap();
    assert_eq!(
        journal.answer_verification.as_ref().unwrap().status,
        crate::answer_verifier::AnswerVerificationStatus::Passed
    );
    assert!(journal.has_answer_verification_for("corrected candidate"));
    assert_eq!(journal.step_results.len(), 3);
    assert_eq!(prior.tool_calls_total, 3);
    assert_eq!(prior.successful_action_fingerprints.len(), 1);
    assert!(prior.output_vars.contains_key(ANSWER_REPAIR_USED));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn unavailable_rewrite_verification_retains_candidate_and_durable_one_shot_limit() {
    let (_server, calls, state, task, cp, mut prior) = fixture(Scenario::InvalidAfterRewrite).await;
    let reply = try_resume(&state, &task, "fixture", None, &cp, &mut prior)
        .await
        .unwrap()
        .unwrap();
    assert!(reply.text.is_empty());
    let journal = reply.task_journal.unwrap();
    let value = journal.task_checkpoint.unwrap();
    assert_eq!(
        value["boundary_context"]["verification_resume"]["candidate"],
        "corrected candidate"
    );
    assert_eq!(
        value["boundary_context"]["agent_loop_resume_state"]["verification_answer_repair_used"],
        true
    );
    let cp = serde_json::from_value(value).unwrap();
    let reply = run_agent_with_tools_seeded(&state, &task, "fixture", "fixture", None, &cp, &[])
        .await
        .unwrap();
    assert!(
        reply.should_fail_task,
        "repeated rejection must not create an unbounded answer loop"
    );
    assert_eq!(reply.task_journal.unwrap().step_results.len(), 3);
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn real_execution_gap_returns_latest_candidate_and_issue_to_planner_without_stale_delivery() {
    let (_server, calls, state, task, cp, mut prior) = fixture(Scenario::ExecutionGap).await;
    assert!(try_resume(&state, &task, "fixture", None, &cp, &mut prior)
        .await
        .unwrap()
        .is_none());
    let observation = prior.task_observations.last().unwrap();
    assert_eq!(
        observation["model_feedback"]["candidate_answer"],
        "latest candidate"
    );
    assert_eq!(
        observation["missing_evidence_fields"][0],
        "required_dispatch:call_capability:fixture.verify"
    );
    assert!(prior.last_user_visible_respond.is_none());
    assert_eq!(prior.executed_step_results.len(), 3);
    assert_eq!(prior.tool_calls_total, 3);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

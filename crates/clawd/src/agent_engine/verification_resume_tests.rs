use super::*;

fn bound() -> (AppState, ClaimedTask, crate::task_lifecycle::TaskCheckpoint) {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: uuid::Uuid::new_v4().to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: json!({"text":"fixture"}).to_string(),
    };
    let mut loop_state = LoopState::default();
    loop_state
        .successful_action_fingerprints
        .insert("completed-write".into(), 1);
    let mut journal = crate::finalize::build_from_loop_state(
        &task,
        "fixture",
        &loop_state,
        None,
        None,
        true,
        "candidate",
        TaskJournalFinalStatus::Success,
    );
    journal.record_answer_verification(
        AnswerVerificationOutcome::Unavailable {
            error_code: "answer_verifier_provider_unavailable",
            retryable: true,
        },
        "candidate",
    );
    let mut value = verification_checkpoint_payload(&task, &loop_state)["task_checkpoint"].clone();
    bind_checkpoint(&state, &task, &journal, &mut value);
    (state, task, serde_json::from_value(value).unwrap())
}

#[test]
fn exact_candidate_request_revision_and_evidence_are_required() {
    let (_, task, cp) = bound();
    assert_eq!(
        binding_matches(&cp, &task, "fixture", &Value::Null),
        Some("candidate".into())
    );
    assert!(binding_matches(&cp, &task, "changed request", &Value::Null).is_none());
    assert!(binding_matches(
        &cp,
        &task,
        "fixture",
        &json!({"instruction_revision":2,"execution_epoch":1})
    )
    .is_none());
    let mut corrupt = cp.clone();
    corrupt.boundary_context["verification_resume"]["candidate"] = json!("changed");
    assert!(binding_matches(&corrupt, &task, "fixture", &Value::Null).is_none());
    let mut corrupt = cp.clone();
    corrupt.completed_side_effect_refs.clear();
    assert!(binding_matches(&corrupt, &task, "fixture", &Value::Null).is_none());
    let mut changed = task.clone();
    changed.payload_json = json!({"text":"new"}).to_string();
    assert!(binding_matches(&cp, &changed, "fixture", &Value::Null).is_none());
}

#[test]
fn incomplete_step_evidence_returns_to_planning_without_claiming_verification() {
    let (_, task, mut cp) = bound();
    cp.boundary_context["verification_resume"]["complete_step_evidence"] = json!(false);
    assert!(binding_matches(&cp, &task, "fixture", &Value::Null).is_none());
}

#[test]
fn verification_checkpoint_clones_share_the_private_snapshot() {
    let (_, task, _) = bound();
    let journal = crate::finalize::build_from_loop_state(
        &task,
        "fixture",
        &LoopState::default(),
        None,
        None,
        true,
        "answer",
        TaskJournalFinalStatus::Success,
    );
    let cloned = journal.clone();
    assert!(std::sync::Arc::ptr_eq(
        journal.answer_verification_checkpoint.as_ref().unwrap(),
        cloned.answer_verification_checkpoint.as_ref().unwrap()
    ));
}

#[tokio::test]
async fn provider_recovery_resumes_persisted_verification_without_replaying_tools() {
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    let available = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let ready = available.clone();
    let counter = calls.clone();
    let app=axum::Router::new().fallback(move |axum::Json(body): axum::Json<Value>| {
        let ready=ready.clone();let counter=counter.clone();
        async move {
            counter.fetch_add(1,Ordering::SeqCst);
            // A planner call would contain tools; only the verifier is permitted.
            assert!(body.get("tools").is_none());
            if !ready.load(Ordering::SeqCst) {
                return (axum::http::StatusCode::TOO_MANY_REQUESTS,axum::Json(json!({"error":{"code":"insufficient_quota","type":"insufficient_quota"}})));
            }
            let verdict=json!({"operation_checks":[],"output_field_checks":[],"pass":true,"missing_evidence_fields":[],"answer_incomplete_reason":"","should_retry":false,"retry_instruction":"","confidence":0.99});
            (axum::http::StatusCode::OK,axum::Json(json!({"choices":[{"message":{"role":"assistant","content":verdict.to_string()},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}})))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    struct Server(tokio::task::JoinHandle<()>);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _server = Server(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap()
    }));
    let mut state = AppState::test_default_with_fixture_provider()
        .with_seeded_db_schema()
        .with_prompt_layers_installed();
    let mut provider =
        crate::providers::fixture_replay::build_fixture_replay_runtime("verification-fixture");
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
    let (_, mut task, _) = bound();
    task.user_key = Some(identity.user_key.clone());
    state.core.db.get().unwrap().execute("INSERT INTO tasks(task_id,user_id,chat_id,user_key,principal_id,channel,kind,payload_json,status,created_at,updated_at,lease_owner,claim_attempt) VALUES (?1,1,2,?2,?3,'ui','ask',?4,'running','1','1',?5,1)",rusqlite::params![task.task_id,identity.user_key,identity.principal_id,task.payload_json,state.worker.worker_id]).unwrap();
    let mut prior = LoopState::default();
    prior.tool_calls_total = 1;
    prior.total_steps_executed = 1;
    prior
        .successful_action_fingerprints
        .insert("completed-write".into(), 1);
    prior
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "step_1".into(),
            skill: "fs_basic".into(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(r#"{"extra":{"action":"stat","exists":true}}"#.into()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });
    let mut journal = crate::finalize::build_from_loop_state(
        &task,
        "fixture",
        &prior,
        None,
        None,
        true,
        "candidate",
        TaskJournalFinalStatus::Success,
    );
    journal.record_answer_verification(
        AnswerVerificationOutcome::Unavailable {
            error_code: "answer_verifier_provider_unavailable",
            retryable: true,
        },
        "candidate",
    );
    let mut value = verification_checkpoint_payload(&task, &prior)["task_checkpoint"].clone();
    bind_checkpoint(&state, &task, &journal, &mut value);
    let cp = serde_json::from_value(value).unwrap();
    let waiting = try_resume(&state, &task, "fixture", None, &cp, &mut prior)
        .await
        .unwrap()
        .unwrap();
    assert!(waiting.text.is_empty());
    let persisted = waiting.task_journal.unwrap().task_checkpoint.unwrap();
    // Serialize/deserialize the durable checkpoint, not the live in-memory binding.
    let cp = serde_json::from_slice(&serde_json::to_vec(&persisted).unwrap()).unwrap();
    available.store(true, Ordering::SeqCst);
    let reply = run_agent_with_tools_seeded(&state, &task, "fixture", "fixture", None, &cp, &[])
        .await
        .unwrap();
    assert_eq!(reply.text, "candidate");
    let journal = reply.task_journal.as_ref().unwrap();
    assert_eq!(journal.step_results.len(), 1);
    assert_eq!(
        journal.answer_verification.as_ref().unwrap().status,
        crate::answer_verifier::AnswerVerificationStatus::Passed
    );
    assert_eq!(prior.tool_calls_total, 1);
    assert_eq!(
        prior.successful_action_fingerprints.get("completed-write"),
        Some(&1)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE auth_keys SET enabled=0 WHERE user_key=?1",
            [&identity.user_key],
        )
        .unwrap();
    assert_eq!(
        try_resume(&state, &task, "fixture", None, &cp, &mut prior)
            .await
            .err()
            .unwrap(),
        "verification_resume_authorization_revoked"
    );
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET status='canceled' WHERE task_id=?1",
            [&task.task_id],
        )
        .unwrap();
    assert!(try_resume(&state, &task, "fixture", None, &cp, &mut prior)
        .await
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

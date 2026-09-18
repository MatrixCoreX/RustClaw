use super::*;
use crate::task_journal::{TaskJournal, TaskJournalAnswerVerifierSummary, TaskJournalStepTrace};

#[test]
fn ordinary_tool_turn_verifies_inside_loop_without_frontdoor_contract() {
    let request = "write the requested bytes, verify them, then remove the test file";
    let mut journal = TaskJournal::for_task("loop-verifier", "ask", request);
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        r#"{"extra":{"content_bytes":18}}"#,
    ));
    let mut reply = AskReply::non_llm("The write did not match the request".to_string())
        .with_task_journal(journal);
    let contract = answer_contract_for_reply(request, &reply)
        .expect("ordinary turns must reach the verifier before leaving the loop");
    assert_eq!(contract.request_text, request);
    assert!(!contract.output_contract.requires_content_evidence);
    assert!(!contract.output_contract.delivery_required);
    assert!(crate::answer_verifier::should_verify_answer(
        &contract,
        reply.task_journal.as_ref().unwrap(),
        &reply.text,
    ));
    assert!(reply
        .task_journal
        .as_ref()
        .unwrap()
        .output_contract
        .is_none());

    reply.task_journal.as_mut().unwrap().answer_verifier_summary =
        Some(TaskJournalAnswerVerifierSummary {
            pass: false,
            missing_evidence_fields: vec!["requested_result".to_string()],
            answer_incomplete_reason: "observed result differs".to_string(),
            should_retry: true,
            retry_instruction: "untrusted model suggestion".to_string(),
            confidence: 0.95,
        });
    let summary = super::super::answer_verifier_evidence_replan_summary(&reply)
        .expect("result mismatch returns to the existing planner recovery path");
    let mut loop_state = LoopState::default();
    assert!(super::super::prepare_answer_verifier_evidence_replan(
        &mut loop_state,
        summary
    ));
    assert!(!super::super::prepare_answer_verifier_evidence_replan(
        &mut loop_state,
        summary
    ));
    assert_eq!(
        loop_state.task_observations[0]["missing_evidence_fields"],
        json!(["requested_result"])
    );
    assert_eq!(
        loop_state.task_observations[0]["model_feedback"]["trust"],
        "untrusted_model_output"
    );
    assert_eq!(
        loop_state.task_observations[0]["model_feedback"]["retry_instruction"],
        "untrusted model suggestion"
    );
    assert!(loop_state.task_observations[0]
        .get("retry_instruction")
        .is_none());
}

#[test]
fn ordinary_no_io_turn_still_skips_unnecessary_verification() {
    let journal = TaskJournal::for_task("loop-chat", "ask", "hello");
    let reply = AskReply::non_llm("hello".to_string()).with_task_journal(journal);
    let contract = answer_contract_for_reply("hello", &reply).expect("neutral contract");
    assert!(!crate::answer_verifier::should_verify_answer(
        &contract,
        reply.task_journal.as_ref().unwrap(),
        &reply.text,
    ));
}

#[test]
fn ordinary_clarification_does_not_require_tool_evidence() {
    let mut journal = TaskJournal::for_task("loop-clarify", "ask", "request");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);
    let reply = AskReply::non_llm("Which target?".to_string()).with_task_journal(journal);
    let contract = answer_contract_for_reply("request", &reply).expect("neutral contract");
    assert!(!crate::answer_verifier::should_verify_answer(
        &contract,
        reply.task_journal.as_ref().unwrap(),
        &reply.text,
    ));
}

#[test]
fn missing_journal_does_not_fabricate_execution_context() {
    let reply = AskReply::non_llm("answer".to_string());
    assert!(answer_contract_for_reply("request", &reply).is_none());
}

#[test]
fn malformed_verification_audit_never_requests_tool_reexecution() {
    let summary = TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["verification_audit".into()],
        answer_incomplete_reason: "verification_audit_invalid".into(),
        should_retry: true,
        retry_instruction: String::new(),
        confidence: 1.0,
    };
    assert!(!super::super::answer_verifier_gap_requires_planner_observation(&summary));
    let mut loop_state = LoopState::new();
    assert!(!super::super::prepare_answer_verifier_evidence_replan(
        &mut loop_state,
        &summary
    ));
    assert!(loop_state.task_observations.is_empty());
    let mut journal = TaskJournal::for_task("audit-failure", "ask", "request");
    journal.answer_verifier_summary = Some(summary);
    let reply = AskReply::non_llm("candidate".into()).with_task_journal(journal);
    let contract = answer_contract_for_reply("request", &reply).unwrap();
    assert!(super::super::answer_verifier_retry_summary(&reply, Some(&contract)).is_some());
}

#[test]
fn exhausted_verification_audit_keeps_grounded_candidate_visible() {
    let summary = TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["verification_audit".into()],
        answer_incomplete_reason: "verification_audit_invalid".into(),
        should_retry: true,
        retry_instruction: String::new(),
        confidence: 1.0,
    };
    let mut journal = TaskJournal::for_task("audit-visible", "ask", "request");
    journal.answer_verifier_summary = Some(summary.clone());
    let mut reply = AskReply::non_llm(
        "Collection stopped because platform verification was not completed.".into(),
    )
    .with_task_journal(journal);
    super::super::mark_reply_failed_after_answer_verifier_exhausted(
        "request", &mut reply, &summary,
    );
    assert_eq!(
        reply.text,
        "Collection stopped because platform verification was not completed."
    );
    assert!(reply.should_fail_task);
    let error = reply.error_text.expect("machine error payload");
    assert!(error.contains("verification_audit"));
    assert!(!reply
        .text
        .contains("answer_verifier_required_evidence_block"));
}

#[tokio::test]
async fn model_quota_at_verifier_and_planner_boundaries_keeps_evidence() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app = axum::Router::new().fallback(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        async {
            (
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                axum::Json(
                    json!({"error":{"type":"insufficient_quota","code":"insufficient_quota"}}),
                ),
            )
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    struct StopServer(tokio::task::JoinHandle<()>);
    impl Drop for StopServer {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _server = StopServer(server);
    let mut state = AppState::test_default_with_fixture_provider()
        .with_seeded_db_schema()
        .with_prompt_layers_installed();
    state.core.llm_providers = vec![Arc::new(crate::LlmProviderRuntime {
        config: claw_core::config::LlmProviderConfig {
            name: "fixture-quota".into(),
            provider_type: "openai_compat".into(),
            base_url,
            api_key: "fixture-only".into(),
            model: "fixture-model".into(),
            context_window_tokens: None,
            input_modalities: vec!["text".into()],
            supports_tools: true,
            expected_latency_ms: None,
            priority: 1,
            timeout_seconds: 5,
            max_concurrency: 1,
            params: Default::default(),
        },
        pricing: None,
        latency: Arc::new(crate::providers::LlmProviderLatencyTracker::default()),
        client: reqwest::Client::builder().no_proxy().build().unwrap(),
        semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    })];
    state.core.active_provider_type = Some("openai_compat".into());
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: "quota-verifier-retry".into(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: "{}".into(),
    };
    state.core.db.get().unwrap().execute(
        "INSERT INTO tasks (task_id,user_id,chat_id,kind,payload_json,status,created_at,updated_at,lease_owner,claim_attempt) VALUES (?1,1,2,'ask','{}','running',0,0,?2,1)",
        rusqlite::params![task.task_id,state.worker.worker_id.as_str()],
    ).unwrap();
    let mut journal = TaskJournal::for_task(&task.task_id, "ask", "fixture request");
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"stat","exists":true}}"#,
    ));
    let mut reply = AskReply::non_llm("original candidate".into()).with_task_journal(journal);
    let contract = answer_contract_for_reply("fixture request", &reply).unwrap();
    let verifier = TaskJournalAnswerVerifierSummary {
        pass: false,
        should_retry: true,
        confidence: 0.99,
        missing_evidence_fields: vec!["output_format".into()],
        answer_incomplete_reason: "fixture".into(),
        retry_instruction: json!({"schema_version":1,"repair_kind":"exact_user_literal",
                                 "required_exact_answer":"candidate"})
        .to_string(),
    };
    let execution_gap = TaskJournalAnswerVerifierSummary {
        missing_evidence_fields: vec!["requested_result".into()],
        ..verifier.clone()
    };
    assert!(
        !super::super::try_bounded_answer_verifier_synthesis_retry(
            &state,
            &task,
            "fixture request",
            &contract,
            &execution_gap,
            &mut reply,
        )
        .await
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(state.task_provider_blocker(&task.task_id).is_none());
    assert_eq!(reply.text, "original candidate");
    let accepted = super::super::try_bounded_answer_verifier_synthesis_retry(
        &state,
        &task,
        "fixture request",
        &contract,
        &verifier,
        &mut reply,
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(state.task_provider_blocker(&task.task_id).is_some());
    assert!(
        !accepted,
        "a missing quota-blocked verifier is not an acceptance"
    );
    assert_eq!(reply.text, "original candidate");

    state = state.with_minimal_builtin_registry();
    let mut prior = LoopState::new();
    prior.round_no = 2;
    prior.total_steps_executed = 1;
    prior.tool_calls_total = 1;
    prior
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "step_1".into(),
            skill: "write_file".into(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(r#"{"path":"fixture.txt","content_bytes":18}"#.into()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });
    prior
        .successful_action_fingerprints
        .insert("fixture-write".into(), 1);
    let payload = crate::agent_engine::support::build_agent_loop_checkpoint_progress_payload(
        &task,
        &prior,
        "provider_blocker_wait_background",
        3,
        4,
    );
    let checkpoint = serde_json::from_value(payload["task_checkpoint"].clone()).unwrap();
    let continued = crate::agent_engine::run_agent_with_tools_seeded(
        &state,
        &task,
        "fixture goal",
        "fixture request",
        None,
        &checkpoint,
        &[],
    )
    .await
    .expect("quota returns a nonterminal handoff, not a state-losing error");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let journal = continued.task_journal.unwrap();
    assert_eq!(journal.task_lifecycle.as_ref().unwrap()["state"], "waiting");
    let checkpoint = journal.task_checkpoint.unwrap();
    assert_eq!(checkpoint["budget"]["tool_calls"], 1);
    assert_eq!(checkpoint["budget"]["round"], 3);
    assert_eq!(
        checkpoint["completed_side_effect_refs"],
        json!(["fixture-write"])
    );
    assert_eq!(
        checkpoint["boundary_context"]["agent_loop_resume_state"]["executed_step_results"][0]
            ["output"],
        json!(prior.executed_step_results[0].output)
    );
    assert!(!continued.should_fail_task);
    assert!(continued.text.is_empty());
}

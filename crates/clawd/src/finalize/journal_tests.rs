use super::*;

#[test]
fn ensure_task_metrics_backfills_missing_v1_fields() {
    let mut journal = TaskJournal::for_task("task-1", "ask", "prompt");
    let messages = vec!["final answer".to_string()];

    ensure_task_metrics(&mut journal, "final answer", &messages);

    assert_eq!(journal.task_metrics.used_evidence_ids_count, Some(0));
    assert_eq!(journal.task_metrics.delivery_consistent, Some(true));
}

#[test]
fn ensure_task_metrics_preserves_finalizer_evidence_count() {
    let mut journal = TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_finalizer_summary(TaskJournalFinalizerSummary {
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        used_evidence_ids_count: 3,
        ..Default::default()
    });

    ensure_task_metrics(&mut journal, "answer", &[]);

    assert_eq!(journal.task_metrics.used_evidence_ids_count, Some(3));
    assert_eq!(journal.task_metrics.delivery_consistent, Some(true));
}

#[test]
fn build_from_loop_state_records_budget_stop_signal() {
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: "task-budget".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = LoopState::new(2);
    loop_state.last_stop_signal = Some("recipe_repair_budget_exhausted".to_string());

    let journal = build_from_loop_state(
        &task,
        "继续修复",
        &loop_state,
        None,
        None,
        true,
        "修复次数已达到上限。",
        TaskJournalFinalStatus::Failure,
    );

    assert_eq!(
        journal.final_stop_signal.as_deref(),
        Some("recipe_repair_budget_exhausted")
    );
    assert_eq!(
        journal.final_failure_attribution.as_deref(),
        Some("budget_exhausted")
    );
}

#[test]
fn build_from_loop_state_records_finalizer_recovered_terminal_stop_signal() {
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: "task-finalizer-recovered".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = LoopState::new(2);
    loop_state.last_stop_signal = Some("synthesize_answer_failed".to_string());
    let finalizer_summary = TaskJournalFinalizerSummary {
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        used_evidence_ids_count: 2,
        ..Default::default()
    };

    let journal = build_from_loop_state(
        &task,
        "test",
        &loop_state,
        None,
        Some(finalizer_summary),
        true,
        r#"{"status":"ok"}"#,
        TaskJournalFinalStatus::Success,
    );

    assert_eq!(
        journal.final_stop_signal.as_deref(),
        Some("finalizer_recovered_terminal_answer")
    );
    assert!(journal.task_observations.is_empty());
}

#[test]
fn build_from_loop_state_records_rollout_switches() {
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: "task-rollout".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = LoopState::new(2);
    loop_state.output_vars.insert(
        "rollout_switches_enabled".to_string(),
        "registry_idempotency_guard_scope,answer_verifier_enforce_required_scope".to_string(),
    );

    let journal = build_from_loop_state(
        &task,
        "test",
        &loop_state,
        None,
        None,
        true,
        "ok",
        TaskJournalFinalStatus::Success,
    );

    assert_eq!(
        journal.rollout_switches_enabled,
        vec![
            "answer_verifier_enforce_required_scope".to_string(),
            "registry_idempotency_guard_scope".to_string()
        ]
    );
    assert_eq!(
        journal
            .to_summary_json()
            .pointer("/rollout_switches_enabled/0")
            .and_then(serde_json::Value::as_str),
        Some("answer_verifier_enforce_required_scope")
    );
    assert_eq!(
        journal
            .to_trace_json()
            .pointer("/rollout_switches_enabled/1")
            .and_then(serde_json::Value::as_str),
        Some("registry_idempotency_guard_scope")
    );
}

#[test]
fn build_from_loop_state_records_task_observations() {
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: "task-observations".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = LoopState::new(2);
    loop_state.task_observations.push(serde_json::json!({
        "schema_version": 1,
        "owner_layer": "agent_hooks",
        "stage": "pre_tool_use",
        "decision": "allow",
        "reason_code": "pre_tool_use_allowed",
        "action_ref": "fs_basic.list_dir"
    }));

    let journal = build_from_loop_state(
        &task,
        "test",
        &loop_state,
        None,
        None,
        true,
        "ok",
        TaskJournalFinalStatus::Success,
    );

    assert_eq!(journal.task_observations.len(), 1);
    assert_eq!(
        journal.task_observations[0]
            .pointer("/owner_layer")
            .and_then(serde_json::Value::as_str),
        Some("agent_hooks")
    );
}

#[tokio::test]
async fn terminal_builder_executes_stop_and_session_end_at_real_owner() {
    let state = AppState::test_default_with_fixture_provider();
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: "task-terminal-hooks".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = LoopState::new(2);
    loop_state.last_stop_signal = Some("completed".to_string());

    let journal = build_terminal_from_loop_state(
        &state,
        &task,
        "test",
        &mut loop_state,
        None,
        None,
        true,
        "ok",
        TaskJournalFinalStatus::Success,
    )
    .await;

    assert_eq!(journal.task_observations.len(), 2);
    assert_eq!(journal.task_observations[0]["stage"], "stop");
    assert_eq!(journal.task_observations[0]["reason_code"], "stop_success");
    assert_eq!(journal.task_observations[1]["stage"], "session_end");
    assert_eq!(
        journal.task_observations[1]["reason_code"],
        "session_end_success"
    );
}

#[test]
fn build_from_loop_state_persists_lifecycle_checkpoint_projection() {
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: "task-checkpoint".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = LoopState::new(2);
    loop_state.task_lifecycle = Some(serde_json::json!({
        "schema_version": 1,
        "state": "waiting",
        "source": "agent_loop_soft_budget",
        "resume_reason": "agent_loop_max_rounds",
        "next_check_after": 1781800060,
        "checkpoint_id": "ckpt-soft-budget"
    }));
    loop_state.task_checkpoint = Some(serde_json::json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-soft-budget",
        "resume_entrypoint": "next_planner_round"
    }));

    let journal = build_from_loop_state(
        &task,
        "test",
        &loop_state,
        None,
        None,
        true,
        "ok",
        TaskJournalFinalStatus::Success,
    );
    let summary = journal.to_summary_json();

    assert_eq!(
        summary
            .pointer("/task_lifecycle/state")
            .and_then(serde_json::Value::as_str),
        Some("waiting")
    );
    assert_eq!(
        summary
            .pointer("/task_lifecycle/checkpoint_id")
            .and_then(serde_json::Value::as_str),
        Some("ckpt-soft-budget")
    );
    assert_eq!(
        summary
            .pointer("/task_checkpoint/resume_entrypoint")
            .and_then(serde_json::Value::as_str),
        Some("next_planner_round")
    );
}

#[test]
fn build_from_loop_state_records_rollout_attribution() {
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: "task-rollout-attribution".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = LoopState::new(2);
    loop_state.rollout_attribution.push(
        crate::task_journal::TaskJournalRolloutAttribution::registry_idempotency_guard_block(
            "registry_idempotency_repeat_completed_action",
            "config_edit",
            Some("apply_config_change".to_string()),
            "action",
            "skill:config_edit:action:apply_config_change",
            Some(1),
            None,
        ),
    );

    let journal = build_from_loop_state(
        &task,
        "test",
        &loop_state,
        None,
        None,
        true,
        "ok",
        TaskJournalFinalStatus::Success,
    );
    let summary = journal.to_summary_json();

    assert_eq!(
        summary
            .pointer("/rollout_attribution/0/switch_name")
            .and_then(serde_json::Value::as_str),
        Some("registry_idempotency_guard_scope")
    );
    assert_eq!(
        summary
            .pointer("/rollout_attribution/0/action")
            .and_then(serde_json::Value::as_str),
        Some("apply_config_change")
    );
    assert_eq!(
        summary
            .pointer("/rollout_attribution/0/dedup_scope")
            .and_then(serde_json::Value::as_str),
        Some("action")
    );
}

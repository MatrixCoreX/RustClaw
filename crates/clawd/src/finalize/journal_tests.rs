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
fn build_from_loop_state_records_rollout_switches() {
    let task = ClaimedTask {
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
        "registry_idempotency_guard,agent_decides_semantic_route".to_string(),
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
            "agent_decides_semantic_route".to_string(),
            "registry_idempotency_guard".to_string()
        ]
    );
    assert_eq!(
        journal
            .to_summary_json()
            .pointer("/rollout_switches_enabled/0")
            .and_then(serde_json::Value::as_str),
        Some("agent_decides_semantic_route")
    );
    assert_eq!(
        journal
            .to_trace_json()
            .pointer("/rollout_switches_enabled/1")
            .and_then(serde_json::Value::as_str),
        Some("registry_idempotency_guard")
    );
}

#[test]
fn build_from_loop_state_records_rollout_attribution() {
    let task = ClaimedTask {
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
        Some("registry_idempotency_guard")
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

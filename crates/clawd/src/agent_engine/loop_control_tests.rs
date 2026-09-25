use super::{
    action_result_boundary_requires_planner, answer_contract_for_reply,
    answer_verifier_retry_summary, append_conversation_input_context,
    apply_active_task_boundary_controls, apply_structured_respond_clarify_to_loop_state,
    budget_replan_cause, child_loop_budget_limits, coding_workflow_ready_for_model_finalization,
    commit_answer_verifier_retry_answer, forced_boundary_observation_clarify_intent,
    initial_execution_recipe_spec, initial_round_for_agent_loop, next_resumable_budget_action,
    observe_only_round_should_continue, planner_owned_observation_round_should_continue,
    post_write_content_evidence_recovery_policy,
    prefer_terminal_model_answer_for_verifier_candidate,
    record_agent_loop_decision_envelope_output_vars, retry_rewritten_answer_is_publishable,
    retry_verifier_accepts_rewritten_answer, round_is_policy_terminal, round_model_finished,
    select_round_task_budget_profile, should_stop_for_observed_finalize,
    structured_field_selector_observation_can_finalize,
    structured_respond_terminal_intent_from_plan,
    suppress_answer_verifier_retry_if_structurally_satisfied, terminal_user_answer_stop_signal,
    try_recover_inconsistent_boundary_clarify, verified_action_budget_requirements,
    AgentLoopGuardPolicy, RoundOutcome,
};
use crate::agent_engine::support::{
    AnswerVerifierRequiredEvidenceScope, RegistryIdempotencyGuardScope,
};
use crate::{
    agent_engine::{AgentRunContext, LoopState},
    execution_recipe::{
        ExecutionRecipeKind, ExecutionRecipeProfile, ExecutionRecipeRuntimeState,
        ExecutionRecipeSpec, ExecutionRecipeTargetScope,
    },
    executor::{StepExecutionResult, StepExecutionStatus},
    AgentAction, AskReply, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
    OutputResponseShape,
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn conversation_input_context_preserves_multilingual_text_as_structured_user_input() {
    let input_id = Uuid::new_v4();
    let record = claw_core::conversation_input::ConversationInputRecord {
        receipt: claw_core::conversation_input::ConversationInputReceipt {
            schema_version: 1,
            input_id,
            client_message_id: "message-2".to_string(),
            input_seq: 2,
            scope: claw_core::conversation_input::ConversationInputScopeRef {
                conversation_id: "conversation-1".to_string(),
                agent_id: "main".to_string(),
                channel: "ui".to_string(),
                channel_account_id: "browser-session".to_string(),
            },
            preparation_state:
                claw_core::conversation_input::ConversationInputPreparationState::Ready,
            disposition: claw_core::conversation_input::ConversationInputDisposition::Applied,
            target_task_id: Some(Uuid::new_v4()),
            decision_ref: None,
            instruction_revision: 2,
            execution_epoch: 1,
            accepted_at_ts: 10,
            updated_at_ts: 11,
            replayed: false,
        },
        content: vec![
            claw_core::conversation_input::ConversationInputContent::Text {
                text: "不要停止。继续，但保留最初结果。続けてください。".to_string(),
            },
        ],
        delivery_mode: claw_core::conversation_input::ConversationInputDeliveryMode::Auto,
        expected_task_id: None,
        expected_instruction_revision: None,
        source: claw_core::conversation_input::ConversationInputSource::default(),
    };
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let mut text = "original request".to_string();
    append_conversation_input_context(&state, &mut text, &[record]).expect("append context");

    let envelope = text
        .strip_prefix("original request\n\n[conversation_input_batch]")
        .expect("structured envelope suffix");
    let value: serde_json::Value = serde_json::from_str(envelope).expect("valid JSON envelope");
    assert_eq!(value["kind"], "conversation_input_batch");
    assert_eq!(value["inputs"][0]["input_id"], input_id.to_string());
    assert_eq!(
        value["inputs"][0]["content"][0]["text"],
        "不要停止。继续，但保留最初结果。続けてください。"
    );
}

#[test]
fn active_turn_cancel_is_applied_to_the_task_at_the_safe_boundary() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        claim_attempt: 1,
        task_id: "active-turn-safe-boundary-cancel".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    state
        .core
        .db
        .get()
        .expect("database")
        .execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, kind, payload_json, status,
                created_at, updated_at, lease_owner, claim_attempt
             ) VALUES (?1, 1, 2, 'ask', '{}', 'running', 0, 0, ?2, 1)",
            rusqlite::params![task.task_id, state.worker.worker_id.as_str()],
        )
        .expect("running task");
    crate::repo::task_control_mailbox::enqueue_task_control(
        &state,
        crate::repo::task_control_mailbox::EnqueueTaskControl {
            task_id: task.task_id.clone(),
            action: "cancel".to_string(),
            issued_by: "agent_loop".to_string(),
            payload: json!({"instruction_revision": 3}),
            idempotency_key: Some("active-turn:3:stop".to_string()),
            expected_control_seq: None,
        },
    )
    .expect("enqueue cancel")
    .expect("active task directive");

    let error = apply_active_task_boundary_controls(
        &state,
        &task,
        &mut "initial request".to_string(),
        &mut LoopState::new(),
    )
    .expect_err("cancel must end the active turn");

    assert_eq!(error, crate::agent_engine::TASK_CANCELED_ERR);
    assert!(
        crate::repo::pending_task_control_directives(&state, &task.task_id, 4)
            .expect("pending directives")
            .is_empty()
    );
    let status: String = state
        .core
        .db
        .get()
        .expect("database")
        .query_row(
            "SELECT status FROM tasks WHERE task_id = ?1",
            rusqlite::params![task.task_id],
            |row| row.get(0),
        )
        .expect("task status");
    assert_eq!(status, "canceled");
}

#[test]
fn safe_boundary_drains_dense_ready_inputs_before_planning() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state
        .core
        .db
        .get()
        .expect("database")
        .execute_batch(
            "INSERT OR IGNORE INTO principals(
                principal_id, role, status, revision, created_at, updated_at
             ) VALUES ('principal-1', 'user', 'active', 1, '1', '1');
             INSERT INTO auth_keys(user_key, role, enabled, created_at, principal_id)
             VALUES ('dense-input-key', 'user', 1, '1', 'principal-1');",
        )
        .expect("authorized fixture principal");
    let task_id = Uuid::new_v4();
    let task = crate::ClaimedTask {
        claim_attempt: 1,
        task_id: task_id.to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let scope = claw_core::conversation_input::OwnedConversationInputScope {
        owner_principal_id: "principal-1".to_string(),
        conversation: claw_core::conversation_input::ConversationInputScopeRef {
            conversation_id: "conversation-pages".to_string(),
            agent_id: "main".to_string(),
            channel: "ui".to_string(),
            channel_account_id: "browser-session".to_string(),
        },
    };
    let make_input = |index: usize| crate::repo::conversation_inputs::AcceptConversationInput {
        owner_principal_id: scope.owner_principal_id.clone(),
        submission: claw_core::conversation_input::ConversationInputSubmission {
            schema_version: claw_core::conversation_input::CONVERSATION_INPUT_SCHEMA_VERSION,
            client_message_id: format!("message-{index}"),
            scope: scope.conversation.clone(),
            content: vec![
                claw_core::conversation_input::ConversationInputContent::Text {
                    text: format!("instruction {index}"),
                },
            ],
            delivery_mode: claw_core::conversation_input::ConversationInputDeliveryMode::Auto,
            expected_task_id: None,
            expected_instruction_revision: None,
            source: Default::default(),
        },
        preparation_state: claw_core::conversation_input::ConversationInputPreparationState::Ready,
    };
    let initial =
        crate::repo::conversation_inputs::accept_conversation_input(&state.core.db, &make_input(0))
            .expect("accept initial");
    state
        .core
        .db
        .get()
        .expect("database")
        .execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, principal_id, kind, payload_json, status,
                created_at, updated_at, lease_owner, claim_attempt
             ) VALUES (?1, 1, 2, ?2, 'ask', '{}', 'running', 0, 0, ?3, 1)",
            rusqlite::params![
                task.task_id,
                scope.owner_principal_id,
                state.worker.worker_id.as_str()
            ],
        )
        .expect("running task");
    crate::repo::conversation_inputs::bind_conversation_input_to_task(
        &state.core.db,
        &scope,
        initial.record.receipt.input_id,
        task_id,
        crate::repo::conversation_inputs::ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind initial");
    for index in 1..=101 {
        crate::repo::conversation_inputs::accept_conversation_input(
            &state.core.db,
            &make_input(index),
        )
        .expect("accept follow-up");
    }

    let mut user_text = "initial".to_string();
    let mut loop_state = LoopState::new();
    assert_eq!(
        apply_active_task_boundary_controls(&state, &task, &mut user_text, &mut loop_state,)
            .expect("apply all pages"),
        super::ActiveTaskBoundaryControl::Continue
    );
    assert!(
        !crate::repo::conversation_inputs::task_has_pending_conversation_inputs(
            &state.core.db,
            &task.task_id,
        )
        .expect("pending state")
    );
    assert!(user_text.contains("instruction 101"));
    assert_eq!(loop_state.conversation_input_revision, 2);
    assert_eq!(loop_state.conversation_execution_epoch, 2);
}

#[test]
fn resumed_agent_loop_starts_after_the_checkpoint_round() {
    let mut loop_state = LoopState::new();
    assert_eq!(initial_round_for_agent_loop(&loop_state), 1);

    loop_state.round_no = 7;
    assert_eq!(initial_round_for_agent_loop(&loop_state), 8);
}

fn state_with_workspace_registry() -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load workspace skills registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = std::sync::Arc::new(crate::SkillViewsSnapshot {
        binding: Default::default(),
        registry: Some(std::sync::Arc::new(registry)),
        skills_list: std::sync::Arc::new(enabled),
    });
    state
}

#[test]
fn verified_action_budget_uses_registry_timeout_and_execution_mode() {
    let state = state_with_workspace_registry();
    assert_eq!(
        verified_action_budget_requirements(
            &state,
            &AgentAction::CallSkill {
                skill: "image_vision".to_string(),
                args: json!({"action": "describe", "image": "fixture.jpg"}),
            },
        ),
        (120, true)
    );
    assert_eq!(
        verified_action_budget_requirements(
            &state,
            &AgentAction::CallSkill {
                skill: "office_workspace".to_string(),
                args: json!({"action": "office.inspect", "path": "fixture.docx"}),
            },
        ),
        (120, false)
    );
    assert_eq!(
        verified_action_budget_requirements(
            &state,
            &AgentAction::CallCapability {
                capability: "system.run_command".to_string(),
                args: json!({"command": "sleep 120", "async_start": true}),
            },
        ),
        (180, true)
    );
}

#[test]
fn child_loop_budget_comes_only_from_structured_child_contract() {
    let limits = child_loop_budget_limits(
        &json!({
            "task_role": "subagent_child",
            "child_task_contract": {
                "schema_version": 2,
                "budget": {
                    "max_rounds": 7,
                    "max_tool_calls": 11,
                    "max_tokens": 500000
                }
            }
        })
        .to_string(),
    )
    .expect("child limits");
    assert_eq!(limits.max_rounds, 7);
    assert_eq!(limits.max_tool_calls, 11);
    assert_eq!(limits.max_tokens, 500000);
    assert_eq!(limits.runtime_deadline_ms, None);
    let legacy = child_loop_budget_limits(
        &json!({
            "task_role": "subagent_child",
            "child_task_contract": {
                "schema_version": 1,
                "budget": {
                    "max_rounds": 3,
                    "max_tool_calls": 5,
                    "timeout_ms": 180000
                }
            }
        })
        .to_string(),
    )
    .expect("legacy child limits");
    assert_eq!(legacy.runtime_deadline_ms, Some(180000));
    assert!(child_loop_budget_limits(
        &json!({
            "task_role": "ordinary",
            "child_task_contract": {"budget": {"max_rounds": 1}}
        })
        .to_string()
    )
    .is_none());
}

#[test]
fn zero_action_verifier_replan_round_is_not_model_finished() {
    let outcome = RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: Some("replan_from_verifier_signal".to_string()),
        no_progress: false,
    };

    assert!(!round_model_finished(Some(&outcome)));
}

#[test]
fn zero_action_observation_ready_round_is_not_model_finished() {
    let outcome = RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("structured_observation_already_ready".to_string()),
        next_goal_hint: None,
        no_progress: true,
    };

    assert!(!round_model_finished(Some(&outcome)));
}

#[test]
fn successful_action_result_boundary_is_not_model_finished() {
    for signal in [
        "independent_read_batch_observed",
        "material_action_observed",
    ] {
        let mut loop_state = LoopState::new();
        let outcome = RoundOutcome {
            executed_actions: 1,
            had_error: false,
            stop_signal: Some(signal.to_string()),
            next_goal_hint: None,
            no_progress: false,
        };

        assert!(action_result_boundary_requires_planner(
            &loop_state,
            &outcome,
        ));

        let continued = RoundOutcome {
            stop_signal: Some("action_result_continue_round".to_string()),
            ..outcome.clone()
        };
        assert!(!round_model_finished(Some(&continued)));

        loop_state
            .delivery_messages
            .push("FILE:/tmp/result".to_string());
        assert!(!action_result_boundary_requires_planner(
            &loop_state,
            &outcome,
        ));
    }
}

#[test]
fn capability_scope_load_round_is_not_model_finished() {
    for signal in [
        "capability_groups_loaded",
        "capability_catalog_searched",
        "capability_contracts_expanded",
    ] {
        let outcome = RoundOutcome {
            executed_actions: 1,
            had_error: false,
            stop_signal: Some(signal.to_string()),
            next_goal_hint: None,
            no_progress: false,
        };

        assert!(
            !round_model_finished(Some(&outcome)),
            "{signal} must continue into the next planner round"
        );
    }
}

#[test]
fn mcp_capability_scope_load_round_is_not_model_finished() {
    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("mcp_capabilities_loaded".to_string()),
        next_goal_hint: None,
        no_progress: false,
    };

    assert!(!round_model_finished(Some(&outcome)));
}

#[test]
fn budget_telemetry_uses_machine_replan_and_resume_tokens() {
    use crate::task_budget_contract::BudgetDecision;

    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("post_write_validation_reserve".to_string()),
        next_goal_hint: None,
        no_progress: false,
    };
    assert_eq!(
        budget_replan_cause(BudgetDecision::Continue, Some(&outcome)),
        Some("post_write_validation_reserve")
    );
    assert_eq!(
        budget_replan_cause(BudgetDecision::Finish, Some(&outcome)),
        None
    );
    assert_eq!(
        next_resumable_budget_action(BudgetDecision::CheckpointRequeue, None),
        Some("resume_checkpoint")
    );
    assert_eq!(
        next_resumable_budget_action(BudgetDecision::Waiting, Some("background")),
        Some("poll_async_job")
    );
    assert_eq!(
        next_resumable_budget_action(BudgetDecision::NeedsUser, Some("needs_user")),
        Some("await_user_input")
    );
}

#[test]
fn verifier_retry_cannot_publish_unobserved_local_code_status() {
    assert!(!retry_rewritten_answer_is_publishable(
        r#"{"changed_files":["test_calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"not_observed_in_trace"}"#,
    ));
    assert!(!retry_rewritten_answer_is_publishable(
        r#"{"status":"not_observed_in_trace"}"#,
    ));
    assert!(retry_rewritten_answer_is_publishable(
        "The trace notes that no unresolved machine status was returned.",
    ));
}

#[test]
fn later_verified_plan_budget_can_widen_but_not_narrow() {
    use crate::task_budget_contract::TaskBudgetProfile;

    assert_eq!(
        select_round_task_budget_profile(None, TaskBudgetProfile::FastRead),
        (TaskBudgetProfile::FastRead, true)
    );
    assert_eq!(
        select_round_task_budget_profile(
            Some(TaskBudgetProfile::FastRead),
            TaskBudgetProfile::MultiStepWorkspace,
        ),
        (TaskBudgetProfile::MultiStepWorkspace, true)
    );
    assert_eq!(
        select_round_task_budget_profile(
            Some(TaskBudgetProfile::MultiStepWorkspace),
            TaskBudgetProfile::FastRead,
        ),
        (TaskBudgetProfile::MultiStepWorkspace, false)
    );
}

#[test]
fn verified_coding_workflow_hands_off_to_model_finalization() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"src/lib.rs","resolved_path":"/workspace/src/lib.rs"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "run_cmd",
        r#"{"extra":{"command":"cargo test -p demo"}}"#,
    ));

    assert!(coding_workflow_ready_for_model_finalization(&loop_state));
}

#[test]
fn unverified_or_read_only_workflow_does_not_finalize() {
    let mut unverified = LoopState::new();
    unverified.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"src/lib.rs","resolved_path":"/workspace/src/lib.rs"}}"#,
    ));
    assert!(!coding_workflow_ready_for_model_finalization(&unverified));

    let mut read_only = LoopState::new();
    read_only.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        r#"{"extra":{"command":"cargo test -p demo"}}"#,
    ));
    assert!(!coding_workflow_ready_for_model_finalization(&read_only));
}

#[test]
fn latest_command_validation_closes_workflow_when_step_output_omits_command() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"src/lib.rs","resolved_path":"/workspace/src/lib.rs"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "run_cmd",
        "test result without command metadata",
    ));
    loop_state.latest_validation_result = Some(serde_json::json!({
        "status": "passed",
        "verification_scope": "command",
        "global_step": 2,
    }));

    assert!(coding_workflow_ready_for_model_finalization(&loop_state));

    loop_state.latest_validation_result = Some(serde_json::json!({
        "status": "failed",
        "verification_scope": "command",
        "global_step": 2,
    }));
    assert!(!coding_workflow_ready_for_model_finalization(&loop_state));
}

#[test]
fn zero_action_terminal_round_is_model_finished() {
    let outcome = RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("respond".to_string()),
        next_goal_hint: None,
        no_progress: false,
    };

    assert!(round_model_finished(Some(&outcome)));
}

#[test]
fn repeated_completed_action_replans_until_the_repeat_limit() {
    let replan = RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("repeat_completed_action".to_string()),
        next_goal_hint: None,
        no_progress: true,
    };
    assert!(!round_is_policy_terminal(Some(&replan)));

    let exhausted = RoundOutcome {
        stop_signal: Some("repeat_action_limit".to_string()),
        ..replan
    };
    assert!(round_is_policy_terminal(Some(&exhausted)));
}

fn route_result(shape: OutputResponseShape) -> IntentOutputContract {
    IntentOutputContract {
        exact_sentence_count: None,
        response_shape: shape,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        locator_hint: String::new(),
        selection: crate::OutputSelectionContract::default(),
    }
}

fn answer_contract(route: &IntentOutputContract) -> crate::answer_verifier::AnswerContract {
    crate::answer_verifier::AnswerContract::new("test request", route.clone())
}

#[test]
fn answer_contract_for_reply_uses_journal_output_contract() {
    let mut output_contract = IntentOutputContract::default();
    output_contract.response_shape = OutputResponseShape::Strict;
    output_contract.selection.structured_field_selector = Some("path".to_string());
    output_contract.locator_kind = OutputLocatorKind::Path;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-effective-route",
        "ask",
        "probe service status",
    );
    journal.record_output_contract(&output_contract);
    let reply = AskReply::non_llm("ok".to_string()).with_task_journal(journal);

    let selected =
        answer_contract_for_reply("probe service status", &reply).expect("answer contract");

    assert_eq!(
        selected
            .output_contract
            .selection
            .structured_field_selector
            .as_deref(),
        Some("path")
    );
    assert_eq!(
        crate::evidence_policy::required_evidence_fields_for_output_contract(
            &selected.output_contract,
        ),
        vec!["path".to_string()]
    );
}

fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    }
}

fn plan_result_with_raw_and_steps(
    raw_plan_text: &str,
    steps: Vec<crate::PlanStep>,
) -> crate::PlanResult {
    crate::PlanResult {
        goal: "test".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: None,
        steps,
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: raw_plan_text.to_string(),
    }
}

fn test_policy() -> AgentLoopGuardPolicy {
    AgentLoopGuardPolicy {
        max_actions_per_turn: 8,
        repeat_action_limit: 3,
        answer_verifier_enforce_required_scope: AnswerVerifierRequiredEvidenceScope::Off,
        registry_idempotency_guard_scope: RegistryIdempotencyGuardScope::Off,
        fast_read: Default::default(),
        grounded_summary: Default::default(),
        multi_step_workspace: Default::default(),
        ops_closed_loop: Default::default(),
    }
}

#[path = "loop_control_tests/clarify_control.rs"]
mod clarify_control;
#[path = "loop_control_tests/observed_finalize.rs"]
mod observed_finalize;
#[path = "loop_control_tests/post_write_validation_reserve.rs"]
mod post_write_validation_reserve;
#[path = "loop_control_tests/soft_budget_checkpoint.rs"]
mod soft_budget_checkpoint;
#[path = "loop_control_tests/terminal_answer_stop.rs"]
mod terminal_answer_stop;
#[path = "loop_control_tests/verifier_retry_suppression.rs"]
mod verifier_retry_suppression;

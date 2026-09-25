use super::{
    effective_locale_hint, load_active_session_snapshot, normalized_locale_hint,
    ActiveSessionPointers, ActiveSessionSnapshot, ConversationState, SessionAliasBinding,
};
use crate::runtime::AppState;
use crate::ClaimedTask;
use rusqlite::params;
use serde_json::json;

#[test]
fn locale_hint_prefers_response_language_then_language_then_locale() {
    assert_eq!(
        normalized_locale_hint(Some(
            &json!({"response_language":"en-US","language":"zh-CN"})
        )),
        Some("en-US".to_string())
    );
    assert_eq!(
        normalized_locale_hint(Some(&json!({"language":"zh-CN"}))),
        Some("zh-CN".to_string())
    );
    assert_eq!(
        normalized_locale_hint(Some(&json!({"locale":"en-US"}))),
        Some("en-US".to_string())
    );
    assert_eq!(normalized_locale_hint(Some(&json!({}))), None);
}

#[test]
fn effective_locale_hint_preserves_prior_locale_when_payload_is_empty() {
    let prior_state = ConversationState {
        locale_hint: Some("en-US".to_string()),
        ..ConversationState::default()
    };
    assert_eq!(
        effective_locale_hint(Some(&prior_state), Some(&json!({}))),
        Some("en-US".to_string())
    );
    assert_eq!(
        effective_locale_hint(Some(&prior_state), Some(&json!({"language":"zh-CN"}))),
        Some("zh-CN".to_string())
    );
}

#[test]
fn conversation_state_defaults_are_empty() {
    let state = ConversationState::default();
    assert!(state.active_followup_task_id.is_none());
    assert!(state.active_clarify_task_id.is_none());
    assert!(state.active_observed_facts_task_id.is_none());
    assert!(state.alias_bindings.is_empty());
}

#[test]
fn active_session_snapshot_defaults_to_empty() {
    let snapshot = ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(snapshot.conversation_state.is_none());
    assert!(snapshot.active_followup_frame.is_none());
    assert!(snapshot.active_clarify_state.is_none());
    assert!(snapshot.active_observed_facts.is_none());
}

fn output_contract_for_test() -> crate::IntentOutputContract {
    crate::IntentOutputContract::default()
}

fn empty_journal_for_test() -> crate::task_journal::TaskJournal {
    crate::task_journal::TaskJournal::new("test")
}

fn journal_with_final_status(
    status: crate::task_journal::TaskJournalFinalStatus,
) -> crate::task_journal::TaskJournal {
    let mut journal = crate::task_journal::TaskJournal::new("test");
    journal.record_final_status(status);
    journal
}

fn journal_with_planner_relation(
    relation: &str,
    status: crate::task_journal::TaskJournalFinalStatus,
) -> crate::task_journal::TaskJournal {
    let mut journal = crate::task_journal::TaskJournal::new("test");
    journal.record_plan_result(&crate::PlanResult {
        goal: "respond".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: None,
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "response",
                "terminal_intent": if relation == "clarify" { "clarify" } else { "answer" },
                "conversation_relation": relation
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    });
    journal.record_final_status(status);
    journal
}

fn next_last_primary_task_prompt(
    prior_state: Option<&ConversationState>,
    route_result: &crate::IntentOutputContract,
    turn_analysis: Option<&crate::turn_context::TurnAnalysis>,
    prompt: &str,
    resolved_prompt_for_execution: &str,
) -> Option<String> {
    super::next_last_primary_task_prompt(
        prior_state,
        route_result,
        turn_analysis,
        &empty_journal_for_test(),
        prompt,
        resolved_prompt_for_execution,
    )
}

fn next_last_primary_task_output(
    prior_state: Option<&ConversationState>,
    route_result: &crate::IntentOutputContract,
    turn_analysis: Option<&crate::turn_context::TurnAnalysis>,
    resolved_prompt_for_execution: &str,
    answer_text: &str,
    answer_messages: &[String],
) -> Option<String> {
    super::next_last_primary_task_output(
        prior_state,
        route_result,
        turn_analysis,
        &empty_journal_for_test(),
        resolved_prompt_for_execution,
        answer_text,
        answer_messages,
    )
}

#[test]
fn plain_chat_without_task_turn_does_not_promote_primary_task() {
    let route_result = output_contract_for_test();
    let promoted = next_last_primary_task_prompt(
        None,
        &route_result,
        None,
        "刚才记住的编号是什么？",
        "RC-CONT-CN-0428-A",
    );
    assert!(promoted.is_none());

    let prior_state = ConversationState {
        last_primary_task_prompt: Some("帮我写个方案".to_string()),
        ..ConversationState::default()
    };
    let preserved = next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        None,
        "刚才记住的编号是什么？",
        "RC-CONT-CN-0428-A",
    );
    assert_eq!(preserved.as_deref(), Some("帮我写个方案"));
}

#[test]
fn planner_relation_starts_primary_task_without_pre_route_analysis() {
    let route_result = output_contract_for_test();
    let journal = journal_with_planner_relation(
        "start_followup",
        crate::task_journal::TaskJournalFinalStatus::Success,
    );

    let prompt = super::next_last_primary_task_prompt(
        None,
        &route_result,
        None,
        &journal,
        "Write a three-step checklist.",
        "Write a three-step checklist.",
    );
    let output = super::next_last_primary_task_output(
        None,
        &route_result,
        None,
        &journal,
        "Write a three-step checklist.",
        "1. Prepare\n2. Verify\n3. Release",
        &[],
    );

    assert_eq!(prompt.as_deref(), Some("Write a three-step checklist."));
    assert_eq!(output.as_deref(), Some("1. Prepare\n2. Verify\n3. Release"));
}

#[test]
fn planner_relation_amends_primary_task_without_pre_route_analysis() {
    let route_result = output_contract_for_test();
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a three-step checklist.".to_string()),
        last_primary_task_output: Some("1. Prepare\n2. Verify\n3. Release".to_string()),
        ..ConversationState::default()
    };
    let journal = journal_with_planner_relation(
        "amend_current",
        crate::task_journal::TaskJournalFinalStatus::Success,
    );

    let prompt = super::next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        None,
        &journal,
        "Mention Python 3.11, not Python 3.10.",
        "Mention Python 3.11, not Python 3.10.",
    );
    let output = super::next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        None,
        &journal,
        "Mention Python 3.11, not Python 3.10.",
        "1. Install Python 3.11\n2. Verify\n3. Release",
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some(
            "Task so far:\nWrite a three-step checklist.\n\nAmendment: Mention Python 3.11, not Python 3.10."
        )
    );
    assert_eq!(
        output.as_deref(),
        Some("1. Install Python 3.11\n2. Verify\n3. Release")
    );
}

#[test]
fn planner_side_reply_preserves_primary_task_without_pre_route_analysis() {
    let route_result = output_contract_for_test();
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a three-step checklist.".to_string()),
        last_primary_task_output: Some("1. Prepare\n2. Verify\n3. Release".to_string()),
        ..ConversationState::default()
    };
    let journal = journal_with_planner_relation(
        "side_reply",
        crate::task_journal::TaskJournalFinalStatus::Success,
    );

    let prompt = super::next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        None,
        &journal,
        "What is SQLite?",
        "What is SQLite?",
    );
    let output = super::next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        None,
        &journal,
        "What is SQLite?",
        "SQLite is an embedded database.",
        &[],
    );

    assert_eq!(prompt, prior_state.last_primary_task_prompt);
    assert_eq!(output, prior_state.last_primary_task_output);
}

#[test]
fn failed_planner_amendment_preserves_primary_task() {
    let route_result = output_contract_for_test();
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a three-step checklist.".to_string()),
        last_primary_task_output: Some("1. Prepare\n2. Verify\n3. Release".to_string()),
        ..ConversationState::default()
    };
    let journal = journal_with_planner_relation(
        "amend_current",
        crate::task_journal::TaskJournalFinalStatus::Failure,
    );

    let prompt = super::next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        None,
        &journal,
        "Mention Python 3.11.",
        "Mention Python 3.11.",
    );
    let output = super::next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        None,
        &journal,
        "Mention Python 3.11.",
        "Provider unavailable.",
        &[],
    );

    assert_eq!(prompt, prior_state.last_primary_task_prompt);
    assert_eq!(output, prior_state.last_primary_task_output);
}

#[test]
fn standalone_task_request_preserves_existing_primary_task() {
    let route_result = output_contract_for_test();
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("帮我写个方案".to_string()),
        last_primary_task_output: Some("三条登录模块要点".to_string()),
        ..ConversationState::default()
    };

    let prompt = next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "问一个独立概念问题",
        "问一个独立概念问题",
    );
    let output = next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "问一个独立概念问题",
        "独立概念回答",
        &[],
    );

    assert_eq!(prompt.as_deref(), Some("帮我写个方案"));
    assert_eq!(output.as_deref(), Some("三条登录模块要点"));
}

#[test]
fn standalone_side_answer_preserves_existing_primary_task() {
    let mut route_result = output_contract_for_test();
    route_result.response_shape = crate::OutputResponseShape::OneSentence;
    route_result.requires_content_evidence = false;
    route_result.delivery_required = false;
    route_result.locator_kind = crate::OutputLocatorKind::None;
    route_result.delivery_intent = crate::OutputDeliveryIntent::None;
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a short release note for Agent Runtime.".to_string()),
        last_primary_task_output: Some(
            "- Update Agent Runtime to the latest version.\n- Keep Python 3.11.\n- Try the new features."
                .to_string(),
        ),
        ..ConversationState::default()
    };
    let resolved =
        "Answer the side question in one sentence.\nanswer_candidate: SQLite is a local SQL database.";

    let prompt = next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "BTW, what is SQLite in one sentence? Do not change the checklist task.",
        resolved,
    );
    let output = next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        resolved,
        "SQLite is a local SQL database.",
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("Write a short release note for Agent Runtime.")
    );
    assert_eq!(
        output.as_deref(),
        Some(
            "- Update Agent Runtime to the latest version.\n- Keep Python 3.11.\n- Try the new features."
        )
    );
}

#[test]
fn direct_standalone_side_answer_preserves_existing_primary_task() {
    let mut route_result = output_contract_for_test();
    route_result.response_shape = crate::OutputResponseShape::OneSentence;
    route_result.requires_content_evidence = false;
    route_result.delivery_required = false;
    route_result.locator_kind = crate::OutputLocatorKind::None;
    route_result.delivery_intent = crate::OutputDeliveryIntent::None;
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a short release note for Agent Runtime.".to_string()),
        last_primary_task_output: Some(
            "Agent Runtime shipped a focused runtime update.".to_string(),
        ),
        ..ConversationState::default()
    };
    let resolved =
        "Answer the side question in one sentence.\nanswer_candidate: SQLite is a local SQL database.";

    let prompt = next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "BTW, what is SQLite in one sentence? Do not change the release task.",
        resolved,
    );
    let output = next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        resolved,
        "SQLite is a local SQL database.",
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("Write a short release note for Agent Runtime.")
    );
    assert_eq!(
        output.as_deref(),
        Some("Agent Runtime shipped a focused runtime update.")
    );
}

#[test]
fn standalone_new_deliverable_replaces_existing_primary_task() {
    let route_result = output_contract_for_test();
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "primary_task_update": "replace",
            "active_task_boundary": "new_deliverable"
        })),
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a short release note for Agent Runtime".to_string()),
        last_primary_task_output: Some(
            "Agent Runtime is easier for non-technical users.".to_string(),
        ),
        ..ConversationState::default()
    };

    let prompt = next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "Write one deployment note that mentions Python 3.10",
        "Write one deployment note that mentions Python 3.10",
    );
    let output = next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "Write one deployment note that mentions Python 3.10",
        "Agent Runtime deployment should use Python 3.10.",
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("Write one deployment note that mentions Python 3.10")
    );
    assert_eq!(
        output.as_deref(),
        Some("Agent Runtime deployment should use Python 3.10.")
    );
}

#[test]
fn standalone_task_request_without_prior_can_start_primary_task() {
    let route_result = output_contract_for_test();
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    let prompt = next_last_primary_task_prompt(
        None,
        &route_result,
        Some(&turn_analysis),
        "帮我写个方案",
        "帮我写个方案",
    );
    let output = next_last_primary_task_output(
        None,
        &route_result,
        Some(&turn_analysis),
        "帮我写个方案",
        "方案正文",
        &[],
    );

    assert_eq!(prompt.as_deref(), Some("帮我写个方案"));
    assert_eq!(output.as_deref(), Some("方案正文"));
}

#[test]
fn standalone_freeform_answer_candidate_without_prior_starts_primary_task() {
    let route_result = output_contract_for_test();
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let resolved = "Write one deployment note mentioning Python 3.10\nanswer_candidate: **Deployment Note**\n\nUse Python 3.10 before deploying.";

    let prompt = next_last_primary_task_prompt(
        None,
        &route_result,
        Some(&turn_analysis),
        "Write one deployment note that mentions Python 3.10",
        resolved,
    );
    let output = next_last_primary_task_output(
        None,
        &route_result,
        Some(&turn_analysis),
        resolved,
        "**Deployment Note**\n\nUse Python 3.10 before deploying.",
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("Write one deployment note that mentions Python 3.10")
    );
    assert_eq!(
        output.as_deref(),
        Some("**Deployment Note**\n\nUse Python 3.10 before deploying.")
    );
}

#[test]
fn standalone_freeform_answer_candidate_with_prior_preserves_primary_task() {
    let route_result = output_contract_for_test();
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a release note".to_string()),
        last_primary_task_output: Some("Existing release note.".to_string()),
        ..ConversationState::default()
    };
    let resolved = "Answer a separate freeform request\nanswer_candidate: Separate answer.";

    let prompt = next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "Answer a separate freeform request",
        resolved,
    );
    let output = next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        resolved,
        "Separate answer.",
        &[],
    );

    assert_eq!(prompt.as_deref(), Some("Write a release note"));
    assert_eq!(output.as_deref(), Some("Existing release note."));
}

#[test]
fn standalone_replacement_answer_candidate_replaces_prior_primary_task() {
    let route_result = output_contract_for_test();
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "primary_task_update": "replace",
            "active_task_boundary": "new_deliverable"
        })),
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a release note".to_string()),
        last_primary_task_output: Some("Existing release note.".to_string()),
        ..ConversationState::default()
    };
    let resolved =
        "Write one deployment note mentioning Python 3.10\nanswer_candidate: New deployment note.";

    let prompt = next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "Write one deployment note that mentions Python 3.10",
        resolved,
    );
    let output = next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        resolved,
        "New deployment note.",
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("Write one deployment note that mentions Python 3.10")
    );
    assert_eq!(output.as_deref(), Some("New deployment note."));
}

#[test]
fn active_task_non_success_preserves_prior_primary_output() {
    let route_result = output_contract_for_test();
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskCorrect),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: Some(json!({"target": "Python 3.10 -> Python 3.11"})),
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a short release note for Agent Runtime".to_string()),
        last_primary_task_output: Some(
            "1. Manage settings easily\n2. Track work clearly\n3. Communicate naturally"
                .to_string(),
        ),
        ..ConversationState::default()
    };
    let journal = journal_with_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);

    assert!(super::active_primary_non_success_preserves_prior(
        Some(&turn_analysis),
        &journal
    ));
    let output = super::next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        &journal,
        "Correction: mention Python 3.11, not Python 3.10.",
        "The model is temporarily unavailable.",
        &[],
    );

    assert_eq!(
        output.as_deref(),
        Some("1. Manage settings easily\n2. Track work clearly\n3. Communicate naturally")
    );
}

#[test]
fn model_fallback_preserves_primary_state_from_structured_source() {
    let journal = journal_with_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);

    assert!(super::model_fallback_preserves_primary_state(
        Some(crate::fallback::ClarifyFallbackSource::LlmUnavailable),
        &journal
    ));
    assert!(super::model_fallback_preserves_primary_state(
        Some(crate::fallback::ClarifyFallbackSource::EmptyResponse),
        &journal
    ));
    assert!(!super::model_fallback_preserves_primary_state(
        Some(crate::fallback::ClarifyFallbackSource::IntentUnresolved),
        &journal
    ));
}

#[test]
fn standalone_preference_or_memory_turn_clears_prior_primary_task() {
    let route_result = output_contract_for_test();
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::PreferenceOrMemory),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some(
            "compare README.md and AGENTS.md, tell me which one is larger".to_string(),
        ),
        last_primary_task_output: Some("README.md is larger.".to_string()),
        ..ConversationState::default()
    };

    let prompt = next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "后面我要提到你的时候，统一按“巡检爪”这个称呼来",
        "用户要求统一使用“巡检爪”作为称呼",
    );
    let output = next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "用户要求统一使用“巡检爪”作为称呼",
        "已记住：巡检爪。",
        &[],
    );

    assert!(prompt.is_none());
    assert!(output.is_none());
}

#[test]
fn standalone_answer_candidate_request_without_prior_does_not_start_primary_task() {
    let mut route_result = output_contract_for_test();
    route_result.response_shape = crate::OutputResponseShape::Scalar;
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let resolved = "查询之前记住的编号\nanswer_candidate: RC-CONT-CN-0428-A";

    let prompt = next_last_primary_task_prompt(
        None,
        &route_result,
        Some(&turn_analysis),
        "刚才让你记住的编号是什么？只回答编号。",
        resolved,
    );
    let output = next_last_primary_task_output(
        None,
        &route_result,
        Some(&turn_analysis),
        resolved,
        "RC-CONT-CN-0428-A",
        &[],
    );

    assert!(prompt.is_none());
    assert!(output.is_none());
}

#[test]
fn standalone_scalar_chat_request_without_answer_marker_does_not_start_primary_task() {
    let mut route_result = output_contract_for_test();
    route_result.response_shape = crate::OutputResponseShape::Scalar;
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let resolved = "Answer the continuous test marker, which is RC-CONT-EN-0428-B.";

    let prompt = next_last_primary_task_prompt(
        None,
        &route_result,
        Some(&turn_analysis),
        "What continuous test marker did I ask you to remember? Answer only the marker.",
        resolved,
    );
    let output = next_last_primary_task_output(
        None,
        &route_result,
        Some(&turn_analysis),
        resolved,
        "RC-CONT-EN-0428-B",
        &[],
    );

    assert!(prompt.is_none());
    assert!(output.is_none());
}

#[test]
fn evidence_backed_standalone_task_replaces_prior_scalar_primary_task() {
    let mut route_result = output_contract_for_test();
    route_result.requires_content_evidence = true;
    route_result.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some(
            "What continuous test marker did I ask you to remember?".to_string(),
        ),
        last_primary_task_output: Some("RC-CONT-EN-0428-B".to_string()),
        ..ConversationState::default()
    };

    let prompt = next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "Write a short release note for Agent Runtime.",
        "Write a short release note for Agent Runtime.",
    );
    let output = next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "Write a short release note for Agent Runtime.",
        "Agent Runtime 0.1.7 is now available.",
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("Write a short release note for Agent Runtime.")
    );
    assert_eq!(
        output.as_deref(),
        Some("Agent Runtime 0.1.7 is now available.")
    );
}

#[test]
fn unannotated_evidence_backed_deliverable_starts_primary_task() {
    let mut route_result = output_contract_for_test();
    route_result.requires_content_evidence = true;
    route_result.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

    let prompt = next_last_primary_task_prompt(
        None,
        &route_result,
        None,
        "Write a short release note for Agent Runtime",
        "Write a short release note for Agent Runtime",
    );
    let output = next_last_primary_task_output(
        None,
        &route_result,
        None,
        "Write a short release note for Agent Runtime",
        "Agent Runtime 0.1.7 is easier to update and operate.",
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("Write a short release note for Agent Runtime")
    );
    assert_eq!(
        output.as_deref(),
        Some("Agent Runtime 0.1.7 is easier to update and operate.")
    );
}

#[test]
fn unannotated_structured_listing_replaces_prior_primary_task() {
    let route_result = output_contract_for_test();
    let mut journal = crate::task_journal::TaskJournal::new("list");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                serde_json::json!({
                    "action": "inventory_dir",
                    "resolved_path": "/tmp/logs",
                    "names": ["act_plan.log", "clawd.log", "clawd.run.log"]
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("先列出 document 目录下前 5 个文件名".to_string()),
        last_primary_task_output: Some(
            "builtin_write_smoke.txt\nfull_suite_trace_note.txt".to_string(),
        ),
        ..ConversationState::default()
    };

    let prompt = super::next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        None,
        &journal,
        "那 logs 目录下前 5 个文件名呢",
        "列出 logs 目录下前 5 个文件名",
    );
    let output = super::next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        None,
        &journal,
        "列出 logs 目录下前 5 个文件名",
        "act_plan.log\nclawd.log\nclawd.run.log",
        &[],
    );

    assert_eq!(prompt.as_deref(), Some("那 logs 目录下前 5 个文件名呢"));
    assert_eq!(
        output.as_deref(),
        Some("act_plan.log\nclawd.log\nclawd.run.log")
    );
}

#[test]
fn unannotated_scalar_evidence_result_does_not_start_primary_task() {
    let mut route_result = output_contract_for_test();
    route_result.requires_content_evidence = true;
    route_result.response_shape = crate::OutputResponseShape::Scalar;
    route_result.selection.structured_field_selector = Some("count".to_string());

    let prompt = next_last_primary_task_prompt(
        None,
        &route_result,
        None,
        "Count files under logs",
        "Count files under logs",
    );
    let output = next_last_primary_task_output(
        None,
        &route_result,
        None,
        "Count files under logs",
        "2",
        &[],
    );

    assert!(prompt.is_none());
    assert!(output.is_none());
}

#[test]
fn task_append_persists_compact_primary_without_runtime_envelope() {
    let route_result = output_contract_for_test();
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("帮我写个方案".to_string()),
        ..ConversationState::default()
    };
    let persisted = next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        Some(&crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskAppend),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(json!({"audience":"boss"})),
            attachment_processing_required: false,
        }),
        "面向老板",
        "Current task:\n帮我写个方案\n\nKeep the same task...",
    )
    .expect("primary prompt");
    assert!(persisted.contains("帮我写个方案"));
    assert!(persisted.contains("面向老板"));
    assert!(persisted.contains("\"audience\":\"boss\""));
    assert!(!persisted.contains("Continuity rules"));
    assert!(!persisted.contains("Current task:"));
}

#[test]
fn repeated_task_append_keeps_single_task_so_far_header() {
    let persisted = super::merge_primary_task_prompt(
        Some("Task so far:\n帮我写个方案\n\nAdditional instruction: 面向老板"),
        "不要太技术",
        crate::turn_context::TurnType::TaskAppend,
        None,
    );
    assert_eq!(persisted.matches("Task so far:").count(), 1);
    assert!(persisted.contains("Additional instruction: 面向老板"));
    assert!(persisted.contains("Additional instruction: 不要太技术"));
}

#[test]
fn authoritative_snapshot_filters_components_by_task_ids() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: "task-2".to_string(),
        user_id: 7,
        chat_id: 9,
        user_key: Some("user-key".to_string()),
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    {
        let db = state.core.db.get().expect("db");
        db.execute(
            "INSERT INTO followup_frames (
                user_id, chat_id, user_key, frame_json, source_task_id, updated_at_ts, expires_at_ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                task.user_id,
                task.chat_id,
                "user-key",
                serde_json::to_string(&crate::followup_frame::FollowupFrame {
                    source_request: "read file".to_string(),
                    source_task_id: "task-old".to_string(),
                    updated_at_ts: crate::now_ts_u64(),
                    expires_at_ts: crate::now_ts_u64() + 60,
                    ..crate::followup_frame::FollowupFrame::default()
                })
                .expect("frame json"),
                "task-old",
                crate::now_ts_u64() as i64,
                (crate::now_ts_u64() + 60) as i64,
            ],
        )
        .expect("insert followup");
        db.execute(
            "INSERT INTO conversation_states (
                user_id, chat_id, user_key, state_json, last_task_id, updated_at_ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                task.user_id,
                task.chat_id,
                "user-key",
                serde_json::to_string(&ConversationState {
                    active_followup_task_id: Some("task-2".to_string()),
                    active_clarify_task_id: None,
                    active_observed_facts_task_id: None,
                    alias_bindings: Vec::new(),
                    last_primary_task_prompt: None,
                    last_primary_task_output: None,
                    locale_hint: None,
                    last_task_id: "task-2".to_string(),
                    updated_at_ts: crate::now_ts_u64(),
                })
                .expect("conversation state json"),
                "task-2",
                crate::now_ts_u64() as i64,
            ],
        )
        .expect("insert conversation state");
    }

    let snapshot = load_active_session_snapshot(&state, &task);
    assert!(snapshot.active_followup_frame.is_none());
}

#[test]
fn replace_active_conversation_state_with_pointers_persists_ids() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: "task-3".to_string(),
        user_id: 11,
        chat_id: 12,
        user_key: Some("user-key".to_string()),
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    super::replace_active_conversation_state_with_pointers(
        &state,
        &task,
        Some(&json!({"response_language":"en-US"})),
        ActiveSessionPointers {
            active_followup_task_id: Some("task-f".to_string()),
            active_clarify_task_id: Some("task-c".to_string()),
            active_observed_facts_task_id: Some("task-o".to_string()),
        },
    );
    let loaded = super::load_active_conversation_state(&state, &task).expect("state");
    assert_eq!(loaded.active_followup_task_id.as_deref(), Some("task-f"));
    assert_eq!(loaded.active_clarify_task_id.as_deref(), Some("task-c"));
    assert_eq!(
        loaded.active_observed_facts_task_id.as_deref(),
        Some("task-o")
    );
    assert_eq!(loaded.locale_hint.as_deref(), Some("en-US"));
}

#[test]
fn current_code_workspace_outcome_refreshes_the_active_anchor() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: "task-alias-with-code-workspace".to_string(),
        user_id: 11,
        chat_id: 12,
        user_key: Some("user-key".to_string()),
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    super::replace_active_conversation_state_with_pointers(
        &state,
        &task,
        None,
        ActiveSessionPointers {
            active_followup_task_id: Some("old-followup".to_string()),
            active_clarify_task_id: Some("old-clarify".to_string()),
            active_observed_facts_task_id: Some("old-observed".to_string()),
        },
    );
    let project_dir = "/home/guagua/agent-runtime/run/nl_eval_tmp/code_workspace_alias_patch";
    let calc_path = format!("{project_dir}/calc_core.py");
    let test_path = format!("{project_dir}/test_calc_core.py");
    let mut route = output_contract_for_test();
    route.response_shape = crate::OutputResponseShape::Strict;
    let mut journal =
        crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "create code workspace");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            serde_json::json!({
                "extra": {
                    "action": "write_text",
                    "resolved_path": calc_path,
                }
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            serde_json::json!({
                "extra": {
                    "action": "write_text",
                    "resolved_path": test_path,
                }
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3", "run_cmd", "OK",
        ));
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);

    super::update_active_session_from_ask_outcome(
        &state,
        &task,
        None,
        "create code workspace",
        &route,
        None,
        "create code workspace",
        r#"{"created_files":["calc_core.py","test_calc_core.py"],"test_status":"passed"}"#,
        &[],
        false,
        &[],
        &journal,
        None,
    );

    let loaded = super::load_active_conversation_state(&state, &task).expect("state");
    assert_eq!(
        loaded.active_followup_task_id.as_deref(),
        Some(task.task_id.as_str())
    );
    assert!(loaded.active_clarify_task_id.is_none());
    let snapshot = load_active_session_snapshot(&state, &task);
    let frame = snapshot
        .active_followup_frame
        .expect("code workspace frame");
    assert_eq!(
        frame.op_kind,
        crate::followup_frame::FollowupOpKind::CodeWorkspace
    );
    assert_eq!(frame.bound_target.as_deref(), Some(project_dir));
}

#[test]
fn alias_surface_match_accepts_user_defined_separator_variants() {
    let bindings = vec![SessionAliasBinding {
        alias: "note_file".to_string(),
        target: "/tmp/release_checklist.md".to_string(),
        updated_at_ts: 1,
    }];

    let matched = super::single_alias_binding_mentioned_in_prompt(
        &bindings,
        "What does the note file refer to?",
    )
    .expect("alias should match across separator variants");

    assert_eq!(matched.target, "/tmp/release_checklist.md");
}

#[test]
fn successful_session_alias_capability_result_replaces_matching_binding() {
    let prior = vec![SessionAliasBinding {
        alias: "release note".to_string(),
        target: "document/old.md".to_string(),
        updated_at_ts: 1,
    }];
    let result = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "session.bind_alias",
        Some("bind_session_alias".to_string()),
        json!({
            "output": {"ignored": true},
            "extra": {
                "execution_binding": {"skill_name": "task_control"},
                "session_alias_bindings": [{
                    "alias": "release_note",
                    "target": "document/release.md",
                    "target_kind": "path"
                }]
            }
        }),
    );

    let merged = super::merge_alias_bindings_from_capability_results(prior, &[result]);

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].alias, "release_note");
    assert_eq!(merged[0].target, "document/release.md");
}

#[test]
fn session_alias_state_requires_a_distinct_typed_machine_target() {
    let result = |alias: &str, target: &str, target_kind: Option<&str>| {
        let mut binding = json!({"alias": alias, "target": target});
        if let Some(target_kind) = target_kind {
            binding["target_kind"] = json!(target_kind);
        }
        claw_core::capability_result::CapabilityResultEnvelope::ok(
            "session.bind_alias",
            Some("bind_session_alias".to_string()),
            json!({
                "extra": {
                    "execution_binding": {"skill_name": "task_control"},
                    "session_alias_bindings": [binding]
                }
            }),
        )
    };

    let merged = super::merge_alias_bindings_from_capability_results(
        Vec::new(),
        &[
            result("marker", "RC-CONT-0428", None),
            result("RC-CONT-0428", "RC-CONT-0428", Some("resource")),
            result("marker", "RC-CONT-0428", Some("resource")),
        ],
    );

    assert!(merged.is_empty());
}

#[test]
fn session_alias_state_ignores_wrong_failed_and_text_only_results() {
    let mut failed = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "task_control",
        Some("bind_session_alias".to_string()),
        json!({"extra":{"session_alias_bindings":[{"alias":"failed","target":"failed.md"}]}}),
    );
    failed.status = claw_core::capability_result::CapabilityResultStatus::Error;
    let wrong_action = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "task_control",
        Some("other_action".to_string()),
        json!({"extra":{"session_alias_bindings":[{"alias":"wrong","target":"wrong.md"}]}}),
    );
    let text_only = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "task_control",
        Some("bind_session_alias".to_string()),
        json!({"output":"{\"session_alias_bindings\":[{\"alias\":\"text\",\"target\":\"text.md\"}]}"}),
    );

    let merged = super::merge_alias_bindings_from_capability_results(
        Vec::new(),
        &[failed, wrong_action, text_only],
    );

    assert!(merged.is_empty());
}

#[test]
fn session_alias_state_ignores_unresolved_planner_capability_envelope() {
    let unresolved = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "session.bind_alias",
        Some("bind_session_alias".to_string()),
        json!({
            "extra": {
                "session_alias_bindings": [{
                    "alias": "draft",
                    "target": "document/draft.md"
                }]
            }
        }),
    );

    let merged = super::merge_alias_bindings_from_capability_results(Vec::new(), &[unresolved]);

    assert!(merged.is_empty());
}

#[test]
fn meta_turn_types_preserve_active_session_pointers() {
    for turn_type in [
        crate::turn_context::TurnType::RunControl,
        crate::turn_context::TurnType::ApprovalDecision,
        crate::turn_context::TurnType::StatusQuery,
        crate::turn_context::TurnType::FeedbackOrError,
        crate::turn_context::TurnType::PreferenceOrMemory,
    ] {
        assert!(super::should_preserve_active_session_pointers(Some(
            &crate::turn_context::TurnAnalysis {
                turn_type: Some(turn_type),
                target_task_policy: None,
                should_interrupt_active_run: false,
                state_patch: None,
                attachment_processing_required: false,
            }
        )));
    }
    assert!(!super::should_preserve_active_session_pointers(Some(
        &crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskAppend),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }
    )));
}

#[test]
fn ordered_listing_outcome_refreshes_active_session_pointers_for_status_query() {
    let mut journal = crate::task_journal::TaskJournal::new("list");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                serde_json::json!({
                    "action": "inventory_dir",
                    "resolved_path": "/tmp/logs",
                    "names": ["act_plan.log", "clawd.log", "clawd.run.log"]
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let turn_analysis = crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert!(super::should_preserve_active_session_pointers(Some(
        &turn_analysis
    )));
    assert!(super::current_outcome_has_ordered_entries(&journal, false));
    assert!(!super::current_outcome_has_ordered_entries(&journal, true));
}

#[test]
fn clarify_task_request_persists_primary_prompt_for_followups() {
    let route_result = output_contract_for_test();
    let persisted = next_last_primary_task_prompt(
        None,
        &route_result,
        Some(&crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskRequest),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        "帮我写个方案",
        "帮我写个方案",
    );
    assert_eq!(persisted.as_deref(), Some("帮我写个方案"));
}

#[test]
fn clarify_task_prompt_without_turn_analysis_is_not_inferred_from_route_trace() {
    let route_result = output_contract_for_test();
    let persisted = next_last_primary_task_prompt(
        None,
        &route_result,
        None,
        "Help me write a proposal",
        "Help me write a proposal",
    );
    assert_eq!(persisted, None);
}

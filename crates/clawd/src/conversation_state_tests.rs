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

fn route_result_for_test(ask_mode: crate::AskMode, needs_clarify: bool) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode,
        resolved_intent: String::new(),
        needs_clarify,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(0.8),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    }
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

fn next_last_primary_task_prompt(
    prior_state: Option<&ConversationState>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
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
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
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
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
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
fn standalone_task_request_preserves_existing_primary_task() {
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
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
fn pure_chat_agent_loop_side_answer_preserves_existing_primary_task() {
    let mut route_result = route_result_for_test(
        crate::AskMode::Act {
            finalize: crate::ActFinalizeStyle::ChatWrapped,
        },
        false,
    );
    route_result.route_reason = "pure_chat_agent_loop_submode".to_string();
    route_result.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route_result.output_contract.requires_content_evidence = false;
    route_result.output_contract.delivery_required = false;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a short release note for RustClaw.".to_string()),
        last_primary_task_output: Some(
            "- Update RustClaw to the latest version.\n- Keep Python 3.11.\n- Try the new features."
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
        Some("Write a short release note for RustClaw.")
    );
    assert_eq!(
        output.as_deref(),
        Some(
            "- Update RustClaw to the latest version.\n- Keep Python 3.11.\n- Try the new features."
        )
    );
}

#[test]
fn direct_answer_pure_chat_side_answer_preserves_existing_primary_task() {
    let mut route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    route_result.route_reason = "pure_chat_agent_loop_submode".to_string();
    route_result.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route_result.output_contract.requires_content_evidence = false;
    route_result.output_contract.delivery_required = false;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a short release note for RustClaw.".to_string()),
        last_primary_task_output: Some("RustClaw shipped a focused runtime update.".to_string()),
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
        Some("Write a short release note for RustClaw.")
    );
    assert_eq!(
        output.as_deref(),
        Some("RustClaw shipped a focused runtime update.")
    );
}

#[test]
fn standalone_new_deliverable_replaces_existing_primary_task() {
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "primary_task_update": "replace",
            "active_task_boundary": "new_deliverable"
        })),
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a short release note for RustClaw".to_string()),
        last_primary_task_output: Some("RustClaw is easier for non-technical users.".to_string()),
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
        "RustClaw deployment should use Python 3.10.",
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("Write one deployment note that mentions Python 3.10")
    );
    assert_eq!(
        output.as_deref(),
        Some("RustClaw deployment should use Python 3.10.")
    );
}

#[test]
fn standalone_task_request_without_prior_can_start_primary_task() {
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
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
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
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
fn unannotated_direct_chat_substantial_text_starts_primary_task() {
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    let answer = "# Deployment Note\n\n- Use Python 3.10 for deployment.\n- Confirm runtime variables before release.\n- Keep rollback steps available.";

    let prompt = super::unannotated_chat_primary_prompt_for_output(
        None,
        &route_result,
        None,
        "Write one deployment note that mentions Python 3.10",
        "Write one deployment note that mentions Python 3.10",
        answer,
        &[],
    );
    let output = next_last_primary_task_output(
        None,
        &route_result,
        None,
        "Write one deployment note that mentions Python 3.10",
        answer,
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("Write one deployment note that mentions Python 3.10")
    );
    assert_eq!(output.as_deref(), Some(answer));
}

#[test]
fn unannotated_short_chat_does_not_start_primary_task() {
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    let answer = "RC-CONT-CN-0428-A";

    let prompt = super::unannotated_chat_primary_prompt_for_output(
        None,
        &route_result,
        None,
        "What was the saved code?",
        "What was the saved code?",
        answer,
        &[],
    );
    let output = next_last_primary_task_output(
        None,
        &route_result,
        None,
        "What was the saved code?",
        answer,
        &[],
    );

    assert!(prompt.is_none());
    assert!(output.is_none());
}

#[test]
fn unannotated_compact_sentence_chat_starts_primary_task() {
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    let answer = "部署前请确保已安装 Python 3.10 或更高版本。";

    let prompt = super::unannotated_chat_primary_prompt_for_output(
        None,
        &route_result,
        None,
        "帮我写一句部署说明，要求提到 Python 3.10",
        "帮我写一句部署说明，要求提到 Python 3.10",
        answer,
        &[],
    );
    let output = next_last_primary_task_output(
        None,
        &route_result,
        None,
        "帮我写一句部署说明，要求提到 Python 3.10",
        answer,
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("帮我写一句部署说明，要求提到 Python 3.10")
    );
    assert_eq!(output.as_deref(), Some(answer));
}

#[test]
fn unannotated_compact_question_chat_does_not_start_primary_task() {
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    let answer = "请问是哪方面的方案？比如功能设计、项目计划、技术选型，还是其他？";

    let prompt = super::unannotated_chat_primary_prompt_for_output(
        None,
        &route_result,
        None,
        "帮我写个方案",
        "帮我写个方案",
        answer,
        &[],
    );
    let output =
        next_last_primary_task_output(None, &route_result, None, "帮我写个方案", answer, &[]);

    assert!(prompt.is_none());
    assert!(output.is_none());
}

#[test]
fn standalone_freeform_answer_candidate_with_prior_preserves_primary_task() {
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
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
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
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
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: Some(json!({"target": "Python 3.10 -> Python 3.11"})),
        attachment_processing_required: false,
    };
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("Write a short release note for RustClaw".to_string()),
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
    let mut route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    route_result.should_refresh_long_term_memory = true;
    route_result.agent_display_name_hint = "巡检爪".to_string();
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
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
fn memory_grounded_comparison_chat_becomes_latest_primary_task() {
    let mut route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let prior_state = ConversationState {
        last_primary_task_prompt: Some(
            "再看一下 scripts/nl_tests/fixtures/device_local/logs 目录有多少个直接子项，只输出数字"
                .to_string(),
        ),
        last_primary_task_output: Some("2".to_string()),
        ..ConversationState::default()
    };

    let prompt = next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        Some(&crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        "上上个目录和上个目录相比，哪个直接子项更多？只用一句话",
        "比较 docs（3个直接子项）和 logs（2个直接子项）的直接子项数量。",
    );
    let output = next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        Some(&crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        "比较 docs（3个直接子项）和 logs（2个直接子项）的直接子项数量。",
        "上上个目录（docs）的直接子项更多，有3个，而上个目录（logs）是2个。",
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("上上个目录和上个目录相比，哪个直接子项更多？只用一句话")
    );
    assert_eq!(
        output.as_deref(),
        Some("上上个目录（docs）的直接子项更多，有3个，而上个目录（logs）是2个。")
    );
}

#[test]
fn standalone_answer_candidate_request_without_prior_does_not_start_primary_task() {
    let mut route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
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
    let mut route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
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
    let mut route_result =
        route_result_for_test(crate::AskMode::planner_execute_with_chat_finalizer(), false);
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
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
        "Write a short release note for RustClaw.",
        "Write a short release note for RustClaw.",
    );
    let output = next_last_primary_task_output(
        Some(&prior_state),
        &route_result,
        Some(&turn_analysis),
        "Write a short release note for RustClaw.",
        "RustClaw 0.1.7 is now available.",
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("Write a short release note for RustClaw.")
    );
    assert_eq!(output.as_deref(), Some("RustClaw 0.1.7 is now available."));
}

#[test]
fn unannotated_evidence_backed_deliverable_starts_primary_task() {
    let mut route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

    let prompt = next_last_primary_task_prompt(
        None,
        &route_result,
        None,
        "Write a short release note for RustClaw",
        "Write a short release note for RustClaw",
    );
    let output = next_last_primary_task_output(
        None,
        &route_result,
        None,
        "Write a short release note for RustClaw",
        "RustClaw 0.1.7 is easier to update and operate.",
        &[],
    );

    assert_eq!(
        prompt.as_deref(),
        Some("Write a short release note for RustClaw")
    );
    assert_eq!(
        output.as_deref(),
        Some("RustClaw 0.1.7 is easier to update and operate.")
    );
}

#[test]
fn unannotated_structured_listing_replaces_prior_primary_task() {
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
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
    let mut route_result =
        route_result_for_test(crate::AskMode::planner_execute_with_chat_finalizer(), false);
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;

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
    let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
    let prior_state = ConversationState {
        last_primary_task_prompt: Some("帮我写个方案".to_string()),
        ..ConversationState::default()
    };
    let persisted = next_last_primary_task_prompt(
        Some(&prior_state),
        &route_result,
        Some(&crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskAppend),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
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
        crate::intent_router::TurnType::TaskAppend,
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
fn alias_only_state_patch_clears_stale_active_pointers() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = ClaimedTask {
        task_id: "task-alias-update".to_string(),
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
    let route = route_result_for_test(crate::AskMode::direct_answer(), false);
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "alias_bindings": [{
                "alias": "ALPHA_DOC",
                "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
            }]
        })),
        attachment_processing_required: false,
    };

    super::update_active_session_from_ask_outcome(
        &state,
        &task,
        None,
        "Correction: ALPHA_DOC now refers to scripts/nl_tests/fixtures/device_local/docs/release_checklist.md.",
        &route,
        Some(&turn_analysis),
        "Update ALPHA_DOC alias to point to scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
        "alias updated via i18n",
        &[],
        false,
        &[],
        &empty_journal_for_test(),
        None,
    );

    let loaded = super::load_active_conversation_state(&state, &task).expect("state");
    assert!(loaded.active_followup_task_id.is_none());
    assert!(loaded.active_clarify_task_id.is_none());
    assert!(loaded.active_observed_facts_task_id.is_none());
    assert!(loaded.alias_bindings.iter().any(|binding| {
        binding.alias == "ALPHA_DOC"
            && binding.target == "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    }));
}

#[test]
fn merge_alias_bindings_prefers_structured_state_patch() {
    let prior = ConversationState {
        alias_bindings: vec![SessionAliasBinding {
            alias: "那个文件".to_string(),
            target: "/tmp/old.md".to_string(),
            updated_at_ts: 1,
        }],
        ..ConversationState::default()
    };
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "alias_bindings": [
                {"alias": "那个文件", "target": "/tmp/new.md"},
                {"alias": "那个日志", "target": "/tmp/app.log"}
            ]
        })),
        attachment_processing_required: false,
    };
    let merged = super::merge_alias_bindings(Some(&prior), Some(&turn_analysis));
    assert_eq!(merged.len(), 2);
    assert!(merged
        .iter()
        .any(|binding| binding.alias == "那个文件" && binding.target == "/tmp/new.md"));
    assert!(merged
        .iter()
        .any(|binding| { binding.alias == "那个日志" && binding.target == "/tmp/app.log" }));
    assert!(!merged
        .iter()
        .any(|binding| binding.target == "/tmp/regex.md"));
}

#[test]
fn structured_alias_state_patch_suppresses_prompt_alias_heuristics() {
    let route = route_result_for_test(crate::AskMode::direct_answer(), false);
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: None,
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "alias_bindings": [{
                "alias": "ALPHA_DOC",
                "target": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
            }]
        })),
        attachment_processing_required: false,
    };

    let merged = super::merge_alias_bindings_for_turn(
        None,
        Some(&turn_analysis),
        "For this conversation, remember that ALPHA_DOC refers to scripts/nl_tests/fixtures/device_local/docs/service_notes.md. Reply only remembered.",
        &route,
        "Establish ALPHA_DOC as a temporary alias for scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
    );

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].alias, "ALPHA_DOC");
    assert_eq!(
        merged[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );
}

#[test]
fn merge_alias_bindings_accepts_alias_key_compatibility_patch() {
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "that_file_alias": "/tmp/device/README.md",
            "currentFolderAlias": {"target": "/tmp/device/docs"}
        })),
        attachment_processing_required: false,
    };

    let merged = super::merge_alias_bindings(None, Some(&turn_analysis));

    assert_eq!(merged.len(), 2);
    assert!(merged
        .iter()
        .any(|binding| binding.alias == "that_file" && binding.target == "/tmp/device/README.md"));
    assert!(merged
        .iter()
        .any(|binding| binding.alias == "currentFolder" && binding.target == "/tmp/device/docs"));
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
fn memory_turn_with_single_locator_derives_short_alias_suffixes() {
    let mut route = route_result_for_test(crate::AskMode::direct_answer(), false);
    route.should_refresh_long_term_memory = true;
    let merged = super::merge_alias_bindings_for_turn(
        None,
        None,
        "Remember that the note file means scripts/nl_tests/fixtures/device_local/docs/service_notes.md. Reply only confirmed.",
        &route,
        "",
    );

    assert!(merged.iter().any(|binding| {
        binding.alias == "note file"
            && binding.target == "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    }));
}

#[test]
fn preference_memory_turn_with_single_locator_derives_alias_without_refresh_flag() {
    let route = route_result_for_test(crate::AskMode::direct_answer(), false);
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    let merged = super::merge_alias_bindings_for_turn(
        None,
        Some(&turn_analysis),
        "Remember that the note file means scripts/nl_tests/fixtures/device_local/docs/service_notes.md. Reply only confirmed.",
        &route,
        "",
    );

    assert!(merged.iter().any(|binding| {
        binding.alias == "note file"
            && binding.target == "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    }));
}

#[test]
fn preference_memory_turn_with_machine_alias_derives_exact_token() {
    let route = route_result_for_test(crate::AskMode::direct_answer(), false);
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    let merged = super::merge_alias_bindings_for_turn(
        None,
        Some(&turn_analysis),
        "For this conversation, remember that ALPHA_DOC refers to scripts/nl_tests/fixtures/device_local/docs/service_notes.md. Reply only remembered.",
        &route,
        "",
    );

    assert!(merged.iter().any(|binding| {
        binding.alias == "ALPHA_DOC"
            && binding.target == "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    }));
    assert!(!merged
        .iter()
        .any(|binding| binding.alias.contains("refers")));
}

#[test]
fn compact_alias_memory_turn_with_single_locator_derives_structured_binding() {
    let route = route_result_for_test(crate::AskMode::direct_answer(), false);
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    let merged = super::merge_alias_bindings_for_turn(
        None,
        Some(&turn_analysis),
        "先记一下，甲文件是 scripts/nl_tests/fixtures/device_local/docs/service_notes.md。只回复已记住。",
        &route,
        "",
    );

    assert!(merged.iter().any(|binding| {
        binding.alias == "甲文件"
            && binding.target == "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    }));
}

#[test]
fn chat_ack_single_locator_derives_compact_alias_without_memory_turn_classification() {
    let route = route_result_for_test(crate::AskMode::direct_answer(), false);
    let merged = super::merge_alias_bindings_for_turn(
        None,
        None,
        "先记一下，甲文件是 scripts/nl_tests/fixtures/device_local/docs/service_notes.md。只回复已记住。",
        &route,
        "",
    );

    assert!(merged.iter().any(|binding| {
        binding.alias == "甲文件"
            && binding.target == "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    }));
}

#[test]
fn planner_execute_single_locator_does_not_create_prompt_alias_binding() {
    let route = route_result_for_test(crate::AskMode::planner_execute_plain(), false);
    let merged = super::merge_alias_bindings_for_turn(
        None,
        None,
        "读取 scripts/nl_tests/fixtures/device_local/docs/service_notes.md 的标题",
        &route,
        "",
    );

    assert!(merged.is_empty());
}

#[test]
fn quoted_alias_with_single_locator_binds_without_memory_turn_analysis() {
    let route = route_result_for_test(crate::AskMode::clarify(), true);
    let merged = super::merge_alias_bindings_for_turn(
        None,
        None,
        "先记一下，后面我说“那个文件”就是 /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md",
        &route,
        "",
    );

    assert!(merged.iter().any(|binding| {
        binding.alias == "那个文件"
            && binding.target
                == "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md"
    }));
}

#[test]
fn current_locator_rebinds_existing_alias_without_language_phrase_table() {
    let prior = ConversationState {
        alias_bindings: vec![SessionAliasBinding {
            alias: "note file".to_string(),
            target: "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string(),
            updated_at_ts: 1,
        }],
        ..ConversationState::default()
    };

    let bindings = super::structural_alias_rebinds_from_prompt(
        Some(&prior),
        "Correction: the note file now means scripts/nl_tests/fixtures/device_local/docs/release_checklist.md. Reply only updated.",
    );
    let binding = bindings
        .first()
        .expect("existing alias should rebind to the current locator");

    assert_eq!(binding.alias, "note file");
    assert_eq!(
        binding.target,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
}

#[test]
fn compact_alias_current_locator_rebinds_existing_alias() {
    let prior = ConversationState {
        alias_bindings: vec![SessionAliasBinding {
            alias: "甲文件".to_string(),
            target: "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string(),
            updated_at_ts: 1,
        }],
        ..ConversationState::default()
    };

    let bindings = super::structural_alias_rebinds_from_prompt(
        Some(&prior),
        "不对，甲文件改成 scripts/nl_tests/fixtures/device_local/docs/release_checklist.md。只回复已更新。",
    );
    let binding = bindings
        .first()
        .expect("existing compact alias should rebind to the current locator");

    assert_eq!(binding.alias, "甲文件");
    assert_eq!(
        binding.target,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
}

#[test]
fn current_locator_rebinds_all_mentioned_alias_surfaces() {
    let prior = ConversationState {
        alias_bindings: vec![
            SessionAliasBinding {
                alias: "note file".to_string(),
                target: "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string(),
                updated_at_ts: 1,
            },
            SessionAliasBinding {
                alias: "the note file".to_string(),
                target: "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string(),
                updated_at_ts: 1,
            },
        ],
        ..ConversationState::default()
    };

    let bindings = super::structural_alias_rebinds_from_prompt(
        Some(&prior),
        "Correction: the note file now means scripts/nl_tests/fixtures/device_local/docs/release_checklist.md. Reply only updated.",
    );

    assert_eq!(bindings.len(), 2);
    assert!(bindings.iter().all(|binding| {
        binding.target == "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    }));
}

#[test]
fn state_patch_accepts_path_like_direct_alias_map() {
    let patch = json!({
        "甲文件": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "甲文件");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
}

#[test]
fn state_patch_rejects_non_locator_direct_alias_map() {
    let patch = json!({
        "甲文件": "the checklist from before"
    });

    assert!(!super::state_patch_is_alias_bindings_only(&patch));
    assert!(super::session_alias_bindings_from_state_patch(Some(&patch)).is_empty());
}

#[test]
fn structural_prompt_alias_binding_uses_quote_and_single_locator() {
    let mut route = route_result_for_test(crate::AskMode::direct_answer(), false);
    route.risk_ceiling = crate::RiskCeiling::Low;

    let binding = super::structural_alias_binding_from_prompt(
        "先记一下，后面我说“那个文件”就是 /tmp/device/README.md",
        &route,
        "remember that quoted alias maps to /tmp/device/README.md",
    )
    .expect("binding");

    assert_eq!(binding.alias, "那个文件");
    assert_eq!(binding.target, "/tmp/device/README.md");
}

#[test]
fn merge_alias_bindings_ignores_prompt_text_without_structured_patch() {
    let prior = ConversationState {
        alias_bindings: vec![SessionAliasBinding {
            alias: "那个文件".to_string(),
            target: "/tmp/old.md".to_string(),
            updated_at_ts: 1,
        }],
        ..ConversationState::default()
    };
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let merged = super::merge_alias_bindings(Some(&prior), Some(&turn_analysis));
    assert_eq!(merged, prior.alias_bindings);
}

#[test]
fn meta_turn_types_preserve_active_session_pointers() {
    for turn_type in [
        crate::intent_router::TurnType::RunControl,
        crate::intent_router::TurnType::ApprovalDecision,
        crate::intent_router::TurnType::StatusQuery,
        crate::intent_router::TurnType::FeedbackOrError,
        crate::intent_router::TurnType::PreferenceOrMemory,
    ] {
        assert!(super::should_preserve_active_session_pointers(Some(
            &crate::intent_router::TurnAnalysis {
                turn_type: Some(turn_type),
                target_task_policy: None,
                should_interrupt_active_run: false,
                state_patch: None,
                attachment_processing_required: false,
            }
        )));
    }
    assert!(!super::should_preserve_active_session_pointers(Some(
        &crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskAppend),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
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
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
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
    let mut route_result = route_result_for_test(crate::AskMode::clarify(), true);
    route_result.resolved_intent = "帮我写个方案".to_string();
    route_result.clarify_question = "请补充主题".to_string();
    route_result.route_reason = "clarify".to_string();
    let persisted = next_last_primary_task_prompt(
        None,
        &route_result,
        Some(&crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
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
fn clarify_task_prompt_without_turn_analysis_is_preserved_when_not_locator_driven() {
    let mut route_result = route_result_for_test(crate::AskMode::clarify(), true);
    route_result.resolved_intent = "Help me write a proposal".to_string();
    route_result.clarify_question = "What is the topic and audience?".to_string();
    route_result.route_reason = "missing_task_slots".to_string();
    let persisted = next_last_primary_task_prompt(
        None,
        &route_result,
        None,
        "Help me write a proposal",
        "Help me write a proposal",
    );
    assert_eq!(persisted.as_deref(), Some("Help me write a proposal"));
}

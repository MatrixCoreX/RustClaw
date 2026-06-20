use serde_json::json;

use super::{
    binding_context_json, resolve_resume_runtime_prompt, select_resume_runtime_binding,
    ResumeContextBinding, ResumeContextSource,
};

fn test_task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: "resume-task".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: Some("test-user".to_string()),
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn route_with_resume_behavior(resume_behavior: crate::ResumeBehavior) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "continue".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    }
}

#[test]
fn binding_context_marks_recent_failed_candidate_without_mutating_source() {
    let binding = ResumeContextBinding {
        source: ResumeContextSource::RecentFailedCandidate,
        resume_context: json!({"resume_context_id":"ctx-1"}),
        failed_ts: Some(42),
        has_newer_successful_ask_after_failed_task: true,
    };
    let value = binding_context_json("manual", false, Some(&binding));
    assert_eq!(
        value.get("resume_context_source").and_then(|v| v.as_str()),
        Some("recent_failed_resume_candidate")
    );
    assert_eq!(
        value
            .get("is_resume_continue_source")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        value
            .get("has_newer_successful_ask_after_failed_task")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn binding_context_marks_active_checkpoint_candidate() {
    let binding = ResumeContextBinding {
        source: ResumeContextSource::ActiveCheckpointCandidate,
        resume_context: json!({"source":"active_checkpoint_resume"}),
        failed_ts: None,
        has_newer_successful_ask_after_failed_task: false,
    };
    let value = binding_context_json("manual", false, Some(&binding));
    assert_eq!(
        value.get("resume_context_source").and_then(|v| v.as_str()),
        Some("active_checkpoint_resume")
    );
    assert_eq!(
        value
            .get("failed_resume_context_ts")
            .and_then(|v| v.as_i64()),
        None
    );
    assert_eq!(
        value
            .get("has_newer_successful_ask_after_failed_task")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[test]
fn runtime_resume_binding_is_disabled_when_normalizer_rejects_resume() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "list current workspace".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    let binding = ResumeContextBinding {
        source: ResumeContextSource::RecentFailedCandidate,
        resume_context: json!({"resume_context_id":"ctx-2"}),
        failed_ts: Some(7),
        has_newer_successful_ask_after_failed_task: false,
    };
    assert!(select_resume_runtime_binding(&route, Some(&binding)).is_none());
}

#[test]
fn explicit_continue_prefers_payload_resume_prompt() {
    let state =
        crate::AppState::test_default_with_fixture_provider().with_prompt_layers_installed();
    let payload = json!({
        "resume_user_text": "继续修复",
        "resume_context": {"remaining_steps":["step-a"]},
        "resume_instruction": "继续执行剩余步骤",
        "resume_steps": ["step-a"]
    });
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "continue".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::ResumeExecute,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    let binding = ResumeContextBinding {
        source: ResumeContextSource::ExplicitContinue,
        resume_context: json!({"remaining_steps":["step-a"]}),
        failed_ts: None,
        has_newer_successful_ask_after_failed_task: false,
    };
    let task = test_task();
    let out =
        resolve_resume_runtime_prompt(&state, &task, &payload, "继续修复", &route, Some(&binding));
    assert!(out.should_apply_context);
    assert!(!out.should_discuss_context);
    assert!(out.runtime_prompt.contains("step-a"));
}

#[test]
fn explicit_continue_falls_back_to_resolved_intent_when_prompt_missing() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let payload = json!({
        "resume_user_text": "继续修复",
        "resume_context": {"remaining_steps":["step-a"]},
        "resume_instruction": "继续执行剩余步骤",
        "resume_steps": ["step-a"]
    });
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "continue".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::ResumeExecute,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    let binding = ResumeContextBinding {
        source: ResumeContextSource::ExplicitContinue,
        resume_context: json!({"remaining_steps":["step-a"]}),
        failed_ts: None,
        has_newer_successful_ask_after_failed_task: false,
    };
    let task = test_task();
    let out =
        resolve_resume_runtime_prompt(&state, &task, &payload, "继续修复", &route, Some(&binding));
    assert!(out.should_apply_context);
    assert_eq!(out.runtime_prompt, "continue");
}

#[test]
fn active_checkpoint_resume_prompts_render_machine_context_without_failure_framing() {
    let state =
        crate::AppState::test_default_with_fixture_provider().with_prompt_layers_installed();
    let payload = json!({});
    let binding = ResumeContextBinding {
        source: ResumeContextSource::ActiveCheckpointCandidate,
        resume_context: json!({
            "source": "active_checkpoint_resume",
            "task_id": "task-123",
            "task_lifecycle": {
                "state": "background",
                "resume_executor": {
                    "checkpoint_id": "checkpoint-1",
                    "executor_state": "ready_for_planner_resume"
                }
            },
            "task_checkpoint": {
                "checkpoint_id": "checkpoint-1"
            }
        }),
        failed_ts: None,
        has_newer_successful_ask_after_failed_task: false,
    };
    let task = test_task();

    let execute_route = route_with_resume_behavior(crate::ResumeBehavior::ResumeExecute);
    let execute = resolve_resume_runtime_prompt(
        &state,
        &task,
        &payload,
        "继续",
        &execute_route,
        Some(&binding),
    );
    assert!(execute.should_apply_context);
    assert!(!execute.should_discuss_context);
    assert!(execute.runtime_prompt.contains("active_checkpoint_resume"));
    assert!(execute.runtime_prompt.contains("checkpoint-1"));
    assert!(execute.runtime_prompt.contains("ready_for_planner_resume"));
    assert!(!execute.runtime_prompt.contains("prior failure"));

    let discuss_route = route_with_resume_behavior(crate::ResumeBehavior::ResumeDiscuss);
    let discuss = resolve_resume_runtime_prompt(
        &state,
        &task,
        &payload,
        "现在还剩什么",
        &discuss_route,
        Some(&binding),
    );
    assert!(!discuss.should_apply_context);
    assert!(discuss.should_discuss_context);
    assert!(discuss.runtime_prompt.contains("active_checkpoint_resume"));
    assert!(discuss.runtime_prompt.contains("checkpoint-1"));
    assert!(discuss.runtime_prompt.contains("ready_for_planner_resume"));
    assert!(!discuss.runtime_prompt.contains("prior failure"));
}

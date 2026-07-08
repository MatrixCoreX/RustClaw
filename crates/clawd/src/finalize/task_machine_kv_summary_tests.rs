use super::{
    apply_requested_machine_kv_summary_to_final_answer,
    web_search_candidate_title_sources_from_output,
};
use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind,
};
use serde_json::json;

#[test]
fn web_search_candidate_sources_ignore_visible_text_payload() {
    let output = r#"{"extra":{"candidates":[{"title":"Observed","source":"example.com"}]},"text":"{\"candidates\":[{\"title\":\"must_not_parse_text\",\"source\":\"bad.example\"}]}"}"#;

    let pairs = web_search_candidate_title_sources_from_output(output);

    assert!(pairs.iter().any(|(title, _)| title == "Observed"));
    assert!(!pairs
        .iter()
        .any(|(title, _)| title == "must_not_parse_text"));
}

#[test]
fn service_status_final_guard_preserves_observed_one_sentence_status_summary() {
    let mut route = service_status_one_sentence_route();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.route_reason = "agent_loop_default_entry".to_string();
    let prompt = ["check", "ssh", "service", "status"].join(" ");
    let service_name = "sshd";
    let observed_state = "active";
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-service-kv-preserve-status-summary",
        "ask",
        &prompt,
    );
    journal.record_route_result(&route);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "service_control",
            json!({
                "status": "ok",
                "target": service_name,
                "service_name": service_name,
                "manager_type": "systemd",
                "requested_action": "status",
                "executed_actions": ["status"],
                "pre_state": observed_state,
                "post_state": observed_state,
                "verified": true,
                "summary": format!("Status: {observed_state}")
            })
            .to_string(),
        ));

    let expected_answer = [service_name, observed_state].join(" ");
    let mut answer_text = expected_answer.clone();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        &prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));
    assert_eq!(answer_text, expected_answer);
    assert_eq!(answer_messages, vec![expected_answer]);
}

#[test]
fn generic_route_preserves_structured_media_dry_run_report_over_short_machine_summary() {
    let prompt = "return dry_run=true provider/model planned_outputs and output_path";
    let route = generic_free_route(prompt);
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-generic-media-dry-run-kv-preserve",
        "ask",
        prompt,
    );
    journal.record_route_result(&route);
    let expected_answer = concat!(
        "dry_run=true\n",
        "provider=minimax\n",
        "model=image-01\n",
        "model_kind=dry_run\n",
        "output_path=/home/guagua/rustclaw/document/media_dry_run/image_status_card.png\n",
        "planned_outputs=[{\"path\":\"/home/guagua/rustclaw/document/media_dry_run/image_status_card.png\",\"type\":\"image_file\"}]\n",
        "pending_async_job_contract={\"job_id\":\"provider:image_generate:minimax:dry_run\",\"status\":\"accepted\"}"
    );
    let mut answer_text = expected_answer.to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));
    assert_eq!(answer_text, expected_answer);
    assert_eq!(answer_messages, vec![expected_answer.to_string()]);
}

fn generic_free_route(prompt: &str) -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: prompt.to_string(),
        needs_clarify: false,
        route_reason: "agent_loop_default_entry".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Medium,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

fn service_status_one_sentence_route() -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: ["check", "service", "status"].join(" "),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

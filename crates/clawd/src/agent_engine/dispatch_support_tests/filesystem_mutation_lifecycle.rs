use super::{filesystem_mutation_lifecycle_structured_answer, ok_step};
use crate::agent_engine::{AgentRunContext, LoopState};

#[test]
fn filesystem_mutation_lifecycle_structured_answer_combines_all_steps() {
    let mut loop_state = LoopState::new(1);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"make_dir","path":"tmp/nl_codex_resume_smoke","resolved_path":"/home/guagua/rustclaw/tmp/nl_codex_resume_smoke"},"text":"created directory /home/guagua/rustclaw/tmp/nl_codex_resume_smoke"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"tmp/nl_codex_resume_smoke/note.txt","resolved_path":"/home/guagua/rustclaw/tmp/nl_codex_resume_smoke/note.txt","content_bytes":6},"text":"written 6 bytes to /home/guagua/rustclaw/tmp/nl_codex_resume_smoke/note.txt"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "fs_basic",
        r#"{"extra":{"action":"append_text","path":"tmp/nl_codex_resume_smoke/note.txt","resolved_path":"/home/guagua/rustclaw/tmp/nl_codex_resume_smoke/note.txt","append":true,"content_bytes":5},"text":"appended 5 bytes to /home/guagua/rustclaw/tmp/nl_codex_resume_smoke/note.txt"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"tmp/nl_codex_resume_smoke/note.txt","resolved_path":"/home/guagua/rustclaw/tmp/nl_codex_resume_smoke/note.txt","excerpt":"1|alpha\n2|beta","total_lines":2},"text":"{}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_5",
        "fs_basic",
        r#"{"extra":{"action":"remove_path","path":"tmp/nl_codex_resume_smoke","resolved_path":"/home/guagua/rustclaw/tmp/nl_codex_resume_smoke","target_kind":"directory","recursive":true},"text":"removed /home/guagua/rustclaw/tmp/nl_codex_resume_smoke"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_6",
        "synthesize_answer",
        "removed /home/guagua/rustclaw/tmp/nl_codex_resume_smoke",
    ));
    let ctx = AgentRunContext {
        route_result: Some(filesystem_mutation_route()),
        ..AgentRunContext::default()
    };

    let answer = filesystem_mutation_lifecycle_structured_answer(&loop_state, Some(&ctx))
        .expect("filesystem lifecycle answer");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json answer");

    assert_eq!(
        value
            .pointer("/semantic_kind")
            .and_then(serde_json::Value::as_str),
        Some("filesystem_mutation_result")
    );
    assert_eq!(
        value
            .pointer("/steps")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(5)
    );
    assert_eq!(
        value
            .pointer("/readbacks/0/excerpt")
            .and_then(serde_json::Value::as_str),
        Some("1|alpha\n2|beta")
    );
    assert_eq!(
        value
            .pointer("/final_state/cleanup_observed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert!(answer.contains("note.txt"), "answer: {answer}");
    assert!(answer.contains("alpha"), "answer: {answer}");
    assert!(answer.contains("beta"), "answer: {answer}");
    assert!(answer.contains("remove_path"), "answer: {answer}");
}

fn filesystem_mutation_route() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "scratch filesystem lifecycle".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "filesystem mutation result".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::High,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::FilesystemMutationResult,
            locator_hint: "tmp/nl_codex_resume_smoke".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

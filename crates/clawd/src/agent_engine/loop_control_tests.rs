use super::{
    answer_contract_route_result_for_reply, answer_verifier_gap_has_observed_content_evidence,
    answer_verifier_gap_requests_observed_content_rewrite,
    answer_verifier_output_format_machine_payload_gap, answer_verifier_retry_budget_available,
    answer_verifier_retry_summary, apply_structured_respond_clarify_to_loop_state,
    commit_answer_verifier_retry_answer, commit_local_code_strict_json_projection_after_readback,
    evaluate_round_outcome, executable_contract_observe_only_round_should_continue,
    forced_boundary_observation_clarify_intent, initial_execution_recipe_spec,
    machine_status_visible_output_format_gap, mark_reply_failed_after_answer_verifier_exhausted,
    parse_log_analyze_finding, post_write_content_evidence_readback_recovery_policy,
    prefer_terminal_model_answer_for_verifier_candidate,
    promote_local_code_projection_from_machine_evidence_for_verifier_candidate,
    promote_publishable_strict_json_projection_for_verifier_candidate,
    record_agent_loop_decision_envelope_output_vars, retry_verifier_accepts_rewritten_answer,
    should_stop_for_observed_finalize, structured_respond_terminal_intent_from_plan,
    structured_respond_terminal_intent_from_pre_loop_clarify_candidate,
    structured_respond_terminal_intent_from_route_owned_clarify,
    suppress_answer_verifier_retry_if_confirmed_missing_file_delivery,
    suppress_answer_verifier_retry_if_structurally_satisfied,
    suppress_answer_verifier_retry_if_user_locator_disambiguation,
    terminal_user_answer_stop_signal, text_has_exact_marker_line,
    try_accept_language_only_output_format_answer_verifier_gap,
    try_preserve_rss_source_hosts_from_structured_evidence,
    try_recover_content_excerpt_summary_answer_verifier_gap,
    try_recover_document_heading_answer_verifier_gap,
    try_recover_filesystem_mutation_success_answer_verifier_gap,
    try_recover_generic_path_content_read_range_answer_verifier_gap,
    try_recover_http_health_answer_verifier_gap, try_recover_inconsistent_boundary_clarify,
    try_recover_latest_synthesis_answer_verifier_gap, try_recover_local_health_answer_verifier_gap,
    try_recover_local_health_answer_verifier_gap_from_loop_state,
    try_recover_log_analyze_answer_verifier_gap,
    try_recover_machine_kv_summary_output_format_answer_verifier_gap,
    try_recover_recent_artifacts_answer_verifier_gap, try_recover_rss_news_answer_verifier_gap,
    try_recover_structured_count_answer_verifier_gap,
    try_recover_structured_evidence_table_answer_verifier_gap,
    try_recover_structured_listing_answer_verifier_gap,
    try_recover_structured_scalar_output_format_answer_verifier_gap,
    try_recover_structured_search_answer_verifier_gap,
    try_replan_avoidable_side_effect_free_freeform_clarify, AgentLoopGuardPolicy, RoundOutcome,
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
    AgentAction, AskReply, ClaimedTask, IntentOutputContract, OutputDeliveryIntent,
    OutputLocatorKind, OutputResponseShape, OutputSemanticKind, ResumeBehavior, RiskCeiling,
    RouteResult, ScheduleKind,
};
use serde_json::json;

#[test]
fn success_marker_matching_requires_exact_line() {
    assert!(!text_has_exact_marker_line(
        "status=ok\nVALIDATION_PASSED_EXTRA",
        "VALIDATION_PASSED",
    ));
    assert!(text_has_exact_marker_line(
        "status=ok\nVALIDATION_PASSED\nnext=done",
        "VALIDATION_PASSED",
    ));
}

fn route_result(shape: OutputResponseShape) -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "test".to_string(),
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
            response_shape: shape,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

#[test]
fn answer_contract_route_result_prefers_journal_effective_route() {
    let original_route = route_result(OutputResponseShape::Free);
    let mut effective_route = original_route.clone();
    effective_route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    effective_route.output_contract.locator_kind = OutputLocatorKind::None;
    effective_route.output_contract.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(original_route),
        execution_recipe_hint: None,
        execution_recipe_plan_hint: None,
        turn_analysis: None,
        boundary_envelope: None,
        context_bundle_summary: None,
        session_alias_bindings: Vec::new(),
        auto_locator_path: None,
        original_user_request: None,
        user_request: None,
        cross_turn_recent_execution_context: None,
    };
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-effective-route",
        "ask",
        "probe service status",
    );
    journal.record_route_result(&effective_route);
    let reply = AskReply::non_llm("ok".to_string()).with_task_journal(journal);

    let selected = answer_contract_route_result_for_reply(Some(&agent_run_context), &reply)
        .expect("selected route");

    assert_eq!(
        selected.effective_output_contract_semantic_kind(),
        OutputSemanticKind::ServiceStatus
    );
    assert_eq!(
        crate::evidence_policy::required_evidence_fields_for_route(&selected),
        vec!["field_value".to_string()]
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

fn sample_rss_news_output() -> &'static str {
    r#"{"extra":{"action":"latest","category":"general","field_value":{"items":3,"sources_failed":0,"sources_ok":2,"titles":["What a hair loss breakthrough could mean for women like me","Louisiana ICE Facility Mistreated Immigrants, Federal Investigators Say","New NHS drug offers ovarian cancer patients more time and better quality of life"]},"item_count":3,"items":[{"date":"Wed, 03 Jun 2026 23:42:35 GMT","layer":"feed","source":"https://feeds.bbci.co.uk/news/rss.xml","source_host":"feeds.bbci.co.uk","title":"What a hair loss breakthrough could mean for women like me","topic":"other"},{"date":"Wed, 03 Jun 2026 23:40:01 +0000","layer":"feed","source":"https://rss.nytimes.com/services/xml/rss/nyt/HomePage.xml","source_host":"rss.nytimes.com","title":"Louisiana ICE Facility Mistreated Immigrants, Federal Investigators Say","topic":"macro_market"},{"date":"Wed, 03 Jun 2026 23:34:59 GMT","layer":"feed","source":"https://feeds.bbci.co.uk/news/rss.xml","source_host":"feeds.bbci.co.uk","title":"New NHS drug offers ovarian cancer patients more time and better quality of life","topic":"other"}],"mode":"category","schema_version":1,"source_count":2,"sources_failed":0,"sources_ok":2},"text":"sources_ok=2 sources_failed=0 items=3"}"#
}

fn test_task() -> ClaimedTask {
    ClaimedTask {
        task_id: "task-loop-control".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
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
        steps,
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: raw_plan_text.to_string(),
    }
}

#[test]
fn parses_truncated_log_analyze_output_prefix() {
    let finding = parse_log_analyze_finding(
        r#"{"keyword_counts":{"error":115,"failed":48,"panic":23,"timeout":26,"warn":72},"path":"/tmp/logs/clawd.run.log","recent_matches":["... ...(truncated)""#,
    )
    .expect("truncated prefix still contains counts and path");

    assert_eq!(finding.path, "/tmp/logs/clawd.run.log");
    assert_eq!(finding.total_hits, 284);
    assert_eq!(finding.keyword_counts[0], ("error".to_string(), 115));
}

#[test]
fn log_analyze_verifier_exhaustion_recovers_with_structural_summary() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["clawd.run.log".to_string()],
        answer_incomplete_reason: "candidate omitted clawd.run.log counts".to_string(),
        should_retry: true,
        retry_instruction: "include every analyzed log".to_string(),
        confidence: 0.95,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "log_analyze".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"keyword_counts":{"error":1286,"failed":939,"timeout":308},"path":"/logs/model_io.log.2026-05-13","recent_matches":[]}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_2".to_string(),
        skill: "log_analyze".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"keyword_counts":{"error":115,"warn":72,"failed":48},"path":"/logs/clawd.run.log","recent_matches":["...(truncated)"]}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut reply = AskReply::non_llm("partial answer".to_string())
        .with_messages(vec![
            "**执行过程**\n1. 调用技能 `log_analyze`。".to_string(),
            "partial answer".to_string(),
        ])
        .with_task_journal(journal);

    assert!(try_recover_log_analyze_answer_verifier_gap(
        "快速看一下 logs 目录里最近最值得注意的错误或异常",
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply
        .text
        .contains("message_key=clawd.msg.log_analyze.summary"));
    assert!(reply
        .text
        .contains("reason_code=log_analyze_observed_summary"));
    assert!(reply.text.contains("model_io.log.2026-05-13"));
    assert!(reply.text.contains("clawd.run.log"));
    assert!(reply.text.contains("error 115"));
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn structured_search_verifier_exhaustion_recovers_with_full_candidate_list() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string()],
        answer_incomplete_reason:
            "answer reports 1 README file but observed evidence shows 3 README files".to_string(),
        should_retry: true,
        retry_instruction: "answer from the full observed results array".to_string(),
        confidence: 0.94,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"action":"find_name","count":3,"patterns":["README"],"results":["README.md","UI/README.md","docs/README.md"],"root":"/repo"}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut reply = AskReply::non_llm("Found README.md.".to_string())
        .with_messages(vec![
            "**Execution**\n1. Ran tool `fs_basic`.".to_string(),
            "Found README.md.".to_string(),
        ])
        .with_task_journal(journal);
    let mut route = route_result(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;

    assert!(try_recover_structured_search_answer_verifier_gap(
        Some(&route),
        "Find files named README under the current repo.",
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply
        .text
        .contains("message_key=clawd.msg.structured_search.candidates"));
    assert!(reply.text.contains("count=3"));
    assert!(reply.text.contains("README.md"));
    assert!(reply.text.contains("UI/README.md"));
    assert!(reply.text.contains("docs/README.md"));
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn structured_search_recovery_does_not_override_directory_purpose_summary() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string()],
        answer_incomplete_reason: "answer used recursive candidates instead of direct entries"
            .to_string(),
        should_retry: true,
        retry_instruction: "answer from the direct list_dir evidence".to_string(),
        confidence: 0.94,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"action":"find_ext","count":50,"ext":"toml","results":["Cargo.toml","configs/config.toml"],"root":"/repo"}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut reply =
        AskReply::non_llm("Found 50 candidates.".to_string()).with_task_journal(journal);
    let mut route = route_result(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;

    assert!(!try_recover_structured_search_answer_verifier_gap(
        Some(&route),
        "List top-level toml files and explain them briefly.",
        &mut reply
    ));
    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, "Found 50 candidates.");
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_some());
}

#[test]
fn structured_count_verifier_exhaustion_recovers_with_count_inventory() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["count".to_string()],
        answer_incomplete_reason: "answer asks to rerun instead of delivering observed count"
            .to_string(),
        should_retry: true,
        retry_instruction: "use the observed counts.total field".to_string(),
        confidence: 0.94,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"action":"count_inventory","counts":{"dirs":6,"files":58,"hidden":0,"total":64},"path":"/repo/scripts","recursive":false}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    let mut reply = AskReply::non_llm("需要重新触发计数任务。".to_string())
        .with_messages(vec![
            "**执行过程**\n1. 调用工具 `fs_basic`。".to_string(),
            "需要重新触发计数任务。".to_string(),
        ])
        .with_task_journal(journal);

    assert!(try_recover_structured_count_answer_verifier_gap(
        Some(&route),
        "先数一下 scripts 目录直接有多少个子项",
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("64"));
    assert!(reply.text.contains("58"));
    assert!(reply.text.contains("6"));
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn rss_news_verifier_exhaustion_recovers_with_structured_sources() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.resolved_intent = "capability_ref=rss.latest_news category=general".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-rss", "ask", "prompt");
    journal.record_route_result(&route);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["source".to_string()],
        answer_incomplete_reason: "candidate answer source did not match observed field"
            .to_string(),
        should_retry: true,
        retry_instruction: "use observed source_host fields".to_string(),
        confidence: 0.88,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "rss_fetch",
            sample_rss_news_output(),
        ));
    let mut reply =
        AskReply::non_llm("BBC; New York Times; incorrect synthesized source labels".to_string())
            .with_task_journal(journal);

    assert!(try_recover_rss_news_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    assert!(reply.text.contains(
        "title=New NHS drug offers ovarian cancer patients more time and better quality of life | source_host=feeds.bbci.co.uk"
    ));
    assert_eq!(
        reply.text.matches("source_host=feeds.bbci.co.uk").count(),
        2
    );
    assert!(reply.text.contains("source_host=rss.nytimes.com"));
    assert!(!reply.text.contains("纽约时报"));
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn rss_news_passed_verifier_preserves_observed_source_hosts() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.resolved_intent = "capability_ref=rss.latest_news category=general".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-rss", "ask", "prompt");
    journal.record_route_result(&route);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: true,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: String::new(),
        should_retry: false,
        retry_instruction: String::new(),
        confidence: 0.85,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "rss_fetch",
            sample_rss_news_output(),
        ));
    let mut reply = AskReply::non_llm(
        "BBC; New York Times; synthesized source labels without source_host tokens".to_string(),
    )
    .with_task_journal(journal);

    assert!(try_preserve_rss_source_hosts_from_structured_evidence(
        Some(&route),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    assert!(reply.text.contains("source_host=feeds.bbci.co.uk"));
    assert!(reply.text.contains("source_host=rss.nytimes.com"));
    assert!(reply
        .text
        .contains("title=Louisiana ICE Facility Mistreated Immigrants, Federal Investigators Say | source_host=rss.nytimes.com"));
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn content_excerpt_summary_verifier_exhaustion_recovers_with_synthesis_output() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_route_result(&route);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: "final answer dropped synthesized summary".to_string(),
        should_retry: true,
        retry_instruction: "use the full synthesized summary".to_string(),
        confidence: 0.94,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_0",
            "fs_basic",
            r#"{"action":"read_range","path":"README.md","excerpt":"1|# RustClaw\n2|Observed excerpt for summary."}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                "Summary from observed excerpt covering all required facts.".to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    let mut reply = AskReply::non_llm("partial answer".to_string())
        .with_messages(vec![
            "**Execution**\n1. Read file excerpt.".to_string(),
            "partial answer".to_string(),
        ])
        .with_task_journal(journal);

    assert!(try_recover_content_excerpt_summary_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert_eq!(
        reply.text,
        "Summary from observed excerpt covering all required facts."
    );
    assert_eq!(
        reply.messages,
        vec!["Summary from observed excerpt covering all required facts.".to_string()]
    );
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn workspace_project_summary_verifier_exhaustion_recovers_with_synthesis_output() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-workspace", "ask", "prompt");
    journal.record_route_result(&route);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: "retry exhausted after an exploratory miss".to_string(),
        should_retry: true,
        retry_instruction: "answer from the already observed README excerpt".to_string(),
        confidence: 0.95,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"action":"read_range","path":"README.md","excerpt":"15|- multi-channel entry points: Telegram, WeChat, Feishu, Lark, WhatsApp Cloud, WhatsApp Web, browser UI, and optional `webd`"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "synthesize_answer",
            "RustClaw supports multi-channel entry via Telegram, WeChat, Feishu, Lark, WhatsApp Cloud, WhatsApp Web, browser UI, and optional `webd`. Concrete channel setup depends on the chosen channel's documented setup path.",
        ));
    let mut reply =
        AskReply::non_llm("failed exploratory answer".to_string()).with_task_journal(journal);

    assert!(try_recover_content_excerpt_summary_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("multi-channel entry"));
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn workspace_project_summary_verifier_exhaustion_does_not_recover_unsupported_claims() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-workspace", "ask", "prompt");
    journal.record_route_result(&route);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["unsupported_claims".to_string()],
        answer_incomplete_reason: "answer adds setup steps not supported by observed excerpts"
            .to_string(),
        should_retry: true,
        retry_instruction: "rewrite from observed channel surfaces only".to_string(),
        confidence: 0.95,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"action":"read_range","path":"README.md","excerpt":"15|- multi-channel entry points: Telegram, WeChat, Feishu, Lark, WhatsApp Cloud, WhatsApp Web, browser UI, and optional `webd`"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "synthesize_answer",
            "Unsupported setup steps should not be recovered.",
        ));
    let mut reply =
        AskReply::non_llm("failed exploratory answer".to_string()).with_task_journal(journal);

    assert!(!try_recover_content_excerpt_summary_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));
}

#[test]
fn generic_path_content_verifier_exhaustion_does_not_recover_raw_read_range_excerpt() {
    let route = route_result(OutputResponseShape::Free);
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-read-range", "ask", "tail log");
    journal.record_route_result(&route);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string(), "path".to_string()],
        answer_incomplete_reason: "answer did not include read_range fields".to_string(),
        should_retry: true,
        retry_instruction: "include path and content_excerpt".to_string(),
        confidence: 0.5,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "read_range",
                    "mode": "tail",
                    "requested_n": 2,
                    "path": "logs/clawd.log",
                    "resolved_path": "/repo/logs/clawd.log",
                    "excerpt": "41|first log line\n42|second log line"
                })
                .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    let mut reply = AskReply::non_llm("partial answer".to_string())
        .with_messages(vec![
            "**Execution**\n1. Read the file range.".to_string(),
            "partial answer".to_string(),
        ])
        .with_task_journal(journal);

    assert!(
        !try_recover_generic_path_content_read_range_answer_verifier_gap(Some(&route), &mut reply)
    );

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, "partial answer");
    assert_eq!(
        reply.messages,
        vec![
            "**Execution**\n1. Read the file range.".to_string(),
            "partial answer".to_string()
        ]
    );
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_some());
}

#[test]
fn structured_scalar_output_format_gap_recovers_quoted_observed_value() {
    let mut route = route_result(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-scalar-recovery", "ask", "field value");
    journal.record_route_result(&route);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate is an object".to_string(),
        should_retry: true,
        retry_instruction: "Return only the scalar value \"rustclaw\".".to_string(),
        confidence: 0.95,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "/repo/package.json",
                    "excerpt": "1|{\n2|  \"name\": \"rustclaw\",\n3|  \"private\": true,"
                },
                "text": "{\"action\":\"read_range\",\"path\":\"/repo/package.json\",\"excerpt\":\"1|{\\n2|  \\\"name\\\": \\\"rustclaw\\\",\\n3|  \\\"private\\\": true,\"}"
            })
            .to_string(),
        ));
    let mut reply =
        AskReply::non_llm("{\n\"name\": \"rustclaw\",\n\"private\": true\n}".to_string())
            .with_task_journal(journal);

    assert!(
        try_recover_structured_scalar_output_format_answer_verifier_gap(Some(&route), &mut reply)
    );
    assert_eq!(reply.text, "rustclaw");
    assert_eq!(reply.messages, vec!["rustclaw"]);
    assert!(!reply.should_fail_task);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn machine_kv_summary_output_format_gap_recovers_from_observed_read_range_token() {
    let route = route_result(OutputResponseShape::Scalar);
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-kv-recovery",
        "ask",
        "Use read_range only. Answer exactly as machine summary: required=yes script=check_runtime_semantic_rewrite_boundary.py.",
    );
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate returned prose instead of requested machine shape"
            .to_string(),
        should_retry: true,
        retry_instruction: "return required=yes script=check_runtime_semantic_rewrite_boundary.py"
            .to_string(),
        confidence: 0.96,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "system_basic",
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "AGENTS.md",
                    "excerpt": "248|must run `python3 scripts/check_runtime_semantic_rewrite_boundary.py` after boundary changes"
                },
                "text": "{\"action\":\"read_range\",\"excerpt\":\"248|must run `python3 scripts/check_runtime_semantic_rewrite_boundary.py` after boundary changes\"}"
            })
            .to_string(),
        ));
    let mut reply = AskReply::non_llm("prose answer".to_string()).with_task_journal(journal);

    assert!(
        try_recover_machine_kv_summary_output_format_answer_verifier_gap(Some(&route), &mut reply)
    );
    assert_eq!(
        reply.text,
        "required=yes script=check_runtime_semantic_rewrite_boundary.py"
    );
    assert!(!reply.should_fail_task);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[test]
fn machine_kv_summary_output_format_gap_requires_observed_non_flag_value() {
    let route = route_result(OutputResponseShape::Scalar);
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-kv-recovery-missing",
        "ask",
        "Answer exactly as machine summary: required=yes script=missing_guard.py.",
    );
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate returned prose instead of requested machine shape"
            .to_string(),
        should_retry: true,
        retry_instruction: "return required=yes script=missing_guard.py".to_string(),
        confidence: 0.96,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "system_basic",
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "AGENTS.md",
                    "excerpt": "248|must run another_guard.py"
                }
            })
            .to_string(),
        ));
    let mut reply = AskReply::non_llm("prose answer".to_string()).with_task_journal(journal);

    assert!(
        !try_recover_machine_kv_summary_output_format_answer_verifier_gap(Some(&route), &mut reply)
    );
    assert_eq!(reply.text, "prose answer");
}

#[test]
fn document_heading_verifier_gap_recovers_heading_scalar_from_read_range_evidence() {
    let mut route = route_result(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::DocumentHeading;
    route.output_contract.locator_hint = "docs/service_notes.md".to_string();

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "read the document heading");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.record_final_answer("# Service Notes\n\nFull body");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "answer included more than the scalar value".to_string(),
        should_retry: true,
        retry_instruction: "return only the scalar value".to_string(),
        confidence: 0.95,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "docs/service_notes.md",
                    "resolved_path": "/repo/docs/service_notes.md",
                    "excerpt": "1|# Service Notes\n2|\n3|Body"
                },
                "text": "{\"action\":\"read_range\",\"excerpt\":\"1|# Service Notes\\n2|\\n3|Body\"}"
            })
            .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut reply = AskReply::non_llm("# Service Notes\n\nFull body".to_string())
        .with_messages(vec!["# Service Notes\n\nFull body".to_string()])
        .with_task_journal(journal);

    assert!(try_recover_document_heading_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));

    assert_eq!(reply.text, "Service Notes");
    assert_eq!(reply.messages, vec!["Service Notes".to_string()]);
    assert!(!reply.should_fail_task);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert_eq!(
        journal
            .rollout_attribution
            .last()
            .and_then(|item| item.reason_code.as_deref()),
        Some("document_heading_recovered_from_observed_markdown_heading")
    );
}

#[test]
fn alias_prebound_scalar_output_format_gap_recovers_markdown_heading() {
    let mut route = route_result(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "docs/release_checklist.md".to_string();
    route.route_reason =
        "session_alias_locator_prebound_from_current_request; machine_alias_binding".to_string();

    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-1",
        "ask",
        "alias-bound scalar document heading",
    );
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.record_final_answer("# Release Checklist\n\n1. Verify configuration loads correctly.");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "answer included the whole content instead of one scalar"
            .to_string(),
        should_retry: true,
        retry_instruction: "return only the scalar value".to_string(),
        confidence: 0.97,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "docs/release_checklist.md",
                    "resolved_path": "/repo/docs/release_checklist.md",
                    "excerpt": "1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly."
                },
                "text": "{\"action\":\"read_range\",\"excerpt\":\"1|# Release Checklist\\n2|\\n3|1. Verify configuration loads correctly.\"}"
            })
            .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut reply = AskReply::non_llm(
        "# Release Checklist\n\n1. Verify configuration loads correctly.".to_string(),
    )
    .with_messages(vec![
        "# Release Checklist\n\n1. Verify configuration loads correctly.".to_string(),
    ])
    .with_task_journal(journal);

    assert!(try_recover_document_heading_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));

    assert_eq!(reply.text, "Release Checklist");
    assert_eq!(reply.messages, vec!["Release Checklist".to_string()]);
    assert!(!reply.should_fail_task);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert_eq!(
        journal
            .rollout_attribution
            .last()
            .and_then(|item| item.reason_code.as_deref()),
        Some("document_heading_recovered_from_observed_markdown_heading")
    );
}

#[test]
fn language_only_output_format_gap_keeps_best_model_answer_success() {
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.record_final_answer("best model answer");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "shape".to_string(),
        should_retry: true,
        retry_instruction: "retry with requested shape".to_string(),
        confidence: 0.93,
    });
    let mut reply = AskReply::non_llm("best model answer".to_string())
        .with_messages(vec!["best model answer".to_string()])
        .with_task_journal(journal);

    assert!(try_accept_language_only_output_format_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, "best model answer");
    assert!(reply.error_text.is_none());
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert_eq!(journal.final_answer.as_deref(), Some("best model answer"));
}

#[test]
fn language_only_output_format_gap_prefers_latest_terminal_answer_over_stale_text() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let table_only = "| name | score |\n| --- | --- |\n| beta | 12 |";
    let full_answer = "**1. log evidence**\n- WARN=2, ERROR=1\n\n**2. doc summary**\n- service notes\n\n**3. table**\n\n| name | score |\n| --- | --- |\n| beta | 12 |";
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.record_final_answer(table_only);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "transform",
            table_only,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "respond",
            full_answer,
        ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "language-only output shape".to_string(),
        should_retry: true,
        retry_instruction: "retry with requested language".to_string(),
        confidence: 0.88,
    });
    let mut reply = AskReply::non_llm(table_only.to_string())
        .with_messages(vec![table_only.to_string()])
        .with_task_journal(journal);

    assert!(try_accept_language_only_output_format_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));

    assert_eq!(reply.text, full_answer);
    assert_eq!(reply.messages, vec![full_answer.to_string()]);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(journal.final_answer.as_deref(), Some(full_answer));
}

#[test]
fn latest_terminal_recovery_rejects_structured_visible_rewrite_gap() {
    let mut route = route_result(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3".to_string();
    route.output_contract.self_extension.list_selector.limit = Some(3);
    let first_three = "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md";
    let all_four =
        format!("{first_three}\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt");
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "latest retry ignored selector limit".to_string(),
        should_retry: true,
        retry_instruction: "use the first three observed paths".to_string(),
        confidence: 0.94,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"action":"inventory_dir","counts":{"files":4,"total":4},"entries":[{"kind":"file","name":"x_abcd_log.txt","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt"},{"kind":"file","name":"zz_abcd_backup.log","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"},{"kind":"file","name":"abcd_report.md","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md"},{"kind":"file","name":"my_abcd.txt","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt"}]}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_2".to_string(),
            skill: "respond".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(first_three.to_string()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_3".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(all_four.clone()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_4".to_string(),
            skill: "respond".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(all_four),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    let mut reply = AskReply::non_llm("failed".to_string())
        .with_messages(vec!["failed".to_string()])
        .with_task_journal(journal);

    assert!(!try_recover_latest_synthesis_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));
    assert_eq!(reply.text, "failed");
    assert!(reply.task_journal.as_ref().is_some_and(|journal| journal
        .answer_verifier_summary
        .as_ref()
        .is_some_and(|summary| summary.high_confidence_retry_gap())));
}

#[test]
fn latest_terminal_recovery_uses_latest_terminal_for_non_structured_gap() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
            .to_string();
    let first_answer = "# Service Notes\n\nRustClaw test fixture service notes.";
    let stale_answer = "service notes";
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: "latest retry omitted observed excerpt".to_string(),
        should_retry: true,
        retry_instruction: "use the observed excerpt".to_string(),
        confidence: 0.94,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r##"{"extra":{"path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md","content_excerpt":"# Service Notes\n\nRustClaw test fixture service notes."}}"##
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_2".to_string(),
            skill: "respond".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(first_answer.to_string()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_3".to_string(),
            skill: "respond".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(stale_answer.to_string()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    let mut reply = AskReply::non_llm("failed".to_string())
        .with_messages(vec!["failed".to_string()])
        .with_task_journal(journal);

    assert!(try_recover_latest_synthesis_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));
    assert_eq!(reply.text, stale_answer);
    assert!(!reply.should_fail_task);
    assert!(reply.error_text.is_none());
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_stop_signal.as_deref()),
        Some(crate::task_journal::ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL)
    );
}

#[test]
fn http_health_browser_open_extract_capability_gap_recovers_with_structured_status_line() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Url;
    route.output_contract.locator_hint = "http://127.0.0.1:8787/v1/health".to_string();
    route.resolved_intent =
        "capability_ref=browser.open_extract url=http://127.0.0.1:8787/v1/health".to_string();

    let body = json!({
        "ok": true,
        "data": {
            "version": "0.1.7",
            "uptime_seconds": 1227,
            "memory_rss_bytes": 764149760,
            "running_length": 1,
            "channel_gateway_healthy": false,
            "telegram_bot_healthy": true,
            "gateway_instance_statuses": [
                {"kind": "telegram", "name": "primary", "healthy": false, "status": "stale"},
                {"kind": "feishu", "name": "primary", "healthy": true, "status": "running"}
            ]
        },
        "error": null
    });
    let output = json!({
        "extra": {
            "action": "get",
            "url": "http://127.0.0.1:8787/v1/health",
            "status_code": 200,
            "success_status": true,
            "body_preview": body.to_string()
        },
        "text": format!("status=200\n{}", body)
    })
    .to_string();

    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "http_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(output),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: "unsupported generated health summary".to_string(),
        should_retry: true,
        retry_instruction: "use only observed health fields".to_string(),
        confidence: 0.95,
    });
    let mut reply =
        AskReply::non_llm("bad generated health summary".to_string()).with_task_journal(journal);

    assert!(try_recover_http_health_answer_verifier_gap(
        Some(&route),
        &mut reply,
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("http_reachability=reachable"));
    assert!(reply.text.contains("status_code=200"));
    assert!(reply.text.contains("ok=true"));
    assert!(reply.text.contains("version=0.1.7"));
    assert!(reply.text.contains("channel_gateway_healthy=false"));
    assert!(reply.text.contains("telegram:primary:stale:false"));
    assert!(!reply.text.contains("memory"));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[test]
fn http_health_command_summary_gap_recovers_with_structured_status_line() {
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Url;
    route.output_contract.locator_hint = "http://127.0.0.1:8787/v1/health".to_string();

    let body = json!({
        "ok": true,
        "data": {
            "version": "0.1.8",
            "uptime_seconds": 1050,
            "running_length": 1,
            "channel_gateway_healthy": false,
            "telegram_bot_healthy": false,
            "gateway_instance_statuses": [
                {"kind": "telegram", "name": "primary", "healthy": false, "status": "stale"},
                {"kind": "feishu", "name": "primary", "healthy": false, "status": "stopped"}
            ]
        },
        "error": null
    });
    let output = json!({
        "extra": {
            "action": "get",
            "url": "http://127.0.0.1:8787/v1/health",
            "status_code": 200,
            "success_status": true,
            "body_json": body
        },
        "text": "status=200"
    })
    .to_string();

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-http-health", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "http_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(output),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec![
            "output_format".to_string(),
            "unsupported_claims".to_string(),
        ],
        answer_incomplete_reason: "generated summary added unsupported fields".to_string(),
        should_retry: true,
        retry_instruction: "use only observed health fields".to_string(),
        confidence: 0.95,
    });
    let mut reply =
        AskReply::non_llm("bad generated health summary".to_string()).with_task_journal(journal);

    assert!(try_recover_http_health_answer_verifier_gap(
        Some(&route),
        &mut reply,
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("http_reachability=reachable"));
    assert!(reply.text.contains("status_code=200"));
    assert!(reply.text.contains("ok=true"));
    assert!(reply.text.contains("version=0.1.8"));
    assert!(reply.text.contains("uptime_seconds=1050"));
    assert!(reply.text.contains("running_length=1"));
    assert!(reply.text.contains("channel_gateway_healthy=false"));
    assert!(reply.text.contains("telegram_bot_healthy=false"));
    assert!(reply.text.contains("telegram:primary:stale:false"));
    assert!(reply.text.contains("feishu:primary:stopped:false"));
    assert!(!reply.text.contains("memory"));
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_none());
}

#[test]
fn http_health_command_summary_gap_prefers_latest_language_synthesis() {
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Url;
    route.output_contract.locator_hint = "http://127.0.0.1:8787/v1/health".to_string();

    let body = json!({
        "ok": true,
        "data": {
            "version": "0.1.8",
            "uptime_seconds": 1050,
            "running_length": 1,
            "channel_gateway_healthy": false,
            "telegram_bot_healthy": false
        },
        "error": null
    });
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-http-health-language", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "http_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "extra": {
                        "action": "get",
                        "url": "http://127.0.0.1:8787/v1/health",
                        "status_code": 200,
                        "success_status": true,
                        "body_json": body
                    },
                    "text": "status=200"
                })
                .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_2".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                "health 接口可连通，版本 0.1.8 正在运行，但渠道网关和 Telegram 机器人当前不健康。"
                    .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["unsupported_claims".to_string()],
        answer_incomplete_reason: "verifier asked for retry".to_string(),
        should_retry: true,
        retry_instruction: "use only observed health fields".to_string(),
        confidence: 0.95,
    });
    let mut reply =
        AskReply::non_llm("bad generated health summary".to_string()).with_task_journal(journal);

    assert!(try_recover_http_health_answer_verifier_gap(
        Some(&route),
        &mut reply,
    ));

    assert_eq!(
        reply.text,
        "health 接口可连通，版本 0.1.8 正在运行，但渠道网关和 Telegram 机器人当前不健康。"
    );
    assert!(!reply.text.contains("http_reachability="));
    assert!(!reply.should_fail_task);
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_none());
}

fn test_policy() -> AgentLoopGuardPolicy {
    AgentLoopGuardPolicy {
        max_steps: 8,
        max_rounds: 4,
        max_tool_calls: 12,
        recoverable_failure_extra_rounds: 1,
        repeat_action_limit: 3,
        no_progress_limit: 1,
        multi_round_enabled: true,
        answer_verifier_retry_limit: 2,
        answer_verifier_enforce_required_scope: AnswerVerifierRequiredEvidenceScope::Off,
        registry_idempotency_guard_scope: RegistryIdempotencyGuardScope::Off,
        fast_read: Default::default(),
        grounded_summary: Default::default(),
        multi_step_workspace: Default::default(),
        ops_closed_loop: Default::default(),
    }
}

#[test]
fn answer_verifier_retry_budget_does_not_depend_on_global_multi_round_switch() {
    let mut policy = test_policy();
    policy.multi_round_enabled = false;
    policy.answer_verifier_retry_limit = 2;

    assert!(answer_verifier_retry_budget_available(&policy, 0));
    assert!(answer_verifier_retry_budget_available(&policy, 1));
    assert!(!answer_verifier_retry_budget_available(&policy, 2));
}

#[path = "loop_control_tests/answer_verifier_exhaustion.rs"]
mod answer_verifier_exhaustion;
#[path = "loop_control_tests/clarify_control.rs"]
mod clarify_control;

#[path = "loop_control_tests/local_health_recovery.rs"]
mod local_health_recovery;
#[path = "loop_control_tests/machine_status_visible.rs"]
mod machine_status_visible;

#[path = "loop_control_tests/observed_finalize.rs"]
mod observed_finalize;
#[path = "loop_control_tests/post_write_validation_reserve.rs"]
mod post_write_validation_reserve;
#[path = "loop_control_tests/recent_artifacts_recovery.rs"]
mod recent_artifacts_recovery;
#[path = "loop_control_tests/soft_budget_checkpoint.rs"]
mod soft_budget_checkpoint;
#[path = "loop_control_tests/structured_listing_recovery.rs"]
mod structured_listing_recovery;
#[path = "loop_control_tests/terminal_answer_stop.rs"]
mod terminal_answer_stop;
#[path = "loop_control_tests/verifier_retry_suppression.rs"]
mod verifier_retry_suppression;

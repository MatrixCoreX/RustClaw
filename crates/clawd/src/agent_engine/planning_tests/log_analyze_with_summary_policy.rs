use super::*;

#[test]
fn content_excerpt_with_summary_single_log_file_without_slice_uses_log_analyze_plan() {
    let root = TempDirGuard::new("content_excerpt_with_summary_log_file_auto_locator");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    let log = logs_dir.join("clawd.run.log");
    fs::write(&log, "INFO ok\nWARN slow\nERROR old failure\n").expect("write log");
    let log_path = log.display().to_string();
    let state = test_state_with_enabled_skills(&["log_analyze", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    route.output_contract.delivery_required = false;

    let plan = generic_path_content_log_analyze_deterministic_plan_result(
        "inspect the current target",
        &state,
        Some(&route),
        &LoopState::new(1),
        Some(&log_path),
    )
    .expect("single log health synthesis without slice should use log_analyze");

    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.steps[0].action_type, "call_skill");
    assert_eq!(plan.steps[0].skill, "log_analyze");
    assert_eq!(
        plan.steps[0].args.get("path").and_then(Value::as_str),
        Some(log_path.as_str())
    );
}

#[test]
fn content_excerpt_with_summary_single_log_file_with_slice_keeps_bounded_read() {
    let root = TempDirGuard::new("content_excerpt_with_summary_log_file_slice");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    let log = logs_dir.join("clawd.run.log");
    fs::write(&log, "INFO ok\nWARN slow\nERROR old failure\n").expect("write log");
    let log_path = log.display().to_string();
    let state = test_state_with_enabled_skills(&["log_analyze", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    route.output_contract.delivery_required = false;
    route.resolved_intent = "slice_mode=tail slice_n=20".to_string();

    assert!(generic_path_content_log_analyze_deterministic_plan_result(
        "read a bounded log slice and synthesize from it",
        &state,
        Some(&route),
        &LoopState::new(1),
        Some(&log_path),
    )
    .is_none());
}

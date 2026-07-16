use super::*;

#[test]
fn content_excerpt_with_summary_single_log_file_allows_log_analyze_evidence() {
    let root = TempDirGuard::new("content_excerpt_with_summary_log_file_auto_locator");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    let log = logs_dir.join("clawd.run.log");
    fs::write(&log, "INFO ok\nWARN slow\nERROR old failure\n").expect("write log");
    let log_path = log.display().to_string();
    let mut route = route_result(true, OutputResponseShape::Strict);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    route.output_contract.delivery_required = false;

    let contract = route.effective_output_contract();
    let policy = crate::evidence_policy::action_policy_for_output_contract(
        Some(&contract),
        "log_analyze",
        &json!({
            "path": log_path,
            "max_matches": 50,
        }),
    )
    .expect("content excerpt with summary should allow log_analyze evidence");

    assert!(policy.is_allowed(), "{policy:?}");
}

#[test]
fn content_excerpt_with_summary_single_log_file_with_slice_allows_bounded_read_evidence() {
    let root = TempDirGuard::new("content_excerpt_with_summary_log_file_slice");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    let log = logs_dir.join("clawd.run.log");
    fs::write(&log, "INFO ok\nWARN slow\nERROR old failure\n").expect("write log");
    let log_path = log.display().to_string();
    let mut route = route_result(true, OutputResponseShape::Strict);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    route.output_contract.delivery_required = false;
    route.resolved_intent = "slice_mode=tail slice_n=20".to_string();

    let contract = route.effective_output_contract();
    let read_policy = crate::evidence_policy::action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &json!({
            "action": "read_text_range",
            "path": log_path,
            "mode": "tail",
            "n": 20,
        }),
    )
    .expect("content excerpt with summary should allow bounded read evidence");

    assert!(read_policy.is_allowed(), "{read_policy:?}");
    assert!(read_policy.action_matches_preferred(), "{read_policy:?}");
}

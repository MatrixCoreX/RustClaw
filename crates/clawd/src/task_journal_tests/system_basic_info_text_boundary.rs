use super::*;

#[test]
fn system_basic_info_evidence_ignores_json_hidden_in_visible_text() {
    let mut journal =
        TaskJournal::for_task("task-system-basic-info-text-boundary", "ask", "show status");
    let mut route = route_for_semantic(crate::OutputSemanticKind::ServiceStatus);
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "text": "{\"arch\":\"x86_64\",\"cwd\":\"/repo\",\"hostname\":\"host\",\"os\":\"linux\",\"workspace_root\":\"/repo\"}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(
        !coverage
            .observed_extractors
            .contains("system_basic.info.structured_json_v1"),
        "coverage: {coverage:?}"
    );
}

#[test]
fn system_basic_info_evidence_accepts_extra_machine_payload() {
    let mut journal = TaskJournal::for_task("task-system-basic-info-extra", "ask", "show status");
    let mut route = route_for_semantic(crate::OutputSemanticKind::ServiceStatus);
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "extra": {
                    "arch": "x86_64",
                    "cwd": "/repo",
                    "hostname": "host",
                    "os": "linux",
                    "workspace_root": "/repo"
                },
                "text": "display only"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(
        coverage
            .observed_extractors
            .contains("system_basic.info.structured_json_v1"),
        "coverage: {coverage:?}"
    );
}

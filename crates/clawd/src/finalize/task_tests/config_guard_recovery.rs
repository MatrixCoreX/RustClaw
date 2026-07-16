use super::{deterministic_config_guard_candidates_recovery, route_result};

#[test]
fn config_guard_recovery_uses_validation_contract() {
    let mut route = route_result();
    route.route_reason = "capability_ref=config.guard_config".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigValidation;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-config-guard-capability-ref", "ask", "");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string()],
        answer_incomplete_reason: "candidate evidence missing".to_string(),
        should_retry: true,
        retry_instruction: "use structured guard candidates".to_string(),
        confidence: 0.9,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace::ok(
        "step_1",
        "config_basic",
        r#"{"extra":{"action":"guard_config","candidates":["tools.allow_sudo=true"],"count":1,"path":"configs/config.toml"},"text":"{\"candidates\":[\"must_not_parse_text\"]}"}"#,
    ));

    let recovered =
        deterministic_config_guard_candidates_recovery(&route, &journal).expect("recovery");
    let payload: serde_json::Value = serde_json::from_str(&recovered).expect("json payload");

    assert_eq!(
        payload
            .pointer("/candidates/0")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow_sudo=true")
    );
    assert_eq!(payload.pointer("/candidates/1"), None);
}

#[test]
fn config_guard_recovery_uses_risk_assessment_contract() {
    let mut route = route_result();
    route.route_reason = "capability_ref=config.risk".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-config-risk-capability-shape", "ask", "");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string()],
        answer_incomplete_reason: "candidate evidence missing".to_string(),
        should_retry: true,
        retry_instruction: "use structured risk candidates".to_string(),
        confidence: 0.9,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace::ok(
        "step_1",
        "config_basic",
        r#"{"extra":{"action":"guard_config","candidates":["tools.allow_sudo=true"],"count":1,"path":"configs/config.toml"},"text":"{}"}"#,
    ));

    let recovered =
        deterministic_config_guard_candidates_recovery(&route, &journal).expect("recovery");
    let payload: serde_json::Value = serde_json::from_str(&recovered).expect("json payload");

    assert_eq!(
        payload.get("count").and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        payload
            .pointer("/candidates/0")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow_sudo=true")
    );
}

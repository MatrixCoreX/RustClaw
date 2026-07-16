use super::*;

#[test]
fn system_basic_info_evidence_ignores_json_hidden_in_visible_text() {
    let mut journal =
        TaskJournal::for_task("task-system-basic-info-text-boundary", "ask", "show status");
    let mut route = route_for_semantic(crate::OutputSemanticKind::ServiceStatus);
    route.requires_content_evidence = true;
    journal.record_output_contract(&route.clone());
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

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
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
    route.requires_content_evidence = true;
    journal.record_output_contract(&route.clone());
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

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(
        coverage
            .observed_extractors
            .contains("system_basic.info.structured_json_v1"),
        "coverage: {coverage:?}"
    );
}

#[test]
fn embedded_http_body_evidence_ignores_status_json_hidden_in_visible_text() {
    let body = serde_json::json!({
        "ok": true,
        "data": {
            "worker_state": "running",
            "uptime_seconds": 95
        }
    });
    let observed = observed_evidence_from_output(Some(
        &serde_json::json!({
            "text": format!("status=200\n{}", body)
        })
        .to_string(),
    ))
    .expect("wrapper output should still produce generic evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");

    assert!(
        !items.iter().any(|item| {
            item.get("source").and_then(Value::as_str) == Some("json_output.text.body_json")
                || item
                    .get("field")
                    .and_then(Value::as_str)
                    .is_some_and(|field| field.starts_with("body."))
        }),
        "visible text must not act as embedded body protocol: {items:?}"
    );
}

#[test]
fn embedded_http_body_evidence_accepts_extra_body_preview() {
    let body = serde_json::json!({
        "ok": true,
        "data": {
            "worker_state": "running",
            "uptime_seconds": 95
        }
    });
    let observed = observed_evidence_from_output(Some(
        &serde_json::json!({
            "extra": {
                "body_preview": body.to_string()
            },
            "text": "display only"
        })
        .to_string(),
    ))
    .expect("wrapper output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");

    assert!(items.iter().any(|item| {
        item.get("source").and_then(Value::as_str) == Some("json_output.extra.body_json")
            && item.get("field").and_then(Value::as_str) == Some("body.data.worker_state")
            && item.get("excerpt").and_then(Value::as_str) == Some("running")
    }));
}

use super::*;

fn command_result(stdout: Option<Value>) -> Value {
    let metadata = (0..48)
        .map(|index| (format!("field_{index}"), json!(index)))
        .collect::<serde_json::Map<_, _>>();
    let mut result = json!({
        "artifact_refs": [{"id": "artifact-fixture", "metadata": metadata}],
        "artifacts": [{"id": "artifact-fixture", "metadata": metadata}],
        "exit_code": 0,
        "output": "observed-output",
        "output_truncated": false,
        "stderr": "",
        "truncated": false
    });
    if let Some(stdout) = stdout {
        result["stdout"] = stdout;
    }
    result
}

fn command_journal(output: Value, status: crate::executor::StepExecutionStatus) -> TaskJournal {
    let mut journal = TaskJournal::for_task("stdio-evidence", "ask", "fixture");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".into(),
        skill: "run_cmd".into(),
        status,
        output: Some(output.to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal
}

fn stdio_contract() -> crate::IntentOutputContract {
    crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        selection: crate::pipeline_types::OutputSelectionContract {
            structured_field_selector: Some("exit_code,stdout".into()),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn artifact_metadata_cannot_starve_observed_stdout() {
    let journal = command_journal(
        command_result(Some(json!("observed-output"))),
        crate::executor::StepExecutionStatus::Ok,
    );
    let coverage = evidence_coverage_for_output_contract(&stdio_contract(), &journal);
    assert!(coverage.is_complete(), "{coverage:?}");
    assert!(coverage.observed_fields.contains("stdout"));
    let trace = journal.to_trace_json();
    let observed = &trace["step_results"][0]["observed_evidence"];
    assert!(observed["items"].as_array().unwrap().len() <= 40);
    assert_eq!(observed["truncated"], true);
}

#[test]
fn artifact_metadata_cannot_starve_nested_stdout_or_empty_stdout() {
    for stdout in ["", "observed-output"] {
        for key in ["extra", "output"] {
            let journal = command_journal(
                json!({(key): command_result(Some(json!(stdout)))}),
                crate::executor::StepExecutionStatus::Ok,
            );
            let coverage = evidence_coverage_for_output_contract(&stdio_contract(), &journal);
            assert!(
                coverage.is_complete(),
                "key={key} stdout={stdout:?} {coverage:?}"
            );
        }
    }
}

#[test]
fn command_output_or_model_answer_cannot_invent_missing_stdout_evidence() {
    let mut journal = command_journal(
        command_result(None),
        crate::executor::StepExecutionStatus::Ok,
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".into(),
        skill: "respond".into(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"exit_code": 0, "stdout": "invented"}).to_string()),
        error: None,
        started_at: 2,
        finished_at: 3,
    });
    let coverage = evidence_coverage_for_output_contract(&stdio_contract(), &journal);
    assert_eq!(coverage.missing_evidence, vec!["stdout"]);
}

#[test]
fn preserved_stdio_evidence_keeps_redaction_and_truncation_flags() {
    let mut result = command_result(Some(json!("line ".repeat(100))));
    result["truncated"] = json!(true);
    result["output_truncated"] = json!(true);
    result["stderr"] = json!("sk-test-super-secret-token-value-1234567890");
    let observed = observed_evidence_from_output(Some(&result.to_string())).unwrap();
    let items = observed["items"].as_array().unwrap();
    for field in [
        "stdout",
        "stderr",
        "exit_code",
        "truncated",
        "output_truncated",
    ] {
        assert!(
            items.iter().any(|item| item["field"] == field),
            "missing {field}"
        );
    }
    let stderr = items.iter().find(|item| item["field"] == "stderr").unwrap();
    assert_eq!(stderr["redacted"], true);
    assert!(stderr.get("excerpt").is_none());
    assert!(!observed
        .to_string()
        .contains("sk-test-super-secret-token-value"));
}

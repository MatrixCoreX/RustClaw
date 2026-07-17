use super::{task_report_json, task_report_text_lines};

#[test]
fn task_report_projects_machine_cost_and_budget_without_unlisted_fields() {
    let task = crate::task::TaskStatusView {
        task_id: "task-cost".to_string(),
        status: "succeeded".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "completed",
            "result_json": {
                "task_journal": {
                    "summary": {
                        "task_metrics": {
                            "llm_cost": {
                                "status": "unknown",
                                "logical_call_count": 2,
                                "covered_logical_call_count": 2,
                                "unknown_record_count": 1,
                                "estimated_cost_usd_nanos": 1250000,
                                "unknown_reasons": ["usage_unavailable"],
                                "secret_field": "must_not_project"
                            },
                            "llm_cost_budget": {
                                "status": "soft_exceeded",
                                "enforcement": "checkpoint",
                                "task_known_cost_usd_nanos": 1250000,
                                "soft_task_limit_usd_nanos": 1000000,
                                "hard_task_limit_usd_nanos": 5000000,
                                "hard_exceeded": false,
                                "signals": ["soft_task_cost_exceeded"],
                                "api_key": "must_not_project"
                            }
                        }
                    }
                }
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let report = task_report_json(&task, false);
    let governance = &report["llm"]["cost_governance"];
    let encoded = serde_json::to_string(governance).expect("cost governance JSON");

    assert_eq!(governance["cost"]["status"], "unknown");
    assert_eq!(governance["cost"]["estimated_cost_usd_nanos"], 1_250_000);
    assert_eq!(governance["budget"]["status"], "soft_exceeded");
    assert_eq!(governance["budget"]["soft_task_limit_usd_nanos"], 1_000_000);
    assert!(!encoded.contains("must_not_project"));
    assert!(!encoded.contains("secret_field"));
    assert!(!encoded.contains("api_key"));

    let lines = task_report_text_lines(&task, &report);
    assert!(lines.contains(&"llm_cost_status: unknown".to_string()));
    assert!(lines.contains(&"llm_estimated_cost_usd_nanos: 1250000".to_string()));
    assert!(lines.contains(&"llm_cost_budget_status: soft_exceeded".to_string()));
    assert!(lines.contains(&"llm_cost_budget_signal: soft_task_cost_exceeded".to_string()));
}

#[test]
fn task_report_omits_cost_projection_when_journal_has_no_cost_contract() {
    let task = crate::task::TaskStatusView {
        task_id: "task-no-cost".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({"execution_state": "running"}),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let report = task_report_json(&task, false);

    assert_eq!(report["llm"]["cost_governance"], serde_json::Value::Null);
    assert!(!task_report_text_lines(&task, &report)
        .iter()
        .any(|line| line.starts_with("llm_cost_")));
}

use super::task_report_json;

#[test]
fn task_report_uses_current_verification_and_preserves_historical_failures() {
    let task = crate::task::TaskStatusView {
        task_id: "task-coding-red-green-report".to_string(),
        status: "succeeded".to_string(),
        raw_data: serde_json::json!({
            "result_json": {
                "task_journal": {
                    "summary": {
                        "coding_workflow": {
                            "schema_version": 2,
                            "projection_revision": 9,
                            "latest_verification_step_ref": "step_green",
                            "current_phase_hint": "summarize",
                            "next_step": "summarize",
                            "changed_file_count": 1,
                            "changed_files": ["calc.py"],
                            "verification_command_count": 2,
                            "verification_commands": [
                                "python3 -m unittest test_calc.py",
                                "python3 -m unittest test_calc.py -v"
                            ],
                            "verification_status": "verified",
                            "failure_kind_count": 0,
                            "failure_kinds": [],
                            "historical_failure_kind_count": 1,
                            "historical_failure_kinds": ["test"],
                            "repair_attempt_count": 1,
                            "repair_attempt_refs": ["step:step_red"],
                            "checkpoint_ref_count": 1,
                            "checkpoint_refs": ["coding_checkpoint:verification_command:step_green"],
                            "completed_side_effect_count": 0,
                            "completed_side_effect_refs": [],
                            "remaining_risks": [],
                            "done_condition_coverage": [],
                            "validation_gate": {
                                "gate_status": "satisfied",
                                "can_report_fully_verified": true
                            }
                        }
                    },
                    "trace": {
                        "step_results": [
                            {
                                "step_id": "step_red",
                                "status": "error",
                                "skill": "run_cmd",
                                "command": "python3 -m unittest test_calc.py"
                            },
                            {
                                "step_id": "step_green",
                                "status": "ok",
                                "skill": "run_cmd",
                                "command": "python3 -m unittest test_calc.py -v"
                            }
                        ]
                    }
                }
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let report = task_report_json(&task, false);
    assert_eq!(report["coding"]["schema_version"], 2);
    assert_eq!(report["coding"]["projection_revision"], 9);
    assert_eq!(
        report["coding"]["latest_verification_step_ref"],
        "step_green"
    );
    assert_eq!(report["coding"]["state"]["verification_status"], "verified");
    assert_eq!(report["coding"]["state"]["has_failed_verification"], false);
    assert_eq!(report["coding"]["failure_count"], 0);
    assert!(report["coding"]["failures"]
        .as_array()
        .is_some_and(Vec::is_empty));
    assert_eq!(report["coding"]["historical_failure_count"], 1);
    assert_eq!(
        report["coding"]["historical_verification_failure_kinds"][0],
        "test"
    );
}

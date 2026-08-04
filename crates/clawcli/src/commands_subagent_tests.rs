use super::subagent_report_json;

#[test]
fn subagent_report_json_collects_child_results_and_events() {
    let task = crate::task::TaskStatusView {
        task_id: "task-subagents".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "result_json": {
                "child_results": [{
                    "child_run_id": "subagent:1:2:explorer",
                    "subagent_id": "explorer",
                    "status": "succeeded",
                    "result_status": "completed",
                    "role_metadata": {"tool_permission_profile": "read_only"},
                    "timeout_policy": {
                        "timeout_ms": 30000,
                        "source": "agent_guard.subagents.default_timeout_ms"
                    },
                    "outcome_code": "subagent_parallel_readonly_completed",
                    "conflict_count": 1,
                    "failure_isolated": true,
                    "confidence_summary": {"min": 0.72, "max": 0.93},
                    "main_thread_decision": {
                        "decision_status": "needs_conflict_resolution"
                    },
                    "finding_refs": ["finding:1"],
                    "evidence_refs": ["evidence:1"]
                }],
                "team_spec": {
                    "team_id": "subagent-batch:1:2",
                    "parent_task_id": "task-subagents",
                    "max_parallel": 2,
                    "write_permission": "read_only",
                    "conflict_policy": "parent_loop_resolution_required",
                    "child_task_ids": ["subagent:1:2:explorer"],
                    "children": [{"child_task_id": "subagent:1:2:explorer"}]
                }
            }
        }),
        result_text: None,
        error_text: None,
        events: vec![crate::events::TaskEventLine {
            event_type: "subagent".to_string(),
            line: "type=subagent child_run_id=subagent:1:2:verifier".to_string(),
            fields: std::collections::BTreeMap::from([
                (
                    "child_run_id".to_string(),
                    "subagent:1:2:verifier".to_string(),
                ),
                ("subagent_id".to_string(), "verifier".to_string()),
                ("status".to_string(), "succeeded".to_string()),
                (
                    "tool_permission_profile".to_string(),
                    "read_only".to_string(),
                ),
                (
                    "execution_mode".to_string(),
                    "inline_readonly_child_run".to_string(),
                ),
            ]),
        }],
    };

    let report = subagent_report_json(&task);
    assert_eq!(report["report_kind"], "agent_subagent_report");
    assert_eq!(report["task_id"], "task-subagents");
    assert_eq!(report["team_count"], 1);
    assert_eq!(report["teams"][0]["team_id"], "subagent-batch:1:2");
    assert_eq!(report["teams"][0]["child_count"], 1);
    assert_eq!(
        report["teams"][0]["child_task_ids"][0],
        "subagent:1:2:explorer"
    );
    assert_eq!(report["subagent_count"], 2);
    assert_eq!(
        report["subagents"][0]["child_run_id"],
        "subagent:1:2:explorer"
    );
    assert_eq!(report["subagents"][0]["result_status"], "completed");
    assert_eq!(
        report["subagents"][0]["outcome_code"],
        "subagent_parallel_readonly_completed"
    );
    assert_eq!(report["subagents"][0]["conflict_count"], 1);
    assert_eq!(
        report["subagents"][0]["decision_status"],
        "needs_conflict_resolution"
    );
    assert_eq!(report["subagents"][0]["confidence_min"], 0.72);
    assert_eq!(report["subagents"][0]["confidence_max"], 0.93);
    assert_eq!(report["subagents"][0]["failure_isolated"], true);
    assert_eq!(
        report["subagents"][0]["tool_permission_profile"],
        "read_only"
    );
    assert_eq!(report["subagents"][0]["read_only_enforced"], true);
    assert_eq!(
        report["subagents"][0]["write_isolation_status"],
        "not_supported"
    );
    assert_eq!(report["subagents"][0]["timeout_ms"], 30000);
    assert_eq!(
        report["subagents"][0]["timeout_source"],
        "agent_guard.subagents.default_timeout_ms"
    );
    assert_eq!(report["subagents"][0]["finding_refs"][0], "finding:1");
    assert_eq!(
        report["subagents"][1]["child_run_id"],
        "subagent:1:2:verifier"
    );
    assert_eq!(
        report["subagents"][1]["tool_permission_profile"],
        "read_only"
    );
    assert_eq!(report["subagents"][1]["read_only_enforced"], true);
}

use super::*;

#[test]
fn parent_planner_receives_both_child_results_before_bulk_graph_and_stale_tool_output() {
    let mut state = LoopState::new();
    let handoff = json!({"schema_version":1,"observation_kind":"subagent_structured_results",
        "parent_task_id":"parent","instruction_authority":"none","results":[
        {"child_task_id":"parent:child:a","status":"succeeded","result":{"component":"Panel","line":9},"evidence_refs":["task:child:a:result_json"]},
        {"child_task_id":"parent:child:b","status":"succeeded","result":{"handler":"get_status","line":317},"evidence_refs":["task:child:b:result_json"]}
    ]});
    let observation = json!({"schema_version":2,"owner_layer":"subagent_runtime",
        "action":"subagent_child_task_merge_reused","status":"ready",
        "child_task_merge":{"bulk_debug": "x".repeat(160_000)},"model_observation":handoff});
    state.last_output = Some(observation.to_string());
    state.task_observations.push(observation);
    let output = planner_last_observation(&state);
    let parsed: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed["results"][0]["result"]["component"], "Panel");
    assert_eq!(parsed["results"][1]["result"]["handler"], "get_status");
    assert_eq!(parsed["results"][1]["child_task_id"], "parent:child:b");
    assert!(output.len() < 4096);
    assert!(!output.contains("bulk_debug"));
    let entries = crate::agent_engine::observed_output::observed_output_entries(&state);
    assert!(entries[0].contains("get_status"));
    assert!(entries[0].contains("Panel"));
    state.last_output = Some("latest_tool_failure".to_string());
    assert_eq!(planner_last_observation(&state), "latest_tool_failure");
}

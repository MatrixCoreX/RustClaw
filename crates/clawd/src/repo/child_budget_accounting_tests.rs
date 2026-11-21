use super::*;

#[test]
fn child_budget_counts_execution_calls_from_trace_instead_of_empty_summary() {
    let result = json!({"task_journal":{
        "summary":{"task_metrics":{"llm_calls_per_task":4,"llm_elapsed_ms_per_task":12,
            "llm_cost":{"input_tokens":100,"output_tokens":25}}},
        "trace":{"step_results":[
            {"requested_action_type":"call_capability","status":"ok"},
            {"requested_action_type":"call_tool","status":"error"},
            {"requested_action_type":null,"action_kind":"call_skill","status":"error"},
            {"requested_action_type":"respond","status":"ok"}
        ]}
    }});
    let consumed = child_budget_consumed(&result);
    assert_eq!(consumed["tool_calls"], 3);
    assert_eq!(consumed["model_turns"], 4);
    assert_eq!(consumed["tokens"], 125);
}

#[test]
fn compacted_trace_does_not_reclaim_spent_child_call_budget() {
    let result = json!({"task_journal":{
        "summary":{"cost_budget":{"observed":{"tool_calls":22}}},
        "trace":{"step_results":[{"requested_action_type":"call_capability","status":"ok"}]}
    }});
    assert_eq!(child_budget_consumed(&result)["tool_calls"], 22);
}

use super::*;

fn capability_round(round_no: usize, capability: &str, args: Value) -> TaskJournalRoundTrace {
    TaskJournalRoundTrace {
        round_no,
        goal: "preview repair".to_string(),
        plan_result: Some(crate::PlanResult {
            goal: "preview repair".to_string(),
            missing_slots: Vec::new(),
            needs_confirmation: false,
            output_contract: None,
            steps: vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_capability".to_string(),
                skill: capability.to_string(),
                args: args.clone(),
                depends_on: Vec::new(),
                why: format!("capability:{capability}"),
            }],
            planner_notes: String::new(),
            plan_kind: crate::PlanKind::Single,
            raw_plan_text: json!({
                "steps": [{
                    "type": "call_capability",
                    "capability": capability,
                    "args": args,
                }]
            })
            .to_string(),
        }),
        ..Default::default()
    }
}

fn capability_resolution(round_no: usize, requested: &str, resolved: Option<&str>) -> Value {
    json!({
        "observation_kind": "capability_resolution",
        "owner_layer": "capability_resolver",
        "outcome": if resolved.is_some() { "resolved" } else { "unresolved" },
        "requested_capability": requested,
        "resolved_capability": resolved,
        "resolved_tool_or_skill": resolved.map(|_| "tool:task_control"),
        "round_no": round_no,
        "global_step": 1,
        "step_in_round": 1,
    })
}

#[test]
fn executed_round_wins_over_earlier_rejected_and_unresolved_capabilities() {
    let canonical = "coding_workflow.preview_repair";
    let mut journal = TaskJournal::for_task("task-capability-round", "ask", "preview repair");
    journal.rounds.push(capability_round(
        1,
        canonical,
        json!({"dry_run": true, "unexpected": true}),
    ));
    journal.rounds.push(capability_round(
        2,
        "task_control.preview_repair",
        json!({"action": "preview_coding_repair"}),
    ));
    journal
        .rounds
        .push(capability_round(3, canonical, json!({})));

    journal.push_task_observation(capability_resolution(1, canonical, Some(canonical)));
    journal.push_task_observation(capability_resolution(
        2,
        "task_control.preview_repair",
        None,
    ));
    journal.push_task_observation(capability_resolution(3, canonical, Some(canonical)));
    journal.push_task_observation(json!({
        "event_type": "post_tool_use",
        "round_no": 3,
        "global_step": 1,
        "step_in_round": 1,
        "tool_or_skill": "task_control",
        "status": "ok",
    }));
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "task_control".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"extra": {"dry_run": true}}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let step = trace
        .pointer("/step_results/0")
        .expect("step trace should be present");
    assert_eq!(
        step.get("requested_capability").and_then(Value::as_str),
        Some(canonical)
    );
    assert_eq!(
        step.get("resolved_capability").and_then(Value::as_str),
        Some(canonical)
    );
}

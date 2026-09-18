use super::*;
use crate::task_journal::checkpoint_step_provenance_records;

fn executed_step(step_id: &str, skill: &str) -> crate::executor::StepExecutionResult {
    crate::executor::StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"status": "ok"}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    }
}

#[test]
fn checkpoint_uses_dispatch_ids_instead_of_repeated_plan_local_ids() {
    let rounds = vec![
        capability_round(2, "video.generate", json!({"prompt": "test clip"})),
        capability_round(3, "filesystem.stat_paths", json!({"paths": ["clip.mp4"]})),
        capability_round(4, "filesystem.remove_path", json!({"path": "clip.mp4"})),
    ];
    let mut observations = Vec::new();
    for (round_no, global_step, capability, executable) in [
        (2, 2, "video.generate", "video_generate"),
        (3, 3, "filesystem.stat_paths", "fs_basic"),
        (4, 4, "filesystem.remove_path", "fs_basic"),
    ] {
        observations.push(json!({
            "observation_kind": "capability_resolution",
            "outcome": "resolved",
            "requested_capability": capability,
            "resolved_capability": capability,
            "resolved_tool_or_skill": format!("skill:{executable}"),
            "round_no": round_no,
            "global_step": global_step,
            "step_in_round": 1
        }));
        observations.push(json!({
            "event_type": "post_tool_use",
            "round_no": round_no,
            "global_step": global_step,
            "step_in_round": 1,
            "tool_or_skill": executable
        }));
    }
    let executed = vec![
        executed_step("step_1", "load_capability_groups"),
        executed_step("step_2", "video_generate"),
        executed_step("step_3", "fs_basic"),
        executed_step("step_4", "fs_basic"),
    ];
    let provenance = checkpoint_step_provenance_records(&rounds, &executed, &observations);
    assert_eq!(provenance.len(), 3);
    for (record, (step_id, capability)) in provenance.iter().zip([
        ("step_2", "video.generate"),
        ("step_3", "filesystem.stat_paths"),
        ("step_4", "filesystem.remove_path"),
    ]) {
        assert_eq!(record["step_id"], step_id);
        assert_eq!(record["requested_capability"], capability);
    }

    // A second checkpoint can have no original rounds, only restored provenance.
    let restored_provenance = checkpoint_step_provenance_records(&[], &executed, &provenance);
    assert_eq!(restored_provenance, provenance);
    let mut journal = TaskJournal::for_task("task-async-provenance", "ask", "generate and inspect");
    for record in restored_provenance {
        journal.push_task_observation(record);
    }
    for result in executed {
        journal.push_step_result(&result);
    }
    let trace = journal.to_trace_json();
    assert!(trace["step_results"][0]["requested_capability"].is_null());
    assert_eq!(
        trace["step_results"][1]["requested_capability"],
        "video.generate"
    );
    assert_eq!(
        trace["step_results"][2]["requested_capability"],
        "filesystem.stat_paths"
    );
    assert_eq!(
        trace["step_results"][3]["requested_capability"],
        "filesystem.remove_path"
    );
}

#[test]
fn completed_capability_result_restores_identity_when_resumed_rounds_are_absent() {
    let mut journal =
        TaskJournal::for_task("task-resumed-result", "ask", "transcribe downloaded audio");
    journal.task_observations.push(json!({
        "observation_kind": "capability_resolution",
        "outcome": "resolved",
        "requested_capability": "audio.transcribe",
        "resolved_capability": "audio.transcribe",
        "resolved_tool_or_skill": "skill:audio_transcribe",
        "round_no": 4,
        "global_step": 3,
        "step_in_round": 1,
    }));
    let mut result = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "audio.transcribe",
        Some("transcribe".to_string()),
        json!({"status": "ok"}),
    );
    result
        .evidence
        .push(claw_core::capability_result::EvidenceRef {
            id: "step_3".to_string(),
            source: "audio.transcribe".to_string(),
            locator: None,
            digest: None,
            metadata: json!({}),
        });
    journal.capability_results.push(result);
    journal.push_step_result(&executed_step("step_3", "audio_transcribe"));

    let operations = journal.executed_operation_evidence();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0]["requested_action_type"], "call_capability");
    assert_eq!(operations[0]["requested_capability"], "audio.transcribe");
    assert_eq!(operations[0]["resolved_capability"], "audio.transcribe");
}

#[test]
fn unknown_executable_does_not_consume_a_later_requested_capability() {
    let mut journal = TaskJournal::for_task("task-unknown-step", "ask", "inspect");
    journal
        .rounds
        .push(capability_round(2, "task_control.inspect", json!({})));
    journal.push_step_result(&executed_step("step_1", "load_capability_groups"));
    journal.push_step_result(&executed_step("step_2", "task_control"));
    let trace = journal.to_trace_json();
    assert!(trace["step_results"][0]["requested_capability"].is_null());
    assert_eq!(
        trace["step_results"][1]["requested_capability"],
        "task_control.inspect"
    );
}

#[test]
fn missing_dispatch_binding_does_not_guess_between_same_executable_actions() {
    let rounds = vec![
        capability_round(1, "task_control.status", json!({})),
        capability_round(2, "task_control.cancel", json!({})),
    ];
    let executed = [executed_step("step_1", "task_control")];
    assert!(checkpoint_step_provenance_records(&rounds, &executed, &[]).is_empty());
}

#[test]
fn verification_only_step_does_not_steal_replanned_execution_identity() {
    let rounds = vec![
        capability_round(1, "filesystem.stat_paths", json!({"paths": ["note.txt"]})),
        capability_round(2, "filesystem.remove_path", json!({"path": "note.txt"})),
    ];
    let resolution = |round_no, capability: &str, verify_only| {
        let mut value = json!({
            "observation_kind": "capability_resolution",
            "outcome": "resolved",
            "requested_capability": capability,
            "resolved_capability": capability,
            "resolved_tool_or_skill": "tool:fs_basic",
            "round_no": round_no,
            "global_step": 1,
            "step_in_round": 1
        });
        if verify_only {
            value["resolution_stage"] = json!("verify");
        }
        value
    };
    let observations = vec![
        resolution(1, "filesystem.stat_paths", true),
        resolution(2, "filesystem.remove_path", true),
        resolution(2, "filesystem.remove_path", false),
        json!({
            "event_type": "post_tool_use", "round_no": 2, "global_step": 1,
            "step_in_round": 1, "tool_or_skill": "remove_file", "status": "ok"
        }),
    ];
    let executed = [executed_step("step_1", "fs_basic")];
    assert!(checkpoint_step_provenance_records(&rounds, &executed, &observations[..1]).is_empty());
    let provenance = checkpoint_step_provenance_records(&rounds, &executed, &observations);
    assert_eq!(provenance.len(), 1);
    assert_eq!(
        provenance[0]["requested_capability"],
        "filesystem.remove_path"
    );
    assert_eq!(provenance[0]["round_no"], 2);

    let mut journal = TaskJournal::for_task("task-verify-replan", "ask", "inspect then remove");
    journal.rounds = rounds;
    for observation in observations {
        journal.push_task_observation(observation);
    }
    journal.push_step_result(&executed[0]);
    let trace = journal.to_trace_json();
    assert_eq!(
        trace["step_results"][0]["requested_capability"],
        "filesystem.remove_path"
    );
}

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

#[test]
fn restored_step_provenance_precedes_new_terminal_round_attribution() {
    let mut journal = TaskJournal::for_task("task-restored-provenance", "ask", "run then report");
    journal.push_task_observation(json!({
        "schema_version": 1,
        "observation_kind": "checkpoint_step_provenance",
        "owner_layer": "task_journal",
        "step_id": "step_2",
        "requested_action_type": "call_capability",
        "requested_capability": "system.run_command",
        "requested_action_ref": "system.run_command",
        "resolved_capability": "system.run_command",
        "resolved_tool_or_skill": "run_cmd",
        "round_no": 2,
        "step_in_round": 1,
        "dispatch_executed": true,
    }));
    journal.rounds.push(TaskJournalRoundTrace {
        round_no: 3,
        goal: "report completion".to_string(),
        plan_result: Some(crate::PlanResult {
            goal: "report completion".to_string(),
            missing_slots: Vec::new(),
            needs_confirmation: false,
            output_contract: None,
            steps: vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "respond".to_string(),
                skill: "respond".to_string(),
                args: json!({"text": "done"}),
                depends_on: Vec::new(),
                why: "report completion".to_string(),
            }],
            planner_notes: String::new(),
            plan_kind: crate::PlanKind::Single,
            raw_plan_text: json!({"steps": [{"type": "respond", "text": "done"}]}).to_string(),
        }),
        ..Default::default()
    });
    for (step_id, skill, output) in [
        ("step_2", "run_cmd", "command complete"),
        ("step_1", "respond", "done"),
    ] {
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(output.to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });
    }

    let trace = journal.to_trace_json();
    let steps = trace["step_results"].as_array().expect("step results");
    assert_eq!(steps[0]["requested_capability"], "system.run_command");
    assert_eq!(steps[0]["requested_action_ref"], "system.run_command");
    assert_eq!(steps[0]["requested_action_type"], "call_capability");
    assert_eq!(steps[1]["requested_capability"], "respond");
}

use super::*;
use crate::task_journal::{TaskJournal, TaskJournalRoundTrace, TaskJournalVerifySummary};

fn recorded_round() -> TaskJournalRoundTrace {
    let proposed = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Scalar,
        ..Default::default()
    };
    let effective = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        requires_content_evidence: true,
        selection: crate::pipeline_types::OutputSelectionContract {
            output_field_labels: vec!["code".into()],
            ..Default::default()
        },
        ..Default::default()
    };
    TaskJournalRoundTrace {
        round_no: 7,
        goal: "read fixture".into(),
        execution_recipe_summary: Some("phase=validation".into()),
        plan_result: Some(crate::PlanResult {
            goal: "read fixture".into(),
            missing_slots: Vec::new(),
            needs_confirmation: false,
            output_contract: Some(proposed),
            steps: vec![crate::PlanStep {
                step_id: "step_7".into(),
                action_type: "call_capability".into(),
                skill: "filesystem.read_text_range".into(),
                args: json!({"path":"fixture.txt"}),
                depends_on: Vec::new(),
                why: String::new(),
            }],
            planner_notes: String::new(),
            plan_kind: crate::PlanKind::Incremental,
            raw_plan_text: String::new(),
        }),
        effective_output_contract: Some(effective),
        verify_result: Some(TaskJournalVerifySummary {
            mode: crate::verifier::VerifyMode::Enforce,
            approved: true,
            permission_decision: json!({"schema_version":1,"decision":"allow"}),
            ..Default::default()
        }),
    }
}

#[test]
fn checkpoint_round_history_and_contract_evidence_survive_repeated_resumes() {
    let mut original = LoopState::default();
    original.round_traces.push(recorded_round());
    original.output_contract = original.round_traces[0].effective_output_contract.clone();
    let expected = serde_json::to_value(&original.round_traces).unwrap();
    let mut journal = TaskJournal::for_task("trace-fixture", "ask", "fixture");
    journal.rounds = original.round_traces.clone();
    let expected_trace = journal.to_trace_json()["rounds"].clone();

    for _ in 0..3 {
        let bytes = serde_json::to_vec(&json!({"agent_loop_resume_state":
            build_checkpoint_resume_state(&original, AgentCheckpointStage::Verification)}))
        .unwrap();
        let boundary: Value = serde_json::from_slice(&bytes).unwrap();
        let mut restored = LoopState::default();
        assert_eq!(
            restore_checkpoint_resume_state(&mut restored, &boundary),
            AgentCheckpointStage::Verification
        );
        // Restoring the same durable snapshot must not duplicate rounds.
        restore_checkpoint_resume_state(&mut restored, &boundary);
        assert_eq!(
            serde_json::to_value(&restored.round_traces).unwrap(),
            expected
        );
        journal.rounds = restored.round_traces.clone();
        let trace = journal.to_trace_json();
        assert_eq!(trace["rounds"], expected_trace);
        assert_eq!(
            trace["rounds"][0]["proposed_output_contract"]["response_shape"],
            "scalar"
        );
        assert_eq!(
            trace["rounds"][0]["effective_output_contract"]["response_shape"],
            "strict"
        );
        assert_eq!(
            trace["rounds"][0]["decision_envelope"]["source"],
            "planner_round_action"
        );
        assert!(restored.executed_step_results.is_empty());
        assert_eq!(restored.tool_calls_total, 0);
        original = restored;
    }
}

#[test]
fn old_checkpoint_without_round_history_does_not_fabricate_trace() {
    let boundary = json!({"agent_loop_resume_state":{"schema_version":1,"stage":"verification"}});
    let mut restored = LoopState::default();
    restore_checkpoint_resume_state(&mut restored, &boundary);
    assert!(restored.round_traces.is_empty());
    let mut legacy = serde_json::to_value(recorded_round()).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("effective_output_contract");
    let round: TaskJournalRoundTrace = serde_json::from_value(legacy).unwrap();
    assert!(round.effective_output_contract.is_none());
}

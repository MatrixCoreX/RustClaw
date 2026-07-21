use super::{
    BudgetDecision, BudgetHardCeilings, BudgetObservation, BudgetProgress, TaskBudgetProfile,
    TaskBudgetSlice,
};

fn slice() -> TaskBudgetSlice {
    TaskBudgetSlice::new(
        TaskBudgetProfile::MultiStepWorkspace,
        30_000,
        BudgetHardCeilings {
            model_turns: 20,
            tool_calls: 40,
            elapsed_ms: 600_000,
            continuations: 8,
        },
    )
}

#[test]
fn progress_with_capacity_continues() {
    let mut slice = slice();
    let decision = slice.observe(BudgetObservation {
        cumulative_model_turns: 5,
        cumulative_tool_calls: 13,
        cumulative_elapsed_ms: 40_000,
        progress: BudgetProgress {
            evidence_count: 2,
            completed_plan_nodes: 1,
            ..BudgetProgress::default()
        },
        resumable: true,
        ..BudgetObservation::default()
    });

    assert_eq!(decision, BudgetDecision::Continue);
    assert!(slice.progress.observed_progress());
    assert_eq!(slice.cumulative_tool_calls, 13);
}

#[test]
fn terminal_model_turn_finishes_without_checkpoint() {
    let mut slice = slice();
    let decision = slice.observe(BudgetObservation {
        cumulative_model_turns: 2,
        model_finished: true,
        ..BudgetObservation::default()
    });

    assert_eq!(decision, BudgetDecision::Finish);
}

#[test]
fn soft_slice_exhaustion_requeues_resumable_work() {
    let mut slice = slice();
    let decision = slice.observe(BudgetObservation {
        cumulative_model_turns: 7,
        cumulative_tool_calls: 15,
        cumulative_elapsed_ms: 30_000,
        soft_slice_exhausted: true,
        resumable: true,
        ..BudgetObservation::default()
    });

    assert_eq!(decision, BudgetDecision::CheckpointRequeue);
}

#[test]
fn soft_slice_exhaustion_is_terminal_without_resume_state() {
    let mut slice = slice();
    let decision = slice.observe(BudgetObservation {
        cumulative_elapsed_ms: 30_000,
        soft_slice_exhausted: true,
        resumable: false,
        ..BudgetObservation::default()
    });

    assert_eq!(decision, BudgetDecision::Terminal);
}

#[test]
fn waiting_and_user_input_are_distinct_machine_decisions() {
    let mut waiting = slice();
    assert_eq!(
        waiting.observe(BudgetObservation {
            waiting: true,
            ..BudgetObservation::default()
        }),
        BudgetDecision::Waiting
    );

    let mut needs_user = slice();
    assert_eq!(
        needs_user.observe(BudgetObservation {
            needs_user: true,
            ..BudgetObservation::default()
        }),
        BudgetDecision::NeedsUser
    );
}

#[test]
fn administrator_ceiling_is_terminal_even_with_progress() {
    let mut slice = slice();
    let decision = slice.observe(BudgetObservation {
        cumulative_model_turns: 6,
        cumulative_tool_calls: 40,
        progress: BudgetProgress {
            artifact_count: 1,
            ..BudgetProgress::default()
        },
        resumable: true,
        ..BudgetObservation::default()
    });

    assert_eq!(decision, BudgetDecision::Terminal);
}

#[test]
fn checkpoint_round_trip_resumes_once() {
    let mut original = slice();
    original.observe(BudgetObservation {
        cumulative_model_turns: 8,
        cumulative_tool_calls: 17,
        cumulative_elapsed_ms: 90_000,
        soft_slice_exhausted: true,
        resumable: true,
        progress: BudgetProgress {
            verified_state_transitions: 3,
            ..BudgetProgress::default()
        },
        ..BudgetObservation::default()
    });

    let restored = TaskBudgetSlice::from_machine_json(&original.to_machine_json())
        .expect("budget slice round trip")
        .resumed();
    assert_eq!(restored.continuation_index, 1);
    assert_eq!(restored.cumulative_tool_calls, 17);
    assert_eq!(restored.progress.verified_state_transitions, 3);
    assert_eq!(restored.last_decision, BudgetDecision::Continue);
}

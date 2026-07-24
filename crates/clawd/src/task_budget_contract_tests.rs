use super::{
    profile_for_verified_plan, task_budget_policy_from_toml, BudgetDecision, BudgetHardCeilings,
    BudgetObservation, BudgetProfilePolicy, BudgetProgress, BudgetTimeoutClass, TaskBudgetProfile,
    TaskBudgetSlice, VerifiedPlanBudgetFacts,
};

fn slice() -> TaskBudgetSlice {
    TaskBudgetSlice::new(
        TaskBudgetProfile::MultiStepWorkspace,
        30_000,
        BudgetHardCeilings {
            model_turns: 20,
            tool_calls: 40,
            total_tokens: 100_000,
            cost_usd_nanos: 1_000_000_000,
            elapsed_ms: 600_000,
            continuations: 8,
            non_resumable_tool_runtime_ms: 60_000,
        },
    )
}

#[test]
fn progress_crosses_former_round_and_tool_thresholds() {
    let mut slice = slice();
    let decision = slice.observe(BudgetObservation {
        cumulative_model_turns: 6,
        cumulative_tool_calls: 14,
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
    assert_eq!(slice.cumulative_tool_calls, 14);
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
fn historical_progress_does_not_hide_repeated_stagnation() {
    let mut slice = slice();
    slice.progress.evidence_count = 4;
    slice.progress.completed_plan_nodes = 2;
    let decision = slice.observe(BudgetObservation {
        cumulative_model_turns: 6,
        cumulative_tool_calls: 9,
        progress: BudgetProgress {
            evidence_count: 4,
            completed_plan_nodes: 2,
            stagnation_count: 3,
            ..BudgetProgress::default()
        },
        stagnation_exhausted: true,
        resumable: true,
        ..BudgetObservation::default()
    });

    assert_eq!(decision, BudgetDecision::Terminal);
}

#[test]
fn changed_machine_digest_advances_progress_when_evidence_count_is_stable() {
    let mut slice = slice();
    slice.progress.evidence_count = 1;
    slice.progress.machine_progress_digest = Some("sha256:pending".to_string());

    let decision = slice.observe(BudgetObservation {
        cumulative_model_turns: 2,
        progress: BudgetProgress {
            evidence_count: 1,
            machine_progress_digest: Some("sha256:complete".to_string()),
            stagnation_count: 3,
            ..BudgetProgress::default()
        },
        stagnation_exhausted: true,
        resumable: true,
        ..BudgetObservation::default()
    });

    assert_eq!(decision, BudgetDecision::Continue);
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

#[test]
fn machine_config_selects_profile_slice_and_admin_ceilings() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[agent.task_budget]
admin_max_model_turns = 80
admin_max_tool_calls = 160
admin_max_total_tokens = 500000
admin_max_cost_usd_nanos = 9000000000
admin_max_elapsed_seconds = 7200
admin_max_continuations = 12
admin_max_non_resumable_tool_seconds = 900

[agent.task_budget.profiles.multi_step_workspace]
soft_slice_seconds = 420
stagnation_tolerance = 5
provider_timeout_class = "standard"
tool_timeout_class = "long_tail"
"#,
    )
    .expect("task budget config");

    let policy = task_budget_policy_from_toml(&parsed);
    let profile = policy.profile(TaskBudgetProfile::MultiStepWorkspace);

    assert_eq!(profile.soft_slice_ms, 420_000);
    assert_eq!(profile.stagnation_tolerance, 5);
    assert_eq!(profile.provider_timeout_class, BudgetTimeoutClass::Standard);
    assert_eq!(profile.tool_timeout_class, BudgetTimeoutClass::LongTail);
    assert_eq!(policy.hard_ceilings.model_turns, 80);
    assert_eq!(policy.hard_ceilings.tool_calls, 160);
    assert_eq!(policy.hard_ceilings.total_tokens, 500_000);
    assert_eq!(policy.hard_ceilings.cost_usd_nanos, 9_000_000_000);
    assert_eq!(policy.hard_ceilings.elapsed_ms, 7_200_000);
    assert_eq!(policy.hard_ceilings.continuations, 12);
    assert_eq!(policy.hard_ceilings.non_resumable_tool_runtime_ms, 900_000);
}

#[test]
fn verified_machine_plan_facts_select_profiles_without_user_text() {
    assert_eq!(
        profile_for_verified_plan(VerifiedPlanBudgetFacts {
            action_count: 1,
            observe_count: 1,
            ..VerifiedPlanBudgetFacts::default()
        }),
        TaskBudgetProfile::FastRead
    );
    assert_eq!(
        profile_for_verified_plan(VerifiedPlanBudgetFacts {
            action_count: 2,
            observe_count: 2,
            evidence_required: true,
            ..VerifiedPlanBudgetFacts::default()
        }),
        TaskBudgetProfile::GroundedSummary
    );
    assert_eq!(
        profile_for_verified_plan(VerifiedPlanBudgetFacts {
            action_count: 3,
            mutate_count: 1,
            needs_confirmation: true,
            ..VerifiedPlanBudgetFacts::default()
        }),
        TaskBudgetProfile::MultiStepWorkspace
    );
    assert_eq!(
        profile_for_verified_plan(VerifiedPlanBudgetFacts {
            ops_closed_loop: true,
            ..VerifiedPlanBudgetFacts::default()
        }),
        TaskBudgetProfile::OpsClosedLoop
    );
}

#[test]
fn verified_plan_profiles_only_widen_after_the_initial_selection() {
    assert_eq!(
        TaskBudgetProfile::FastRead.widen_with(TaskBudgetProfile::MultiStepWorkspace),
        TaskBudgetProfile::MultiStepWorkspace
    );
    assert_eq!(
        TaskBudgetProfile::MultiStepWorkspace.widen_with(TaskBudgetProfile::FastRead),
        TaskBudgetProfile::MultiStepWorkspace
    );
    assert_eq!(
        TaskBudgetProfile::GroundedSummary.widen_with(TaskBudgetProfile::OpsClosedLoop),
        TaskBudgetProfile::OpsClosedLoop
    );
}

#[test]
fn timeout_classes_are_bounded_by_soft_slice_and_administrator_ceiling() {
    let mut slice = TaskBudgetSlice::new_with_policy(
        TaskBudgetProfile::MultiStepWorkspace,
        BudgetProfilePolicy {
            soft_slice_ms: 900_000,
            stagnation_tolerance: 4,
            provider_timeout_class: BudgetTimeoutClass::Standard,
            tool_timeout_class: BudgetTimeoutClass::LongTail,
        },
        BudgetHardCeilings {
            non_resumable_tool_runtime_ms: 300_000,
            ..BudgetHardCeilings::default()
        },
    );

    assert_eq!(slice.provider_call_timeout_seconds(), 180);
    assert_eq!(slice.tool_call_timeout_seconds(), 300);

    slice.soft_slice_ms = 45_000;
    assert_eq!(slice.provider_call_timeout_seconds(), 44);
    assert_eq!(slice.tool_call_timeout_seconds(), 44);

    slice.soft_slice_ms = 500;
    assert_eq!(slice.provider_call_timeout_seconds(), 1);
    assert_eq!(slice.tool_call_timeout_seconds(), 1);
}

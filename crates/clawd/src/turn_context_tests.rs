use super::{TargetTaskPolicy, TurnType};

#[test]
fn planner_turn_machine_tokens_are_stable() {
    let turn_types = [
        (TurnType::TaskRequest, "task_request"),
        (TurnType::TaskAppend, "task_append"),
        (TurnType::TaskReplace, "task_replace"),
        (TurnType::TaskCorrect, "task_correct"),
        (TurnType::TaskScopeUpdate, "task_scope_update"),
        (TurnType::RunControl, "run_control"),
        (TurnType::ApprovalDecision, "approval_decision"),
        (TurnType::StatusQuery, "status_query"),
        (TurnType::FeedbackOrError, "feedback_or_error"),
        (TurnType::PreferenceOrMemory, "preference_or_memory"),
    ];
    for (value, expected) in turn_types {
        assert_eq!(value.as_str(), expected);
    }

    let task_policies = [
        (TargetTaskPolicy::ReuseActive, "reuse_active"),
        (TargetTaskPolicy::ReplaceActive, "replace_active"),
        (TargetTaskPolicy::PauseAndQueue, "pause_and_queue"),
        (TargetTaskPolicy::Standalone, "standalone"),
    ];
    for (value, expected) in task_policies {
        assert_eq!(value.as_str(), expected);
    }
}

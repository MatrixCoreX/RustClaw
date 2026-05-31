use super::*;

fn all_states() -> [AskState; 11] {
    [
        AskState::Received,
        AskState::Routing,
        AskState::Clarifying,
        AskState::Chatting,
        AskState::ResumeExecuting,
        AskState::ResumeDiscussing,
        AskState::ScheduleDirect,
        AskState::Executing,
        AskState::Finalizing,
        AskState::Completed,
        AskState::Failed,
    ]
}

#[test]
fn as_str_is_stable_and_unique() {
    let labels: Vec<&'static str> = all_states().iter().map(|s| s.as_str()).collect();
    let unique: std::collections::HashSet<&'static str> = labels.iter().copied().collect();
    assert_eq!(labels.len(), unique.len(), "as_str labels must be unique");
    assert!(
        labels.iter().all(|s| !s.is_empty()),
        "as_str must not be empty"
    );
}

#[test]
fn terminal_states_cannot_transition() {
    for next in all_states() {
        assert!(
            !AskState::Completed.can_transition_to(next),
            "Completed must not transition to {:?}",
            next
        );
        assert!(
            !AskState::Failed.can_transition_to(next),
            "Failed must not transition to {:?}",
            next
        );
    }
}

#[test]
fn any_non_terminal_can_fail() {
    for s in all_states() {
        if !s.is_terminal() {
            assert!(
                s.can_transition_to(AskState::Failed),
                "{:?} should be allowed to fail",
                s
            );
        }
    }
}

#[test]
fn happy_path_act_is_legal() {
    let path = [
        AskState::Received,
        AskState::Routing,
        AskState::Executing,
        AskState::Executing,
        AskState::Finalizing,
        AskState::Completed,
    ];
    for w in path.windows(2) {
        assert!(
            w[0].can_transition_to(w[1]),
            "{:?} → {:?} must be legal",
            w[0],
            w[1]
        );
    }
}

#[test]
fn happy_path_chat_is_legal() {
    for next in [AskState::Finalizing, AskState::Completed] {
        assert!(AskState::Chatting.can_transition_to(next));
    }
}

#[test]
fn happy_path_clarify_is_legal() {
    assert!(AskState::Received.can_transition_to(AskState::Routing));
    assert!(AskState::Routing.can_transition_to(AskState::Clarifying));
    assert!(AskState::Clarifying.can_transition_to(AskState::Completed));
}

#[test]
fn resume_execution_path_is_legal() {
    assert!(AskState::Routing.can_transition_to(AskState::ResumeExecuting));
    assert!(AskState::ResumeExecuting.can_transition_to(AskState::Executing));
    assert!(AskState::ResumeExecuting.can_transition_to(AskState::Finalizing));
    assert!(AskState::ResumeExecuting.can_transition_to(AskState::Completed));
}

#[test]
fn schedule_direct_path_is_legal() {
    assert!(AskState::Routing.can_transition_to(AskState::ScheduleDirect));
    assert!(AskState::ScheduleDirect.can_transition_to(AskState::Completed));
    assert!(!AskState::ScheduleDirect.can_transition_to(AskState::Finalizing));
}

#[test]
fn illegal_transitions_are_rejected() {
    // 不能跳过 Routing 直接 Executing
    assert!(!AskState::Received.can_transition_to(AskState::Executing));
    // 不能从 Routing 直接到 Completed（必须经过分支状态）
    assert!(!AskState::Routing.can_transition_to(AskState::Completed));
    // Chatting 不能进 Executing
    assert!(!AskState::Chatting.can_transition_to(AskState::Executing));
    // Clarifying 不能进 Finalizing
    assert!(!AskState::Clarifying.can_transition_to(AskState::Finalizing));
    // Executing 不能直接到 Completed（必须经过 Finalizing）
    assert!(!AskState::Executing.can_transition_to(AskState::Completed));
    // 不能自循环（除 Executing 外）
    assert!(!AskState::Routing.can_transition_to(AskState::Routing));
    assert!(!AskState::Finalizing.can_transition_to(AskState::Finalizing));
}

#[test]
fn ask_transition_records_metadata() {
    let t = AskTransition::new(
        Some(AskState::Routing),
        AskState::Executing,
        "act_branch",
        1_700_000_000_000,
        None,
    );
    assert_eq!(t.from, Some(AskState::Routing));
    assert_eq!(t.to, AskState::Executing);
    assert_eq!(t.reason, "act_branch");
    assert_eq!(t.at_ms, 1_700_000_000_000);
    assert_eq!(t.round_no, None);
}

#[test]
fn executing_self_loop_is_legal() {
    assert!(AskState::Executing.can_transition_to(AskState::Executing));
}

#[test]
fn now_unix_ms_is_monotonic_enough() {
    let a = super::now_unix_ms();
    let b = super::now_unix_ms();
    assert!(
        b >= a,
        "now_unix_ms should be non-decreasing in same call sequence"
    );
    assert!(a > 0, "now_unix_ms should be positive after UNIX_EPOCH");
}

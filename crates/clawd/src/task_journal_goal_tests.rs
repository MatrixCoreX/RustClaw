use serde_json::{json, Value};

use super::{TaskJournal, TaskJournalFinalStatus};

fn step_result(
    step_id: &str,
    skill: &str,
    status: crate::executor::StepExecutionStatus,
    output: Option<String>,
    error: Option<String>,
) -> crate::executor::StepExecutionResult {
    crate::executor::StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status,
        output,
        error,
        started_at: 1,
        finished_at: 2,
    }
}

#[test]
fn summary_json_includes_machine_readable_task_goal() {
    let mut journal = TaskJournal::for_task("task-goal-summary", "ask", "change and verify");
    journal.push_step_result(&step_result(
        "step_1",
        "run_cmd",
        crate::executor::StepExecutionStatus::Ok,
        Some(
            json!({
                "extra": {
                    "command": "cargo test -p clawd"
                },
                "validation_result": {
                    "status": "passed",
                    "status_code": "tests_passed",
                    "message_key": "clawd.validation.tests_passed"
                }
            })
            .to_string(),
        ),
        None,
    ));
    journal.record_final_status(TaskJournalFinalStatus::Success);

    let summary = journal.to_summary_json();
    let goal = summary.get("task_goal").expect("task_goal");

    assert_eq!(goal.get("schema_version").and_then(Value::as_u64), Some(1));
    assert_eq!(
        goal.get("goal_id").and_then(Value::as_str),
        Some("task:task-goal-summary")
    );
    assert_eq!(
        goal.get("goal_status").and_then(Value::as_str),
        Some("completed")
    );
    assert_eq!(
        goal.get("goal_status_source").and_then(Value::as_str),
        Some("journal_final_status")
    );
    assert_eq!(
        goal.get("validation_status").and_then(Value::as_str),
        Some("passed")
    );
    assert_eq!(
        goal.pointer("/verification_commands/0")
            .and_then(Value::as_str),
        Some("cargo test -p clawd")
    );
    assert_eq!(
        goal.pointer("/success_evidence_refs/0")
            .and_then(Value::as_str),
        Some("coding_checkpoint:verification_command:step_1")
    );
    assert!(goal
        .get("current_progress")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.as_str() == Some("final_status=success"))));
    assert!(goal.get("message_zh").is_none());
    assert!(goal.get("message_en").is_none());
}

#[test]
fn summary_json_merges_payload_task_goal_spec() {
    let mut journal = TaskJournal::for_task("task-goal-spec", "ask", "continue goal");
    journal.record_task_goal_spec(json!({
        "goal_id": "goal-user-1",
        "objective": "ship feature",
        "constraints": ["no runtime natural-language matching"],
        "done_conditions": ["tests_pass"],
        "verification_commands": ["cargo test -p clawd task_goal -- --quiet"],
        "allowed_files_or_scopes": ["crates/clawd"],
        "forbidden_actions": ["external_publish"],
        "goal_status": "created"
    }));

    let summary = journal.to_summary_json();
    let goal = summary.get("task_goal").expect("task_goal");

    assert_eq!(
        goal.get("goal_id").and_then(Value::as_str),
        Some("goal-user-1")
    );
    assert_eq!(
        goal.get("objective").and_then(Value::as_str),
        Some("ship feature")
    );
    assert_eq!(
        goal.pointer("/constraints/0").and_then(Value::as_str),
        Some("no runtime natural-language matching")
    );
    assert_eq!(
        goal.pointer("/done_conditions/0").and_then(Value::as_str),
        Some("tests_pass")
    );
    assert_eq!(
        goal.pointer("/allowed_files_or_scopes/0")
            .and_then(Value::as_str),
        Some("crates/clawd")
    );
    assert_eq!(
        goal.pointer("/forbidden_actions/0").and_then(Value::as_str),
        Some("external_publish")
    );
    assert_eq!(
        goal.get("goal_status").and_then(Value::as_str),
        Some("created")
    );
    assert_eq!(
        goal.get("goal_status_source").and_then(Value::as_str),
        Some("goal")
    );
}

#[test]
fn summary_json_prefers_evidence_status_and_merges_goal_commands() {
    let mut journal = TaskJournal::for_task("task-goal-verified", "ask", "verify goal");
    journal.record_task_goal_spec(json!({
        "objective": "verify code",
        "verification_commands": ["cargo check -p clawd"],
        "goal_status": "created"
    }));
    journal.push_step_result(&step_result(
        "step_1",
        "run_cmd",
        crate::executor::StepExecutionStatus::Ok,
        Some(
            json!({
                "extra": {
                    "command": "cargo test -p clawd task_goal -- --quiet"
                },
                "validation_result": {
                    "status": "passed",
                    "status_code": "tests_passed"
                }
            })
            .to_string(),
        ),
        None,
    ));
    journal.record_final_status(TaskJournalFinalStatus::Success);

    let summary = journal.to_summary_json();
    let goal = summary.get("task_goal").expect("task_goal");

    assert_eq!(
        goal.get("goal_status").and_then(Value::as_str),
        Some("completed")
    );
    assert_eq!(
        goal.get("goal_status_source").and_then(Value::as_str),
        Some("journal_final_status")
    );
    let commands = goal
        .get("verification_commands")
        .and_then(Value::as_array)
        .expect("verification_commands")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(commands.contains(&"cargo check -p clawd"));
    assert!(commands.contains(&"cargo test -p clawd task_goal -- --quiet"));
}

#[test]
fn summary_json_marks_missing_evidence_as_remaining_work() {
    let mut journal = TaskJournal::for_task("task-goal-missing", "ask", "list files");
    let mut route = crate::RouteResult {
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.record_final_status(TaskJournalFinalStatus::Success);

    let summary = journal.to_summary_json();
    let goal = summary.get("task_goal").expect("task_goal");

    assert_eq!(
        goal.get("goal_status").and_then(Value::as_str),
        Some("blocked")
    );
    assert_eq!(
        goal.get("goal_status_source").and_then(Value::as_str),
        Some("evidence_coverage")
    );
    assert!(goal
        .get("remaining_work")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty()));
    assert!(goal.get("missing_evidence").is_some());
}

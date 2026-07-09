use serde_json::json;

use super::{local_missing_evidence_verifier_gap_for_answer, route_with_mode};

#[test]
fn local_missing_evidence_gap_skips_git_and_package_status_observations() {
    let mut route = route_with_mode(crate::AskMode::act_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.requires_content_evidence = true;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-git-package-status-gap", "ask", "status");

    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "status",
                    "branch": "main",
                    "changed_count": 0,
                    "clean": true,
                    "field_value": {
                        "action": "status",
                        "branch": "main",
                        "changed_count": 0,
                        "clean": true
                    }
                },
                "text": "exit=0\n## main...origin/main"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "git_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "log",
                    "commit_count": 1,
                    "field_value": {
                        "action": "log",
                        "commit_count": 1
                    },
                    "commits": [
                        {"sha": "abc1234", "subject": "Test commit"}
                    ]
                },
                "text": "exit=0\nabc1234 Test commit"
            })
            .to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_3".to_string(),
        skill: "package_manager".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "detect",
                    "available": true,
                    "manager": "apt-get",
                    "manager_scope": "system",
                    "version_present": true
                },
                "text": "manager=apt-get available=true version_present=true"
            })
            .to_string(),
        ),
        error: None,
        started_at: 5,
        finished_at: 6,
    });

    assert!(local_missing_evidence_verifier_gap_for_answer(
        &route,
        &journal,
        "当前分支：main\n工作区变更文件数：0\n最近 1 条提交：abc1234 Test commit\n系统包管理器：apt-get"
    )
    .is_none());
}

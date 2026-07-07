use super::{recover_requested_machine_kv_summary_final_answer, route_result};

use serde_json::json;

#[test]
fn requested_machine_kv_summary_failure_recovery_replaces_publishable_git_prose() {
    let prompt = "检查当前 git 状态，只返回 branch、worktree_state、changed_count。";
    let route = route_result(crate::AskMode::act_plain());
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-git-status-machine-kv", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "git_basic",
            json!({
                "extra": {
                    "action": "status",
                    "branch": "main",
                    "changed_count": 0,
                    "field_value": {
                        "branch": "main",
                        "changed_count": 0,
                        "worktree_state": "clean"
                    },
                    "worktree_state": "clean"
                },
                "text": "exit=0\n## main...origin/main\n"
            })
            .to_string(),
        ));
    let mut answer_text = "Git 检查已完成，但最终无法稳定输出 branch、worktree_state、changed_count。示例 branch=main worktree_state=clean changed_count=0。".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(recover_requested_machine_kv_summary_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
        true,
    ));

    assert_eq!(
        answer_text,
        "branch=main worktree_state=clean changed_count=0"
    );
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert!(journal
        .answer_verifier_summary
        .as_ref()
        .is_some_and(|summary| summary.pass));
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}

#[test]
fn requested_machine_kv_summary_verifier_gap_recovery_projects_git_remotes() {
    let prompt = "Return only branch and remotes.";
    let route = route_result(crate::AskMode::act_plain());
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-git-remotes-machine-kv", "ask", prompt);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["path".to_string()],
        answer_incomplete_reason: "missing_required_evidence:path".to_string(),
        should_retry: true,
        retry_instruction: "collect_required_evidence_fields:path".to_string(),
        confidence: 0.5,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "git_basic",
            json!({
                "extra": {
                    "action": "current_branch",
                    "branch": "main",
                    "field_value": {
                        "branch": "main"
                    }
                }
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "git_basic",
            json!({
                "extra": {
                    "action": "remote",
                    "field_value": {
                        "remotes": ["origin"],
                        "remote_names": ["origin"],
                        "remote_urls": ["git@github.com:MatrixCoreX/RustClaw.git"]
                    },
                    "remote_names": ["origin"],
                    "remote_urls": ["git@github.com:MatrixCoreX/RustClaw.git"],
                    "remotes": [
                        {
                            "direction": "fetch",
                            "name": "origin",
                            "url": "git@github.com:MatrixCoreX/RustClaw.git"
                        }
                    ]
                }
            })
            .to_string(),
        ));
    let mut answer_text = "Current branch: main. Configured remote: origin.".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(recover_requested_machine_kv_summary_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
        true,
    ));

    assert_eq!(answer_text, r#"branch=main remotes=["origin"]"#);
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert!(journal
        .answer_verifier_summary
        .as_ref()
        .is_some_and(|summary| summary.pass));
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}

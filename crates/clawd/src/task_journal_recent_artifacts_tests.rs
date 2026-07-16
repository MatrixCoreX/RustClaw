use serde_json::json;

use super::{evidence_coverage_for_output_contract, TaskJournal, TaskJournalStepTrace};

fn recent_artifacts_route() -> crate::IntentOutputContract {
    crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::RecentArtifactsJudgment,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        requires_content_evidence: true,
        ..Default::default()
    }
}

#[test]
fn recent_artifacts_directory_structure_satisfies_directory_judgment_evidence() {
    let route = recent_artifacts_route();
    let mut journal = TaskJournal::for_task(
        "recent-artifacts-dir-structure",
        "ask",
        "judge recent workspace directories",
    );
    journal.record_output_contract(&route.clone());
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        json!({
            "action": "list_dir",
            "path": ".",
            "entries": [
                {"name": "logs", "kind": "dir"},
                {"name": "data", "kind": "dir"},
                {"name": "tmp", "kind": "dir"}
            ]
        })
        .to_string(),
    ));
    journal
        .step_results
        .push(TaskJournalStepTrace::ok(
            "step_2",
            "system_basic",
            json!({
                "action": "tree_summary",
                "path": ".",
                "tree": {
                    "path": ".",
                    "kind": "dir",
                    "children": [
                        {"path": "logs", "kind": "dir", "children": [{"path": "logs/runtime.log", "kind": "file"}]},
                        {"path": "data", "kind": "dir", "children": [{"path": "data/clawd.db", "kind": "file"}]},
                        {"path": "tmp", "kind": "dir", "children": [{"path": "tmp/archive.zip", "kind": "file"}]}
                    ]
                }
            })
            .to_string(),
        ));

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("directory_structure"));
    assert!(
        coverage.is_complete(),
        "unexpected missing evidence: {:?}",
        coverage.missing_evidence
    );
}

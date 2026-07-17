use serde_json::json;

use super::compacted_machine_ref_gap::local_compacted_machine_ref_answer_verifier_gap;

fn journal_with_continuity_refs(refs: &[&str]) -> crate::task_journal::TaskJournal {
    let mut journal = crate::task_journal::TaskJournal::default();
    journal.push_task_observation(crate::task_journal::context_compaction_record_observation(
        json!({
            "schema_version": 1,
            "continuity_refs": refs
                .iter()
                .map(|machine_ref| json!({"ref": machine_ref}))
                .collect::<Vec<_>>(),
        }),
    ));
    journal
}

#[test]
fn compacted_machine_refs_accept_exact_namespaces() {
    let journal = journal_with_continuity_refs(&[
        "fact:build_green",
        "artifact:README.md",
        "owner:release_team",
    ]);

    assert!(local_compacted_machine_ref_answer_verifier_gap(
        &journal,
        "fact:build_green artifact:README.md owner:release_team",
    )
    .is_none());
}

#[test]
fn compacted_machine_refs_reject_selected_bare_values() {
    let journal = journal_with_continuity_refs(&[
        "fact:build_green",
        "artifact:README.md",
        "owner:release_team",
    ]);

    let gap = local_compacted_machine_ref_answer_verifier_gap(
        &journal,
        "facts: build_green\nartifact: README.md\nowner: release_team",
    )
    .expect("selected compacted refs must preserve namespaces");

    assert_eq!(
        gap.answer_incomplete_reason,
        "compacted_machine_reference_namespace_omitted"
    );
    assert_eq!(gap.missing_evidence_fields.len(), 3);
    assert!(gap.retry_instruction.contains("\"fact:build_green\""));
    assert!(gap.retry_instruction.contains("\"artifact:README.md\""));
    assert!(gap.retry_instruction.contains("\"owner:release_team\""));
}

#[test]
fn compacted_machine_refs_ignore_single_incidental_bare_value() {
    let journal = journal_with_continuity_refs(&["artifact:README.md", "fact:build_green"]);

    assert!(local_compacted_machine_ref_answer_verifier_gap(
        &journal,
        "The README.md file is available.",
    )
    .is_none());
}

#[test]
fn compacted_machine_refs_do_not_require_unselected_history() {
    let journal = journal_with_continuity_refs(&[
        "fact:build_green",
        "artifact:README.md",
        "owner:release_team",
    ]);

    assert!(local_compacted_machine_ref_answer_verifier_gap(
        &journal,
        "The current risk remains under review.",
    )
    .is_none());
}

#[test]
fn compacted_machine_refs_respect_machine_token_boundaries() {
    let journal = journal_with_continuity_refs(&["fact:build_green", "fact:canary_ready"]);

    assert!(local_compacted_machine_ref_answer_verifier_gap(
        &journal,
        "not_build_green_backup and canary_ready_later",
    )
    .is_none());
}

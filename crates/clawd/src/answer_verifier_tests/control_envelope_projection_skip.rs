use super::*;

#[test]
fn should_verify_answer_skips_grounded_agent_loop_control_envelope() {
    let mut route = route_with_mode();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;

    let envelope = json!({
        "output_format": "machine_json",
        "owner_layer": "agent_loop_control",
        "required_machine_fields": [
            "decision_envelope.control_intent",
            "decision_envelope.capability_ref"
        ],
        "decision_envelope": {
            "control_intent": "act",
            "capability_ref": "fs_basic"
        },
        "output_contract": null
    })
    .to_string();
    let answer = format!("document_heading\n{envelope}");
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-control-envelope-skip",
        "ask",
        "control envelope",
    );
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });

    assert!(!should_verify_answer(&route, &journal, &answer));

    let ungrounded = crate::task_journal::TaskJournal::for_task(
        "task-control-envelope-needs-verifier",
        "ask",
        "control envelope",
    );
    assert!(should_verify_answer(&route, &ungrounded, &answer));
}

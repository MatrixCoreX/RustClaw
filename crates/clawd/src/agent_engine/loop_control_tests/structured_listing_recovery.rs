use super::*;

fn verifier_gap_journal() -> crate::task_journal::TaskJournal {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string()],
        answer_incomplete_reason: "answer omitted observed listing names".to_string(),
        should_retry: true,
        retry_instruction: "answer from the full observed names_by_kind arrays".to_string(),
        confidence: 0.94,
    });
    journal
}

fn push_inventory_trace(journal: &mut crate::task_journal::TaskJournal, output: &str) {
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(output.to_string()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
}

fn push_inventory_step_result(journal: &mut crate::task_journal::TaskJournal, output: &str) {
    journal.push_step_result(&StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
}

#[test]
fn structured_listing_recovery_projects_complete_names_by_kind_for_workspace_summary() {
    let mut journal = verifier_gap_journal();
    push_inventory_trace(
        &mut journal,
        r#"{"action":"inventory_dir","counts":{"dirs":2,"files":3,"total":5},"names_by_kind":{"dirs":["crates","prompts"],"files":["AGENTS.md","Cargo.toml","README.md"],"other":[]},"path":"."}"#,
    );
    let mut reply =
        AskReply::non_llm("crates and README.md".to_string()).with_task_journal(journal);
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;

    assert!(try_recover_structured_listing_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("dirs.count=2"));
    assert!(reply.text.contains("- crates"));
    assert!(reply.text.contains("- prompts"));
    assert!(reply.text.contains("files.count=3"));
    assert!(reply.text.contains("- AGENTS.md"));
    assert!(reply.text.contains("- Cargo.toml"));
    assert!(reply.text.contains("- README.md"));
    assert!(reply.task_journal.as_ref().is_some_and(|journal| {
        journal.final_status == Some(crate::task_journal::TaskJournalFinalStatus::Success)
            && journal.answer_verifier_summary.is_none()
    }));
}

#[test]
fn structured_listing_recovery_uses_planner_listing_evidence_without_route_semantic_marker() {
    let mut journal = verifier_gap_journal();
    push_inventory_trace(
        &mut journal,
        r#"{"action":"inventory_dir","counts":{"dirs":1,"files":4,"total":5},"names_by_kind":{"dirs":["crates"],"files":["AGENTS.md","Cargo.toml","README.md","rustclaw.service"],"other":[]},"path":"/repo","resolved_path":"/repo"}"#,
    );
    let mut reply = AskReply::non_llm("AGENTS.md, Cargo.toml, README.md".to_string())
        .with_task_journal(journal);
    let route = route_result(OutputResponseShape::Free);

    assert!(try_recover_structured_listing_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("dirs.count=1"));
    assert!(reply.text.contains("- crates"));
    assert!(reply.text.contains("files.count=4"));
    assert!(reply.text.contains("- AGENTS.md"));
    assert!(reply.text.contains("- Cargo.toml"));
    assert!(reply.text.contains("- README.md"));
    assert!(reply.text.contains("- rustclaw.service"));
}

#[test]
fn structured_listing_recovery_accepts_directory_lookup_legacy_delivery_flags_from_compact_journal()
{
    let mut journal = verifier_gap_journal();
    push_inventory_step_result(
        &mut journal,
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":2,"files":4,"total":6},"entries":[{"hidden":false,"kind":"dir","modified_ts":1,"name":"crates","path":"crates","size_bytes":0},{"hidden":false,"kind":"file","modified_ts":2,"name":"AGENTS.md","path":"AGENTS.md","size_bytes":123}],"names_by_kind":{"dirs":["crates","prompts"],"files":["AGENTS.md","Cargo.toml","README.md","USAGE.md"],"other":[]},"path":"/repo","resolved_path":"/repo","sort_by":"name","include_hidden":true},"text":"large listing payload"}"#,
    );
    let excerpt = journal.step_results[0]
        .output_excerpt
        .as_deref()
        .expect("journal excerpt");
    assert!(excerpt.contains(r#""names_by_kind""#));
    assert!(excerpt.contains("USAGE.md"));
    assert!(excerpt.contains("modified_ts"));

    let mut route = route_result(OutputResponseShape::Free);
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::DirectoryLookup;
    let mut reply =
        AskReply::non_llm("listing was incomplete".to_string()).with_task_journal(journal);

    assert!(try_recover_structured_listing_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));

    assert!(reply.text.contains("dirs.count=2"));
    assert!(reply.text.contains("files.count=4"));
    assert!(reply.text.contains("- USAGE.md"));
}

#[test]
fn structured_listing_recovery_does_not_override_artifact_file_delivery() {
    let mut journal = verifier_gap_journal();
    push_inventory_trace(
        &mut journal,
        r#"{"action":"inventory_dir","names_by_kind":{"dirs":["crates"],"files":["README.md"],"other":[]},"path":"."}"#,
    );
    let mut route = route_result(OutputResponseShape::FileToken);
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    let mut reply =
        AskReply::non_llm("FILE delivery still missing".to_string()).with_task_journal(journal);

    assert!(!try_recover_structured_listing_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));
    assert_eq!(reply.text, "FILE delivery still missing");
}

#[test]
fn structured_listing_recovery_does_not_override_workspace_summary_with_content_read() {
    let mut journal = verifier_gap_journal();
    push_inventory_trace(
        &mut journal,
        r#"{"action":"inventory_dir","names_by_kind":{"dirs":["crates"],"files":["README.md"],"other":[]},"path":"."}"#,
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_2".to_string(),
            skill: "fs_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                r#"{"action":"read_range","path":"README.md","excerpt":"1|# RustClaw"}"#
                    .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    let mut reply = AskReply::non_llm("RustClaw is a local agent runtime.".to_string())
        .with_task_journal(journal);
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;

    assert!(!try_recover_structured_listing_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));

    assert_eq!(reply.text, "RustClaw is a local agent runtime.");
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_some());
}

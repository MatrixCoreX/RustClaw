use super::apply_requested_machine_kv_summary_to_final_answer;
use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind,
};

#[test]
fn generic_route_preserves_structured_media_dry_run_report_over_short_machine_summary() {
    let prompt = "return dry_run=true provider/model planned_outputs and output_path";
    let route = generic_free_route();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-generic-media-dry-run-kv-preserve",
        "ask",
        prompt,
    );
    journal.record_output_contract(&route.clone());
    let expected_answer = concat!(
        "dry_run=true\n",
        "provider=minimax\n",
        "model=image-01\n",
        "model_kind=dry_run\n",
        "output_path=/home/guagua/rustclaw/document/media_dry_run/image_status_card.png\n",
        "planned_outputs=[{\"path\":\"/home/guagua/rustclaw/document/media_dry_run/image_status_card.png\",\"type\":\"image_file\"}]\n",
        "pending_async_job_contract={\"job_id\":\"provider:image_generate:minimax:dry_run\",\"status\":\"accepted\"}"
    );
    let mut answer_text = expected_answer.to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));
    assert_eq!(answer_text, expected_answer);
    assert_eq!(answer_messages, vec![expected_answer.to_string()]);
}

#[test]
fn generic_route_preserves_requested_token_json_over_machine_summary() {
    let prompt = "最终仅输出 JSON，包含 created_files、test_command、test_status。";
    let route = generic_free_route();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-finalize-required-json", "ask", prompt);
    let expected_answer = r#"{"created_files":["calc_core.py","test_calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed"}"#;
    let mut answer_text = expected_answer.to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));
    assert_eq!(answer_text, expected_answer);
    assert_eq!(answer_messages, vec![expected_answer.to_string()]);
}

#[test]
fn generic_route_restores_journal_requested_token_json_over_machine_summary() {
    let prompt = "最后只输出 JSON，包含 changed_files、test_command、test_status、functions。";
    let route = generic_free_route();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-finalize-restore-requested-json",
        "ask",
        prompt,
    );
    let expected_answer = r#"{"changed_files":["calc_core.py","test_calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed","functions":["add","sub","mul"]}"#;
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_7",
            "synthesize_answer",
            expected_answer,
        ));
    let mut answer_text =
        "changed_files=[\"calc_core.py\",\"test_calc_core.py\"] test_command test_status"
            .to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));
    assert_eq!(answer_text, expected_answer);
    assert_eq!(answer_messages, vec![expected_answer.to_string()]);
}

fn generic_free_route() -> IntentOutputContract {
    IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        selection: crate::OutputSelectionContract::default(),
    }
}

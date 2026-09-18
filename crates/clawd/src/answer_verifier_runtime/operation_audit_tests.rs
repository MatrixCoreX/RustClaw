use super::*;
use crate::task_journal::TaskJournalStepTrace;
use serde_json::json;

fn journal() -> TaskJournal {
    let mut journal = TaskJournal::for_task("audit-fixture", "ask", "fixture request");
    journal
        .step_results
        .push(TaskJournalStepTrace::ok("s1", "fs_basic", "{}"));
    journal
        .step_results
        .push(TaskJournalStepTrace::ok("s2", "respond", "candidate"));
    journal.task_observations.push(json!({
        "schema_version":1, "owner_layer":"task_journal", "observation_kind":"checkpoint_step_provenance",
        "step_id":"s1", "requested_action_type":"call_capability",
        "requested_capability":"fixture.inspect", "resolved_capability":"fixture.inspect",
        "resolved_tool_or_skill":"fs_basic", "dispatch_executed":true,
    }));
    journal
}

fn model(mut checks: serde_json::Value) -> ModelVerifierOut {
    for check in checks.as_array_mut().unwrap() {
        check
            .as_object_mut()
            .unwrap()
            .entry("required_dispatches")
            .or_insert_with(
                || json!([{"action_type":"call_capability","action_ref":"fixture.inspect"}]),
            );
    }
    serde_json::from_value(json!({
        "operation_checks":checks,"pass":true,"missing_evidence_fields":[],
        "answer_incomplete_reason":"","should_retry":false,"retry_instruction":"","confidence":0.9,
    }))
    .unwrap()
}

#[test]
fn different_successful_method_cannot_substitute_for_required_dispatch() {
    let check = json!({"requested_operation":"fixture required method", "evidence_step_ids":["s1"],
        "required_dispatches":[{"action_type":"call_capability","action_ref":"fixture.read"}],
        "method_observed":true,"result_observed":true});
    let verdict = validate_operation_audit(model(json!([check])), &journal());
    assert!(!verdict.pass);
    assert_eq!(verdict.missing_evidence_fields, vec!["verification_audit"]);
}

#[test]
fn dispatch_matches_verified_alias_or_canonical_but_not_a_different_action() {
    let operation = json!({"requested_action_type":"call_capability", "requested_capability":"fixture.alias",
        "resolved_capability":"fixture.canonical", "executed_skill":"fixture"});
    for name in ["fixture.alias", "fixture.canonical"] {
        assert!(RequiredDispatch {
            action_type: "call_capability".into(),
            action_ref: name.into()
        }
        .matches(&operation));
    }
    assert!(!RequiredDispatch {
        action_type: "call_skill".into(),
        action_ref: "fixture".into()
    }
    .matches(&operation));
    assert!(!RequiredDispatch {
        action_type: "call_capability".into(),
        action_ref: "fixture.other".into()
    }
    .matches(&operation));
}

#[test]
fn outcome_only_check_does_not_invent_a_dispatch_requirement() {
    let check = json!({"requested_operation":"fixture observed outcome", "evidence_step_ids":["s1"],
        "required_dispatches":[],"method_observed":false,"result_observed":true});
    assert!(validate_operation_audit(model(json!([check.clone()])), &journal()).pass);
    let mut missing = check;
    missing["evidence_step_ids"] = json!([]);
    assert!(validate_operation_audit(model(json!([missing])), &journal()).pass);
}

#[test]
fn empty_presentation_check_does_not_invalidate_successful_dispatches() {
    let checks = json!([
        {"requested_operation":"fetch news", "evidence_step_ids":["s1"],
         "required_dispatches":[{"action_type":"call_capability","action_ref":"fixture.inspect"}],
         "method_observed":true,"result_observed":true},
        {"requested_operation":"summarize fetched news", "evidence_step_ids":[],
         "required_dispatches":[],"method_observed":false,"result_observed":true}
    ]);
    assert!(validate_operation_audit(model(checks), &journal()).pass);
}

#[test]
fn outcome_only_output_row_does_not_invalidate_completed_transcribe() {
    let mut journal = journal();
    journal
        .step_results
        .push(TaskJournalStepTrace::ok("s3", "audio_transcribe", "{}"));
    journal.task_observations.push(json!({
        "schema_version":1, "owner_layer":"task_journal", "observation_kind":"checkpoint_step_provenance",
        "step_id":"s3", "requested_action_type":"call_capability",
        "requested_capability":"audio.transcribe", "resolved_capability":"audio.transcribe",
        "resolved_tool_or_skill":"audio_transcribe", "dispatch_executed":true,
    }));
    let checks = json!([
        {"requested_operation":"download video", "evidence_step_ids":["s1"],
         "required_dispatches":[{"action_type":"call_capability","action_ref":"fixture.inspect"}],
         "method_observed":true,"result_observed":true},
        {"requested_operation":"transcribe audio", "evidence_step_ids":["s3"],
         "required_dispatches":[{"action_type":"call_capability","action_ref":"audio.transcribe"}],
         "method_observed":true,"result_observed":true},
        {"requested_operation":"return transcript text", "evidence_step_ids":["s3"],
         "required_dispatches":[],"method_observed":true,"result_observed":true}
    ]);
    assert!(validate_operation_audit(model(checks), &journal).pass);
}

fn media_download_journal(steps: &[(&str, &str, &str)]) -> TaskJournal {
    let mut journal = TaskJournal::for_task(
        "media-download-audit",
        "ask",
        "download media and convert requested text",
    );
    for (step_id, capability, skill) in steps {
        journal
            .step_results
            .push(TaskJournalStepTrace::ok(*step_id, *skill, "{}"));
        journal.task_observations.push(json!({
            "schema_version":1,
            "owner_layer":"task_journal",
            "observation_kind":"checkpoint_step_provenance",
            "step_id":*step_id,
            "requested_action_type":"call_capability",
            "requested_capability":*capability,
            "requested_action_ref":*capability,
            "resolved_capability":*capability,
            "resolved_tool_or_skill":*skill,
            "dispatch_executed":true,
        }));
    }
    journal
}

#[test]
fn media_download_delivery_rows_do_not_invalidate_completed_actions() {
    let download_article =
        media_download_journal(&[("step_3", "media_download.download", "media_download")]);
    let article_checks = json!([
        {"requested_operation":"Fetch/download the Douyin image-text post",
         "evidence_step_ids":["step_3"],
         "required_dispatches":[{"action_type":"call_capability","action_ref":"media_download.download"}],
         "method_observed":true,"result_observed":true},
        {"requested_operation":"Deliver the platform caption text to the user",
         "evidence_step_ids":["step_3"],
         "required_dispatches":[],"method_observed":true,"result_observed":true}
    ]);
    assert!(validate_operation_audit(model(article_checks), &download_article).pass);

    let image_text = media_download_journal(&[
        ("step_2", "media_download.download", "media_download"),
        ("step_3", "image_vision.extract_text", "image_vision"),
    ]);
    let image_checks = json!([
        {"requested_operation":"Download the Douyin image-text post",
         "evidence_step_ids":["step_2"],
         "required_dispatches":[{"action_type":"call_capability","action_ref":"media_download.download"}],
         "method_observed":true,"result_observed":true},
        {"requested_operation":"Convert image to text",
         "evidence_step_ids":["step_3"],
         "required_dispatches":[{"action_type":"call_capability","action_ref":"image_vision.extract_text"}],
         "method_observed":true,"result_observed":true},
        {"requested_operation":"Deliver recognized image text",
         "evidence_step_ids":["step_3"],
         "required_dispatches":[],"method_observed":true,"result_observed":true}
    ]);
    assert!(validate_operation_audit(model(image_checks), &image_text).pass);

    let local_ocr = media_download_journal(&[("step_2", "media_download.ocr", "media_download")]);
    let ocr_checks = json!([
        {"requested_operation":"Run local image OCR",
         "evidence_step_ids":["step_2"],
         "required_dispatches":[{"action_type":"call_capability","action_ref":"media_download.ocr"}],
         "method_observed":true,"result_observed":true},
        {"requested_operation":"Deliver OCR text to the user",
         "evidence_step_ids":["step_2"],
         "required_dispatches":[],"method_observed":true,"result_observed":true}
    ]);
    assert!(validate_operation_audit(model(ocr_checks), &local_ocr).pass);

    let transcribe = media_download_journal(&[
        ("step_2", "media_download.download", "media_download"),
        ("step_4", "audio.transcribe", "audio_transcribe"),
    ]);
    let transcribe_checks = json!([
        {"requested_operation":"Download the Douyin video",
         "evidence_step_ids":["step_2"],
         "required_dispatches":[{"action_type":"call_capability","action_ref":"media_download.download"}],
         "method_observed":true,"result_observed":true},
        {"requested_operation":"Extract audio from the downloaded video",
         "evidence_step_ids":["step_2"],
         "required_dispatches":[{"action_type":"call_capability","action_ref":"media_download.download"}],
         "method_observed":true,"result_observed":true},
        {"requested_operation":"Transcribe the audio to text",
         "evidence_step_ids":["step_4"],
         "required_dispatches":[{"action_type":"call_capability","action_ref":"audio.transcribe"}],
         "method_observed":true,"result_observed":true},
        {"requested_operation":"Deliver the transcription result to the user",
         "evidence_step_ids":["step_4"],
         "required_dispatches":[],"method_observed":true,"result_observed":true}
    ]);
    assert!(validate_operation_audit(model(transcribe_checks), &transcribe).pass);
}

#[test]
fn empty_presentation_check_still_rejects_method_or_blocked_claims() {
    for check in [
        json!({"requested_operation":"summarize fetched news","evidence_step_ids":[],
            "required_dispatches":[],"method_observed":true,"result_observed":true}),
        json!({"requested_operation":"summarize fetched news","evidence_step_ids":[],
            "required_dispatches":[],"method_observed":false,"result_observed":false,"blocked":true}),
        json!({"requested_operation":"summarize fetched news","evidence_step_ids":[],
            "required_dispatches":[],"method_observed":false,"result_observed":true,"applicable":false}),
    ] {
        let verdict = validate_operation_audit(model(json!([check])), &journal());
        assert_eq!(verdict.missing_evidence_fields, vec!["verification_audit"]);
        assert!(!verdict.pass);
    }
}

#[test]
fn failed_required_dispatch_cannot_borrow_an_unrelated_success() {
    let mut journal = journal();
    journal
        .step_results
        .push(TaskJournalStepTrace::ok("s3", "fs_basic", "{}"));
    journal.step_results[2].status = crate::executor::StepExecutionStatus::Error;
    journal.task_observations.push(json!({
        "schema_version":1, "owner_layer":"task_journal", "observation_kind":"checkpoint_step_provenance",
        "step_id":"s3", "requested_action_type":"call_capability",
        "requested_capability":"fixture.read", "resolved_capability":"fixture.read",
        "resolved_tool_or_skill":"fs_basic", "dispatch_executed":true,
    }));
    let mut check = json!({"requested_operation":"fixture required method", "evidence_step_ids":["s1","s3"],
        "required_dispatches":[{"action_type":"call_capability","action_ref":"fixture.read"}],
        "method_observed":true,"result_observed":true});
    assert!(!validate_operation_audit(model(json!([check.clone()])), &journal).pass);
    check["method_observed"] = json!(false);
    check["result_observed"] = json!(false);
    check["blocked"] = json!(true);
    assert!(validate_operation_audit(model(json!([check])), &journal).pass);
}

#[test]
fn missing_method_cannot_be_overridden_by_a_known_result_or_pass_boolean() {
    let check = json!({"requested_operation":"fixture method","evidence_step_ids":["s1"],
                      "method_observed":false,"result_observed":true});
    let verdict = validate_operation_audit(model(json!([check])), &journal());
    assert!(!verdict.pass);
    assert!(verdict.should_retry);
    assert_eq!(verdict.missing_evidence_fields, vec!["requested_result"]);
}

#[test]
fn successful_observations_and_empty_non_action_audit_preserve_verdict() {
    let check = json!({"requested_operation":"fixture method","evidence_step_ids":["s1"],
                      "method_observed":true,"result_observed":true});
    assert!(validate_operation_audit(model(json!([check])), &journal()).pass);
    assert!(validate_operation_audit(model(json!([])), &journal()).pass);
}

#[test]
fn completed_resumed_capability_result_satisfies_required_dispatch_audit() {
    let mut journal = TaskJournal::for_task(
        "audit-resumed-capability",
        "ask",
        "transcribe downloaded audio",
    );
    journal
        .step_results
        .push(TaskJournalStepTrace::ok("step_4", "audio_transcribe", "{}"));
    journal.task_observations.push(json!({
        "observation_kind": "capability_resolution",
        "outcome": "resolved",
        "requested_capability": "audio.transcribe",
        "resolved_capability": "audio.transcribe",
        "resolved_tool_or_skill": "skill:audio_transcribe",
        "round_no": 4,
        "global_step": 4,
        "step_in_round": 1,
    }));
    let mut result = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "audio.transcribe",
        Some("transcribe".to_string()),
        json!({"status": "ok"}),
    );
    result
        .evidence
        .push(claw_core::capability_result::EvidenceRef {
            id: "step_4".to_string(),
            source: "audio.transcribe".to_string(),
            locator: None,
            digest: None,
            metadata: json!({}),
        });
    journal.capability_results.push(result);
    let check = json!({
        "requested_operation":"transcribe downloaded audio",
        "evidence_step_ids":["step_4"],
        "required_dispatches":[
            {"action_type":"call_capability","action_ref":"audio.transcribe"}
        ],
        "method_observed":true,
        "result_observed":true
    });

    assert!(validate_operation_audit(model(json!([check])), &journal).pass);
}

#[test]
fn conditional_branch_requires_successful_condition_and_preservation_evidence() {
    let check = json!({"requested_operation":"conditional operation","evidence_step_ids":["s1"],
                      "applicable":false,"method_observed":false,"result_observed":true});
    assert!(validate_operation_audit(model(json!([check.clone()])), &journal()).pass);
    let mut failed = journal();
    failed.step_results[0].status = crate::executor::StepExecutionStatus::Error;
    assert!(!validate_operation_audit(model(json!([check.clone()])), &failed).pass);
    for ids in [json!([]), json!(["unknown"]), json!(["s2"])] {
        let mut invalid = check.clone();
        invalid["evidence_step_ids"] = ids;
        assert!(!validate_operation_audit(model(json!([invalid])), &journal()).pass);
    }
    for (field, value) in [
        ("method_observed", true),
        ("result_observed", false),
        ("blocked", true),
        ("applicable", true),
    ] {
        let mut invalid = check.clone();
        invalid[field] = json!(value);
        assert!(!validate_operation_audit(model(json!([invalid])), &journal()).pass);
    }
    let mut rejected = model(json!([check]));
    rejected.verdict.pass = false;
    rejected.verdict.missing_evidence_fields = vec!["unsupported_claims".to_string()];
    assert!(!validate_operation_audit(rejected, &journal()).pass);
}

#[test]
fn fabricated_assistant_and_missing_evidence_refs_do_not_verify_execution() {
    for ids in [json!(["unknown"]), json!(["s2"]), json!([])] {
        let check = json!({"requested_operation":"fixture method","evidence_step_ids":ids,
                          "method_observed":true,"result_observed":true});
        let verdict = validate_operation_audit(model(json!([check])), &journal());
        assert_eq!(verdict.missing_evidence_fields, vec!["verification_audit"]);
        assert!(!verdict.pass);
    }
}

#[test]
fn blocked_closeout_requires_actual_failure_evidence() {
    let check = json!({"requested_operation":"fixture method","evidence_step_ids":["s1"],
                      "method_observed":false,"result_observed":false,"blocked":true});
    assert!(!validate_operation_audit(model(json!([check.clone()])), &journal()).pass);
    let mut journal = journal();
    journal.step_results[0].status = crate::executor::StepExecutionStatus::Error;
    assert!(validate_operation_audit(model(json!([check])), &journal).pass);
}

#[test]
fn schema_requires_audit_and_does_not_coerce_boolean_or_allow_extra_fields() {
    let base = json!({"operation_checks":[],"pass":true,"missing_evidence_fields":[],
                     "answer_incomplete_reason":"","should_retry":false,"retry_instruction":"","confidence":0.9});
    let schema = crate::prompt_utils::PromptSchemaId::AnswerVerifier;
    assert!(
        crate::prompt_utils::validate_against_schema::<ModelVerifierOut>(&base.to_string(), schema)
            .is_ok()
    );
    let mut missing = base.clone();
    missing.as_object_mut().unwrap().remove("operation_checks");
    assert!(
        crate::prompt_utils::validate_against_schema::<ModelVerifierOut>(
            &missing.to_string(),
            schema
        )
        .is_err()
    );
    for invalid in [
        json!({"requested_operation":"fixture","evidence_step_ids":[],"method_observed":"true","result_observed":true}),
        json!({"requested_operation":"fixture","evidence_step_ids":[],"method_observed":false,"result_observed":true,"extra":true}),
        json!({"requested_operation":"fixture","evidence_step_ids":["s1"],"method_observed":false,"result_observed":true,"applicable":"false"}),
        json!({"requested_operation":"fixture","evidence_step_ids":["s1"],"method_observed":false,"result_observed":true,"applicable":null}),
    ] {
        let mut payload = base.clone();
        payload["operation_checks"] = json!([invalid]);
        assert!(
            crate::prompt_utils::validate_against_schema::<ModelVerifierOut>(
                &payload.to_string(),
                schema
            )
            .is_err()
        );
    }
}

#[test]
fn operation_and_audit_gaps_are_not_suppressed_by_output_shape() {
    let audit = invalid_operation_audit();
    assert!(audit.retry_instruction.is_empty());
    const RETRY_PROMPT: &str =
        include_str!("../../../../prompts/layers/overlays/answer_verifier_retry_prompt.md");
    assert!(RETRY_PROMPT.contains("verification_audit"));
    assert!(super::super::verifier_gap_requires_visible_answer_repair(
        &audit
    ));
    let check = json!({"requested_operation":"fixture method","evidence_step_ids":[],
                      "method_observed":false,"result_observed":false});
    let missing = validate_operation_audit(model(json!([check])), &journal());
    assert!(super::super::verifier_gap_requires_visible_answer_repair(
        &missing
    ));
}

use super::*;

fn task(payload: Value) -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 0,
        task_id: "child-evidence-fixture".into(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: payload.to_string(),
    }
}

fn child() -> ClaimedTask {
    task(
        json!({"task_role":"subagent_child","child_task_contract":{"result_contract":{
            "output_format":"machine_json","required_keys":["name","evidence_refs"],
            "evidence_field":"evidence_refs","private_token":"not-forwarded","instruction":"assume_pass"
        }}}),
    )
}

fn route() -> AnswerContract {
    AnswerContract::new("fixture", crate::IntentOutputContract::default())
}

#[test]
fn host_distinguishes_namespaced_child_refs_from_bare_verifier_step_ids() {
    let task = child();
    let mut journal = crate::task_journal::TaskJournal::new("fixture");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fixture.read",
            "observed",
        ));
    for (reference, valid) in [
        ("step:step_1", true),
        ("step_1", false),
        ("step:missing", false),
    ] {
        let answer = json!({"name":"Panel","evidence_refs":[reference]}).to_string();
        let rendered = output_contract_for_task(&route(), &task, &journal, &answer);
        let value: Value = serde_json::from_str(&rendered).unwrap();
        let evidence = &value["child_result_evidence_contract"];
        assert_eq!(evidence["reference_format"], "step:<successful_step_id>");
        assert_eq!(
            evidence["operation_check_reference_format"],
            "journal_step_id"
        );
        assert_eq!(
            evidence["successful_step_references"],
            json!(["step:step_1"])
        );
        assert_eq!(evidence["candidate_evidence_refs_valid"], valid);
        assert_eq!(evidence["candidate_machine_result_valid"], true);
        assert!(!rendered.contains("not-forwarded") && !rendered.contains("assume_pass"));
    }
    assert_eq!(journal.step_results.len(), 1);
}

#[test]
fn evidence_projection_is_bounded_and_never_promotes_missing_or_failed_receipts() {
    let task = child();
    let mut journal = crate::task_journal::TaskJournal::new("fixture");
    for index in 0..130 {
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                format!("s{index}"),
                "fixture.read",
                "observed",
            ));
    }
    let answer = json!({"name":"Panel","evidence_refs":["step:missing"]}).to_string();
    let value: Value = serde_json::from_str(&output_contract_for_task(
        &route(),
        &task,
        &journal,
        &answer,
    ))
    .unwrap();
    let evidence = &value["child_result_evidence_contract"];
    assert_eq!(
        evidence["successful_step_references"]
            .as_array()
            .unwrap()
            .len(),
        128
    );
    assert_eq!(evidence["reference_list_complete"], false);
    assert_eq!(evidence["candidate_evidence_refs_valid"], false);
    journal.step_results.truncate(1);
    journal.step_results[0].status = crate::executor::StepExecutionStatus::Error;
    let answer = json!({"name":"Panel","evidence_refs":["step:s0"]}).to_string();
    let value: Value = serde_json::from_str(&output_contract_for_task(
        &route(),
        &task,
        &journal,
        &answer,
    ))
    .unwrap();
    assert_eq!(
        value["child_result_evidence_contract"]["successful_step_references"],
        json!([])
    );
    assert_eq!(
        value["child_result_evidence_contract"]["candidate_evidence_refs_valid"],
        false
    );
}

#[test]
fn ordinary_tasks_do_not_acquire_child_contract_metadata() {
    let journal = crate::task_journal::TaskJournal::new("fixture");
    assert_eq!(
        output_contract_for_task(
            &route(),
            &task(json!({"text":"ordinary"})),
            &journal,
            "answer"
        ),
        output_contract_prompt_block(&route())
    );
}

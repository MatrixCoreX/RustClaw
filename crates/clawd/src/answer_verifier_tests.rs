use serde_json::json;

use super::{health_check_value_from_output, observed_find_ext_results};
use super::{
    local_compound_listing_answer_verifier_gap, local_missing_evidence_verifier_gap,
    local_missing_evidence_verifier_gap_for_answer, observed_scalar_values_from_evidence_map,
    observed_scalar_values_from_evidence_map_for_route,
    observed_single_path_values_from_evidence_map, observed_strict_list_items_from_evidence_map,
    observed_strict_list_items_from_evidence_map_for_route, observed_table_cells_from_evidence_map,
    post_write_content_evidence_missing_before_verifier,
    recent_structured_scalar_values_from_journal, route_contract_marker_is_scalar_path_only,
    should_verify_answer, strict_list_route_allows_observed_subset,
    structural_satisfaction_can_skip_verifier, structurally_satisfies_answer_contract,
};

fn route_with_mode() -> crate::answer_verifier::AnswerContract {
    crate::answer_verifier::AnswerContract::new(
        "test intent",
        crate::IntentOutputContract::default(),
    )
}

#[test]
fn verifier_contract_markers_require_planner_semantic_contract() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    assert!(strict_list_route_allows_observed_subset(&route));

    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    assert!(route_contract_marker_is_scalar_path_only(&route));
}

#[test]
fn direct_answer_route_skips_answer_verifier() {
    let route = route_with_mode();
    let journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "hello");
    assert!(!should_verify_answer(&route, &journal, "hi"));
}

#[test]
fn unclassified_freeform_response_skips_answer_verifier() {
    let mut route = route_with_mode();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "direct response request");

    assert!(!should_verify_answer(
        &route,
        &journal,
        "candidate response"
    ));
}

#[test]
fn unclassified_terminal_respond_step_skips_answer_verifier() {
    let mut route = route_with_mode();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "direct response request");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "respond",
            "candidate response",
        ));

    assert!(!should_verify_answer(
        &route,
        &journal,
        "candidate response"
    ));
}

#[test]
fn planner_plain_terminal_answer_only_skips_answer_verifier() {
    let mut route = route_with_mode();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-terminal-answer",
        "ask",
        "explain a runtime contract",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "respond",
            "candidate response",
        ));

    assert!(!should_verify_answer(
        &route,
        &journal,
        "candidate response"
    ));
}

#[test]
fn planner_plain_answer_with_tool_observation_still_uses_answer_verifier() {
    let mut route = route_with_mode();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-tool-answer", "ask", "summarize result");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"path":"README.md","exists":true}"#,
        ));

    assert!(should_verify_answer(&route, &journal, "README.md exists"));
}

#[test]
fn grounded_machine_kv_projection_skips_answer_verifier() {
    let mut route = route_with_mode();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "AGENTS.md".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-machine-kv-projection-skip",
        "ask",
        "Only keep no_hardmatch_guard=check_no_nl_hardmatch.py.",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"grep_text","matches":[{"line":246,"path":"AGENTS.md","text":"run `python3 scripts/check_no_nl_hardmatch.py` after boundary changes"}],"query":"check_no_nl_hardmatch.py","results":["AGENTS.md"]},"text":"AGENTS.md"}"#,
        ));

    assert!(!should_verify_answer(
        &route,
        &journal,
        "no_hardmatch_guard=check_no_nl_hardmatch.py"
    ));
    assert!(should_verify_answer(&route, &journal, "AGENTS.md"));
}

#[test]
fn unclassified_chat_with_backend_reference_does_not_require_model_answer_verifier() {
    let mut route = route_with_mode();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "identity response request");

    assert!(!should_verify_answer(
        &route,
        &journal,
        "decision: decision:minimax_primary"
    ));
}

#[test]
fn classified_contract_marker_still_uses_answer_verifier() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "summarize");

    assert!(should_verify_answer(&route, &journal, "summary"));
}

#[test]
fn output_contract_marker_verification_does_not_depend_on_route_trace() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "summarize");

    assert!(should_verify_answer(&route, &journal, "summary"));
}

#[test]
fn clarify_final_status_skips_answer_verifier() {
    let mut route = route_with_mode();
    route.output_contract.requires_content_evidence = true;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "hello");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);

    assert!(!should_verify_answer(
        &route,
        &journal,
        "please provide the missing path"
    ));
}

#[test]
fn local_missing_evidence_gap_reports_required_fields() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-local-gap", "ask", "exists?");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"path": "/tmp/a.txt", "exists": true}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let gap = local_missing_evidence_verifier_gap(&route, &journal).expect("missing kind evidence");
    assert_eq!(gap.missing_evidence_fields, vec!["kind"]);
    assert!(gap.should_retry);
    assert!(gap.high_confidence_gap());
}

#[test]
fn local_missing_evidence_gap_uses_contract_not_route_trace() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-local-gap-trace", "ask", "exists?");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"path": "/tmp/a.txt", "exists": true}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let gap = local_missing_evidence_verifier_gap(&route, &journal)
        .expect("evidence contract should not depend on legacy route trace");
    assert_eq!(gap.missing_evidence_fields, vec!["kind"]);
    assert!(gap.should_retry);
}

#[test]
fn local_missing_evidence_gap_skips_non_path_status_observation() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-docker-status-gap", "ask", "docker");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "docker_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "version",
                    "available": false,
                    "command_succeeded": false,
                    "output": "docker unavailable"
                },
                "text": "docker unavailable"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "docker_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "ps",
                    "available": false,
                    "command_succeeded": false,
                    "output": "docker unavailable"
                },
                "text": "docker unavailable"
            })
            .to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });

    assert!(local_missing_evidence_verifier_gap_for_answer(
        &route,
        &journal,
        "Docker is unavailable."
    )
    .is_none());
}

#[test]
fn local_missing_evidence_gap_skips_when_required_fields_are_observed() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-local-gap-ok", "ask", "list names");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"names": ["Cargo.toml"]}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    assert!(local_missing_evidence_verifier_gap(&route, &journal).is_none());
}

#[test]
fn config_guard_machine_payload_skips_missing_evidence_verifier_gap() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigValidation;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let answer = json!({
        "message_key": "clawd.msg.config_edit.guard",
        "reason_code": "config_edit_guard_risk_found",
        "path": "configs/config.toml",
        "risk_count": 2,
        "count": 2,
        "candidates": [
            "tools.allow_sudo=true",
            "tools.allow_path_outside_workspace=true"
        ]
    })
    .to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-config-guard-json", "ask", "config");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "guard_config",
                "path": "configs/config.toml",
                "risk_count": 2,
                "candidates": [
                    "tools.allow_sudo=true",
                    "tools.allow_path_outside_workspace=true"
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });

    assert!(local_missing_evidence_verifier_gap_for_answer(&route, &journal, &answer).is_none());
    assert!(structural_satisfaction_can_skip_verifier(
        &route, &journal, &answer
    ));
}

#[test]
fn local_missing_evidence_gap_does_not_block_on_negative_evidence_only() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-local-negative-evidence-only",
        "ask",
        "current workspace path",
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "path_batch_facts",
                "count": 1,
                "facts": [{
                    "path": ".",
                    "exists": false,
                    "kind": "missing"
                }],
                "include_missing": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = crate::task_journal::evidence_coverage_for_output_contract(
        &route.output_contract,
        &journal,
    );
    assert_eq!(
        coverage.missing_evidence,
        vec!["negative_evidence(exists_false)"]
    );
    assert!(local_missing_evidence_verifier_gap(&route, &journal).is_none());
}

#[test]
fn local_missing_evidence_gap_skips_structured_not_found_terminal_finalizer() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "definitely_missing_dir_rustclaw_xyz/".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-local-gap-not-found", "ask", "list");
    journal.record_output_contract(&route.output_contract);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(crate::skills::structured_skill_error_from_parts(
            "fs_basic",
            "not_found",
            "target not found",
            Some("linux"),
            Some(json!({
                "operation": "list_dir",
                "path": "definitely_missing_dir_rustclaw_xyz/"
            })),
        )),
        started_at: 1,
        finished_at: 2,
    });
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

    assert!(local_missing_evidence_verifier_gap(&route, &journal).is_none());
}

#[test]
fn structural_satisfaction_skips_latest_not_found_answer_with_same_path() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/tmp/rustclaw-missing.md".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-local-gap-not-found-answer",
        "ask",
        "read missing path",
    );
    journal.record_output_contract(&route.output_contract);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some("__RC_READ_FILE_NOT_FOUND__:/tmp/rustclaw-missing.md".to_string()),
        started_at: 1,
        finished_at: 2,
    });

    assert!(structural_satisfaction_can_skip_verifier(
        &route,
        &journal,
        "/tmp/rustclaw-missing.md"
    ));
}

#[test]
fn structural_satisfaction_skips_successful_stat_missing_answer_with_same_path() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/tmp/rustclaw-missing.md | README.md".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-local-gap-stat-missing-answer",
        "ask",
        "stat missing path and summarize readme",
    );
    journal.record_output_contract(&route.output_contract);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "path_batch_facts",
                "count": 1,
                "facts": [
                    {
                        "exists": false,
                        "kind": "missing",
                        "path": "/tmp/rustclaw-missing.md"
                    }
                ],
                "include_missing": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    assert!(structural_satisfaction_can_skip_verifier(
        &route,
        &journal,
        "exists=false path=/tmp/rustclaw-missing.md kind=missing"
    ));
}

#[test]
fn structural_satisfaction_keeps_verifier_when_not_found_answer_omits_path() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/tmp/rustclaw-missing.md".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-local-gap-not-found-answer-no-path",
        "ask",
        "read missing path",
    );
    journal.record_output_contract(&route.output_contract);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some("__RC_READ_FILE_NOT_FOUND__:/tmp/rustclaw-missing.md".to_string()),
        started_at: 1,
        finished_at: 2,
    });

    assert!(!structural_satisfaction_can_skip_verifier(
        &route, &journal, "missing"
    ));
}

#[test]
fn should_verify_answer_skips_permission_denied_terminal_finalizer() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/etc/shadow".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-terminal-permission", "ask", "read");
    journal.record_output_contract(&route.output_contract);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(crate::skills::structured_skill_error_from_parts(
            "system_basic",
            "permission_denied",
            "read operation failed",
            Some("linux"),
            Some(json!({
                "operation": "read_file",
                "path": "/etc/shadow"
            })),
        )),
        started_at: 1,
        finished_at: 2,
    });
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
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);

    assert!(!should_verify_answer(
        &route,
        &journal,
        "message_key=content_permission_denied path=/etc/shadow"
    ));
}

#[test]
fn should_verify_answer_skips_grounded_structured_machine_projection() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFilePathReport;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-machine-projection", "ask", "generate");
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

    assert!(should_verify_answer(&route, &journal, "provider=minimax"));
    assert!(!should_verify_answer(
        &route,
        &journal,
        concat!(
            "dry_run=true\n",
            "provider=minimax\n",
            "model=speech-2.8-turbo\n",
            "model_kind=dry_run\n",
            "output_path=/home/guagua/rustclaw/document/media_dry_run/audio_check.mp3\n",
            "planned_outputs=[{\"path\":\"/home/guagua/rustclaw/document/media_dry_run/audio_check.mp3\",\"type\":\"audio_file\"}]",
        )
    ));
    assert!(!should_verify_answer(
        &route,
        &journal,
        r#"{"functions":["add","sub","mul","safe_div"],"error_codes":["division_by_zero"],"test_status":"passed","evidence_files":["calc_core.py","test_calc_core.py"]}"#
    ));
}

#[test]
fn local_missing_evidence_gap_skips_crypto_account_access_terminal_finalizer() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-local-gap-crypto", "ask", "positions");
    journal.record_output_contract(&route.output_contract);
    let marker = r#"__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:{"exchange":"binance","detail":"binance error status=401: {\"code\":-2015,\"msg\":\"Invalid API-key, IP, or permissions for action.\"}"}"#;
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "crypto".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "crypto",
                "error_kind": "unknown",
                "error_text": marker,
                "extra": null
            })
        )),
        started_at: 1,
        finished_at: 2,
    });
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

    assert!(local_missing_evidence_verifier_gap(&route, &journal).is_none());
}

#[test]
fn local_missing_evidence_gap_keeps_gap_for_non_missing_terminal_error() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "maybe_dir/".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-local-gap-error", "ask", "list");
    journal.record_output_contract(&route.output_contract);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(crate::skills::structured_skill_error_from_parts(
            "fs_basic",
            "invalid_args",
            "invalid list arguments",
            Some("linux"),
            Some(json!({
                "operation": "list_dir",
                "path": "maybe_dir/"
            })),
        )),
        started_at: 1,
        finished_at: 2,
    });
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

    let gap = local_missing_evidence_verifier_gap(&route, &journal).expect("gap should remain");

    assert_eq!(gap.missing_evidence_fields, vec!["candidates"]);
}

#[test]
fn local_compound_listing_gap_rejects_answer_that_drops_observed_names() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.request_text = "selector_limit=3; summarize listed content".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-compound-list", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["archive", "release_checklist.md", "service_notes.md"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "excerpt": "1|# Release Checklist\n3|1. Verify configuration loads correctly."
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let gap = local_compound_listing_answer_verifier_gap(
        &route,
        &journal,
        "release_checklist.md is an operator checklist.",
    )
    .expect("compound listing gap");

    assert_eq!(gap.missing_evidence_fields, vec!["candidates"]);
    assert_eq!(
        gap.answer_incomplete_reason,
        "observed_listing_candidates_omitted"
    );
    assert_eq!(
        gap.retry_instruction,
        "retry_policy=use_observed_listing_candidates_and_content_excerpt;repeat_rejected_answer=false"
    );
    assert!(gap.should_retry);
}

#[test]
fn local_compound_listing_gap_accepts_answer_with_observed_names() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = true;
    route.request_text = "selector_limit=3; summarize listed content".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-compound-list-ok", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["archive", "release_checklist.md", "service_notes.md"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "excerpt": "1|# Release Checklist\n3|1. Verify configuration loads correctly."
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    assert!(local_compound_listing_answer_verifier_gap(
        &route,
        &journal,
        "archive, release_checklist.md, and service_notes.md are listed, and release_checklist.md is an operator checklist."
    )
    .is_none());
}

#[test]
fn local_compound_listing_gap_applies_to_directory_purpose_summary() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.requires_content_evidence = true;
    route.request_text = "selector_limit=3; summarize purpose".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-dir-purpose-gap", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["alpha.md", "beta.json", "notes.txt"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "excerpt": "1|# Alpha\n2|setup notes"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let gap = local_compound_listing_answer_verifier_gap(
        &route,
        &journal,
        "alpha.md and notes.txt look documentation-oriented.",
    )
    .expect("directory purpose summary should require observed listing items");

    assert_eq!(gap.missing_evidence_fields, vec!["candidates"]);
    assert_eq!(
        gap.answer_incomplete_reason,
        "observed_listing_candidates_omitted"
    );
    assert_eq!(
        gap.retry_instruction,
        "retry_policy=use_observed_listing_candidates_and_content_excerpt;repeat_rejected_answer=false"
    );
}

#[test]
fn directory_purpose_summary_structurally_satisfies_listing_content_answer() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.requires_content_evidence = true;
    route.request_text = "selector_limit=3; summarize purpose".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-dir-purpose-ok", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["alpha.md", "beta.json", "notes.txt"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "path": "document/alpha.md",
                "excerpt": "1|# Alpha\n2|setup notes"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "alpha.md, beta.json, and notes.txt are listed; based on the observed excerpt, this looks documentation-oriented."
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "alpha.md and notes.txt are listed; based on the observed excerpt, this looks documentation-oriented."
    ));
}

#[test]
fn compound_listing_gap_respects_requested_numeric_limit() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.requires_content_evidence = true;
    route.request_text = "selector_limit=2; summarize purpose".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-dir-purpose-limit", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["alpha.md", "beta.json", "notes.txt"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "excerpt": "1|# Alpha\n2|setup notes"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let answer =
        "alpha.md and beta.json are listed; based on the observed excerpt, this looks documented.";

    assert!(local_compound_listing_answer_verifier_gap(&route, &journal, answer).is_none());
    assert!(structurally_satisfies_answer_contract(
        &route, &journal, answer
    ));
}

#[test]
fn directory_purpose_summary_line_count_number_is_not_listing_limit() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.requires_content_evidence = true;
    route.request_text = "summarize directory and keep answer within 5 lines".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-dir-purpose-line-count", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": [
                    "answer_verifier.schema.json",
                    "contract_repair_judge.schema.json",
                    "delivery_text_classifier.schema.json",
                    "intent_normalizer.schema.json"
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "path": "prompts/schemas/intent_normalizer.schema.json",
                "excerpt": "1|title IntentNormalizerOut"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let answer = "intent_normalizer.schema.json; content_excerpt=IntentNormalizerOut";

    assert!(local_compound_listing_answer_verifier_gap(&route, &journal, answer).is_none());
    assert!(structurally_satisfies_answer_contract(
        &route, &journal, answer
    ));
}

#[test]
fn content_excerpt_summary_line_count_number_is_not_listing_limit() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = true;
    route.request_text = "summarize observed content and keep answer within 5 lines".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-content-summary-line-count",
        "ask",
        "prompt",
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": [
                    "answer_verifier.schema.json",
                    "contract_repair_judge.schema.json",
                    "delivery_text_classifier.schema.json",
                    "intent_normalizer.schema.json"
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "path": "prompts/schemas/intent_normalizer.schema.json",
                "excerpt": "1|title IntentNormalizerOut"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let answer = "intent_normalizer.schema.json; content_excerpt=IntentNormalizerOut";

    assert!(local_compound_listing_answer_verifier_gap(&route, &journal, answer).is_none());
}

#[test]
fn unbounded_directory_purpose_summary_does_not_require_all_listing_names() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.requires_content_evidence = true;
    route.request_text = "summarize workspace organization from top-level directories".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-dir-purpose-unbounded", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["UI", "configs", "crates", "scripts", "target"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "path": "README.md",
                "excerpt": "1|# RustClaw\n2|local Rust agent runtime"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let answer = "RustClaw is organized around a Rust core in crates, with UI, configs, and scripts around it.";

    assert!(local_compound_listing_answer_verifier_gap(&route, &journal, answer).is_none());
    assert!(structurally_satisfies_answer_contract(
        &route, &journal, answer
    ));
}

#[test]
fn workspace_project_summary_inventory_names_do_not_skip_model_language_verifier() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.requires_content_evidence = true;
    route.request_text =
        "summarize current workspace organization from top-level directories".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-workspace-summary-inventory",
        "ask",
        "prompt",
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "list_dir",
                "entries": [
                    {"name": "UI", "kind": "dir"},
                    {"name": "configs", "kind": "dir"},
                    {"name": "crates", "kind": "dir"},
                    {"name": "scripts", "kind": "dir"}
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "path": "README.md",
                "excerpt": "1|# RustClaw\n2|local agent runtime"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let answer = "RustClaw keeps the runtime under crates, the browser console in UI, and helper automation in scripts.";

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, answer
    ));
    assert!(!structural_satisfaction_can_skip_verifier(
        &route, &journal, answer
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "This workspace has a clear project layout."
    ));
}

#[test]
fn structural_satisfaction_does_not_skip_missing_contract_evidence() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-structural-gap", "ask", "exists?");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "path_batch_facts",
                "facts": [{
                    "path": "/tmp/a.txt",
                    "exists": true
                }]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "/tmp/a.txt exists"
    ));
    let gap = local_missing_evidence_verifier_gap(&route, &journal).expect("missing kind evidence");
    assert_eq!(gap.missing_evidence_fields, vec!["kind"]);
    assert!(!structural_satisfaction_can_skip_verifier(
        &route,
        &journal,
        "/tmp/a.txt exists"
    ));
}

#[test]
fn structural_satisfaction_skips_verifier_for_health_check_diagnostic_fields() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.requires_content_evidence = true;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-health-check-structural",
        "ask",
        "health check",
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "health_check".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "clawd_process_count": 1,
                    "clawd_health_port_open": true,
                    "telegramd_process_count": 0,
                    "clawd_log": {"exists": true, "keyword_error_count": 43},
                    "telegramd_log": {"exists": true, "keyword_error_count": 1},
                    "system_health": {
                        "os_family": "linux",
                        "service_manager": "systemd",
                        "cpu_count": 8,
                        "load_avg_1m": 7.65,
                        "load_avg_5m": 6.1,
                        "load_avg_15m": 3.37,
                        "memory_available_bytes": 8403259392u64,
                        "memory_total_bytes": 15937286144u64,
                        "disk_root_available_bytes": 14794739712u64,
                        "disk_root_total_bytes": 156546629632u64,
                        "warnings": ["disk_root_low"]
                    }
                },
                "text": "{}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    let answer = concat!(
        "health_check.summary: clawd.status=running; clawd_process_count=1; ",
        "clawd_health_port_open=true; telegramd_process_count=0; ",
        "clawd_log.exists=true; clawd_log.keyword_error_count=43; ",
        "telegramd_log.exists=true; telegramd_log.keyword_error_count=1; ",
        "system_health.os_family=linux; system_health.service_manager=systemd; ",
        "system_health.cpu_count=8; system_health.load_avg_1m=7.65; ",
        "system_health.load_avg_5m=6.1; system_health.load_avg_15m=3.37; ",
        "system_health.memory_available_bytes=8403259392; ",
        "system_health.memory_total_bytes=15937286144; ",
        "system_health.disk_root_available_bytes=14794739712; ",
        "system_health.disk_root_total_bytes=156546629632; ",
        "system_health.warnings=disk_root_low."
    );

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, answer
    ));
    assert!(structural_satisfaction_can_skip_verifier(
        &route, &journal, answer
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "health_check.summary: clawd.status=running; clawd_process_count=1."
    ));
}

#[test]
fn structural_satisfaction_skips_verifier_for_deterministic_finalizer_summary() {
    let mut route = route_with_mode();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-finalizer-summary-skip", "ask", "list");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names_by_kind": {
                    "dirs": ["configs"],
                    "files": ["README.md"],
                    "other": []
                },
                "counts": {"dirs": 1, "files": 1, "total": 2}
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
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

    assert!(structural_satisfaction_can_skip_verifier(
        &route,
        &journal,
        "dirs:\n- configs\nfiles:\n- README.md"
    ));
}

#[path = "answer_verifier_tests/delivery_and_scalar_grounding.rs"]
mod delivery_and_scalar_grounding;

#[path = "answer_verifier_tests/matrix_shape_contracts.rs"]
mod matrix_shape_contracts;

#[path = "answer_verifier_tests/matrix_grounding.rs"]
mod matrix_grounding;

#[path = "answer_verifier_tests/service_control_capability_grounding.rs"]
mod service_control_capability_grounding;

#[path = "answer_verifier_tests/scalar_capability_shape.rs"]
mod scalar_capability_shape;

#[path = "answer_verifier_tests/text_protocol_boundary.rs"]
mod text_protocol_boundary;

#[path = "answer_verifier_tests/local_status_evidence.rs"]
mod local_status_evidence;

#[path = "answer_verifier_tests/prompt_schema_and_evidence.rs"]
mod prompt_schema_and_evidence;

#[path = "answer_verifier_tests/strict_json_projection_skip.rs"]
mod strict_json_projection_skip;

#[path = "answer_verifier_tests/control_envelope_projection_skip.rs"]
mod control_envelope_projection_skip;

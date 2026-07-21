use super::super::loop_control_post_write_evidence_guard::{
    enforce_code_mutation_validation_success_guard, enforce_post_write_content_evidence_guard,
};
use super::{
    answer_contract, answer_verifier_retry_summary, commit_answer_verifier_retry_answer, ok_step,
    post_write_content_evidence_recovery_policy,
    prefer_terminal_model_answer_for_verifier_candidate,
    promote_local_code_projection_from_machine_evidence_for_verifier_candidate,
    promote_publishable_strict_json_projection_for_verifier_candidate,
    retry_verifier_accepts_rewritten_answer, route_result,
    suppress_answer_verifier_retry_if_structurally_satisfied, test_policy,
};
use crate::{
    agent_engine::LoopState, executor::StepExecutionStatus, AskReply, OutputDeliveryIntent,
    OutputLocatorKind, OutputResponseShape,
};
use serde_json::json;

fn local_code_context_with_required_fields(
    fields: serde_json::Value,
) -> crate::agent_engine::AgentRunContext {
    crate::agent_engine::AgentRunContext {
        turn_analysis: Some(crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskRequest),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(json!({ "required_machine_fields": fields })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    }
}

#[test]
fn post_write_guard_requires_content_evidence_after_code_write_and_validation() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-post-write-guard", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"},"text":"written 98 bytes to /workspace/calc_core.py"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"},"text":"written 571 bytes to /workspace/test_calc_core.py"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "run_cmd",
            "Ran 3 tests in 0.000s\nOK",
        ));
    let mut reply = AskReply::non_llm(
        r#"{"changed_files":["calc_core.py","test_calc_core.py"],"test_status":"OK"}"#.to_string(),
    )
    .with_task_journal(journal);

    assert!(enforce_post_write_content_evidence_guard(&mut reply));
    let summary = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .expect("post-write verifier summary");
    assert!(!summary.pass);
    assert_eq!(summary.missing_evidence_fields, vec!["content_excerpt"]);
    assert!(summary.should_retry);
}

#[test]
fn code_mutation_validation_failure_creates_retry_gap() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-code-validation-failed", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"},"text":"written 120 bytes"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::new(
            "step_2",
            "run_cmd",
            StepExecutionStatus::Error,
            None,
            Some(
                r#"__RC_SKILL_ERROR__:{"skill":"run_cmd","error_kind":"nonzero_exit","error_text":"command failed with exit code 1","extra":{"exit_code":1,"stderr":"AssertionError"}}"#
                    .to_string(),
            ),
        ));
    let mut reply = AskReply::non_llm(
        r#"{"changed_files":["test_calc_core.py"],"test_status":"failed"}"#.to_string(),
    )
    .with_task_journal(journal);

    assert!(enforce_code_mutation_validation_success_guard(&mut reply));
    let summary = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .expect("validation failure gap");
    assert_eq!(summary.missing_evidence_fields, vec!["validation_success"]);
    assert_eq!(
        summary.answer_incomplete_reason,
        "post_write_validation_failed"
    );
    assert!(summary.should_retry);
}

#[test]
fn code_mutation_unresolved_test_status_creates_retry_gap() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-code-validation-unobserved",
        "ask",
        "prompt",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"},"text":"written"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def safe_div(a,b): pass"},"text":"ok"}"#,
        ));
    let mut reply = AskReply::non_llm(
        r#"{"changed_files":["calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"not_observed","functions":["safe_div"]}"#
            .to_string(),
    )
    .with_task_journal(journal);

    assert!(enforce_code_mutation_validation_success_guard(&mut reply));
    let summary = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .expect("unresolved validation gap");
    assert_eq!(summary.missing_evidence_fields, vec!["validation_success"]);
    assert_eq!(
        summary.answer_incomplete_reason,
        "post_write_unresolved_machine_fields"
    );
    assert!(summary.should_retry);
}

#[test]
fn post_write_readback_recovery_reserves_bounded_plan_capacity() {
    let policy = test_policy();
    let recovery = post_write_content_evidence_recovery_policy(&policy, 2);
    assert_eq!(recovery.max_steps, policy.max_steps);

    let mut narrow_policy = policy.clone();
    narrow_policy.max_steps = 1;
    let expanded = post_write_content_evidence_recovery_policy(&narrow_policy, 3);
    assert_eq!(expanded.max_steps, 3);
}

#[test]
fn post_write_guard_overrides_output_format_gap_when_content_evidence_missing() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-post-write-gap-priority", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"},"text":"written 98 bytes to /workspace/calc_core.py"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"},"text":"written 571 bytes to /workspace/test_calc_core.py"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "run_cmd",
            "Ran 4 tests in 0.000s\nOK",
        ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate answer is raw command output".to_string(),
        should_retry: true,
        retry_instruction: "rewrite the answer shape from observed evidence".to_string(),
        confidence: 0.9,
    });
    let mut reply =
        AskReply::non_llm("Ran 4 tests in 0.000s\nOK".to_string()).with_task_journal(journal);

    assert!(enforce_post_write_content_evidence_guard(&mut reply));
    let summary = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .expect("post-write verifier summary");
    assert_eq!(summary.missing_evidence_fields, vec!["content_excerpt"]);
    assert!(summary
        .answer_incomplete_reason
        .starts_with("post_write_content_evidence_required"));
}

#[test]
fn publishable_strict_json_projection_replaces_stale_verifier_candidate() {
    let answer = r#"{"changed_files":["/workspace/calc_core.py","/workspace/test_calc_core.py"],"test_command":["python3 test_calc_core.py","python3 - <<'PY'\nfrom calc_core import safe_div\nprint(safe_div(1,0))\nPY"],"test_status":"passed","functions":["add","sub","mul","safe_div"],"error_codes":["division_by_zero"]}"#;
    let mut route = route_result(OutputResponseShape::Strict);
    route.requires_content_evidence = true;
    route.delivery_required = false;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-strict-json-projection-promote",
        "ask",
        "prompt",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "run_cmd",
            "All tests passed\n",
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
        "step_3",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a,b): return a+b\n2|def sub(a,b): return a-b\n3|def mul(a,b): return a*b\n4|def safe_div(a,b): return {'ok': False, 'error_code': 'division_by_zero'}"}}"#,
    ));
    journal.push_task_observation(json!({
        "kind": "agent_loop_strict_json_projection",
        "owner_layer": "agent_loop",
        "schema_version": 1,
        "publishable": true,
        "output": answer,
    }));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "stale raw readback candidate".to_string(),
        should_retry: true,
        retry_instruction: "use publishable projection".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm("def safe_div(a,b): ...".to_string())
        .with_messages(vec!["def safe_div(a,b): ...".to_string()])
        .with_task_journal(journal);
    let mut loop_state = LoopState::new();
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_publishable".to_string(),
        "true".to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_output".to_string(),
        answer.to_string(),
    );

    assert!(
        promote_publishable_strict_json_projection_for_verifier_candidate(
            &mut reply,
            Some(&answer_contract(&route)),
            &loop_state,
        )
    );
    assert_eq!(reply.text, answer);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[test]
fn local_code_machine_projection_replaces_stale_verifier_candidate_before_verifier() {
    let user_text =
        "最后只输出 JSON，包含 changed_files、test_command、test_status、functions、error_codes。";
    let mut loop_state = LoopState::new();
    loop_state.output_vars.insert(
        "agent_loop.run_cmd_commands".to_string(),
        serde_json::json!([
            "python3 test_calc_core.py",
            "python3 - <<'PY'\nfrom calc_core import safe_div\nprint(safe_div(1,0))\nPY"
        ])
        .to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"}}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "run_cmd", "ALL TESTS PASSED\n"));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "run_cmd",
        r#"{"ok":false,"error_code":"division_by_zero"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_5",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|\n4|def sub(a, b):\n5|    return a - b\n6|\n7|def mul(a, b):\n8|    return a * b\n9|\n10|def safe_div(a, b):\n11|    if b == 0:\n12|        return {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_6",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, sub, mul, safe_div\n2|def test_safe_div_zero(): pass"}}"#,
    ));
    let journal =
        crate::task_journal::TaskJournal::for_task("task-local-code-promote", "ask", user_text);
    let mut reply = AskReply::non_llm("calc_core.py\ntest_calc_core.py".to_string())
        .with_messages(vec!["calc_core.py\ntest_calc_core.py".to_string()])
        .with_task_journal(journal);
    let context = local_code_context_with_required_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions",
        "error_codes"
    ]));

    assert!(
        promote_local_code_projection_from_machine_evidence_for_verifier_candidate(
            &mut reply,
            user_text,
            &loop_state,
            Some(&context),
        )
    );
    let value: serde_json::Value = serde_json::from_str(&reply.text).expect("strict json");
    assert_eq!(
        value["functions"],
        serde_json::json!(["add", "sub", "mul", "safe_div"])
    );
    assert_eq!(
        value["error_codes"],
        serde_json::json!(["division_by_zero"])
    );
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert!(journal.task_observations.iter().any(|observation| {
        observation.get("kind").and_then(serde_json::Value::as_str)
            == Some("agent_loop_strict_json_projection")
    }));
}

#[test]
fn post_write_guard_accepts_post_write_read_range_content_evidence() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-post-write-guard-ok", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"},"text":"written 98 bytes to /workspace/calc_core.py"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"},"text":"written 571 bytes to /workspace/test_calc_core.py"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "fs_basic",
            r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def mul(a, b):\n4|    return a * b"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_4",
            "fs_basic",
            r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, sub, mul\n2|assert mul(2, 3) == 6"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_5",
            "run_cmd",
            "Ran 3 tests in 0.000s\nOK",
        ));
    let mut reply = AskReply::non_llm(
        r#"{"changed_files":["calc_core.py","test_calc_core.py"],"test_status":"OK"}"#.to_string(),
    )
    .with_task_journal(journal);

    assert!(!enforce_post_write_content_evidence_guard(&mut reply));
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_none());
}

#[test]
fn post_write_guard_clears_stale_gap_after_later_readback() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-post-write-guard-clear", "ask", "prompt");
    journal.record_answer_verifier_summary(crate::answer_verifier::AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason:
            "post_write_content_evidence_required paths=/workspace/test_calc_core.py".to_string(),
        should_retry: true,
        retry_instruction: "collect bounded content excerpts".to_string(),
        confidence: 0.96,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"},"text":"written 571 bytes to /workspace/test_calc_core.py"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "run_cmd",
            "Ran 3 tests in 0.000s\nOK",
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "fs_basic",
            r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import safe_div\n2|assert safe_div(1,0)['error_code'] == 'division_by_zero'"}}"#,
        ));
    let mut reply = AskReply::non_llm(
        r#"{"changed_files":["test_calc_core.py"],"test_status":"OK","error_codes":["division_by_zero"]}"#
            .to_string(),
    )
    .with_task_journal(journal);

    assert!(!enforce_post_write_content_evidence_guard(&mut reply));
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_none());
}

#[test]
fn post_write_guard_detects_run_cmd_shell_redirection_code_write_without_inline_content() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-post-write-shell-guard", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "run_cmd",
            "exit=0 command=python gen_calc.py > calc_core.py",
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "run_cmd",
            "Ran 3 tests in 0.000s\nOK",
        ));
    let mut reply = AskReply::non_llm(
        r#"{"changed_files":["calc_core.py"],"test_status":"OK","functions":["mul"]}"#.to_string(),
    )
    .with_task_journal(journal);

    assert!(enforce_post_write_content_evidence_guard(&mut reply));
    let summary = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .expect("post-write verifier summary");
    assert_eq!(summary.missing_evidence_fields, vec!["content_excerpt"]);
    assert!(summary
        .answer_incomplete_reason
        .contains("post_write_content_evidence_required"));

    let route = route_result(OutputResponseShape::Free);
    assert!(!suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&answer_contract(&route))
    ));
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_some());
}

#[test]
fn post_write_guard_accepts_run_cmd_heredoc_inline_content_evidence() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-post-write-shell-heredoc-ok",
        "ask",
        "prompt",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "run_cmd",
            "exit=0 command=cat > calc_core.py <<'PYEOF'\ndef mul(a, b):\n    return a * b\nPYEOF",
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "run_cmd",
            "Ran 3 tests in 0.000s\nOK",
        ));
    let mut reply = AskReply::non_llm(
        r#"{"changed_files":["calc_core.py"],"test_status":"OK","functions":["mul"]}"#.to_string(),
    )
    .with_task_journal(journal);

    assert!(!enforce_post_write_content_evidence_guard(&mut reply));
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_none());
}

#[test]
fn post_write_guard_accepts_shell_write_with_later_absolute_readback() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-post-write-shell-guard-ok",
        "ask",
        "prompt",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "run_cmd",
            "exit=0 command=cat > calc_core.py <<'PYEOF'\ndef mul(a, b):\n    return a * b\nPYEOF",
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def mul(a, b):\n2|    return a * b"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "run_cmd",
            "Ran 3 tests in 0.000s\nOK",
        ));
    let mut reply = AskReply::non_llm(
        r#"{"changed_files":["calc_core.py"],"test_status":"OK","functions":["mul"]}"#.to_string(),
    )
    .with_task_journal(journal);

    assert!(!enforce_post_write_content_evidence_guard(&mut reply));
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_none());
}

#[test]
fn retry_verifier_pass_accepts_rewritten_answer() {
    let accepted = crate::answer_verifier::AnswerVerifierOut {
        pass: true,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: String::new(),
        should_retry: false,
        retry_instruction: String::new(),
        confidence: 0.95,
    };
    let rejected = crate::answer_verifier::AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate still violates the requested shape".to_string(),
        should_retry: true,
        retry_instruction: "rewrite the terminal answer".to_string(),
        confidence: 0.95,
    };

    assert!(retry_verifier_accepts_rewritten_answer(
        &accepted,
        "grounded rewritten answer"
    ));
    assert!(!retry_verifier_accepts_rewritten_answer(
        &rejected,
        "grounded rewritten answer"
    ));

    assert!(!retry_verifier_accepts_rewritten_answer(
        &accepted,
        r#"{"changed_files":["calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"not_observed"}"#
    ));
}

#[test]
fn verifier_retry_commit_replaces_stale_visible_reply() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-verifier-retry-commit", "ask", "prompt");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate omitted the requested terminal shape".to_string(),
        should_retry: true,
        retry_instruction: "rewrite the final answer from observed evidence".to_string(),
        confidence: 0.96,
    });
    let mut reply = AskReply::non_llm("stale raw observation".to_string())
        .with_messages(vec!["stale raw observation".to_string()])
        .with_task_journal(journal);

    assert!(commit_answer_verifier_retry_answer(
        &mut reply,
        "grounded rewritten answer".to_string()
    ));

    assert_eq!(reply.text, "grounded rewritten answer");
    assert_eq!(
        reply.messages,
        vec!["grounded rewritten answer".to_string()]
    );
    assert!(!reply.should_fail_task);
    assert!(reply.error_text.is_none());
    assert!(reply.is_llm_reply);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert_eq!(
        journal.final_answer.as_deref(),
        Some("grounded rewritten answer")
    );
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert_eq!(
        journal.final_stop_signal.as_deref(),
        Some(crate::task_journal::ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL)
    );
}

#[test]
fn verifier_retry_commit_rejects_local_code_json_without_validation_signal() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-verifier-retry-no-validation",
        "ask",
        "prompt",
    );
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def safe_div(a,b):\n2|    return {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
    ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate omitted the requested terminal shape".to_string(),
        should_retry: true,
        retry_instruction: "rewrite the final answer from observed evidence".to_string(),
        confidence: 0.96,
    });
    let mut reply = AskReply::non_llm("stale raw observation".to_string())
        .with_messages(vec!["stale raw observation".to_string()])
        .with_task_journal(journal);

    assert!(!commit_answer_verifier_retry_answer(
        &mut reply,
        r#"{"changed_files":["calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed","functions":["safe_div"],"error_codes":["division_by_zero"]}"#
            .to_string()
    ));
    assert_eq!(reply.text, "stale raw observation");
    assert_eq!(reply.messages, vec!["stale raw observation".to_string()]);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_some());
    assert!(journal.final_answer.is_none());
}

#[test]
fn answer_verifier_retry_summary_requires_retryable_high_confidence_gap() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["path".to_string()],
        answer_incomplete_reason: "missing fallback path".to_string(),
        should_retry: true,
        retry_instruction: "search fallback path".to_string(),
        confidence: 0.8,
    });
    let reply = AskReply::non_llm("wrong path".to_string()).with_task_journal(journal);

    let summary = answer_verifier_retry_summary(&reply, None).expect("retry gap");
    assert_eq!(summary.missing_evidence_fields, vec!["path"]);
}

#[test]
fn answer_verifier_retry_summary_allows_recoverable_verifier_failure_reply() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
    journal.final_failure_attribution = Some("contract_gap".to_string());
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "field label instead of clear final answer".to_string(),
        should_retry: true,
        retry_instruction: "rewrite from observed machine state".to_string(),
        confidence: 0.62,
    });
    let mut reply =
        AskReply::non_llm("approval_pending_task".to_string()).with_task_journal(journal);
    reply.should_fail_task = true;

    let summary = answer_verifier_retry_summary(&reply, None).expect("recoverable verifier gap");

    assert_eq!(summary.missing_evidence_fields, vec!["output_format"]);
}

#[test]
fn answer_verifier_retry_summary_allows_preterminal_should_fail_reply() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.final_failure_attribution = Some("contract_gap".to_string());
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate needs a corrected terminal shape".to_string(),
        should_retry: true,
        retry_instruction: "rewrite using the requested terminal contract".to_string(),
        confidence: 0.9,
    });
    let mut reply = AskReply::non_llm("field_label_only".to_string()).with_task_journal(journal);
    reply.should_fail_task = true;

    let summary = answer_verifier_retry_summary(&reply, None).expect("preterminal retry gap");

    assert_eq!(summary.missing_evidence_fields, vec!["output_format"]);
}

#[test]
fn answer_verifier_retry_summary_rejects_low_confidence_retry_flag() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: "answer omitted requested synthesis".to_string(),
        should_retry: true,
        retry_instruction: "include the requested synthesis".to_string(),
        confidence: 0.2,
    });
    let reply = AskReply::non_llm("single candidate".to_string()).with_task_journal(journal);

    assert!(answer_verifier_retry_summary(&reply, None).is_none());
}

#[test]
fn answer_verifier_retry_summary_skips_clarify_final_status() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["path".to_string()],
        answer_incomplete_reason: "missing fallback path".to_string(),
        should_retry: true,
        retry_instruction: "search fallback path".to_string(),
        confidence: 0.8,
    });
    let reply = AskReply::non_llm("please provide the path".to_string()).with_task_journal(journal);

    assert!(answer_verifier_retry_summary(&reply, None).is_none());
}

#[test]
fn terminal_model_answer_suppresses_output_format_only_verifier_retry() {
    let answer = "RustClaw combines the local clawd runtime, channel entry points, and skill dispatch into one deployable stack.";
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::None;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.push_step_result(&ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","excerpt":"RustClaw runtime overview","path":"README.md"},"text":"RustClaw runtime overview"}"#,
    ));
    journal.push_step_result(&ok_step("step_2", "synthesize_answer", answer));
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "terminal answer shape mismatch".to_string(),
        should_retry: true,
        retry_instruction: "rewrite terminal answer".to_string(),
        confidence: 0.9,
    });
    let mut reply = AskReply::non_llm(answer.to_string())
        .with_messages(vec![answer.to_string()])
        .with_task_journal(journal);

    assert!(suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&answer_contract(&route))
    ));
    assert!(answer_verifier_retry_summary(&reply, Some(&answer_contract(&route))).is_none());
}

#[test]
fn raw_observation_output_format_gap_does_not_suppress_structural_retry() {
    let raw_answer = "2026-04-01 WARN latency increased\n2026-04-01 ERROR provider timeout";
    let mut route = route_result(OutputResponseShape::Free);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/app.log | docs/service_notes.md".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.push_step_result(&ok_step(
        "step_1",
        "log_analyze",
        r#"{"keyword_counts":{"warn":1,"error":1},"matches":[{"level":"WARN"},{"level":"ERROR"}]}"#,
    ));
    journal.push_step_result(&ok_step(
        "step_2",
        "doc_parse",
        r##"{"extra":{"content_excerpt":"# Service Notes\nbody","path":"docs/service_notes.md"}}"##,
    ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason:
            "answer dumped raw observations and omitted the requested summary/table".to_string(),
        should_retry: true,
        retry_instruction: "rewrite from observed evidence in the requested shape".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm(raw_answer.to_string())
        .with_messages(vec![raw_answer.to_string()])
        .with_task_journal(journal);

    assert!(!suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&answer_contract(&route))
    ));
    assert!(answer_verifier_retry_summary(&reply, Some(&answer_contract(&route))).is_some());
}

#[test]
fn terminal_model_answer_does_not_suppress_non_format_evidence_gap() {
    let answer = "RustClaw combines the local clawd runtime, channel entry points, and skill dispatch into one deployable stack.";
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::None;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.push_step_result(&ok_step("step_1", "synthesize_answer", answer));
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string(), "content_excerpt".to_string()],
        answer_incomplete_reason: "content evidence is still missing".to_string(),
        should_retry: true,
        retry_instruction: "collect content evidence".to_string(),
        confidence: 0.9,
    });
    let mut reply = AskReply::non_llm(answer.to_string())
        .with_messages(vec![answer.to_string()])
        .with_task_journal(journal);

    assert!(!suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&answer_contract(&route))
    ));
    assert!(answer_verifier_retry_summary(&reply, Some(&answer_contract(&route))).is_some());
}

#[test]
fn terminal_model_answer_replaces_direct_observation_before_verifier() {
    let raw_readme = "# RustClaw\n\nRustClaw is a local Rust agent runtime centered on `clawd`.";
    let answer = "RustClaw 是以 `clawd` 为核心的本地 Rust 智能体运行时。它整合多渠道聊天、任务执行、工具和技能路由等能力。它面向通过聊天应用或浏览器完成日常使用和管理。";
    let mut route = route_result(OutputResponseShape::Strict);
    route.exact_sentence_count = Some(3);
    route.locator_kind = OutputLocatorKind::None;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    let read_step_output = json!({
        "extra": {
            "action": "read_range",
            "excerpt": raw_readme,
            "path": "README.md",
        },
        "text": raw_readme,
    })
    .to_string();
    journal.push_step_result(&ok_step("step_1", "fs_basic", &read_step_output));
    journal.push_step_result(&ok_step("step_2", "respond", answer));
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    let mut reply = AskReply::non_llm(raw_readme.to_string())
        .with_messages(vec![raw_readme.to_string()])
        .with_task_journal(journal);

    assert!(prefer_terminal_model_answer_for_verifier_candidate(
        &mut reply,
        Some(&answer_contract(&route))
    ));
    assert_eq!(reply.text, answer);
}

#[test]
fn terminal_model_answer_does_not_replace_richer_machine_projection_with_observed_scalar() {
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::None;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-service-status-terminal", "ask", "status");
    let service_output = json!({
        "extra": {
            "manager_type": "rustclaw",
            "post_state": "telegramd=running",
            "pre_state": "telegramd=running",
            "service_name": "telegramd",
            "status": "ok",
            "summary": "Status: telegramd=running",
            "target": "telegramd",
            "verified": true
        },
        "text": "Status: telegramd=running"
    })
    .to_string();
    journal.push_step_result(&ok_step("step_1", "service_control", &service_output));
    journal.push_step_result(&ok_step("step_2", "respond", "Status: telegramd=running"));
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    let observed_projection = "target=telegramd service_name=telegramd post_state=telegramd=running pre_state=telegramd=running status=ok verified=true manager_type=rustclaw source=service_control";
    let mut reply = AskReply::non_llm(observed_projection.to_string())
        .with_messages(vec![observed_projection.to_string()])
        .with_task_journal(journal);

    assert!(!prefer_terminal_model_answer_for_verifier_candidate(
        &mut reply,
        Some(&answer_contract(&route))
    ));
    assert_eq!(reply.text, observed_projection);
}

#[test]
fn terminal_model_answer_does_not_replace_requested_exact_observation_fields_with_stdout() {
    let mut route = route_result(OutputResponseShape::Free);
    route.configure_exact_command_output();
    route.locator_kind = OutputLocatorKind::Path;
    route.selection.structured_field_selector =
        Some("command,created_path,stdout,status".to_string());
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-command-fields-terminal", "ask", "prompt");
    journal.push_step_result(&ok_step("step_1", "run_cmd", "checkpoint_resume_ok"));
    journal.push_step_result(&ok_step(
        "step_2",
        "synthesize_answer",
        "checkpoint_resume_ok",
    ));
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    let observed_projection = "\
command=mkdir -p generated && printf '%s' checkpoint_resume_ok > generated/result.txt
created_path=/workspace/generated/result.txt
stdout=checkpoint_resume_ok
status=ok";
    let mut reply = AskReply::non_llm(observed_projection.to_string())
        .with_messages(vec![observed_projection.to_string()])
        .with_task_journal(journal);

    assert!(!prefer_terminal_model_answer_for_verifier_candidate(
        &mut reply,
        Some(&answer_contract(&route))
    ));
    assert_eq!(reply.text, observed_projection);
}

#[test]
fn terminal_model_answer_does_not_replace_single_machine_projection_with_observed_scalar() {
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "AGENTS.md".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-single-machine-projection-terminal",
        "ask",
        "prompt",
    );
    let grep_output = json!({
        "extra": {
            "action": "grep_text",
            "match_count": 1,
            "matches": [{
                "line": 244,
                "path": "AGENTS.md",
                "text": "run `python3 scripts/check_no_nl_hardmatch.py` after boundary changes"
            }],
            "query": "check_no_nl_hardmatch.py",
            "results": ["AGENTS.md"],
            "root": "AGENTS.md"
        },
        "text": "AGENTS.md"
    })
    .to_string();
    journal.push_step_result(&ok_step("step_1", "fs_basic", &grep_output));
    journal.push_step_result(&ok_step("step_2", "respond", "AGENTS.md"));
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    let observed_projection = "no_hardmatch_guard=check_no_nl_hardmatch.py";
    let mut reply = AskReply::non_llm(observed_projection.to_string())
        .with_messages(vec![observed_projection.to_string()])
        .with_task_journal(journal);

    assert!(!prefer_terminal_model_answer_for_verifier_candidate(
        &mut reply,
        Some(&answer_contract(&route))
    ));
    assert_eq!(reply.text, observed_projection);
}

#[test]
fn file_token_delivery_suppresses_list_count_verifier_retry_when_grounded() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-loop-control-file-token-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let file = root.join("report.txt");
    std::fs::write(&file, "report").expect("write temp file");

    let mut route = route_result(OutputResponseShape::FileToken);
    route.delivery_required = true;
    route.delivery_intent = OutputDeliveryIntent::FileSingle;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "inventory_dir",
                    "resolved_path": root.display().to_string(),
                    "names": ["report.txt", "other.txt"],
                    "entries": [
                        {
                            "kind": "file",
                            "name": "report.txt",
                            "path": file.display().to_string()
                        }
                    ]
                })
                .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason:
            "answer provides only 1 file path when evidence shows the directory contains many files"
                .to_string(),
        should_retry: true,
        retry_instruction: "list all files".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm(format!("FILE:{}", file.display()))
        .with_messages(vec![
            "**执行过程**\n1. 调用工具 `fs_basic`。".to_string(),
            format!("FILE:{}", file.display()),
        ])
        .with_task_journal(journal);

    assert!(suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&answer_contract(&route))
    ));
    assert!(answer_verifier_retry_summary(&reply, Some(&answer_contract(&route))).is_none());

    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn file_token_delivery_does_not_suppress_when_token_is_not_grounded() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-loop-control-file-token-ungrounded-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let observed = root.join("observed.txt");
    let ungrounded = root.join("ungrounded.txt");
    std::fs::write(&observed, "observed").expect("write observed file");
    std::fs::write(&ungrounded, "ungrounded").expect("write ungrounded file");

    let mut route = route_result(OutputResponseShape::FileToken);
    route.delivery_required = true;
    route.delivery_intent = OutputDeliveryIntent::FileSingle;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "inventory_dir",
                    "resolved_path": root.display().to_string(),
                    "entries": [
                        {
                            "kind": "file",
                            "name": "observed.txt",
                            "path": observed.display().to_string()
                        }
                    ]
                })
                .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: "candidate file is not supported by evidence".to_string(),
        should_retry: true,
        retry_instruction: "select a grounded file".to_string(),
        confidence: 0.95,
    });
    let mut reply =
        AskReply::non_llm(format!("FILE:{}", ungrounded.display())).with_task_journal(journal);

    assert!(!suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&answer_contract(&route))
    ));
    assert!(answer_verifier_retry_summary(&reply, Some(&answer_contract(&route))).is_some());

    let _ = std::fs::remove_file(&observed);
    let _ = std::fs::remove_file(&ungrounded);
    let _ = std::fs::remove_dir_all(&root);
}

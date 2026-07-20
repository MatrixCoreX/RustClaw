use super::*;

#[test]
fn should_verify_answer_skips_journal_grounded_strict_json_code_projection() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let answer = r#"{"created_files":["/workspace/calc_core.py","/workspace/test_calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed"}"#;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-code-json", "ask", "create code");
    journal.record_output_contract(&route.output_contract);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "run_cmd",
            "Ran 2 tests in 0.001s\nOK\n",
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_4",
            "synthesize_answer",
            answer,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_5",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_6",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, sub\n2|assert add(1,2)==3\n3|assert sub(3,1)==2"},"text":"ok"}"#,
        ));

    assert!(!should_verify_answer(&route, &journal, answer));
}

#[test]
fn should_verify_answer_skips_post_readback_strict_json_projection_observation() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let answer = r#"{"changed_files":["/workspace/calc_core.py","/workspace/test_calc_core.py"],"functions":["add","sub","mul"],"test_command":"python3 test_calc_core.py","test_status":"passed"}"#;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-code-json-post-readback",
        "ask",
        "update code",
    );
    journal.record_output_contract(&route.output_contract);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "run_cmd",
            "All tests passed\n",
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_4",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def mul(a, b):\n6|    return a * b"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_5",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, sub, mul\n2|assert add(1,2)==3\n3|assert sub(3,1)==2\n4|assert mul(2,3)==6"},"text":"ok"}"#,
        ));
    journal.push_task_observation(json!({
        "kind": "agent_loop_strict_json_projection",
        "owner_layer": "agent_loop",
        "schema_version": 1,
        "publishable": true,
        "output": answer,
    }));

    assert!(!should_verify_answer(&route, &journal, answer));
}

#[test]
fn should_verify_answer_skips_readback_only_local_code_projection_with_validation() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let answer = r#"{"project_dir":"/workspace","functions":["add","sub","mul","safe_div"],"error_codes":["division_by_zero"],"test_status":"passed","evidence_files":["/workspace/calc_core.py","/workspace/test_calc_core.py"]}"#;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-code-json-readback-only",
        "ask",
        "inspect code",
    );
    journal.record_output_contract(&route.output_contract);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a,b): return a+b\n2|def safe_div(a,b):\n3|    return {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, safe_div\n2|def test_safe_div_zero(): pass"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "run_cmd",
            "All tests passed\n",
        ));
    journal.push_task_observation(json!({
        "kind": "agent_loop_strict_json_projection",
        "owner_layer": "agent_loop",
        "schema_version": 1,
        "publishable": true,
        "output": answer,
    }));

    assert!(!should_verify_answer(&route, &journal, answer));
}

#[test]
fn should_verify_answer_rejects_unresolved_strict_json_projection_observation() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let answer = r#"{"changed_files":["/workspace/calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"not_observed"}"#;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-code-json-unresolved",
        "ask",
        "update code",
    );
    journal.record_output_contract(&route.output_contract);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def safe_div(a, b):\n2|    return a / b"},"text":"ok"}"#,
        ));
    journal.push_task_observation(json!({
        "kind": "agent_loop_strict_json_projection",
        "owner_layer": "agent_loop",
        "schema_version": 1,
        "publishable": true,
        "output": answer,
    }));

    assert!(should_verify_answer(&route, &journal, answer));
}

#[test]
fn should_verify_answer_skips_publishable_code_projection_when_route_shape_is_not_strict() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    let answer = r#"{"created_files":["/workspace/calc_core.py","/workspace/test_calc_core.py"],"test_command":"cd /workspace && python3 test_calc_core.py","test_status":"passed"}"#;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-code-json-route-free",
        "ask",
        "create code",
    );
    journal.record_output_contract(&route.output_contract);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "run_cmd",
            "Ran 2 tests in 0.001s\nOK\n",
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_4",
            "synthesize_answer",
            answer,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_5",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_6",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, sub\n2|assert add(1,2)==3\n3|assert sub(3,1)==2"},"text":"ok"}"#,
        ));
    journal.push_task_observation(json!({
        "kind": "agent_loop_strict_json_projection",
        "owner_layer": "agent_loop",
        "schema_version": 1,
        "publishable": true,
        "output": answer,
    }));

    assert!(!should_verify_answer(&route, &journal, answer));
}

#[test]
fn should_verify_answer_keeps_verifier_for_publishable_code_projection_without_readback() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    let answer = r#"{"created_files":["/workspace/calc_core.py"],"test_command":"cd /workspace && python3 test_calc_core.py","test_status":"passed"}"#;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-code-json-route-free-missing-readback",
        "ask",
        "create code",
    );
    journal.record_output_contract(&route.output_contract);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "run_cmd",
            "Ran 1 test in 0.001s\nOK\n",
        ));
    journal.push_task_observation(json!({
        "kind": "agent_loop_strict_json_projection",
        "owner_layer": "agent_loop",
        "schema_version": 1,
        "publishable": true,
        "output": answer,
    }));

    assert!(should_verify_answer(&route, &journal, answer));
}

#[test]
fn post_write_content_gap_defers_early_verifier_until_readback_exists() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let answer = r#"{"created_files":["/workspace/calc_core.py"],"test_command":"cd /workspace && python3 test_calc_core.py","test_status":"passed"}"#;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-code-json-post-write-gap",
        "ask",
        "create code",
    );
    journal.record_output_contract(&route.output_contract);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "run_cmd",
            "Ran 1 test in 0.001s\nOK\n",
        ));

    assert!(post_write_content_evidence_missing_before_verifier(
        &journal, answer
    ));

    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b"},"text":"ok"}"#,
        ));

    assert!(!post_write_content_evidence_missing_before_verifier(
        &journal, answer
    ));
}

#[test]
fn post_write_content_gap_ignores_pre_write_readback() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let answer = r#"{"changed_files":["/workspace/calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed","functions":["add","sub","mul"]}"#;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-code-json-pre-write-readback",
        "ask",
        "update code",
    );
    journal.record_output_contract(&route.output_contract);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "run_cmd",
            "Ran 1 test in 0.001s\nOK\n",
        ));

    assert!(post_write_content_evidence_missing_before_verifier(
        &journal, answer
    ));
    assert!(should_verify_answer(&route, &journal, answer));

    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_4",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def mul(a, b):\n6|    return a * b"},"text":"ok"}"#,
        ));
    journal.push_task_observation(json!({
        "kind": "agent_loop_strict_json_projection",
        "owner_layer": "agent_loop",
        "schema_version": 1,
        "publishable": true,
        "output": answer,
    }));

    assert!(!post_write_content_evidence_missing_before_verifier(
        &journal, answer
    ));
    assert!(!should_verify_answer(&route, &journal, answer));
}

#[test]
fn should_verify_answer_requires_matching_synthesis_for_strict_json_skip() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let answer = r#"{"created_files":["/workspace/calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed"}"#;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-code-json-missing-synthesis",
        "ask",
        "create code",
    );
    journal.record_output_contract(&route.output_contract);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py"},"text":"ok"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "run_cmd",
            "Ran 2 tests in 0.001s\nOK\n",
        ));

    assert!(should_verify_answer(&route, &journal, answer));
}

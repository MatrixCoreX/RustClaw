use super::super::loop_control_post_write_evidence_guard::post_write_validation_reserve_actions;
use super::ok_step;
use crate::{
    agent_engine::{AgentRunContext, LoopState},
    AgentAction, AskReply,
};
use serde_json::json;

#[test]
fn post_write_validation_reserve_uses_latest_plan_observe_validate_actions_only() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-post-write-validation-reserve",
        "ask",
        "prompt",
    );
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
    let reply =
        AskReply::non_llm(r#"{"changed_files":["calc_core.py","test_calc_core.py"]}"#.to_string())
            .with_task_journal(journal);

    let mut loop_state = LoopState::new();
    loop_state.last_stop_signal = Some("post_write_validation_required".to_string());
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def safe_div(a,b): return {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import safe_div\n2|assert safe_div(1,0)[\"error_code\"] == \"division_by_zero\""}}"#,
    ));
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "add safe_div and validate".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(super::plan_result_with_raw_and_steps(
                "post-write validation reserve fixture",
                vec![
                    crate::PlanStep {
                        step_id: "write_again".to_string(),
                        action_type: "call_tool".to_string(),
                        skill: "fs_basic.write_text".to_string(),
                        args: json!({"path": "/workspace/calc_core.py", "content": "new"}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "search_noise".to_string(),
                        action_type: "call_tool".to_string(),
                        skill: "fs_basic".to_string(),
                        args: json!({"action": "find_entries", "root": "/workspace", "pattern": "*.py"}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "readback_calc".to_string(),
                        action_type: "call_tool".to_string(),
                        skill: "fs_basic.read_text_range".to_string(),
                        args: json!({"path": "/workspace/calc_core.py", "mode": "head", "n": 80}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "validate_tests".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({"command": "python3 -m unittest test_calc_core.py", "cwd": "/workspace"}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "finish".to_string(),
                        action_type: "synthesize_answer".to_string(),
                        skill: "synthesize_answer".to_string(),
                        args: json!({"evidence_refs": ["last_output"]}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                ],
            )),
            verify_result: None,
        });

    let actions =
        post_write_validation_reserve_actions(&state, &reply, &loop_state, 8, "prompt", None);
    assert_eq!(actions.len(), 2);
    match &actions[0] {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("read_text_range")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("/workspace/calc_core.py")
            );
        }
        other => panic!("unexpected readback action: {other:?}"),
    }
    match &actions[1] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(|value| value.as_str()),
                Some("python3 -m unittest test_calc_core.py")
            );
        }
        other => panic!("unexpected validation action: {other:?}"),
    }
}

#[test]
fn readback_only_code_validation_reserve_runs_unexecuted_probe_only() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-readback-only-validation-reserve",
        "ask",
        "prompt",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a,b): return a+b\n2|def safe_div(a,b): return {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
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
            "ALL_TESTS_PASSED",
        ));
    let reply =
        AskReply::non_llm(r#"{"changed_files":["calc_core.py","test_calc_core.py"]}"#.to_string())
            .with_task_journal(journal);

    let mut loop_state = LoopState::new();
    loop_state.last_stop_signal = Some("post_write_validation_required".to_string());
    loop_state.output_vars.insert(
        "agent_loop.run_cmd_commands".to_string(),
        serde_json::json!(["python3 test_calc_core.py"]).to_string(),
    );
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 3,
            goal: "validate existing safe_div".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(super::plan_result_with_raw_and_steps(
                "readback-only validation reserve fixture",
                vec![
                    crate::PlanStep {
                        step_id: "validate_tests".to_string(),
                        action_type: "call_capability".to_string(),
                        skill: "system.run_command".to_string(),
                        args: json!({"command": "python3 test_calc_core.py", "cwd": "/workspace"}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "probe_zero".to_string(),
                        action_type: "call_capability".to_string(),
                        skill: "system.run_command".to_string(),
                        args: json!({"command": "python3 - <<'PY'\nfrom calc_core import safe_div\nprint(safe_div(1, 0))\nPY", "cwd": "/workspace"}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "finish".to_string(),
                        action_type: "synthesize_answer".to_string(),
                        skill: "synthesize_answer".to_string(),
                        args: json!({"evidence_refs": ["last_output"]}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                ],
            )),
            verify_result: None,
        });

    let actions =
        post_write_validation_reserve_actions(&state, &reply, &loop_state, 8, "prompt", None);
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "run_cmd");
            let command = args
                .get("command")
                .and_then(|value| value.as_str())
                .expect("probe command");
            assert!(command.contains("safe_div(1, 0)"), "{command}");
        }
        other => panic!("unexpected reserve action: {other:?}"),
    }
}

#[test]
fn readback_only_code_validation_reserve_runs_planned_validation_when_requested() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-readback-only-validation-requested",
        "ask",
        "prompt",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def safe_div(a,b): return {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import safe_div\n2|assert safe_div(1,0)[\"error_code\"] == \"division_by_zero\""}}"#,
        ));
    let reply = AskReply::non_llm("readback only".to_string()).with_task_journal(journal);

    let mut loop_state = LoopState::new();
    loop_state.last_stop_signal = Some("post_write_validation_required".to_string());
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 2,
            goal: "validate existing safe_div".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(super::plan_result_with_raw_and_steps(
                "readback-only requested validation fixture",
                vec![
                    crate::PlanStep {
                        step_id: "validate_tests".to_string(),
                        action_type: "call_capability".to_string(),
                        skill: "system.run_command".to_string(),
                        args: json!({"command": "python3 test_calc_core.py", "cwd": "/workspace"}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "finish".to_string(),
                        action_type: "synthesize_answer".to_string(),
                        skill: "synthesize_answer".to_string(),
                        args: json!({"evidence_refs": ["last_output"]}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                ],
            )),
            verify_result: None,
        });
    let context = AgentRunContext {
        turn_analysis: Some(crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskRequest),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(json!({
                "required_machine_fields": ["changed_files", "test_command", "test_status"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };
    let actions = post_write_validation_reserve_actions(
        &state,
        &reply,
        &loop_state,
        8,
        "Return JSON with changed_files, test_command, test_status.",
        Some(&context),
    );

    assert_eq!(actions.len(), 1);
    match &actions[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(|value| value.as_str()),
                Some("python3 test_calc_core.py")
            );
        }
        other => panic!("unexpected reserve action: {other:?}"),
    }
}

#[test]
fn readback_only_code_validation_reserve_ignores_plain_read_without_validation() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-readback-only-no-validation-reserve",
        "ask",
        "prompt",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a,b): return a+b"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add"}}"#,
        ));
    let reply = AskReply::non_llm("readback only".to_string()).with_task_journal(journal);

    let mut loop_state = LoopState::new();
    loop_state.last_stop_signal = Some("post_write_validation_required".to_string());
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "read files".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(super::plan_result_with_raw_and_steps(
                "plain read fixture",
                vec![crate::PlanStep {
                    step_id: "finish".to_string(),
                    action_type: "synthesize_answer".to_string(),
                    skill: "synthesize_answer".to_string(),
                    args: json!({"evidence_refs": ["last_output"]}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }],
            )),
            verify_result: None,
        });

    let actions =
        post_write_validation_reserve_actions(&state, &reply, &loop_state, 8, "prompt", None);
    assert!(actions.is_empty());
}

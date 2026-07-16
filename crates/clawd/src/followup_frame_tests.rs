use super::{
    derive_bound_target_from_journal, derive_code_workspace_bound_target_from_journal,
    extract_ordered_entries_from_text, load_active_followup_frame,
    ordered_entries_from_listing_json, persist_frame, replace_active_frame_from_ask_outcome,
    FollowupFrame, FollowupOpKind, FollowupSliceKind, FollowupSliceSpec,
};
use crate::{runtime::AppState, IntentOutputContract, OutputLocatorKind, RouteResult};

#[test]
fn extracts_ordered_entries_from_compact_listing_sentence() {
    let entries = extract_ordered_entries_from_text(
        "列表：act_plan.log、clawd.log、clawd.run.log、feishud.log、install_ops.log。",
    );
    assert_eq!(
        entries,
        vec![
            "act_plan.log",
            "clawd.log",
            "clawd.run.log",
            "feishud.log",
            "install_ops.log"
        ]
    );
}

#[test]
fn extracts_ordered_entries_from_bare_compact_listing() {
    let entries = extract_ordered_entries_from_text(
        "act_plan.log,clawd.log,clawd.run.log,feishud.log,install_ops.log",
    );
    assert_eq!(
        entries,
        vec![
            "act_plan.log",
            "clawd.log",
            "clawd.run.log",
            "feishud.log",
            "install_ops.log"
        ]
    );
}

#[test]
fn extracts_ordered_entries_from_comma_list_with_separator_spaces() {
    let entries = extract_ordered_entries_from_text(
        "act_plan.log, clawd.log, clawd.run.log, feishud.log, install_ops.log",
    );
    assert_eq!(
        entries,
        vec![
            "act_plan.log",
            "clawd.log",
            "clawd.run.log",
            "feishud.log",
            "install_ops.log"
        ]
    );
}

#[test]
fn extracts_ordered_entries_from_duplicated_compact_answer_line() {
    let entries = extract_ordered_entries_from_text(
        "act_plan.log, clawd.log, clawd.run.log, feishud.log, install_ops.log\nact_plan.log, clawd.log, clawd.run.log, feishud.log, install_ops.log",
    );
    assert_eq!(
        entries,
        vec![
            "act_plan.log",
            "clawd.log",
            "clawd.run.log",
            "feishud.log",
            "install_ops.log"
        ]
    );
}

#[test]
fn dedupes_ordered_entries_from_repeated_multiline_answer() {
    let entries = extract_ordered_entries_from_text(
        "act_plan.log\nclawd.log\nclawd.run.log\nfeishud.log\ninstall_ops.log\nact_plan.log\nclawd.log\nclawd.run.log\nfeishud.log\ninstall_ops.log",
    );
    assert_eq!(
        entries,
        vec![
            "act_plan.log",
            "clawd.log",
            "clawd.run.log",
            "feishud.log",
            "install_ops.log"
        ]
    );
}

#[test]
fn ignores_prose_prefixed_compact_listing_without_delimiter() {
    let entries = extract_ordered_entries_from_text(
        "前5个条目act_plan.log、clawd.log、clawd.run.log、feishud.log、install_ops.log",
    );
    assert!(
        entries.is_empty(),
        "compact follow-up extraction should not depend on language-specific prefix filters"
    );
}

#[test]
fn extracts_ordered_entries_from_listing_block_before_summary_paragraph() {
    let entries = extract_ordered_entries_from_text(
        "act_plan.log\nclawd.log\nclawd.run.log\nfeishud.log\ninstall_ops.log\n\n这个目录主要放运行日志和排查记录。",
    );
    assert_eq!(
        entries,
        vec![
            "act_plan.log",
            "clawd.log",
            "clawd.run.log",
            "feishud.log",
            "install_ops.log"
        ]
    );
}

#[test]
fn extracts_ordered_entries_from_prose_header_plus_path_lines() {
    let entries = extract_ordered_entries_from_text(
        "在 scripts/nl_tests/fixtures/locator_smart/fuzzy_top3 中匹配 \"abcd\" 的条目共 4 个：\n\
scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md\n\
scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt\n\
scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt\n\
scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log",
    );
    assert_eq!(
        entries,
        vec![
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md",
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt",
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt",
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"
        ]
    );
}

#[test]
fn extracts_ordered_entries_from_tree_summary_json() {
    let entries = ordered_entries_from_listing_json(&serde_json::json!({
        "action": "tree_summary",
        "tree": {
            "children": [
                {"kind": "dir", "path": "/tmp/docs/archive"},
                {"kind": "file", "path": "/tmp/docs/release_checklist.md"},
                {"kind": "file", "path": "/tmp/docs/service_notes.md"}
            ]
        }
    }));
    assert_eq!(
        entries,
        vec!["archive", "release_checklist.md", "service_notes.md"]
    );
}

#[test]
fn extracts_ordered_entries_from_search_result_json() {
    let entries = ordered_entries_from_listing_json(&serde_json::json!({
        "action": "grep_text",
        "query": "abcd",
        "count": 0,
        "match_count": 0,
        "matches": [],
        "name_count": 4,
        "name_results": [
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md",
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt",
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt",
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"
        ]
    }));

    assert_eq!(
        entries,
        vec![
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md",
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt",
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt",
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"
        ]
    );

    let entries = ordered_entries_from_listing_json(&serde_json::json!({
        "action": "find_name",
        "pattern": "abcd",
        "count": 2,
        "results": ["abcd_report.md", "my_abcd.txt"],
        "root": "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"
    }));

    assert_eq!(entries, vec!["abcd_report.md", "my_abcd.txt"]);
}

#[test]
fn ordered_entries_ignore_json_hidden_in_visible_text() {
    let hidden = serde_json::json!({
        "action": "find_name",
        "results": ["alpha.log", "beta.log"]
    })
    .to_string();
    let entries = ordered_entries_from_listing_json(&serde_json::json!({
        "text": hidden
    }));

    assert!(entries.is_empty());
}

#[test]
fn ordered_entries_accept_extra_machine_payload() {
    let entries = ordered_entries_from_listing_json(&serde_json::json!({
        "extra": {
            "action": "find_name",
            "results": ["alpha.log", "beta.log"]
        },
        "text": "display only"
    }));

    assert_eq!(entries, vec!["alpha.log", "beta.log"]);
}

fn journal_with_single_output(output: serde_json::Value) -> crate::task_journal::TaskJournal {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-followup-boundary", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(output.to_string()),
            ..Default::default()
        });
    journal
}

#[test]
fn bound_target_ignores_json_hidden_in_visible_text() {
    let hidden = serde_json::json!({
        "resolved_path": "/tmp/hidden.log"
    })
    .to_string();
    let journal = journal_with_single_output(serde_json::json!({
        "text": hidden
    }));

    assert_eq!(derive_bound_target_from_journal(&journal), None);
}

#[test]
fn bound_target_accepts_extra_machine_payload() {
    let journal = journal_with_single_output(serde_json::json!({
        "extra": {
            "resolved_path": "/tmp/visible-machine.log"
        },
        "text": "display only"
    }));

    assert_eq!(
        derive_bound_target_from_journal(&journal).as_deref(),
        Some("/tmp/visible-machine.log")
    );
}

#[test]
fn workspace_project_summary_evidence_path_does_not_persist_followup_target() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-workspace-summary-evidence".to_string(),
        user_id: 431,
        chat_id: 432,
        user_key: Some("test-user-workspace-summary".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            serde_json::json!({
                "extra": {
                    "action": "read_text_range",
                    "resolved_path": "/home/guagua/rustclaw/plan/post_migration.md",
                    "excerpt": "workspace evidence only"
                }
            })
            .to_string(),
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "write a short RustClaw release note".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "contract:workspace_project_summary".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Medium,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: crate::OutputSemanticKind::WorkspaceProjectSummary,
            ..IntentOutputContract::default()
        },
    };

    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "write a short RustClaw release note",
        &route_result,
        "RustClaw release note draft.",
        &[],
        false,
        &journal,
    );

    assert!(
        load_active_followup_frame(&state, &task).is_none(),
        "workspace summary evidence files must not become the next text-drafting target"
    );
}

#[test]
fn code_workspace_journal_persists_project_dir_for_followup() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-code-workspace".to_string(),
        user_id: 31,
        chat_id: 32,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let project_dir = "/home/guagua/rustclaw/run/nl_eval_tmp/code_workspace_anchor";
    let calc_path = format!("{project_dir}/calc_core.py");
    let test_path = format!("{project_dir}/test_calc_core.py");
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            serde_json::json!({
                "action": "write_text",
                "path": calc_path,
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            serde_json::json!({
                "action": "write_text",
                "path": test_path,
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "run_cmd",
            serde_json::json!({
                "action": "run_cmd",
                "cwd": project_dir,
                "command": "python3 test_calc_core.py",
                "exit_code": 0,
            })
            .to_string(),
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "code workspace update".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "executable_contract_preserved_for_agent_loop".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: crate::OutputResponseShape::Strict,
            ..IntentOutputContract::default()
        },
    };

    assert_eq!(
        derive_code_workspace_bound_target_from_journal(&journal).as_deref(),
        Some(project_dir)
    );
    assert_eq!(
        derive_bound_target_from_journal(&journal).as_deref(),
        Some(project_dir)
    );

    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "code workspace update",
        &route_result,
        r#"{"changed_files":["calc_core.py","test_calc_core.py"],"test_status":"passed"}"#,
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(frame.op_kind, FollowupOpKind::CodeWorkspace);
    assert_eq!(frame.bound_target.as_deref(), Some(project_dir));
}

#[test]
fn readback_validated_code_workspace_persists_project_dir_for_followup() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-code-workspace-readback-only".to_string(),
        user_id: 231,
        chat_id: 232,
        user_key: Some("test-user-readback-only".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let project_dir = "/home/guagua/rustclaw/run/nl_eval_tmp/code_workspace_readback_only";
    let calc_path = format!("{project_dir}/calc_core.py");
    let test_path = format!("{project_dir}/test_calc_core.py");
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            serde_json::json!({
                "extra": {
                    "action": "read_text_range",
                    "path": calc_path,
                    "resolved_path": calc_path,
                    "excerpt": "1|def add(a, b): return a + b\n2|def safe_div(a, b): return {\"ok\": True, \"value\": a / b}",
                }
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            serde_json::json!({
                "extra": {
                    "action": "read_text_range",
                    "path": test_path,
                    "resolved_path": test_path,
                    "excerpt": "1|from calc_core import add, safe_div\n2|def test_safe_div_zero(): pass",
                }
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "run_cmd",
            "ALL_TESTS_PASSED",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "code workspace readback validation".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "executable_contract_preserved_for_agent_loop".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: crate::OutputResponseShape::Strict,
            ..IntentOutputContract::default()
        },
    };

    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "code workspace readback validation",
        &route_result,
        r#"{"changed_files":["calc_core.py","test_calc_core.py"],"test_status":"passed"}"#,
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(frame.op_kind, FollowupOpKind::CodeWorkspace);
    assert_eq!(frame.bound_target.as_deref(), Some(project_dir));
}

#[test]
fn code_workspace_followup_wins_over_delivery_route_noise() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-code-workspace-delivery-noise".to_string(),
        user_id: 131,
        chat_id: 132,
        user_key: Some("test-user-delivery-noise".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let project_dir = "/home/guagua/rustclaw/run/nl_eval_tmp/code_workspace_delivery_noise";
    let calc_path = format!("{project_dir}/calc_core.py");
    let test_path = format!("{project_dir}/test_calc_core.py");
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            serde_json::json!({
                "extra": {
                    "action": "write_text",
                    "path": calc_path,
                    "resolved_path": calc_path,
                }
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            serde_json::json!({
                "extra": {
                    "action": "write_text",
                    "path": test_path,
                    "resolved_path": test_path,
                }
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "run_cmd",
            serde_json::json!({
                "extra": {
                    "action": "run_cmd",
                    "cwd": project_dir,
                    "exit_code": 0,
                }
            })
            .to_string(),
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "code workspace update with delivery noise".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason:
            "file_token_delivery_contract_repair; executable_contract_preserved_for_agent_loop"
                .to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: crate::OutputResponseShape::Strict,
            delivery_required: true,
            ..IntentOutputContract::default()
        },
    };

    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "code workspace update with delivery noise",
        &route_result,
        r#"{"created_files":["calc_core.py","test_calc_core.py"],"test_status":"passed"}"#,
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(frame.op_kind, FollowupOpKind::CodeWorkspace);
    assert_eq!(frame.bound_target.as_deref(), Some(project_dir));
}

#[test]
fn extracts_ordered_entries_from_markdown_numbered_listing() {
    let entries = extract_ordered_entries_from_text(
        "**logs 目录下前 5 个文件：**\n\n1. `act_plan.log`\n2. `clawd.log`\n3. `clawd.run.log`\n4. `feishud.log`\n5. `install_ops.log`\n",
    );
    assert_eq!(
        entries,
        vec![
            "act_plan.log",
            "clawd.log",
            "clawd.run.log",
            "feishud.log",
            "install_ops.log"
        ]
    );
}

#[test]
fn extracts_ordered_entries_from_markdown_bullet_listing_with_surrounding_prose() {
    let entries = extract_ordered_entries_from_text(
        "在 `fuzzy_top3` 目录下找到4个文件名包含 \"abcd\" 的文件：\n- `abcd_report.md`\n- `my_abcd.txt`\n- `x_abcd_log.txt`\n- `zz_abcd_backup.log`\n这些都是模糊匹配测试的 fixture 文件。",
    );
    assert_eq!(
        entries,
        vec![
            "abcd_report.md",
            "my_abcd.txt",
            "x_abcd_log.txt",
            "zz_abcd_backup.log"
        ]
    );
}

#[test]
fn persisted_followup_frame_round_trips_with_slice_and_entries() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-frame".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                serde_json::json!({
                    "action": "read_range",
                    "resolved_path": "/tmp/logs/model_io.log",
                    "mode": "tail",
                    "n": 4,
                    "excerpt": "1|a\n2|b\n3|c\n4|d"
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "看一下那个 model io log 最后 4 行，再一句话说有什么现象".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "model_io.log".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "看一下那个 model io log 最后 4 行，再一句话说有什么现象",
        &route_result,
        "a\nb\nc\nd",
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(
        frame.bound_target.as_deref(),
        Some("/tmp/logs/model_io.log")
    );
    assert_eq!(
        frame.slice_spec,
        Some(FollowupSliceSpec {
            kind: FollowupSliceKind::Tail,
            n: Some(4),
            start_line: None,
            end_line: None,
        })
    );
}

#[test]
fn config_read_field_extra_path_persists_followup_bound_target() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-config-field".to_string(),
        user_id: 3,
        chat_id: 4,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let expected_path =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/configs/app_config.toml";
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "config_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                serde_json::json!({
                    "request_id": "req-config-field",
                    "status": "ok",
                    "text": "RustClaw NL Fixture",
                    "error_text": null,
                    "extra": {
                        "action": "read_field",
                        "path": "scripts/nl_tests/fixtures/device_local/configs/app_config.toml",
                        "resolved_path": expected_path,
                        "field_path": "app.name",
                        "value": "RustClaw NL Fixture"
                    }
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "read structured field".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "fallback.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "read structured field",
        &route_result,
        "RustClaw NL Fixture",
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(frame.op_kind, FollowupOpKind::Read);
    assert_eq!(frame.bound_target.as_deref(), Some(expected_path));
}

#[test]
fn compact_listing_answer_persists_ordered_entries_for_followup() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-compact-list".to_string(),
        user_id: 11,
        chat_id: 12,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "列出 logs 目录下前 5 个文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::FileNames,
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "先列出 logs 目录下前 5 个文件名",
        &route_result,
        "列表：act_plan.log、clawd.log、clawd.run.log、feishud.log、install_ops.log。",
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(frame.op_kind, FollowupOpKind::List);
    assert_eq!(frame.bound_target.as_deref(), Some("logs"));
    assert_eq!(
        frame.ordered_entries,
        vec![
            "act_plan.log",
            "clawd.log",
            "clawd.run.log",
            "feishud.log",
            "install_ops.log"
        ]
    );
    assert_eq!(frame.selected_entry_index, None);
}

#[test]
fn read_answer_with_visible_structural_bullets_persists_ordered_entries_for_followup() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-visible-search-bullets".to_string(),
        user_id: 15,
        chat_id: 16,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let root = "/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3";
    let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "find matching entries under a known directory".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::DirectoryPurposeSummary,
            locator_hint: root.to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "find abcd under fuzzy_top3",
        &route_result,
        "在 `fuzzy_top3` 目录下找到4个文件名包含 \"abcd\" 的文件：\n- `abcd_report.md`\n- `my_abcd.txt`\n- `x_abcd_log.txt`\n- `zz_abcd_backup.log`\n这些都是模糊匹配测试的 fixture 文件。",
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(frame.op_kind, FollowupOpKind::Read);
    assert_eq!(frame.bound_target.as_deref(), Some(root));
    assert_eq!(
        frame.ordered_entries,
        vec![
            "abcd_report.md",
            "my_abcd.txt",
            "x_abcd_log.txt",
            "zz_abcd_backup.log"
        ]
    );
    assert_eq!(
        super::ordered_entry_target_at(&frame, 0).as_deref(),
        Some("/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md")
    );
}

#[test]
fn visible_listing_answer_overrides_full_journal_listing_for_followup() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-visible-list".to_string(),
        user_id: 13,
        chat_id: 14,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "list_dir".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                "act_plan.log\nclawd.log\nclawd.run.log\nfeishud.log\ninstall_ops.log\nnl_manual_qwen.run.log\nservice_ops.log\n".to_string(),
            ),
            ..Default::default()
        });
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "列出 logs 目录下前 5 个文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "先列出 logs 目录下前 5 个文件名",
        &route_result,
        "act_plan.log\nclawd.log\nclawd.run.log\nfeishud.log\ninstall_ops.log",
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(
        frame.ordered_entries,
        vec![
            "act_plan.log",
            "clawd.log",
            "clawd.run.log",
            "feishud.log",
            "install_ops.log"
        ]
    );
}

#[test]
fn fs_basic_inventory_journal_replaces_prior_ordered_entries_for_followup() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-fs-basic-list".to_string(),
        user_id: 31,
        chat_id: 32,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let now_ts = crate::now_ts_u64();
    persist_frame(
        &state,
        &task,
        &FollowupFrame {
            source_request: "先列出 document 目录下前 5 个文件名".to_string(),
            op_kind: FollowupOpKind::List,
            bound_target: Some("/home/guagua/rustclaw/document".to_string()),
            ordered_entries: vec![
                "builtin_write_smoke.txt".to_string(),
                "full_suite_trace_note.txt".to_string(),
                "hello.sh".to_string(),
            ],
            source_task_id: "older-list-task".to_string(),
            updated_at_ts: now_ts,
            expires_at_ts: now_ts + 3600,
            ..FollowupFrame::default()
        },
    )
    .expect("seed prior frame");
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                r#"{"action":"inventory_dir","names":["act_plan.log","clawd.log","clawd.run.log","clawd.test.log","clawd_manual.log"],"names_only":true,"path":"logs","resolved_path":"/home/guagua/rustclaw/logs"}"#
                    .to_string(),
            ),
            ..Default::default()
        });
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "List first 5 filenames in logs directory".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "那 logs 目录下前 5 个文件名呢",
        &route_result,
        "前 5 个文件名为 act_plan.log、clawd.log、clawd.run.log、clawd.test.log、clawd_manual.log。",
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(frame.op_kind, FollowupOpKind::List);
    assert_eq!(
        frame.bound_target.as_deref(),
        Some("/home/guagua/rustclaw/logs")
    );
    assert_eq!(
        frame.ordered_entries,
        vec![
            "act_plan.log",
            "clawd.log",
            "clawd.run.log",
            "clawd.test.log",
            "clawd_manual.log"
        ]
    );
    assert_eq!(
        super::ordered_entry_target_at(&frame, 1).as_deref(),
        Some("/home/guagua/rustclaw/logs/clawd.log")
    );
}

#[test]
fn fs_basic_wrapped_inventory_journal_persists_ordered_entries_for_followup() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-fs-basic-wrapped-list".to_string(),
        user_id: 33,
        chat_id: 34,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let listing_payload = serde_json::json!({
        "action": "inventory_dir",
        "counts": {"dirs": 0, "files": 5, "hidden": 0, "total": 5},
        "entries": [],
        "files_only": true,
        "names": [
            "act_plan.log",
            "clawd-dev.log",
            "clawd.codex.nltest.log",
            "clawd.log",
            "clawd.nl-focus.log"
        ],
        "names_by_kind": {
            "dirs": [],
            "files": [
                "act_plan.log",
                "clawd-dev.log",
                "clawd.codex.nltest.log",
                "clawd.log",
                "clawd.nl-focus.log"
            ],
            "other": []
        },
        "names_only": true,
        "path": "/home/guagua/rustclaw/logs",
        "resolved_path": "/home/guagua/rustclaw/logs",
        "sort_by": "name"
    });
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                serde_json::json!({
                    "extra": listing_payload,
                    "text": listing_payload.to_string()
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "List first 5 filenames in logs directory".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::FileNames,
            locator_hint: "/home/guagua/rustclaw/logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "List first 5 filenames in logs directory",
        &route_result,
        "",
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(frame.op_kind, FollowupOpKind::List);
    assert_eq!(
        frame.bound_target.as_deref(),
        Some("/home/guagua/rustclaw/logs")
    );
    assert_eq!(
        frame.ordered_entries,
        vec![
            "act_plan.log",
            "clawd-dev.log",
            "clawd.codex.nltest.log",
            "clawd.log",
            "clawd.nl-focus.log"
        ]
    );
    assert_eq!(
        super::ordered_entry_target_at(&frame, 1).as_deref(),
        Some("/home/guagua/rustclaw/logs/clawd-dev.log")
    );
}

#[test]
fn ordered_entry_target_does_not_duplicate_prefixed_relative_path() {
    let frame = FollowupFrame {
        op_kind: FollowupOpKind::List,
        bound_target: Some(
            "/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3".to_string(),
        ),
        ordered_entries: vec![
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md".to_string(),
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt".to_string(),
        ],
        ..FollowupFrame::default()
    };

    assert_eq!(
        super::ordered_entry_target_at(&frame, 0).as_deref(),
        Some("scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md")
    );
    assert_eq!(
        super::ordered_entry_target_at(&frame, 1).as_deref(),
        Some("scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt")
    );
}

#[test]
fn empty_generic_outcome_preserves_prior_structured_frame() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-empty-generic".to_string(),
        user_id: 41,
        chat_id: 42,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let now_ts = crate::now_ts_u64();
    let prior_frame = FollowupFrame {
        source_request: "先列出 logs 目录下前 4 个文件名".to_string(),
        op_kind: FollowupOpKind::List,
        bound_target: Some("/tmp/logs".to_string()),
        ordered_entries: vec![
            "act_plan.log".to_string(),
            "clawd.log".to_string(),
            "clawd.run.log".to_string(),
        ],
        source_task_id: "prior-list-task".to_string(),
        updated_at_ts: now_ts,
        expires_at_ts: now_ts + 3600,
        ..FollowupFrame::default()
    };
    persist_frame(&state, &task, &prior_frame).expect("seed prior frame");
    let route_result = RouteResult {
        ask_mode: crate::AskMode::respond_trace(),
        resolved_intent: "plain acknowledgement".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract::default(),
    };
    let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");

    let active_id = replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "好的",
        &route_result,
        "好的",
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");

    assert_eq!(active_id.as_deref(), Some("prior-list-task"));
    assert_eq!(frame, prior_frame);
}

#[test]
fn selected_target_turn_inherits_prior_ordered_entries_and_index() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-selected-entry".to_string(),
        user_id: 21,
        chat_id: 22,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let prior_frame = FollowupFrame {
        source_request: "先列出 logs 目录下前 4 个文件名".to_string(),
        op_kind: FollowupOpKind::List,
        bound_target: Some("logs".to_string()),
        ordered_entries: vec![
            "act_plan.log".to_string(),
            "clawd.log".to_string(),
            "clawd.run.log".to_string(),
            "feishud.log".to_string(),
        ],
        source_task_id: "older-task".to_string(),
        updated_at_ts: 1,
        expires_at_ts: crate::now_ts_u64() + 300,
        ..FollowupFrame::default()
    };
    persist_frame(&state, &task, &prior_frame).expect("persist prior frame");

    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                serde_json::json!({
                    "action": "read_range",
                    "resolved_path": "logs/clawd.log",
                    "mode": "tail",
                    "n": 2,
                    "excerpt": "x\ny"
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "看第二个最后 2 行".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "logs/clawd.log".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "看第二个最后 2 行",
        &route_result,
        "line1\nline2",
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(frame.ordered_entries, prior_frame.ordered_entries);
    assert_eq!(frame.selected_entry_index, Some(1));
    assert_eq!(frame.bound_target.as_deref(), Some("logs/clawd.log"));
}

#[test]
fn scalar_answer_matching_prior_ordered_entry_persists_selected_index() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-scalar-selected-entry".to_string(),
        user_id: 22,
        chat_id: 23,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let prior_frame = FollowupFrame {
        source_request: "list sqlite tables".to_string(),
        op_kind: FollowupOpKind::List,
        bound_target: Some(
            "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
        ),
        ordered_entries: vec![
            "orders".to_string(),
            "service_logs".to_string(),
            "users".to_string(),
        ],
        source_task_id: "older-task".to_string(),
        updated_at_ts: 1,
        expires_at_ts: crate::now_ts_u64() + 300,
        ..FollowupFrame::default()
    };
    persist_frame(&state, &task, &prior_frame).expect("persist prior frame");
    let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    let route_result = RouteResult {
        ask_mode: crate::AskMode::respond_trace(),
        resolved_intent: "select an observed ordered entry".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };

    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "select second entry",
        &route_result,
        "service_logs",
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");

    assert_eq!(frame.op_kind, FollowupOpKind::List);
    assert_eq!(frame.ordered_entries, prior_frame.ordered_entries);
    assert_eq!(frame.selected_entry_index, Some(1));
    assert_eq!(frame.bound_target, prior_frame.bound_target);
}

#[test]
fn scalar_answer_matching_prior_read_candidate_list_keeps_selection_for_next_position() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-read-candidate-selected-entry".to_string(),
        user_id: 24,
        chat_id: 25,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let root = "/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3";
    let prior_frame = FollowupFrame {
        source_request: "find abcd under fuzzy_top3".to_string(),
        op_kind: FollowupOpKind::Read,
        bound_target: Some(root.to_string()),
        ordered_entries: vec![
            "abcd_report.md".to_string(),
            "my_abcd.txt".to_string(),
            "x_abcd_log.txt".to_string(),
            "zz_abcd_backup.log".to_string(),
        ],
        source_task_id: "older-search-task".to_string(),
        updated_at_ts: 1,
        expires_at_ts: crate::now_ts_u64() + 300,
        ..FollowupFrame::default()
    };
    persist_frame(&state, &task, &prior_frame).expect("persist prior frame");
    let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    let selected = format!("{root}/my_abcd.txt");
    let route_result = RouteResult {
        ask_mode: crate::AskMode::respond_trace(),
        resolved_intent: "select an observed ordered path entry".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_hint: selected.clone(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };

    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "select second entry",
        &route_result,
        &selected,
        &[],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");

    assert_eq!(frame.op_kind, FollowupOpKind::Read);
    assert_eq!(frame.ordered_entries, prior_frame.ordered_entries);
    assert_eq!(frame.selected_entry_index, Some(1));
    assert_eq!(frame.bound_target.as_deref(), Some(selected.as_str()));
    assert_eq!(
        super::ordered_entry_target_at(&frame, 0).as_deref(),
        Some("/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md")
    );
}

#[test]
fn delivery_answer_sets_bound_target_from_file_token_and_inherits_selection() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-delivery-entry".to_string(),
        user_id: 31,
        chat_id: 32,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let prior_frame = FollowupFrame {
        source_request: "先列出 logs 目录下前 4 个文件名".to_string(),
        op_kind: FollowupOpKind::List,
        bound_target: Some("logs".to_string()),
        ordered_entries: vec![
            "act_plan.log".to_string(),
            "clawd.log".to_string(),
            "clawd.run.log".to_string(),
            "feishud.log".to_string(),
        ],
        source_task_id: "older-task".to_string(),
        updated_at_ts: 1,
        expires_at_ts: crate::now_ts_u64() + 300,
        ..FollowupFrame::default()
    };
    persist_frame(&state, &task, &prior_frame).expect("persist prior frame");
    let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "把第二个发给我".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: false,
            delivery_required: true,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "把第二个发给我",
        &route_result,
        "FILE:logs/clawd.log",
        &["FILE:logs/clawd.log".to_string()],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(frame.bound_target.as_deref(), Some("logs/clawd.log"));
    assert_eq!(frame.selected_entry_index, Some(1));
    assert_eq!(frame.ordered_entries, prior_frame.ordered_entries);
}

#[test]
fn delivery_answer_with_absolute_file_token_still_inherits_relative_listing_selection() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-delivery-absolute-entry".to_string(),
        user_id: 41,
        chat_id: 42,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let prior_frame = FollowupFrame {
        source_request: "先列出 logs 目录下前 4 个文件名".to_string(),
        op_kind: FollowupOpKind::List,
        bound_target: Some("logs".to_string()),
        ordered_entries: vec![
            "act_plan.log".to_string(),
            "clawd.log".to_string(),
            "clawd.run.log".to_string(),
            "feishud.log".to_string(),
        ],
        source_task_id: "older-task".to_string(),
        updated_at_ts: 1,
        expires_at_ts: crate::now_ts_u64() + 300,
        ..FollowupFrame::default()
    };
    persist_frame(&state, &task, &prior_frame).expect("persist prior frame");
    let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "把第二个发给我".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: false,
            delivery_required: true,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "把第二个发给我",
        &route_result,
        "FILE:/home/guagua/rustclaw/logs/clawd.log",
        &["FILE:/home/guagua/rustclaw/logs/clawd.log".to_string()],
        false,
        &journal,
    );
    let frame = load_active_followup_frame(&state, &task).expect("frame should load");
    assert_eq!(
        frame.bound_target.as_deref(),
        Some("/home/guagua/rustclaw/logs/clawd.log")
    );
    assert_eq!(frame.selected_entry_index, Some(1));
    assert_eq!(frame.ordered_entries, prior_frame.ordered_entries);
}

#[test]
fn clarify_outcome_clears_active_followup_frame() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-clarify".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    let route_result = RouteResult {
        ask_mode: crate::AskMode::clarify_trace(),
        resolved_intent: "看一下那个 README 开头，然后一句话总结".to_string(),
        needs_clarify: true,
        clarify_question: "请提供具体文件路径".to_string(),
        route_reason: "fresh_content_deictic_requires_locator".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "读一下那个 README 开头，然后一句话总结",
        &route_result,
        "请提供具体文件路径。",
        &[],
        true,
        &journal,
    );
    assert!(
        load_active_followup_frame(&state, &task).is_none(),
        "clarify outcomes should be represented by ClarifyState, not a duplicate followup frame"
    );
}

#[test]
fn clarify_outcome_with_stale_locator_hint_still_clears_followup_frame() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-followup-stale-locator".to_string(),
        user_id: 3,
        chat_id: 4,
        user_key: Some("test-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "看一下那个模型日志最后 5 行".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "memory_alias".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "/tmp/rustclaw-workspace/old/logs/model_io.log".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    replace_active_frame_from_ask_outcome(
        &state,
        &task,
        "看看那个模型日志最后 5 行",
        &route_result,
        "LOCATOR_CLARIFY_PROMPT",
        &["LOCATOR_CLARIFY_PROMPT".to_string()],
        true,
        &journal,
    );
    assert!(
        load_active_followup_frame(&state, &task).is_none(),
        "clarify outcomes should not leave a stale followup frame behind"
    );
}

use super::{
    assistant_memory_source_text, drop_execution_summaries_when_delivery_is_scalar,
    journal_has_missing_file_search_evidence, non_failure_final_status,
    resume_context_has_directory_lookup_failure, resume_context_path_batch_facts_are_missing_only,
    resume_failure_is_unbound_path_lookup_clarify_result,
    should_reinsert_execution_summaries_for_delivery, should_use_answer_route_result,
};

use serde_json::json;

fn route_result(ask_mode: crate::AskMode) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode,
        resolved_intent: "test".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    }
}

// ensure_journal_task_metrics_* tests 已搬移到 finalize/journal.rs（Stage 3.1）。

#[test]
fn non_failure_final_status_preserves_clarify_semantics() {
    assert_eq!(
        non_failure_final_status(false),
        crate::task_journal::TaskJournalFinalStatus::Success
    );
    assert_eq!(
        non_failure_final_status(true),
        crate::task_journal::TaskJournalFinalStatus::Clarify
    );
}

#[test]
fn assistant_memory_source_text_filters_execution_summary() {
    let messages = vec![
        "**执行过程**\n1. 调用命令 `pwd`\n   输出：\n```text\n/tmp\n```".to_string(),
        "最终答案".to_string(),
    ];

    assert_eq!(
        assistant_memory_source_text("最终答案", &messages),
        "最终答案"
    );
}

#[test]
fn assistant_memory_source_text_drops_execution_summary_only_answers() {
    let messages = vec![
        "**执行过程**\n1. 调用技能 `rss_fetch`\n   输出：ok".to_string(),
        "**执行过程**\n1. 调用技能 `rss_fetch`\n   输出：ok".to_string(),
    ];

    assert_eq!(
        assistant_memory_source_text(
            "**执行过程**\n1. 调用技能 `rss_fetch`\n   输出：ok",
            &messages
        ),
        ""
    );
}

#[test]
fn scalar_delivery_does_not_reinsert_execution_summary() {
    let mut route = route_result(crate::AskMode::Act {
        finalize: crate::ActFinalizeStyle::Plain,
    });
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert!(!should_reinsert_execution_summaries_for_delivery(
        &route, "1.0.0"
    ));
}

#[test]
fn scalar_delivery_drops_existing_execution_summary_messages() {
    let mut route = route_result(crate::AskMode::Act {
        finalize: crate::ActFinalizeStyle::Plain,
    });
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let mut messages = vec![
        "**执行过程**\n1. 调用工具 `fs_basic`\n   输出：ok".to_string(),
        "{\"workspace\":true}".to_string(),
    ];

    drop_execution_summaries_when_delivery_is_scalar(&route, "{\"workspace\":true}", &mut messages);

    assert_eq!(messages, vec!["{\"workspace\":true}".to_string()]);
}

#[test]
fn config_validation_delivery_drops_existing_execution_summary_messages() {
    let mut route = route_result(crate::AskMode::Act {
        finalize: crate::ActFinalizeStyle::ChatWrapped,
    });
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigValidation;
    let mut messages = vec![
        "**Execution**\n1. Called tool `config_basic`\n   Output: valid".to_string(),
        "pass".to_string(),
    ];

    drop_execution_summaries_when_delivery_is_scalar(&route, "pass", &mut messages);

    assert_eq!(messages, vec!["pass".to_string()]);
}

#[test]
fn free_delivery_keeps_execution_summary_available() {
    let mut route = route_result(crate::AskMode::Act {
        finalize: crate::ActFinalizeStyle::Plain,
    });
    route.output_contract.response_shape = crate::OutputResponseShape::Free;

    assert!(should_reinsert_execution_summaries_for_delivery(
        &route,
        "配置检查通过。"
    ));
}

#[test]
fn journal_missing_file_search_evidence_detects_zero_match_fs_search() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            skill: "fs_search".to_string(),
            output_excerpt: Some(
                json!({
                    "action": "find_name",
                    "count": 0,
                    "results": [],
                    "root": ""
                })
                .to_string(),
            ),
            ..Default::default()
        });
    assert!(journal_has_missing_file_search_evidence(Some(&journal)));
}

#[test]
fn journal_missing_file_search_evidence_detects_path_batch_facts() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            skill: "system_basic".to_string(),
            output_excerpt: Some(
                json!({
                    "action": "path_batch_facts",
                    "count": 1,
                    "facts": [{
                        "exists": false,
                        "path": "/tmp/missing.txt",
                        "error": "not found"
                    }],
                    "include_missing": true
                })
                .to_string(),
            ),
            ..Default::default()
        });
    assert!(journal_has_missing_file_search_evidence(Some(&journal)));
}

#[test]
fn journal_missing_file_search_evidence_detects_not_found_probe() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            skill: "run_cmd".to_string(),
            output_excerpt: Some("NOT_FOUND\n".to_string()),
            ..Default::default()
        });
    assert!(journal_has_missing_file_search_evidence(Some(&journal)));
}

#[test]
fn answer_route_result_overrides_initial_chat_when_execution_trace_exists() {
    let initial = route_result(crate::AskMode::direct_answer());
    let answer_route = route_result(crate::AskMode::planner_execute_chat_wrapped());
    let mut answer_journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    answer_journal.record_plan_result(&crate::PlanResult {
        plan_kind: crate::PlanKind::Single,
        goal: "inspect project".to_string(),
        planner_notes: String::new(),
        raw_plan_text: String::new(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps: Vec::new(),
    });

    assert!(should_use_answer_route_result(
        &initial,
        &answer_route,
        &answer_journal
    ));
}

#[test]
fn answer_route_result_does_not_override_chat_without_execution_trace() {
    let initial = route_result(crate::AskMode::direct_answer());
    let answer_route = route_result(crate::AskMode::planner_execute_chat_wrapped());
    let answer_journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");

    assert!(!should_use_answer_route_result(
        &initial,
        &answer_route,
        &answer_journal
    ));
}

#[test]
fn answer_route_result_overrides_initial_chat_for_clarify_journal() {
    let initial = route_result(crate::AskMode::direct_answer());
    let mut answer_route = route_result(crate::AskMode::clarify());
    answer_route.needs_clarify = true;
    answer_route.clarify_question = "Which file should I send?".to_string();
    answer_route.wants_file_delivery = true;
    answer_route.output_contract.delivery_required = true;
    answer_route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    let mut answer_journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    answer_journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);

    assert!(should_use_answer_route_result(
        &initial,
        &answer_route,
        &answer_journal
    ));
}

#[test]
fn journal_missing_file_search_evidence_detects_read_file_error_marker() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            skill: "read_file".to_string(),
            error_excerpt: Some("__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt".to_string()),
            ..Default::default()
        });
    assert!(journal_has_missing_file_search_evidence(Some(&journal)));
}

#[test]
fn missing_file_delivery_reply_uses_structured_search_evidence() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            skill: "fs_search".to_string(),
            output_excerpt: Some(
                json!({
                    "action": "find_name",
                    "count": 0,
                    "results": [],
                    "root": ""
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let answer = crate::AskReply::llm(
        "文件 `definitely_missing_named_file_rustclaw_001.txt` 未找到。".to_string(),
    )
    .with_task_journal(journal);
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "send definitely_missing_named_file_rustclaw_001.txt".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "explicit filename".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    assert!(route.wants_file_delivery);
    assert!(journal_has_missing_file_search_evidence(
        answer.task_journal.as_ref()
    ));
}

#[test]
fn missing_file_delivery_reply_uses_output_contract_file_token_even_without_wants_flag() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            skill: "fs_search".to_string(),
            output_excerpt: Some(
                json!({
                    "action": "find_name",
                    "count": 0,
                    "results": [],
                    "root": ""
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let answer = crate::AskReply::llm(
        "找不到文件 `definitely_missing_named_file_rustclaw_001.txt`。".to_string(),
    )
    .with_task_journal(journal);
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;

    assert!(super::should_use_missing_file_delivery_reply(
        &route, &answer
    ));
}

#[test]
fn resume_failure_missing_file_delivery_is_success_result() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    let resume_ctx = json!({
        "failed_step": {
            "action": "skill(run_cmd)",
            "error": "__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt"
        },
        "remaining_actions": []
    });

    assert!(super::resume_failure_is_missing_file_delivery_result(
        &route,
        "I couldn't send the requested file because it doesn't exist at the path `/tmp/missing.txt`.",
        &resume_ctx
    ));
}

#[test]
fn resume_failure_unbound_path_lookup_is_clarify_result() {
    let mut route = route_result(crate::AskMode::planner_execute_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "case_only/report.md".to_string();
    let resume_ctx = json!({
        "completed_messages": [
            "subtask#1 skill(system_basic): success\n{\"action\":\"path_batch_facts\",\"count\":1,\"facts\":[{\"error\":\"not found\",\"exists\":false,\"kind\":\"missing\",\"path\":\"case_only/report.md\"}],\"include_missing\":true}"
        ],
        "failed_step": {
            "action": "skill(fs_search)",
            "error": "read_dir failed",
            "structured_error": {
                "skill": "fs_search",
                "error_kind": "unknown",
                "error_text": "read_dir failed"
            }
        },
        "remaining_actions": []
    });

    assert!(resume_context_path_batch_facts_are_missing_only(
        &resume_ctx
    ));
    assert!(resume_failure_is_unbound_path_lookup_clarify_result(
        &route,
        &resume_ctx
    ));
}

#[test]
fn resume_failure_unbound_directory_lookup_is_clarify_result_without_path_batch() {
    let mut route = route_result(crate::AskMode::planner_execute_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "case_only/report.md".to_string();
    let resume_ctx = json!({
        "completed_messages": [],
        "failed_step": {
            "action": "skill(fs_search)",
            "error": "read_dir failed: No such file or directory (os error 2)",
            "structured_error": {
                "skill": "fs_search",
                "error_kind": "unknown",
                "error_text": "read_dir failed: No such file or directory (os error 2)"
            }
        },
        "remaining_actions": []
    });

    assert!(resume_context_has_directory_lookup_failure(&resume_ctx));
    assert!(resume_failure_is_unbound_path_lookup_clarify_result(
        &route,
        &resume_ctx
    ));
}

#[test]
fn resume_failure_unbound_path_lookup_does_not_reclassify_delivery() {
    let mut route = route_result(crate::AskMode::planner_execute_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    let resume_ctx = json!({
        "completed_messages": [
            "subtask#1 skill(system_basic): success\n{\"action\":\"path_batch_facts\",\"count\":1,\"facts\":[{\"exists\":false,\"path\":\"missing.txt\"}],\"include_missing\":true}"
        ],
        "failed_step": {
            "action": "skill(fs_search)",
            "error": "read_dir failed"
        },
        "remaining_actions": []
    });

    assert!(!resume_failure_is_unbound_path_lookup_clarify_result(
        &route,
        &resume_ctx
    ));
}

#[test]
fn resume_failure_structured_service_status_is_success_result() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    let resume_ctx = json!({
        "failed_step": {
            "action": "skill(service_control)",
            "error": "no matching service found for the given target",
            "structured_error": {
                "skill": "service_control",
                "error_kind": "not_found",
                "error_text": "no matching service found for the given target",
                "service_name": "definitely_missing_rustclaw_demo",
                "platform": "linux",
                "manager_type": "unknown"
            }
        },
        "remaining_actions": []
    });

    assert!(super::resume_failure_is_structured_service_status_result(
        &route,
        &resume_ctx
    ));

    let messages = super::resume_context_execution_summary_messages(&resume_ctx, false);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("no matching service found"));
    assert!(!messages[0].contains("__RC_SKILL_ERROR__"));
}

#[test]
fn resume_failure_execution_failed_step_is_success_answer_with_remaining_actions() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExecutionFailedStep;
    let resume_ctx = json!({
        "failed_step": {
            "action": "skill(run_cmd)",
            "error": "command failed with exit code 1; stderr: cat: /definitely_missing_rustclaw_contract_case: No such file or directory (os error 2)",
            "structured_error": {
                "skill": "run_cmd",
                "error_kind": "nonzero_exit",
                "error_text": "Command failed with exit code 1\nstderr:\ncat: /definitely_missing_rustclaw_contract_case: No such file or directory (os error 2)",
                "platform": "linux",
                "extra": {
                    "command": "cat /definitely_missing_rustclaw_contract_case",
                    "exit_code": 1,
                    "stderr": "cat: /definitely_missing_rustclaw_contract_case: No such file or directory (os error 2)\n"
                }
            }
        },
        "remaining_actions": [
            {"type": "call_skill", "skill": "log_analyze"},
            {"type": "synthesize_answer"}
        ]
    });

    let answer = super::resume_failure_execution_failed_step_answer(&route, &resume_ctx, false)
        .expect("execution-failed-step answer");

    assert!(answer.contains("cat /definitely_missing_rustclaw_contract_case"));
    assert!(answer.contains("退出码为 1"));
    assert!(answer.contains("No such file or directory"));
    assert!(!answer.contains("继续"));
    assert!(!answer.contains("暂停"));
}

#[test]
fn resume_context_execution_summary_uses_failed_step() {
    let resume_ctx = json!({
        "failed_step": {
            "action": "skill(run_cmd)",
            "error": "ls: cannot access '/tmp/missing.txt': No such file or directory"
        },
        "remaining_actions": []
    });

    let messages = super::resume_context_execution_summary_messages(&resume_ctx, false);

    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("**执行过程**"));
    assert!(messages[0].contains("skill(run_cmd)"));
    assert!(messages[0].contains("No such file or directory"));
}

use serde_json::{json, Value};

use super::{
    delivery_payload_consistent, evidence_coverage_for_route, observed_evidence_from_output,
    TaskJournal, TaskJournalFinalStatus, TaskJournalFinalizerFallback, TaskJournalFinalizerStage,
    TaskJournalFinalizerSummary, TaskJournalRoundTrace, TaskJournalStepTrace,
    TaskJournalVerifyIssue, TaskJournalVerifySummary,
};

fn route_for_semantic(semantic_kind: crate::OutputSemanticKind) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            semantic_kind,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        },
    }
}

#[test]
fn summary_json_includes_finalizer_and_task_metrics() {
    let mut journal = TaskJournal::for_task("task-1", "ask", "总结 README");
    journal.record_route_result(&crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "不要用现有技能，先规划一个新能力".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "structured self_extension contract".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            self_extension: crate::SelfExtensionContract {
                mode: crate::SelfExtensionMode::PermanentExtension,
                trigger: crate::SelfExtensionTrigger::ExplicitUserRequest,
                execute_now: true,
            },
            ..Default::default()
        },
    });
    journal.record_finalizer_summary(TaskJournalFinalizerSummary {
        stage: Some(TaskJournalFinalizerStage::General),
        disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
        fallback: Some(TaskJournalFinalizerFallback::RawText),
        parsed: false,
        contract_ok: false,
        completion_ok: None,
        grounded_ok: None,
        format_ok: None,
        needs_clarify: None,
        confidence: None,
        used_evidence_ids_count: 2,
        evidence_quotes_count: 0,
    });
    journal.record_delivery_consistent(true);
    journal.record_llm_calls_per_task(3);
    let mut by_prompt = std::collections::HashMap::new();
    by_prompt.insert(
        "normalizer".to_string(),
        crate::LlmPromptBucket {
            count: 1,
            elapsed_ms: 42,
            prompt_truncation_count: 1,
            prompt_bytes_before_max: Some(157_037),
            prompt_bytes_budget_min: Some(125_200),
            prompt_bytes_after_max: Some(125_180),
            prompt_truncated_bytes_total: 31_857,
        },
    );
    journal.record_llm_by_prompt(by_prompt);
    let summary = journal.to_summary_json();

    assert_eq!(
        summary.get("task_id").and_then(Value::as_str),
        Some("task-1")
    );
    assert_eq!(
        summary
            .get("finalizer_summary")
            .and_then(|v| v.get("stage"))
            .and_then(Value::as_str),
        Some("general")
    );
    assert_eq!(
        summary
            .get("finalizer_summary")
            .and_then(|v| v.get("final_answer_shape"))
            .and_then(Value::as_str),
        Some("free")
    );
    assert_eq!(
        summary
            .get("finalizer_summary")
            .and_then(|v| v.get("final_answer_shape_class"))
            .and_then(Value::as_str),
        Some("freeform")
    );
    assert_eq!(
        summary
            .get("finalizer_summary")
            .and_then(|v| v.get("coarse_response_shape"))
            .and_then(Value::as_str),
        Some("free")
    );
    assert_eq!(
        summary
            .get("finalizer_summary")
            .and_then(|v| v.get("allows_model_language"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        summary
            .get("finalizer_summary")
            .and_then(|v| v.get("evidence_coverage_complete"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        summary
            .get("task_metrics")
            .and_then(|v| v.get("used_evidence_ids_count"))
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        summary
            .get("task_metrics")
            .and_then(|v| v.get("delivery_consistent"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        summary
            .get("task_metrics")
            .and_then(|v| v.get("llm_calls_per_task"))
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        summary
            .get("task_metrics")
            .and_then(|v| v.get("prompt_truncation_count"))
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        summary
            .get("task_metrics")
            .and_then(|v| v.get("by_prompt"))
            .and_then(|v| v.get("normalizer"))
            .and_then(|v| v.get("prompt_bytes_before_max"))
            .and_then(Value::as_u64),
        Some(157_037)
    );
    assert_eq!(
        summary
            .get("route_result")
            .and_then(|v| v.get("self_extension"))
            .and_then(|v| v.get("mode"))
            .and_then(Value::as_str),
        Some("permanent_extension")
    );
    assert_eq!(
        summary
            .get("route_result")
            .and_then(|v| v.get("self_extension"))
            .and_then(|v| v.get("trigger"))
            .and_then(Value::as_str),
        Some("explicit_user_request")
    );
    assert_eq!(
        summary
            .get("route_result")
            .and_then(|v| v.get("self_extension"))
            .and_then(|v| v.get("execute_now"))
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn delivery_payload_consistency_uses_last_non_empty_message() {
    assert!(delivery_payload_consistent(
        "最终结果",
        &["".to_string(), "最终结果".to_string()]
    ));
    assert!(!delivery_payload_consistent(
        "最终结果",
        &["中间消息".to_string(), "别的结果".to_string()]
    ));
    assert!(delivery_payload_consistent(
        "第一段\n\n第二段",
        &[
            "**执行过程**\n1. 调用技能 `read_file`".to_string(),
            "第一段".to_string(),
            "第二段".to_string()
        ]
    ));
    assert!(delivery_payload_consistent("任意文本", &[]));
}

#[test]
fn trace_json_includes_execution_recipe_summary() {
    let mut journal = TaskJournal::for_task("task-2", "ask", "修复并验证");
    journal.rounds.push(super::TaskJournalRoundTrace {
        round_no: 1,
        goal: "repair service".to_string(),
        execution_recipe_summary: Some(
            "kind=ops_closed_loop profile=code_change target_scope=external_workspace phase=validate inspect_first=true validation_required=true repair_count=1 max_repairs=3 saw_inspect=true saw_mutation=true saw_validation=false saw_external_target=true saw_greenfield_creation=false".to_string(),
        ),
        ..Default::default()
    });

    let summary = journal.to_summary_json();
    let trace = journal.to_trace_json();

    assert_eq!(
        summary
            .get("latest_execution_recipe_summary")
            .and_then(Value::as_str),
        Some(
            "kind=ops_closed_loop profile=code_change target_scope=external_workspace phase=validate inspect_first=true validation_required=true repair_count=1 max_repairs=3 saw_inspect=true saw_mutation=true saw_validation=false saw_external_target=true saw_greenfield_creation=false"
        )
    );
    assert_eq!(
        trace.get("rounds")
            .and_then(Value::as_array)
            .and_then(|rounds| rounds.first())
            .and_then(|round| round.get("execution_recipe_summary"))
            .and_then(Value::as_str),
        Some(
            "kind=ops_closed_loop profile=code_change target_scope=external_workspace phase=validate inspect_first=true validation_required=true repair_count=1 max_repairs=3 saw_inspect=true saw_mutation=true saw_validation=false saw_external_target=true saw_greenfield_creation=false"
        )
    );
    assert_eq!(trace.get("task_id").and_then(Value::as_str), Some("task-2"));
    assert_eq!(trace.get("kind").and_then(Value::as_str), Some("ask"));
    let log = journal.to_log_json();
    assert_eq!(log.get("task_id").and_then(Value::as_str), Some("task-2"));
    assert_eq!(log.get("kind").and_then(Value::as_str), Some("ask"));
}

#[test]
fn trace_json_includes_memory_trace() {
    let mut journal = TaskJournal::for_task("task-memory", "ask", "根据记忆回复");
    journal.record_memory_trace(json!({
        "stage": "execution",
        "use_policy": "task_relevant",
        "recalled": [
            {
                "source_kind": "memory_fact",
                "source_ref": "memory_fact:1",
                "score": 0.91,
                "included": true,
                "reason": "task_relevant"
            }
        ],
        "budget": {
            "max_chars": 4000,
            "used_chars": 128
        }
    }));

    let summary = journal.to_summary_json();
    let trace = journal.to_trace_json();

    assert_eq!(
        summary
            .get("memory_trace")
            .and_then(|v| v.get("use_policy"))
            .and_then(Value::as_str),
        Some("task_relevant")
    );
    assert_eq!(
        trace
            .get("memory_trace")
            .and_then(|v| v.get("recalled"))
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn attach_to_result_caps_large_trace_and_preserves_contract_summary_fields() {
    let mut journal = TaskJournal::for_task("task-large-trace", "ask", "列出文件名");
    journal.record_route_result(&route_for_semantic(crate::OutputSemanticKind::FileNames));
    for idx in 0..300 {
        journal.push_task_observation(json!({
            "idx": idx,
            "payload": "x".repeat(1200),
            "items": (0..40).map(|value| json!({
                "value": value,
                "note": "y".repeat(1200),
            })).collect::<Vec<_>>(),
        }));
    }

    let result = journal.attach_to_result(json!({"text": "ok"}));
    let trace = result
        .pointer("/task_journal/trace")
        .expect("trace should be attached");
    let serialized = serde_json::to_vec(trace).expect("trace should serialize");
    assert!(
        serialized.len() <= super::MAX_RESULT_TRACE_BYTES,
        "stored trace should be bounded, got {} bytes",
        serialized.len()
    );
    assert_eq!(
        trace
            .pointer("/trace_storage/truncated")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(
        trace
            .pointer("/trace_storage/original_bytes")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            > trace
                .pointer("/trace_storage/stored_bytes")
                .and_then(Value::as_u64)
                .unwrap_or_default()
    );
    assert!(trace
        .pointer("/trace_storage/original_hash")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .starts_with("fnv64:"));
    assert!(
        trace
            .get("contract_matrix")
            .and_then(|value| value.get("contract_match"))
            .and_then(Value::as_str)
            .is_some(),
        "contract snapshot should survive trace compaction"
    );
    assert!(
        trace.get("evidence_coverage").is_some(),
        "evidence coverage should survive trace compaction"
    );
    assert!(
        trace
            .get("task_observations")
            .and_then(Value::as_array)
            .map(|items| items.len() <= super::MAX_RESULT_TRACE_ARRAY_ITEMS)
            .unwrap_or(false),
        "task observations should be truncated by count"
    );
}

#[test]
fn trace_json_includes_verifier_issue_failure_attribution() {
    let mut journal = TaskJournal::for_task("task-verifier-attribution", "ask", "列文件");
    journal.rounds.push(TaskJournalRoundTrace {
        round_no: 1,
        goal: "list files".to_string(),
        verify_result: Some(TaskJournalVerifySummary {
            mode: crate::verifier::VerifyMode::ObserveOnly,
            approved: true,
            blocked_reason: None,
            shadow_blocked_reason: Some("contract action rejected".to_string()),
            needs_confirmation: false,
            issues: vec![TaskJournalVerifyIssue {
                step_id: "step_1".to_string(),
                kind: crate::verifier::VerifyIssueKind::ContractActionRejected,
                detail: "action rejected".to_string(),
            }],
        }),
        ..Default::default()
    });

    let trace = journal.to_trace_json();
    let issue = trace
        .get("rounds")
        .and_then(Value::as_array)
        .and_then(|rounds| rounds.first())
        .and_then(|round| round.get("verify_result"))
        .and_then(|verify| verify.get("issues"))
        .and_then(Value::as_array)
        .and_then(|issues| issues.first())
        .expect("verify issue should be present");
    assert_eq!(
        issue.get("kind").and_then(Value::as_str),
        Some("ContractActionRejected")
    );
    assert_eq!(
        issue.get("failure_attribution").and_then(Value::as_str),
        Some("contract_gap")
    );
}

#[test]
fn final_stop_signal_records_budget_failure_attribution() {
    let mut journal = TaskJournal::for_task("task-budget", "ask", "继续修复直到通过");
    journal.record_final_status(TaskJournalFinalStatus::Failure);
    journal.record_final_stop_signal("recipe_repair_budget_exhausted");

    let summary = journal.to_summary_json();
    let trace = journal.to_trace_json();

    assert_eq!(
        summary.get("final_stop_signal").and_then(Value::as_str),
        Some("recipe_repair_budget_exhausted")
    );
    assert_eq!(
        summary
            .get("final_failure_attribution")
            .and_then(Value::as_str),
        Some("budget_exhausted")
    );
    assert_eq!(
        trace
            .get("final_failure_attribution")
            .and_then(Value::as_str),
        Some("budget_exhausted")
    );
}

#[test]
fn trace_json_distinguishes_requested_tool_from_executed_skill() {
    let mut journal = TaskJournal::for_task("task-3", "ask", "列出当前目录前三项");
    let plan = crate::PlanResult {
        goal: "list workspace".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "call_skill".to_string(),
            skill: "system_basic".to_string(),
            args: json!({"action": "inventory_dir", "path": "."}),
            depends_on: Vec::new(),
            why: "list directory".to_string(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text:
            r#"{"steps":[{"type":"call_tool","tool":"list_dir","args":{"path":".","limit":3}}]}"#
                .to_string(),
    };
    journal.rounds.push(super::TaskJournalRoundTrace {
        round_no: 1,
        goal: "list workspace".to_string(),
        plan_result: Some(plan),
        ..Default::default()
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("README.md\nCargo.toml".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let step = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .expect("step result should be present");
    assert_eq!(
        step.get("requested_action_type").and_then(Value::as_str),
        Some("call_tool")
    );
    assert_eq!(
        step.get("requested_capability").and_then(Value::as_str),
        Some("list_dir")
    );
    let plan_action_ref = trace
        .pointer("/rounds/0/plan_result/steps/0/action_ref")
        .and_then(Value::as_str);
    assert_eq!(plan_action_ref, Some("system_basic.inventory_dir"));
    assert_eq!(
        step.get("requested_action_ref").and_then(Value::as_str),
        Some("system_basic.inventory_dir")
    );
    assert_eq!(
        step.get("executed_skill").and_then(Value::as_str),
        Some("system_basic")
    );
    assert_eq!(
        step.get("skill").and_then(Value::as_str),
        Some("system_basic")
    );
}

#[test]
fn trace_json_compacts_plan_action_ref_to_contract_action() {
    let mut journal = TaskJournal::for_task("task-service", "ask", "check service");
    journal.record_route_result(&crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "check clawd service".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            semantic_kind: crate::OutputSemanticKind::ServiceStatus,
            ..Default::default()
        },
    });
    let plan = crate::PlanResult {
        goal: "check service".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "call_skill".to_string(),
            skill: "process_basic".to_string(),
            args: json!({"action": "ps", "filter": "clawd"}),
            depends_on: Vec::new(),
            why: "inspect process".to_string(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    };
    journal.rounds.push(super::TaskJournalRoundTrace {
        round_no: 1,
        goal: "check service".to_string(),
        plan_result: Some(plan),
        ..Default::default()
    });

    let trace = journal.to_trace_json();
    let plan_action_ref = trace
        .pointer("/rounds/0/plan_result/steps/0/action_ref")
        .and_then(Value::as_str);
    assert_eq!(plan_action_ref, Some("process_basic"));
    assert_eq!(
        trace
            .pointer("/rounds/0/plan_result/steps/0/raw_action_ref")
            .and_then(Value::as_str),
        Some("process_basic.ps")
    );
    assert_eq!(
        trace
            .pointer("/rounds/0/plan_result/steps/0/matrix_action_ref")
            .and_then(Value::as_str),
        Some("process_basic")
    );
}

#[test]
fn trace_json_includes_contract_policy_for_contract_rejection() {
    let mut journal = TaskJournal::for_task("task-contract", "ask", "列出文件名");
    let err = crate::skills::structured_skill_error_from_parts(
        "run_cmd",
        "contract_action_rejected",
        "action `run_cmd` is rejected by contract `file_names`",
        None,
        Some(json!({
            "failure_attribution": "contract_gap",
            "decision": "rejected_not_allowed",
            "action": "run_cmd",
            "contract_match": "file_names",
            "required_evidence": ["candidates"],
            "preferred_actions": ["fs_basic.list_dir"],
            "evidence_expression": {"all_of": ["candidates"], "one_of": [], "any_of": [], "negative_evidence": []},
            "final_answer_shape": "name_list",
        })),
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(err),
        started_at: 1,
        finished_at: 1,
    });

    let trace = journal.to_trace_json();
    let step = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .expect("step result should be present");

    assert_eq!(
        step.get("error_kind").and_then(Value::as_str),
        Some("contract_action_rejected")
    );
    assert_eq!(
        step.get("failure_attribution").and_then(Value::as_str),
        Some("contract_gap")
    );
    assert_eq!(
        step.get("contract_policy")
            .and_then(|value| value.get("decision"))
            .and_then(Value::as_str),
        Some("rejected_not_allowed")
    );
    assert_eq!(
        step.get("contract_policy")
            .and_then(|value| value.get("contract_match"))
            .and_then(Value::as_str),
        Some("file_names")
    );
    assert_eq!(
        step.get("contract_policy")
            .and_then(|value| value.get("evidence_expression"))
            .and_then(|value| value.get("all_of"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str),
        Some("candidates")
    );
}

#[test]
fn trace_json_infers_failure_attribution_from_standard_error_kind() {
    for (error_kind, expected) in [
        ("schema_validation_failed", "schema_error"),
        ("provider_retryable_response", "provider_error"),
        ("channel_send_failed", "delivery_error"),
    ] {
        let mut journal = TaskJournal::for_task(
            format!("task-{error_kind}"),
            "ask",
            "trigger structured error",
        );
        let err = crate::skills::structured_skill_error_from_parts(
            "runtime",
            error_kind,
            "structured failure",
            None,
            None,
        );
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "runtime".to_string(),
            status: crate::executor::StepExecutionStatus::Error,
            output: None,
            error: Some(err),
            started_at: 1,
            finished_at: 1,
        });

        let trace = journal.to_trace_json();
        let step = trace
            .get("step_results")
            .and_then(Value::as_array)
            .and_then(|steps| steps.first())
            .expect("step result should be present");

        assert_eq!(
            step.get("error_kind").and_then(Value::as_str),
            Some(error_kind)
        );
        assert_eq!(
            step.get("failure_attribution").and_then(Value::as_str),
            Some(expected)
        );
    }
}

#[test]
fn final_error_text_records_failure_attribution() {
    for (error_text, expected) in [
        (
            "provider=minimax failed: timeout while reading response",
            "provider_error",
        ),
        (
            "direct_answer_gate schema_validation_failed task_id=t1 err=missing field",
            "schema_error",
        ),
        (
            "wechat send status=500 body={\"err\":\"bad gateway\"}",
            "delivery_error",
        ),
    ] {
        let mut journal =
            TaskJournal::for_task(format!("task-{expected}"), "ask", "trigger final error");
        journal.record_final_failure_attribution_from_error(error_text);
        journal.record_final_status(TaskJournalFinalStatus::Failure);

        assert_eq!(
            journal
                .to_trace_json()
                .get("final_failure_attribution")
                .and_then(Value::as_str),
            Some(expected)
        );
    }
}

#[test]
fn trace_json_includes_redacted_observed_evidence_for_json_output() {
    let mut journal = TaskJournal::for_task("task-observed-evidence", "ask", "读取配置");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_fields",
                "count": 2,
                "extra": {
                    "field_value": "enabled",
                    "api_key": "sk-test-super-secret-token-value-1234567890"
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let observed = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .expect("observed evidence should be present");
    assert_eq!(observed.get("format").and_then(Value::as_str), Some("json"));
    assert_eq!(
        observed.pointer("/extractor/kind").and_then(Value::as_str),
        Some("structured_json")
    );
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("config_basic.read_fields.structured_json_v1")
    );
    assert_eq!(
        observed
            .pointer("/extractor/source_action_ref")
            .and_then(Value::as_str),
        Some("config_basic.read_fields")
    );
    assert_eq!(
        observed
            .pointer("/extractor/strict_shape_eligible")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        observed
            .pointer("/extractor/fallback")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        observed
            .pointer("/extractor/provider_safety/provider_evidence_view")
            .and_then(Value::as_str),
        Some("provider_safe_redacted")
    );
    assert_eq!(
        observed
            .pointer("/extractor/provider_safety/raw_excerpt_policy")
            .and_then(Value::as_str),
        Some("no_full_raw_excerpt")
    );
    assert_eq!(
        observed
            .pointer("/extractor/provider_safety/sensitive_field_policy")
            .and_then(Value::as_str),
        Some("redact_sensitive_keys_and_secret_like_values")
    );
    assert_eq!(
        observed
            .pointer("/extractor/observation_source")
            .and_then(Value::as_str),
        Some("step_output")
    );
    assert_eq!(
        observed.get("storage").and_then(Value::as_str),
        Some("redacted_excerpt_hash")
    );

    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.field_value")
            && item.get("excerpt").and_then(Value::as_str) == Some("enabled")
            && item.get("hash").and_then(Value::as_str).is_some()
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.api_key")
            && item.get("redacted").and_then(Value::as_bool) == Some(true)
            && item.get("excerpt").is_none()
    }));
    assert!(!serde_json::to_string(observed)
        .expect("serialize observed evidence")
        .contains("sk-test-super-secret-token-value"));
}

#[test]
fn image_generate_extra_outputs_path_counts_as_structured_path_evidence() {
    let mut journal = TaskJournal::for_task("task-image-extra-evidence", "ask", "生成图片");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "image_generate".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "text": "FILE:/tmp/rustclaw-image.png",
                "extra": {
                    "provider": "local_fallback",
                    "model": "local-placeholder",
                    "model_kind": "local_fallback",
                    "outputs": [{
                        "type": "image_file",
                        "path": "/tmp/rustclaw-image.png"
                    }]
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let observed = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .expect("observed evidence should be present");
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("image_generate.structured_json_v1")
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.outputs[0].path")
            && item.get("excerpt").and_then(Value::as_str) == Some("/tmp/rustclaw-image.png")
    }));

    let route = route_for_semantic(crate::OutputSemanticKind::GeneratedFileDelivery);
    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.observed_canonical.contains("path"));
    assert!(
        coverage.missing_evidence.is_empty(),
        "{:?}",
        coverage.missing_evidence
    );
}

#[test]
fn rss_fetch_extra_field_value_counts_as_structured_rss_evidence() {
    let mut journal = TaskJournal::for_task("task-rss-extra-evidence", "ask", "抓取 RSS 新闻");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "rss_fetch".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "text": "sources_ok=2 sources_failed=0 items=3\n1. Example item",
                "extra": {
                    "schema_version": 1,
                    "action": "latest",
                    "category": "general",
                    "source_count": 2,
                    "sources_ok": 2,
                    "sources_failed": 0,
                    "item_count": 3,
                    "field_value": {
                        "sources_ok": 2,
                        "sources_failed": 0,
                        "items": 3,
                        "titles": [
                            "Example item",
                            "Second item",
                            "Third item"
                        ]
                    },
                    "items": [{
                        "title": "Example item",
                        "link": "https://example.com/news/1",
                        "source_host": "example.com"
                    }]
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let observed = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .expect("observed evidence should be present");
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("rss_fetch.structured_json_v1")
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.field_value")
            && item
                .get("keys")
                .and_then(Value::as_array)
                .is_some_and(|keys| keys.iter().any(|key| key.as_str() == Some("titles")))
    }));

    let route = route_for_semantic(crate::OutputSemanticKind::RssNewsFetch);
    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(
        coverage.missing_evidence.is_empty(),
        "{:?}",
        coverage.missing_evidence
    );
}

#[test]
fn trace_json_includes_observed_evidence_for_text_output() {
    let mut journal = TaskJournal::for_task("task-observed-text", "ask", "运行命令");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("first line\nsecond line".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let observed = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .expect("observed evidence should be present");
    assert_eq!(observed.get("format").and_then(Value::as_str), Some("text"));
    assert_eq!(
        observed.pointer("/extractor/kind").and_then(Value::as_str),
        Some("text_legacy")
    );
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("run_cmd.text_legacy_v1")
    );
    assert_eq!(
        observed
            .pointer("/extractor/source_action_ref")
            .and_then(Value::as_str),
        Some("run_cmd")
    );
    assert_eq!(
        observed
            .pointer("/extractor/fallback")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert!(observed
        .get("items")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("field").and_then(Value::as_str) == Some("text_excerpt")
                    && item.get("excerpt").and_then(Value::as_str) == Some("first line second line")
                    && item.get("hash").and_then(Value::as_str).is_some()
            })
        }));
}

#[test]
fn explicit_extractor_registry_canonicalizes_virtual_tool_outputs() {
    let mut journal = TaskJournal::for_task("task-explicit-extractor", "ask", "列出文件");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["Cargo.toml"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let extractor = trace
        .pointer("/step_results/0/observed_evidence/extractor")
        .expect("observed evidence extractor");
    assert_eq!(
        extractor.get("extractor_ref").and_then(Value::as_str),
        Some("fs_basic.list_dir.structured_json_v1")
    );
    assert_eq!(
        extractor.get("source_action_ref").and_then(Value::as_str),
        Some("fs_basic.list_dir")
    );
    assert!(extractor
        .get("provided_evidence")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("candidates"))));
    assert!(extractor
        .get("provided_evidence")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.as_str() == Some("modified_ts"))));
    assert!(extractor
        .get("provided_evidence")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("sort_by"))));
}

#[test]
fn matrix_admitted_external_marker_enables_strict_structured_evidence() {
    let mut journal =
        TaskJournal::for_task("task-external-admission-evidence", "ask", "external count");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "external_counter".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "count",
                "text": "3",
                "extra": {
                    "action": "count",
                    "count": 3,
                    "results": ["a", "b", "c"]
                },
                "_matrix_admission": {
                    "schema_version": 1,
                    "source": "skills_registry",
                    "skill": "external_counter",
                    "eligible": true,
                    "extractor_kind": "structured_json",
                    "required_extra_fields": ["extra.count"]
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let observed = trace
        .pointer("/step_results/0/observed_evidence")
        .expect("observed evidence");
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("matrix_admitted_external.structured_json_v1")
    );
    assert_eq!(
        observed
            .pointer("/extractor/strict_shape_eligible")
            .and_then(Value::as_bool),
        Some(true)
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.count")
            && item.get("excerpt").and_then(Value::as_str) == Some("3")
    }));
    assert!(!items.iter().any(|item| {
        item.get("field")
            .and_then(Value::as_str)
            .is_some_and(|field| field.starts_with("_matrix_admission"))
    }));
}

#[test]
fn text_observed_evidence_extracts_count_path_and_candidates() {
    let archive_listing = "exit=0\nArchive: /tmp/test.zip\n  Length Name\n  22 notes.txt\n  20 nested/config.ini\n  42 2 files";
    let observed = observed_evidence_from_output(Some(archive_listing))
        .expect("text evidence should be present");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("count")
            && item.get("excerpt").and_then(Value::as_str) == Some("2")
    }));

    let observed = observed_evidence_from_output(Some("/home/guagua/rustclaw/Cargo.toml"))
        .expect("path evidence should be present");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("path evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("path")
            && item.get("source").and_then(Value::as_str) == Some("text_output.extractor")
    }));
    let observed = observed_evidence_from_output(Some(
        "written 40 bytes to /home/guagua/rustclaw/document/pwd_line.txt",
    ))
    .expect("path token evidence should be present");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("path token evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("path")
            && item.get("excerpt").and_then(Value::as_str)
                == Some("/home/guagua/rustclaw/document/pwd_line.txt")
    }));
    let observed = observed_evidence_from_output(Some(
        "archive_path=/home/guagua/rustclaw/tmp/bundle.zip\nexit=0\n  adding: /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md (deflated 32%)",
    ))
    .expect("labeled archive path evidence should be present");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("labeled path evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("path")
            && item.get("excerpt").and_then(Value::as_str)
                == Some("/home/guagua/rustclaw/tmp/bundle.zip")
    }));

    let mut git_journal =
        TaskJournal::for_task("task-text-git-subject", "ask", "latest git subject");
    git_journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("exit=0\n09342a6a fix: expose nl execution and locator flows\n".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    let observed = git_journal
        .to_trace_json()
        .pointer("/step_results/0/observed_evidence")
        .cloned()
        .expect("git subject evidence should be present");
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("git_basic.text_legacy_v1")
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("git subject evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("subject")
            && item.get("excerpt").and_then(Value::as_str)
                == Some("fix: expose nl execution and locator flows")
    }));

    let mut git_json_journal =
        TaskJournal::for_task("task-json-git-subjects", "ask", "write a release note");
    git_json_journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "log",
                    "exit_code": 0,
                    "output": "exit=0\nf77577da Tighten NL verifier recovery\na30c49fb Tighten grounded channel setup rewrites\n",
                    "raw_action": "log",
                    "subcommand": "log"
                },
                "text": "exit=0\nf77577da Tighten NL verifier recovery\na30c49fb Tighten grounded channel setup rewrites\n"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    let observed = git_json_journal
        .to_trace_json()
        .pointer("/step_results/0/observed_evidence")
        .cloned()
        .expect("structured git log evidence should be present");
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("git_basic.structured_json_v1")
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("structured git log evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("content_excerpt")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|excerpt| excerpt.contains("Tighten NL verifier recovery"))
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("subject")
            && item.get("excerpt").and_then(Value::as_str) == Some("Tighten NL verifier recovery")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("git_subjects")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|excerpt| excerpt.contains("Tighten grounded channel setup rewrites"))
    }));

    let mut journal = TaskJournal::for_task("task-text-candidates", "ask", "列出文件名");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("Cargo.toml\nREADME.md".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete());
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("count"));

    let observed = observed_evidence_from_output(Some(".git\nREADME.md\n.env\nsrc\n"))
        .expect("hidden list evidence should be present");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("hidden list evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("hidden_count")
            && item.get("excerpt").and_then(Value::as_str) == Some("2")
    }));
}

#[test]
fn generic_path_content_list_dir_candidates_satisfy_directory_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-generic-path-directory",
        "ask",
        "summarize selected directory entries",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "list_dir",
                "path": "prompts/schemas",
                "count": 1,
                "entries": [
                    {
                        "name": "intent_normalizer.schema.json",
                        "kind": "file",
                        "size_bytes": 13124
                    }
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("candidates"));
}

#[test]
fn generic_path_content_find_entries_result_path_satisfies_path_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-generic-path-find-entry",
        "ask",
        "return the matching path",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "find_name",
                "root": "scripts/nl_tests/fixtures/locator_smart/stem_unique",
                "patterns": ["abcd"],
                "count": 1,
                "results": ["scripts/nl_tests/fixtures/locator_smart/stem_unique/ABCD.txt"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("candidates"));
}

#[test]
fn file_names_content_search_paths_satisfy_candidate_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-file-names-grep-candidates",
        "ask",
        "search workspace content and list matching files",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::FileNames);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    journal.record_route_result(&route);
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        json!({
            "action": "grep_text",
            "query": "FirstLayerDecision",
            "count": 2,
            "match_count": 3,
            "matches": [
                {"path": "README.md", "line": 54, "text": "FirstLayerDecision"},
                {"path": "crates/clawd/src/intent_router.rs", "line": 14, "text": "FirstLayerDecision"}
            ]
        })
        .to_string(),
    ));

    let coverage = evidence_coverage_for_route(&route, &journal);

    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("content_match"));
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage
        .observed_extractors
        .contains("fs_basic.grep_text.structured_json_v1"));
}

#[test]
fn file_paths_content_search_paths_satisfy_candidate_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-file-paths-grep-candidates",
        "ask",
        "search workspace content and list matching paths",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::FilePaths);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    journal.record_route_result(&route);
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        json!({
            "action": "grep_text",
            "query": "FirstLayerDecision",
            "count": 2,
            "match_count": 3,
            "matches": [
                {"path": "README.md", "line": 54, "text": "FirstLayerDecision"},
                {"path": "crates/clawd/src/intent_router.rs", "line": 14, "text": "FirstLayerDecision"}
            ]
        })
        .to_string(),
    ));

    let coverage = evidence_coverage_for_route(&route, &journal);

    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("content_match"));
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage
        .observed_extractors
        .contains("fs_basic.grep_text.structured_json_v1"));
}

#[test]
fn content_excerpt_summary_directory_inventory_can_complete_from_listing_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-content-summary-listing",
        "ask",
        "summarize repository layout from directory counts",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::ContentExcerptSummary);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "list_dir",
                "path": "crates",
                "counts": {"total": 3, "files": 0, "dirs": 3},
                "entries": [
                    {"name": "clawd", "kind": "dir"},
                    {"name": "skills", "kind": "dir"},
                    {"name": "skill-runner", "kind": "dir"}
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("count"));
}

#[test]
fn excerpt_kind_judgment_directory_counts_can_complete_from_count_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-excerpt-kind-counts",
        "ask",
        "judge repository layout from directory counts",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::ExcerptKindJudgment);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "count_entries",
                "path": "crates",
                "count": 3
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
                "action": "count_entries",
                "path": "crates/skills",
                "count": 8
            })
            .to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("count"));
}

#[test]
fn generic_path_content_directory_counts_can_complete_from_count_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-generic-path-counts",
        "ask",
        "compare direct directory entry counts",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "count_entries",
                "path": "crates",
                "count": 3
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
                "action": "count_entries",
                "path": "crates/skills",
                "count": 8
            })
            .to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("count"));
    assert!(coverage.observed_canonical.contains("path"));
}

#[test]
fn directory_purpose_tree_summary_children_satisfy_candidates_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-directory-purpose-tree-summary",
        "ask",
        "summarize relevant documentation entries",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::DirectoryPurposeSummary);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "tree_summary",
                "path": "document",
                "tree": {
                    "children": [
                        {
                            "kind": "file",
                            "path": "document/README.md",
                            "size_bytes": 128
                        }
                    ]
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage
        .observed_extractors
        .contains("system_basic.tree_summary.structured_json_v1"));
}

#[test]
fn system_basic_info_without_action_uses_info_extractor() {
    let mut journal =
        TaskJournal::for_task("task-system-info", "ask", "return current workspace path");
    let route = route_for_semantic(crate::OutputSemanticKind::ScalarPathOnly);
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "hostname": "devbox",
                "os": "linux",
                "arch": "x86_64",
                "cwd": "/home/guagua/rustclaw",
                "workspace_root": "/home/guagua/rustclaw"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage
        .observed_extractors
        .contains("system_basic.info.structured_json_v1"));
}

#[test]
fn docker_unavailable_text_counts_as_field_value_evidence() {
    let mut journal =
        TaskJournal::for_task("task-docker-unavailable", "ask", "检查 Docker 是否可用");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::DockerContainerLifecycle,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "docker_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("docker unavailable: No such file or directory (os error 2)".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));

    assert!(coverage.observed_canonical.contains("command_output"));
}

#[test]
fn generic_delivery_missing_find_count_satisfies_negative_delivery_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-missing-delivery",
        "ask",
        "send definitely_missing_named_file_golden_001.txt",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "find_name",
                "count": 0,
                "patterns": ["definitely_missing_named_file_golden_001.txt"],
                "results": [],
                "root": ""
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("count"));
}

#[test]
fn generated_file_delivery_wrapped_missing_find_name_supplies_checked_path_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-generated-missing-delivery",
        "ask",
        "send definitely_missing_named_file_golden_001.txt",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::GeneratedFileDelivery);
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "find_name",
                    "count": 0,
                    "exact": false,
                    "patterns": ["definitely_missing_named_file_golden_001.txt"],
                    "results": [],
                    "root": ""
                },
                "text": "{\"action\":\"find_name\",\"count\":0,\"exact\":false,\"patterns\":[\"definitely_missing_named_file_golden_001.txt\"],\"results\":[],\"root\":\"\"}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("exists_false"));
}

#[test]
fn docker_success_exit_text_counts_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-docker-version", "ask", "检查 Docker 是否可用");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::DockerContainerLifecycle,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "docker_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("exit=0\nClient: Docker Engine".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("command_output"));

    let trace = journal.to_trace_json();
    let items = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .and_then(|observed| observed.get("items"))
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("exit")
            && item.get("excerpt").and_then(Value::as_str) == Some("0")
    }));
}

#[test]
fn package_manager_key_value_text_counts_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-package-manager", "ask", "检测包管理器");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::PackageManagerDetection,
        locator_kind: crate::OutputLocatorKind::None,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "package_manager".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("package_manager=apt-get".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));

    let observed = observed_evidence_from_output(Some("package_manager=apt-get"))
        .expect("machine key/value evidence should be present");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("package_manager")
            && item.get("excerpt").and_then(Value::as_str) == Some("apt-get")
    }));
}

#[test]
fn docker_unavailable_text_counts_as_list_candidate_evidence() {
    let mut journal =
        TaskJournal::for_task("task-docker-images-unavailable", "ask", "列出 Docker 镜像");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::DockerImages,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "docker_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("docker unavailable: No such file or directory (os error 2)".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));

    assert!(coverage.observed_canonical.contains("command_output"));
}

#[test]
fn structured_keys_array_counts_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-structured-keys", "ask", "列出配置键");
    let route = route_for_semantic(crate::OutputSemanticKind::StructuredKeys);
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "structured_keys",
                "exists": true,
                "keys": ["app", "features", "paths"],
                "count": 3
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
}

#[test]
fn docker_command_not_found_text_counts_as_docker_contract_evidence() {
    for (semantic_kind, expected_canonical) in [
        (
            crate::OutputSemanticKind::DockerContainerLifecycle,
            "field_value",
        ),
        (crate::OutputSemanticKind::DockerLogs, "candidates"),
    ] {
        let mut journal =
            TaskJournal::for_task("task-docker-command-not-found", "ask", "检查 Docker");
        let route = route_for_semantic(semantic_kind);
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("bash: line 1: docker: command not found\n".to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains(expected_canonical));
    }
}

#[test]
fn scalar_count_json_value_counts_as_count_evidence() {
    let mut journal = TaskJournal::for_task("task-scalar-count", "ask", "输出数量");
    let route = route_for_semantic(crate::OutputSemanticKind::ScalarCount);
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("3\n".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("count"));
}

#[test]
fn log_analyze_output_counts_as_content_excerpt_evidence() {
    let mut journal = TaskJournal::for_task("task-log-summary", "ask", "总结日志异常");
    let mut route = route_for_semantic(crate::OutputSemanticKind::ContentExcerptSummary);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs".to_string();
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "log_analyze".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "path": "logs/clawd.log",
                "level_counts": {"error": 1},
                "recent_notable_lines": ["ERROR sample"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
}

#[test]
fn browser_web_output_counts_as_content_excerpt_evidence() {
    let mut journal = TaskJournal::for_task("task-web-summary", "ask", "总结网页");
    let mut route = route_for_semantic(crate::OutputSemanticKind::WebPageSummary);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Url;
    route.output_contract.locator_hint = "https://example.com".to_string();
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "browser_web".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "items": [{
                    "url": "https://example.com",
                    "title": "Example Domain",
                    "content_excerpt": "Example Domain is for documentation examples."
                }],
                "summary": "Extracted 1 page(s) using browser"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage
        .observed_extractors
        .contains("browser_web.structured_json_v1"));

    let trace = journal.to_trace_json();
    let items = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .and_then(|evidence| evidence.get("items"))
        .and_then(Value::as_array)
        .expect("browser observed evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("items[0].title")
            && item.get("excerpt").and_then(Value::as_str) == Some("Example Domain")
            && item.get("redacted").is_none()
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("items[0].content_excerpt")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|excerpt| excerpt.contains("documentation examples"))
            && item.get("redacted").is_none()
    }));
}

#[test]
fn web_search_extract_output_counts_as_candidates_evidence() {
    let mut journal = TaskJournal::for_task("task-web-search-summary", "ask", "总结搜索结果");
    let mut route = route_for_semantic(crate::OutputSemanticKind::WebSearchSummary);
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "web_search_extract".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "status": "ok",
                "items": [{
                    "title": "Rust Async",
                    "url": "https://example.com",
                    "snippet": "Async Rust tutorial"
                }],
                "extract_urls": ["https://example.com"],
                "summary": "1 result"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage
        .observed_extractors
        .contains("web_search_extract.structured_json_v1"));
}

#[test]
fn web_search_extract_empty_candidates_count_as_candidates_evidence() {
    let mut journal = TaskJournal::for_task("task-web-search-empty", "ask", "总结搜索结果");
    let mut route = route_for_semantic(crate::OutputSemanticKind::WebSearchSummary);
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "web_search_extract".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "text": "{\"status\":\"ok\",\"items\":[],\"summary\":\"No results found\"}",
                "extra": {
                    "schema_version": 1,
                    "action": "search",
                    "status": "ok",
                    "backend": "duckduckgo_html",
                    "backend_connected": true,
                    "field_value": {
                        "status": "ok",
                        "result_count": 0,
                        "summary": "No results found"
                    },
                    "items": [],
                    "candidates": [],
                    "extract_urls": [],
                    "citations": []
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage
        .observed_extractors
        .contains("web_search_extract.structured_json_v1"));
}

#[test]
fn weather_output_counts_as_content_excerpt_evidence() {
    let mut journal = TaskJournal::for_task("task-weather-query", "ask", "查天气");
    let mut route = route_for_semantic(crate::OutputSemanticKind::WeatherQuery);
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "weather".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "status": "ok",
                "text": "Beijing current weather: clear, 22 C.",
                "extra": {"action": "query", "mode": "current", "locale": "en-US"},
                "error_text": null
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage
        .observed_extractors
        .contains("weather.structured_json_v1"));
}

#[test]
fn market_quote_output_counts_as_content_excerpt_evidence() {
    let mut journal = TaskJournal::for_task("task-market-quote", "ask", "查行情");
    let mut route = route_for_semantic(crate::OutputSemanticKind::MarketQuote);
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "stock".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "status": "ok",
                "text": "【SH600519】贵州茅台 现价 1688.00",
                "error_text": null
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage
        .observed_extractors
        .contains("stock.structured_json_v1"));
}

#[test]
fn crypto_quote_extra_content_excerpt_counts_as_market_quote_evidence() {
    let mut journal = TaskJournal::for_task("task-crypto-quote", "ask", "查 BTCUSDT 价格");
    let mut route = route_for_semantic(crate::OutputSemanticKind::MarketQuote);
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "crypto".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "text": "BTCUSDT | 价格来源：- 币安(BINANCE) $69587.260000",
                "extra": {
                    "action": "quote",
                    "content_excerpt": "BTCUSDT | 价格来源：- 币安(BINANCE) $69587.260000",
                    "quote": {
                        "symbol": "BTCUSDT",
                        "price_usd": 69587.26,
                        "exchange": "binance",
                        "source": "binance_api"
                    }
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage
        .observed_extractors
        .contains("crypto.structured_json_v1"));
}

#[test]
fn image_vision_output_counts_as_content_excerpt_evidence() {
    let mut journal = TaskJournal::for_task("task-image-understanding", "ask", "描述图片");
    let mut route = route_for_semantic(crate::OutputSemanticKind::ImageUnderstanding);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Url;
    route.output_contract.locator_hint = "https://example.com/image.png".to_string();
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "image_vision".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "status": "ok",
                "text": "The image shows a Rust logo.",
                "extra": {"action": "describe"},
                "error_text": null
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage
        .observed_extractors
        .contains("image_vision.structured_json_v1"));
}

#[test]
fn x_preview_output_counts_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-publishing-preview", "ask", "预览发布文案");
    let mut route = route_for_semantic(crate::OutputSemanticKind::PublishingPreview);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "x".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("x skill dry_run=1, preview post: RustClaw release notes".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_extractors.contains("x.text_legacy_v1"));
}

#[test]
fn scalar_path_only_results_array_counts_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-scalar-path", "ask", "找到路径");
    let route = route_for_semantic(crate::OutputSemanticKind::ScalarPathOnly);
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "find_name",
                "count": 1,
                "results": ["scripts/nl_tests/fixtures/device_local/package.json"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
}

#[test]
fn json_observed_evidence_prioritizes_complete_candidate_names_before_entry_details() {
    let output = r#"{
        "action": "inventory_dir",
        "counts": {"dirs": 1, "files": 2, "hidden": 0, "total": 3},
        "dirs_only": false,
        "entries": [
            {"hidden": false, "kind": "dir", "modified_ts": 1, "name": "archive", "path": "docs/archive", "size_bytes": 0},
            {"hidden": false, "kind": "file", "modified_ts": 1, "name": "release_checklist.md", "path": "docs/release_checklist.md", "size_bytes": 153},
            {"hidden": false, "kind": "file", "modified_ts": 1, "name": "service_notes.md", "path": "docs/service_notes.md", "size_bytes": 272}
        ],
        "names": ["archive", "release_checklist.md", "service_notes.md"],
        "names_by_kind": {
            "dirs": ["archive"],
            "files": ["release_checklist.md", "service_notes.md"],
            "other": []
        }
    }"#;

    let observed = observed_evidence_from_output(Some(output))
        .expect("json output should produce observed evidence");
    assert_eq!(
        observed.get("truncated").and_then(Value::as_bool),
        Some(true)
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("names[2]")
            && item.get("excerpt").and_then(Value::as_str) == Some("service_notes.md")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("names_by_kind.files[1]")
            && item.get("excerpt").and_then(Value::as_str) == Some("service_notes.md")
    }));
}

#[test]
fn json_read_range_excerpt_preserves_provider_safe_line_evidence() {
    let output = json!({
        "action": "read_range",
        "mode": "tail",
        "path": "logs/clawd.run.log",
        "excerpt": "1695|INFO task_call: [ASK_STATE] ask_state_transition state_from=none state_to=finalizing\n1696|INFO task_call: answer_verifier_skipped_structural_satisfaction\n1697|INFO task_call: task_call_end kind=ask status=success path=normal"
    })
    .to_string();

    let observed = observed_evidence_from_output(Some(&output))
        .expect("json read_range output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");

    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("content_excerpt")
            && item.get("origin_field").and_then(Value::as_str) == Some("excerpt")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|excerpt| {
                    excerpt.contains("task_call_end") && excerpt.contains("status=success")
                })
    }));
}

#[test]
fn json_observed_evidence_prioritizes_health_check_process_counts() {
    let output = json!({
        "clawd_health_port_open": true,
        "clawd_log": {"exists": true, "keyword_error_count": 43, "modified_ts": 1779824680, "size_bytes": 1046356},
        "clawd_process_count": 1,
        "log_dir": "/home/guagua/rustclaw/logs",
        "system_health": {
            "arch": "x86_64",
            "cpu_count": 8,
            "disk_root_available_bytes": 17418850304u64,
            "disk_root_total_bytes": 156546629632u64,
            "hostname": "ThinkPad-X1",
            "kernel_release": "6.17.0-29-generic",
            "load_avg_15m": 1.26,
            "load_avg_1m": 0.15,
            "load_avg_5m": 0.56,
            "memory_available_bytes": 10011176960u64,
            "memory_total_bytes": 15937286144u64,
            "os_family": "linux",
            "service_manager": "systemd",
            "uptime_seconds": 485924,
            "warnings": []
        },
        "telegramd_log": {"exists": true, "keyword_error_count": 1, "modified_ts": 1779821271, "size_bytes": 942},
        "telegramd_process_count": 0,
        "workspace_root": "/home/guagua/rustclaw"
    });

    let observed = observed_evidence_from_output(Some(&output.to_string()))
        .expect("json output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("telegramd_process_count")
            && item.get("excerpt").and_then(Value::as_str) == Some("0")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("clawd_process_count")
            && item.get("excerpt").and_then(Value::as_str) == Some("1")
    }));
}

#[test]
fn text_observed_evidence_parses_status_prefixed_json_body() {
    let output = concat!(
        "status=200\n",
        "{\"ok\":true,\"data\":{\"version\":\"0.1.7\",\"worker_state\":\"running\",\"uptime_seconds\":95,\"telegramd_process_count\":0},\"error\":null}"
    );

    let observed = observed_evidence_from_output(Some(output))
        .expect("status-prefixed json output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("status")
            && item.get("excerpt").and_then(Value::as_str) == Some("200")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("body.ok")
            && item.get("excerpt").and_then(Value::as_str) == Some("true")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("body.data.worker_state")
            && item.get("excerpt").and_then(Value::as_str) == Some("running")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("body.data.telegramd_process_count")
            && item.get("excerpt").and_then(Value::as_str) == Some("0")
    }));
}

#[test]
fn text_observed_evidence_keeps_safe_file_tokens_while_redacting_secret_tokens() {
    let output = concat!(
        "The files are builtin_write_smoke.txt, full_suite_trace_note.txt, gen-1778122040.png, ",
        "and hello.sh; secret sk-123456789012345678901234 should not be exposed."
    );

    let observed = observed_evidence_from_output(Some(output))
        .expect("text output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    let text_excerpt = items
        .iter()
        .find(|item| item.get("field").and_then(Value::as_str) == Some("text_excerpt"))
        .and_then(|item| item.get("excerpt"))
        .and_then(Value::as_str)
        .expect("text excerpt");

    assert!(text_excerpt.contains("full_suite_trace_note.txt"));
    assert!(text_excerpt.contains("gen-1778122040.png"));
    assert!(text_excerpt.contains("hello.sh"));
    assert!(text_excerpt.contains("[redacted]"));
    assert!(!text_excerpt.contains("sk-123456789012345678901234"));
}

#[test]
fn json_observed_evidence_array_items_include_provider_safe_sample_values() {
    let names = vec![
        "builtin_write_smoke.txt",
        "full_suite_trace_note.txt",
        "gen-1778122040.png",
        "gen-1778122536.png",
        "hello.sh",
        "hello_from_manual_test.sh",
        "hello_from_p2_smoke.sh",
        "hello_from_p2_smoke_v2.sh",
        "hello_world.sh",
        "manual_fixture_note.txt",
        "manual_meta.json",
        "manual_meta_variant.json",
        "manual_note.txt",
        "manual_note_variant.txt",
        "minimax_pwd_line.txt",
        "natural_manual_note.txt",
    ];
    let output = json!({
        "action": "inventory_dir",
        "names": names,
        "names_by_kind": {
            "files": names,
            "dirs": [],
            "other": []
        },
        "path": "document"
    });

    let observed = observed_evidence_from_output(Some(&output.to_string()))
        .expect("json output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    let names_item = items
        .iter()
        .find(|item| item.get("field").and_then(Value::as_str) == Some("names"))
        .expect("names array evidence item");
    let sample_values = names_item
        .get("sample_values")
        .and_then(Value::as_array)
        .expect("names array should expose sample_values");
    assert!(sample_values
        .iter()
        .any(|item| item.as_str() == Some("manual_note_variant.txt")));
}

#[test]
fn recent_artifacts_judgment_content_excerpt_satisfies_field_value_requirement() {
    let mut journal = TaskJournal::for_task(
        "task-recent-artifacts-content",
        "ask",
        "list docs and explain the most relevant one",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::RecentArtifactsJudgment);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document".to_string();
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["manual_note.txt", "manual_fixture_note.txt"],
                "names_by_kind": {
                    "files": ["manual_note.txt", "manual_fixture_note.txt"],
                    "dirs": [],
                    "other": []
                },
                "path": "document"
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
                "path": "document/manual_note.txt",
                "excerpt": "1|RustClaw manual test note",
                "start_line": 1,
                "end_line": 1
            })
            .to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage.observed_canonical.contains("field_value"));
}

#[test]
fn large_inventory_dir_observed_evidence_preserves_mtime_metadata_when_truncated() {
    let entries = (0..68)
        .map(|idx| {
            json!({
                "hidden": false,
                "kind": if idx % 2 == 0 { "file" } else { "dir" },
                "modified_ts": 1780000000_u64 - idx,
                "name": format!("entry_{idx}.txt"),
                "path": format!("entry_{idx}.txt"),
                "size_bytes": 100 + idx
            })
        })
        .collect::<Vec<_>>();
    let names = (0..68)
        .map(|idx| format!("entry_{idx}.txt"))
        .collect::<Vec<_>>();
    let output = json!({
        "action": "inventory_dir",
        "counts": {"dirs": 34, "files": 34, "hidden": 0, "total": 68},
        "entries": entries,
        "names": names,
        "names_by_kind": {
            "dirs": ["entry_1.txt", "entry_3.txt", "entry_5.txt"],
            "files": ["entry_0.txt", "entry_2.txt", "entry_4.txt"],
            "other": []
        },
        "path": "/home/guagua/rustclaw",
        "sort_by": "mtime_desc"
    });
    let output_text = output.to_string();

    let observed = observed_evidence_from_output(Some(&output_text))
        .expect("json output should produce observed evidence");
    assert_eq!(
        observed.get("truncated").and_then(Value::as_bool),
        Some(true)
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("sort_by")
            && item.get("excerpt").and_then(Value::as_str) == Some("mtime_desc")
    }));
    let entries_item = items
        .iter()
        .find(|item| item.get("field").and_then(Value::as_str) == Some("entries"))
        .expect("entries array evidence item");
    let sample_keys = entries_item
        .get("sample_keys")
        .and_then(Value::as_array)
        .expect("array object sample keys");
    assert!(sample_keys
        .iter()
        .any(|item| item.as_str() == Some("modified_ts")));
    assert!(sample_keys
        .iter()
        .any(|item| item.as_str() == Some("size_bytes")));

    let mut journal = TaskJournal::for_task("task-large-mtime-dir", "ask", "list recent entries");
    let mut route = route_for_semantic(crate::OutputSemanticKind::DirectoryEntryGroups);
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(output_text),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("modified_ts"));
    assert!(coverage.observed_canonical.contains("sort_by"));
}

#[test]
fn service_status_health_check_fields_count_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-service-status", "ask", "检查 clawd 服务状态");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ServiceStatus,
        locator_kind: crate::OutputLocatorKind::None,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "health_check".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(r#"{"clawd_health_port_open":true,"clawd_process_count":1}"#.to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
}

#[test]
fn doc_parse_metadata_path_counts_as_required_path_before_truncation() {
    let mut journal =
        TaskJournal::for_task("task-doc-parse-path", "ask", "读 README 并用三句话总结");
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    journal.record_route_result(&route);
    let sections = (0..32)
        .map(|idx| {
            json!({
                "id": format!("sec_{idx}"),
                "title": format!("Section {idx}"),
                "level": 2,
                "content": "long section body"
            })
        })
        .collect::<Vec<_>>();
    let output = json!({
        "text": "RustClaw is a local Rust agent runtime.",
        "sections": sections,
        "metadata": {
            "path": "/home/guagua/rustclaw/README.md",
            "type": "md"
        },
        "status": "ok"
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "doc_parse".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);

    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("content_excerpt"));
}

#[test]
fn service_status_run_cmd_output_counts_as_field_value_evidence() {
    let mut journal =
        TaskJournal::for_task("task-service-status-run-cmd", "ask", "检查 clawd 服务状态");
    let route = route_for_semantic(crate::OutputSemanticKind::ServiceStatus);
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            "154421 clawd /home/guagua/rustclaw/target/release/clawd --config /home/guagua/rustclaw/configs/config.toml\n"
                .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("command_output"));
}

#[test]
fn service_status_http_basic_text_counts_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-service-status-http-basic",
        "ask",
        "检查本地 health 接口",
    );
    let route = route_for_semantic(crate::OutputSemanticKind::ServiceStatus);
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "http_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("status=200\n{\"ok\":true,\"service\":\"clawd\"}\n".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("command_output"));
}

#[test]
fn service_status_http_basic_json_wrapper_extracts_embedded_body_status_fields() {
    let mut journal = TaskJournal::for_task(
        "task-service-status-http-basic-json",
        "ask",
        "observe local health endpoint",
    );
    let route = route_for_semantic(crate::OutputSemanticKind::ServiceStatus);
    journal.record_route_result(&route);
    let body = json!({
        "ok": true,
        "data": {
            "version": "0.1.7",
            "worker_state": "running",
            "uptime_seconds": 53,
            "channel_gateway_healthy": false,
            "telegram_bot_statuses": [
                {
                    "name": "primary",
                    "healthy": false,
                    "status": "stale"
                }
            ]
        },
        "error": null
    })
    .to_string();
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "http_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "get",
                    "url": "http://127.0.0.1:8787/v1/health",
                    "status_code": 200,
                    "success_status": true,
                    "body_preview": body.clone(),
                },
                "text": format!("status=200\n{body}")
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage
        .observed_extractors
        .contains("http_basic.structured_json_v1"));
    assert!(coverage
        .observed_fields
        .contains("body.data.channel_gateway_healthy"));
    assert!(coverage.observed_fields.contains("body.data.version"));
    assert!(coverage
        .observed_fields
        .contains("body.data.uptime_seconds"));
    assert!(coverage
        .observed_fields
        .contains("body.data.telegram_bot_statuses[0].name"));
    assert!(coverage
        .observed_fields
        .contains("body.data.telegram_bot_statuses[0].status"));
}

#[test]
fn web_page_summary_http_basic_json_wrapper_body_counts_as_content_excerpt_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-web-summary-http-basic-json",
        "ask",
        "summarize local health endpoint",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::WebPageSummary);
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    let body = json!({
        "ok": true,
        "data": {
            "version": "0.1.7",
            "worker_state": "running",
            "uptime_seconds": 53,
            "channel_gateway_healthy": false,
            "telegram_bot_statuses": [
                {
                    "name": "primary",
                    "healthy": false,
                    "status": "stale"
                }
            ]
        }
    })
    .to_string();
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "http_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "get",
                    "url": "http://127.0.0.1:8787/v1/health",
                    "status_code": 200,
                    "success_status": true,
                    "body_preview": body.clone(),
                },
                "text": format!("status=200\n{body}")
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage
        .observed_fields
        .contains("body.data.channel_gateway_healthy"));
    assert!(coverage.observed_fields.contains("body.data.version"));
    assert!(coverage
        .observed_fields
        .contains("body.data.uptime_seconds"));
}

#[test]
fn raw_command_output_http_basic_text_counts_as_command_output_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-raw-command-http-basic",
        "ask",
        "请求 http://127.0.0.1:8787/v1/health",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::RawCommandOutput);
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "http_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("status=200\n{\"ok\":true,\"service\":\"clawd\"}\n".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("command_output"));
}

#[test]
fn raw_command_output_file_read_excerpt_counts_as_command_output_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-raw-command-file-read",
        "ask",
        "读取 README.md 前 4 行",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::RawCommandOutput);
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"read_range","path":"/tmp/README.md","excerpt":"1|# Demo\n2|body"}"#
                .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage.observed_canonical.contains("command_output"));
}

#[test]
fn git_status_text_counts_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-git-state", "ask", "检查仓库状态");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::GitRepositoryState,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            "exit=0\n## main...origin/main\n M crates/clawd/src/task_journal.rs\n".to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));

    let trace = journal.to_trace_json();
    let items = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .and_then(|observed| observed.get("items"))
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("state")
            && item.get("excerpt").and_then(Value::as_str) == Some("dirty")
    }));
}

#[test]
fn git_subject_plain_text_counts_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-git-subject", "ask", "最近一次 git 提交标题");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::GitCommitSubject,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("Harden contract matrix execution coverage\n".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
}

#[test]
fn git_status_text_ignores_non_ascii_summary_without_panic() {
    assert_eq!(
        super::text_git_state_evidence(
            "exit=0\n## main...origin/main\n执行 cat /definitely_missing_rustclaw_contract_case 失败\n"
        ),
        Some("clean")
    );
}

#[test]
fn config_validation_evidence_coverage_accepts_valid_flag() {
    let mut journal = TaskJournal::for_task("task-config-validation", "ask", "验证配置");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ConfigValidation,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "validate_structured",
                "path": "configs/config.toml",
                "format": "toml",
                "valid": true,
                "root_type": "object",
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
}

#[test]
fn config_mutation_plan_change_evidence_counts_as_valid_plan_proof() {
    let mut journal = TaskJournal::for_task("task-config-plan", "ask", "preview config change");
    let mut route = route_for_semantic(crate::OutputSemanticKind::ConfigMutation);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_edit".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "plan_config_change",
                    "path": "configs/config.toml",
                    "resolved_path": "/repo/configs/config.toml",
                    "field_path": "skills.skill_switches.example",
                    "old_value": null,
                    "new_value": true,
                    "would_change": true,
                    "requires_confirmation": true
                },
                "text": "{\"action\":\"plan_config_change\",\"path\":\"configs/config.toml\",\"field_path\":\"skills.skill_switches.example\",\"new_value\":true,\"would_change\":true,\"requires_confirmation\":true}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("valid"));
}

#[test]
fn config_mutation_apply_validated_flag_counts_as_valid_evidence() {
    let mut journal = TaskJournal::for_task("task-config-apply", "ask", "apply config change");
    let mut route = route_for_semantic(crate::OutputSemanticKind::ConfigMutation);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_edit".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "apply_config_change",
                "path": "configs/config.toml",
                "resolved_path": "/repo/configs/config.toml",
                "field_path": "skills.skill_switches.example",
                "old_value": null,
                "new_value": true,
                "applied": true,
                "validated": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("valid"));
}

#[test]
fn sqlite_database_kind_uses_db_structure_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-sqlite-kind", "ask", "判断 sqlite 数据库类型");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "data/test_contract.sqlite".to_string(),
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "db_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "columns": ["name"],
                "rows": [
                    {"name": "orders"},
                    {"name": "service_logs"},
                    {"name": "users"}
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("field_value"));
}

#[test]
fn quantity_comparison_size_bytes_counts_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-quantity-comparison", "ask", "比较两个文件大小");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::QuantityComparison,
        locator_kind: crate::OutputLocatorKind::Path,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "path_batch_facts",
                "facts": [
                    {
                        "path": "release_checklist.md",
                        "exists": true,
                        "fact": {
                            "kind": "file",
                            "size_bytes": 153
                        }
                    },
                    {
                        "path": "package.json",
                        "exists": true,
                        "fact": {
                            "kind": "file",
                            "size_bytes": 246
                        }
                    }
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("size_bytes"));
}

#[test]
fn quantity_comparison_inventory_dir_entry_keys_count_as_size_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-quantity-comparison-inventory-dir",
        "ask",
        "find largest file by size",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::QuantityComparison);
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "counts": {"dirs": 0, "files": 22, "hidden": 0, "total": 22},
                "dirs_only": false,
                "entries": (0..22)
                    .map(|idx| {
                        json!({
                            "hidden": false,
                            "kind": "file",
                            "modified_ts": 1,
                            "name": format!("schema_{idx}.json"),
                            "path": format!("prompts/schemas/schema_{idx}.json"),
                            "size_bytes": 100 + idx
                        })
                    })
                    .collect::<Vec<_>>(),
                "names": (0..22)
                    .map(|idx| format!("schema_{idx}.json"))
                    .collect::<Vec<_>>(),
                "names_by_kind": {
                    "dirs": [],
                    "files": (0..22)
                        .map(|idx| format!("schema_{idx}.json"))
                        .collect::<Vec<_>>(),
                    "other": []
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("size_bytes"));
}

#[test]
fn quantity_comparison_text_size_bytes_counts_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-quantity-comparison-text",
        "ask",
        "compare two file sizes",
    );
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::QuantityComparison,
        locator_kind: crate::OutputLocatorKind::Path,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            "path=release_checklist.md size_bytes=153\npath=package.json size_bytes=246"
                .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("size_bytes"));
}

#[test]
fn quantity_comparison_count_inventory_total_size_counts_as_size_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-quantity-comparison-count-inventory-size",
        "ask",
        "check directory size",
    );
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::QuantityComparison,
        locator_kind: crate::OutputLocatorKind::Path,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "count_inventory",
                "path": "target",
                "resolved_path": "/tmp/repo/target",
                "recursive": true,
                "counts": {
                    "total": 129116,
                    "files": 121355,
                    "dirs": 7761,
                    "total_size_bytes": 57263840032u64
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("size_bytes"));
}

#[test]
fn trace_json_reports_required_vs_observed_evidence_coverage() {
    let mut journal = TaskJournal::for_task("task-evidence-coverage", "ask", "列出文件名");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({"action": "list_dir", "names": ["Cargo.toml", "README.md"]}).to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let coverage = trace
        .get("evidence_coverage")
        .expect("evidence coverage should be present");
    assert_eq!(
        coverage
            .get("required_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["candidates"])
    );
    assert_eq!(
        coverage
            .get("missing_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(Vec::<&str>::new())
    );
    assert!(coverage
        .get("observed_canonical")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("candidates"))));
    assert!(coverage
        .get("observed_extractors")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.as_str() == Some("fs_basic.list_dir.structured_json_v1"))));
    assert!(coverage
        .pointer("/observed_evidence_sources/candidates")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.as_str() == Some("fs_basic.list_dir.structured_json_v1"))));
    let summary = journal.to_summary_json();
    assert_eq!(
        summary
            .get("task_outcome")
            .and_then(|value| value.get("state"))
            .and_then(Value::as_str),
        Some("in_progress")
    );
}

#[test]
fn config_risk_evidence_coverage_accepts_guard_findings() {
    let mut journal = TaskJournal::for_task("task-config-risk-evidence", "ask", "检查配置风险");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ConfigRiskAssessment,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        requires_content_evidence: true,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_edit".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "guard_config",
                "format": "toml",
                "path": "configs/config.toml",
                "resolved_path": "/home/guagua/rustclaw/configs/config.toml",
                "risk_count": 2,
                "risks": [
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

    let coverage = evidence_coverage_for_route(&route, &journal);
    let trace = journal.to_trace_json();
    let items = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .and_then(|observed| observed.get("items"))
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");

    assert!(coverage.is_complete());
    assert_eq!(coverage.required_evidence, vec!["candidates", "count"]);
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("count"));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("risks[1]")
            && item.get("excerpt").and_then(Value::as_str)
                == Some("tools.allow_path_outside_workspace=true")
            && item.get("redacted").is_none()
    }));
}

#[test]
fn filesystem_mutation_result_accepts_kb_ingest_path_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-kb-ingest-evidence",
        "ask",
        "ingest README into demo_docs_nl",
    );
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FilesystemMutationResult,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        locator_hint: "README.md".to_string(),
        requires_content_evidence: true,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "kb".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "ingest",
                "status": "ok",
                "namespace": "demo_docs_nl",
                "path": "README.md",
                "paths": ["README.md"],
                "stats": {
                    "ingested_docs": 1,
                    "total_docs": 1,
                    "total_chunks": 3
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    let trace = journal.to_trace_json();

    assert!(coverage.is_complete());
    assert_eq!(coverage.required_evidence, vec!["path"]);
    assert!(coverage.observed_canonical.contains("path"));
    assert!(trace
        .pointer("/step_results/0/observed_evidence/extractor/extractor_ref")
        .and_then(Value::as_str)
        .is_some_and(|extractor| extractor == "kb.ingest.structured_json_v1"));
}

#[test]
fn evidence_coverage_ignores_failed_and_synthesis_outputs() {
    let mut journal = TaskJournal::for_task(
        "task-evidence-coverage-filter",
        "ask",
        "summarize file content",
    );
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        requires_content_evidence: true,
        semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "README.md".to_string(),
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_failed".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: Some(json!({"content": "failed read should not count"}).to_string()),
        error: Some("read failed".to_string()),
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_synthesis".to_string(),
        skill: "synthesize_answer".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({"content": "model synthesis should not count as observed content"}).to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);

    assert!(!coverage.is_complete());
    assert_eq!(
        coverage.missing_evidence,
        vec!["any_of(candidates|content_excerpt|count|field_value)"]
    );
    assert!(!coverage.observed_canonical.contains("content_excerpt"));
}

#[test]
fn raw_command_output_error_step_supplies_command_output_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-run-cmd-failure-evidence",
        "ask",
        "cat /definitely_missing_rustclaw_contract_case",
    );
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ExecutionFailedStep,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(crate::skills::structured_skill_error_from_parts(
            "run_cmd",
            "nonzero_exit",
            "Command failed with exit code 1",
            Some("linux"),
            Some(json!({
                "command": "cat /definitely_missing_rustclaw_contract_case",
                "exit_code": 1,
                "stderr": "cat: /definitely_missing_rustclaw_contract_case: No such file or directory (os error 2)\n",
                "stdout": Value::Null,
            })),
        )),
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));

    let trace = journal.to_trace_json();
    let items = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .and_then(|observed| observed.get("items"))
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("stderr")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|value| value.contains("No such file or directory"))
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("command_output")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|value| value.contains("command failed"))
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("field_value")
            && item
                .get("source")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "structured_error.extractor")
    }));
}

#[test]
fn summary_json_includes_user_readable_task_outcome() {
    let mut journal = TaskJournal::for_task("task-outcome", "ask", "列出文件名");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.record_final_status(TaskJournalFinalStatus::Success);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"names": ["Cargo.toml", "README.md"]}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let outcome = journal
        .to_summary_json()
        .get("task_outcome")
        .cloned()
        .expect("task outcome");

    assert_eq!(
        outcome.get("state").and_then(Value::as_str),
        Some("completed")
    );
    assert_eq!(
        outcome.get("final_answer_shape").and_then(Value::as_str),
        Some("name_list")
    );
    assert_eq!(
        outcome
            .get("missing_evidence_count")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert!(outcome.get("message_zh").and_then(Value::as_str).is_some());
    assert!(outcome
        .get("next_step_en")
        .and_then(Value::as_str)
        .is_some());
}

#[test]
fn trace_json_reports_missing_required_evidence() {
    let mut journal = TaskJournal::for_task("task-evidence-missing", "ask", "这个路径是否存在");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
        locator_kind: crate::OutputLocatorKind::Path,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"path": "/tmp/missing.txt", "exists": false}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let coverage = trace
        .get("evidence_coverage")
        .expect("evidence coverage should be present");
    assert_eq!(
        coverage
            .get("missing_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["kind"])
    );
}

#[test]
fn trace_json_uses_evidence_expression_for_confirmed_absence() {
    let mut journal = TaskJournal::for_task("task-evidence-absence", "ask", "这个路径是否存在");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
        locator_kind: crate::OutputLocatorKind::Path,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "path": "/tmp/missing.txt",
                "exists": false,
                "kind": "missing"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete());
    assert!(coverage.observed_canonical.contains("exists_false"));

    let trace = journal.to_trace_json();
    let coverage = trace
        .get("evidence_coverage")
        .expect("evidence coverage should be present");
    assert_eq!(
        coverage
            .get("evidence_expression")
            .and_then(|value| value.get("negative_evidence"))
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["exists_false"])
    );
    assert_eq!(
        coverage
            .get("missing_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(Vec::<&str>::new())
    );
}

#[test]
fn trace_json_reports_missing_evidence_expression_alternative() {
    let mut journal = TaskJournal::for_task("task-evidence-missing-alt", "ask", "这个路径是否存在");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
        locator_kind: crate::OutputLocatorKind::Path,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"path": "/tmp/maybe.txt", "kind": "file"}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert_eq!(
        coverage.missing_evidence,
        vec!["one_of(exists_false|exists_true)"]
    );
}

#[test]
fn content_presence_accepts_excerpt_evidence_alternative() {
    let mut journal = TaskJournal::for_task(
        "task-content-presence-excerpt",
        "ask",
        "check whether the file mentions release",
    );
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ContentPresenceCheck,
        locator_kind: crate::OutputLocatorKind::Path,
        requires_content_evidence: true,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        json!({
            "action": "read_range",
            "path": "/tmp/release_checklist.md",
            "excerpt": "1|# Release Checklist"
        })
        .to_string(),
    ));

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete());
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert_eq!(
        coverage
            .evidence_expression
            .as_ref()
            .and_then(|value| value.get("any_of"))
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["content_excerpt", "content_match"])
    );
}

#[test]
fn content_presence_accepts_structured_not_found_as_negative_match_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-content-presence-missing-path",
        "ask",
        "read /tmp/definitely_missing.md; if missing, say it is missing",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::ContentPresenceCheck);
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    journal.record_route_result(&route);
    journal.step_results.push(TaskJournalStepTrace::new(
        "step_1",
        "fs_basic",
        crate::executor::StepExecutionStatus::Error,
        None,
        Some(format!(
            "__RC_SKILL_ERROR__:{}",
            json!({
                "skill": "system_basic",
                "error_kind": "not_found",
                "error_text": "metadata failed for /tmp/definitely_missing.md",
                "extra": {
                    "operation": "metadata",
                    "path": "/tmp/definitely_missing.md"
                }
            })
        )),
    ));

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_match"));
    assert!(coverage.observed_canonical.contains("exists"));
    assert!(coverage.observed_canonical.contains("path"));
}

#[test]
fn non_content_route_ignores_read_text_observation_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-non-content-read-evidence",
        "ask",
        "current git commit subject",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::GitCommitSubject);
    route.output_contract.requires_content_evidence = false;
    journal.record_route_result(&route);
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_read",
        "fs_basic",
        json!({
            "action": "read_text_range",
            "path": "/tmp/commit-message.txt",
            "content": "abc1234 add contract matrix tests"
        })
        .to_string(),
    ));

    let coverage = evidence_coverage_for_route(&route, &journal);

    assert!(!coverage.is_complete());
    assert_eq!(coverage.missing_evidence, vec!["field_value"]);
    assert!(!coverage.observed_canonical.contains("field_value"));
    assert!(!coverage.observed_canonical.contains("content_excerpt"));
}

#[test]
fn non_content_route_ignores_doc_parse_observation_as_structured_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-non-content-doc-parse-evidence",
        "ask",
        "service status",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::ServiceStatus);
    route.output_contract.requires_content_evidence = false;
    journal.record_route_result(&route);
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_parse",
        "doc_parse",
        json!({
            "action": "parse_doc",
            "path": "/tmp/service-notes.md",
            "status": "running",
            "content": "operator notes say the service should be running"
        })
        .to_string(),
    ));

    let coverage = evidence_coverage_for_route(&route, &journal);

    assert!(!coverage.is_complete());
    assert_eq!(coverage.missing_evidence, vec!["field_value"]);
    assert!(!coverage.observed_canonical.contains("field_value"));
}

#[test]
fn trace_json_counts_nested_builtin_tool_evidence() {
    let mut journal = TaskJournal::for_task("task-nested-evidence", "ask", "这个路径是否存在");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
        locator_kind: crate::OutputLocatorKind::Path,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "path_batch_facts",
                "facts": [{
                    "path": "/tmp/present.txt",
                    "exists": true,
                    "kind": "file"
                }]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let coverage = trace
        .get("evidence_coverage")
        .expect("evidence coverage should be present");
    assert_eq!(
        coverage
            .get("missing_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(Vec::<&str>::new())
    );
    assert!(coverage
        .get("observed_fields")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.as_str() == Some("facts[0].path"))));
}

#[test]
fn trace_json_includes_task_level_contract_matrix_snapshot() {
    let mut journal = TaskJournal::for_task("task-contract-snapshot", "ask", "列出文件名");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);

    let trace = journal.to_trace_json();
    let snapshot = trace
        .get("contract_matrix")
        .expect("contract matrix snapshot should be present");

    assert_eq!(
        snapshot.get("contract_match").and_then(Value::as_str),
        Some("file_names")
    );
    assert_eq!(
        snapshot
            .get("required_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["candidates"])
    );
    assert_eq!(
        snapshot.get("final_answer_shape").and_then(Value::as_str),
        Some("name_list")
    );
    assert!(snapshot
        .get("contract_matrix_hash")
        .and_then(Value::as_str)
        .is_some_and(|hash| !hash.is_empty()));
    let runtime_snapshot = trace
        .get("runtime_contract_snapshot")
        .expect("runtime contract snapshot should be present");
    assert_eq!(
        runtime_snapshot
            .get("contract")
            .and_then(|value| value.get("contract_match"))
            .and_then(Value::as_str),
        Some("file_names")
    );
    assert!(runtime_snapshot
        .get("compact_contract_block")
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .is_some_and(|hash| !hash.is_empty()));
}

#[test]
fn step_trace_includes_contract_and_action_policy_for_success() {
    let mut journal = TaskJournal::for_task("task-step-contract", "ask", "列出文件名");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
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
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.record_plan_result(&crate::PlanResult {
        goal: "list file names".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "call_skill".to_string(),
            skill: "fs_basic".to_string(),
            args: json!({"action": "list_dir", "path": "."}),
            depends_on: Vec::new(),
            why: String::new(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"items": [{"path": "README.md"}]}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let step_contract = trace
        .pointer("/step_results/0/contract")
        .expect("step contract trace should be present");

    assert_eq!(
        step_contract.get("contract_match").and_then(Value::as_str),
        Some("file_names")
    );
    assert_eq!(
        step_contract
            .get("final_answer_shape")
            .and_then(Value::as_str),
        Some("name_list")
    );
    assert_eq!(
        step_contract
            .get("action_policy")
            .and_then(|value| value.get("decision"))
            .and_then(Value::as_str),
        Some("allowed")
    );
    assert_eq!(
        step_contract
            .get("action_policy")
            .and_then(|value| value.get("action_ref"))
            .and_then(Value::as_str),
        Some("fs_basic.list_dir")
    );
    assert!(trace
        .pointer("/step_results/0/observed_evidence/items")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty()));
}

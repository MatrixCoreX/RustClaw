use serde_json::{json, Value};

use super::{
    delivery_payload_consistent, evidence_coverage_for_route, observed_evidence_from_output,
    TaskJournal, TaskJournalFinalStatus, TaskJournalFinalizerStage, TaskJournalFinalizerSummary,
    TaskJournalRolloutAttribution, TaskJournalRoundTrace, TaskJournalStepTrace,
    TaskJournalVerifyIssue, TaskJournalVerifySummary,
};

#[path = "task_journal_tests/observed_evidence_core_tests.rs"]
mod observed_evidence_core;

#[path = "task_journal_tests/skill_output_evidence_tests.rs"]
mod skill_output_evidence;

#[path = "task_journal_tests/system_basic_info_text_boundary.rs"]
mod system_basic_info_text_boundary;

#[path = "task_journal_tests/contract_coverage_tests.rs"]
mod contract_coverage;

#[path = "task_journal_tests/answer_verifier_envelope.rs"]
mod answer_verifier_envelope;
#[path = "task_journal_tests/event_stream_hooks.rs"]
mod event_stream_hooks;
#[path = "task_journal_tests/failure_attribution.rs"]
mod failure_attribution;
#[path = "task_journal_tests/frontdoor_llm_metrics.rs"]
mod frontdoor_llm_metrics;

fn route_for_semantic(semantic_kind: crate::OutputSemanticKind) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
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
        ask_mode: crate::AskMode::act_plain(),
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
                scalar_count_filter: Default::default(),
                list_selector: Default::default(),
                structured_field_selector: None,
            },
            ..Default::default()
        },
    });
    journal.record_finalizer_summary(TaskJournalFinalizerSummary {
        stage: Some(TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
        fallback: None,
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
    journal.record_llm_elapsed_ms_per_task(42);
    let mut by_prompt = std::collections::HashMap::new();
    by_prompt.insert(
        "normalizer".to_string(),
        crate::LlmPromptBucket {
            count: 1,
            elapsed_ms: 42,
            provider_attempt_count: 3,
            provider_retry_count: 2,
            provider_retryable_error_count: 2,
            provider_final_error_count: 0,
            provider_last_retry_error_kinds: std::collections::BTreeMap::from([(
                "timeout".to_string(),
                1,
            )]),
            provider_final_error_kinds: std::collections::BTreeMap::new(),
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
        Some("observed_generic")
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
            .and_then(|v| v.get("provider_attempt_count"))
            .and_then(Value::as_u64),
        Some(3)
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
            .get("cost_budget")
            .and_then(|v| v.get("policy_kind"))
            .and_then(Value::as_str),
        Some("loop_telemetry_rollout_gate")
    );
    assert_eq!(
        summary
            .get("cost_budget")
            .and_then(|v| v.get("semantic_authority"))
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        summary
            .get("cost_budget")
            .and_then(|v| v.pointer("/observed/provider_retries"))
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        summary
            .get("cost_budget")
            .and_then(|v| v.pointer("/observed/prompt_truncations"))
            .and_then(Value::as_u64),
        Some(1)
    );
    assert!(summary
        .get("cost_budget")
        .and_then(|v| v.get("signals"))
        .and_then(Value::as_array)
        .is_some_and(|signals| signals
            .iter()
            .any(|signal| signal.as_str() == Some("prompt_truncation_observed"))));
    assert_eq!(
        summary
            .get("route_result")
            .and_then(|v| v.get("boundary_mode"))
            .and_then(Value::as_str),
        Some("execute")
    );
    assert_eq!(
        summary
            .get("route_result")
            .and_then(|v| v.get("route_trace_decision"))
            .and_then(Value::as_str),
        Some("act")
    );
    for legacy_field in [
        "route_gate_kind",
        "initial_gate_ref",
        "initial_hint_ref",
        "legacy_first_layer_decision",
        "legacy_route_label",
    ] {
        assert!(
            summary
                .get("route_result")
                .and_then(|v| v.get(legacy_field))
                .is_none(),
            "route_result should not expose legacy field `{legacy_field}`"
        );
    }
    assert!(summary
        .get("route_result")
        .and_then(|v| v.get("first_layer_decision"))
        .is_none());
    assert!(summary
        .get("route_result")
        .and_then(|v| v.get("route_label"))
        .is_none());
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
fn summary_json_preserves_task_lifecycle_checkpoint_machine_fields() {
    let mut journal = TaskJournal::for_task("task-checkpoint", "ask", "long task");
    journal.record_task_lifecycle(json!({
        "schema_version": 1,
        "state": "waiting",
        "source": "agent_loop_soft_budget",
        "resume_reason": "agent_loop_no_progress_limit",
        "checkpoint_id": "ckpt-1"
    }));
    journal.record_task_checkpoint(json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-1",
        "resume_entrypoint": "next_planner_round"
    }));

    let summary = journal.to_summary_json();
    let trace = journal.to_trace_json();

    assert_eq!(
        summary
            .pointer("/task_lifecycle/resume_reason")
            .and_then(Value::as_str),
        Some("agent_loop_no_progress_limit")
    );
    assert_eq!(
        trace
            .pointer("/task_checkpoint/checkpoint_id")
            .and_then(Value::as_str),
        Some("ckpt-1")
    );
}

#[test]
fn agent_loop_decision_envelope_uses_structured_respond_clarify_intent() {
    let route = route_for_semantic(crate::OutputSemanticKind::None);
    let plan = crate::PlanResult {
        goal: "collect missing locator".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "",
                "terminal_intent": "clarify",
                "clarify_reason_code": "missing_locator",
                "missing_slot": "locator",
                "message_key": "clawd.msg.clarify.missing_locator",
                "field_path": "package.name",
                "locator_kind": "path"
            }),
            depends_on: Vec::new(),
            why: "structured clarify".to_string(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    };
    let envelope =
        super::decision_envelope::agent_loop_round_plan_decision_envelope_json(&route, &plan);

    assert_eq!(
        envelope.get("decision").and_then(Value::as_str),
        Some("clarify")
    );
    assert_eq!(
        envelope.get("terminal_intent").and_then(Value::as_str),
        Some("clarify")
    );
    assert_eq!(
        envelope.get("control_intent").and_then(Value::as_str),
        Some("clarify")
    );
    assert_eq!(
        envelope.get("control_reason_code").and_then(Value::as_str),
        Some("agent_loop_control_clarify_terminal_intent")
    );
    assert_eq!(
        envelope.get("reason_code").and_then(Value::as_str),
        Some("agent_loop_respond_terminal_intent_clarify")
    );
    assert_eq!(
        envelope.get("clarify_reason_code").and_then(Value::as_str),
        Some("missing_locator")
    );
    assert_eq!(
        envelope.get("missing_slot").and_then(Value::as_str),
        Some("locator")
    );
    assert_eq!(
        envelope
            .get("missing_slots")
            .and_then(Value::as_array)
            .and_then(|slots| slots.first())
            .and_then(Value::as_str),
        Some("locator")
    );
    assert_eq!(
        envelope.get("validation_status").and_then(Value::as_str),
        Some("valid")
    );
    assert_eq!(
        envelope.get("message_key").and_then(Value::as_str),
        Some("clawd.msg.clarify.missing_locator")
    );
    assert_eq!(
        envelope.get("field_path").and_then(Value::as_str),
        Some("package.name")
    );
    assert_eq!(
        envelope.get("locator_kind").and_then(Value::as_str),
        Some("path")
    );
}

#[test]
fn agent_loop_decision_envelope_maps_structured_wait_and_stop_intents() {
    let route = route_for_semantic(crate::OutputSemanticKind::None);
    let wait_plan = crate::PlanResult {
        goal: "require confirmation".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: true,
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "",
                "terminal_intent": "needs_confirmation",
                "message_key": "clawd.msg.confirmation.required"
            }),
            depends_on: Vec::new(),
            why: "structured wait".to_string(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    };
    let wait_envelope =
        super::decision_envelope::agent_loop_round_plan_decision_envelope_json(&route, &wait_plan);

    assert_eq!(
        wait_envelope.get("terminal_intent").and_then(Value::as_str),
        Some("needs_confirmation")
    );
    assert_eq!(
        wait_envelope.get("control_intent").and_then(Value::as_str),
        Some("wait")
    );
    assert_eq!(
        wait_envelope
            .get("control_reason_code")
            .and_then(Value::as_str),
        Some("agent_loop_control_wait_terminal_intent")
    );

    let stop_plan = crate::PlanResult {
        goal: "cannot continue".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "",
                "terminal_intent": "cannot_proceed",
                "message_key": "clawd.msg.cannot_proceed"
            }),
            depends_on: Vec::new(),
            why: "structured stop".to_string(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    };
    let stop_envelope =
        super::decision_envelope::agent_loop_round_plan_decision_envelope_json(&route, &stop_plan);

    assert_eq!(
        stop_envelope.get("terminal_intent").and_then(Value::as_str),
        Some("cannot_proceed")
    );
    assert_eq!(
        stop_envelope.get("control_intent").and_then(Value::as_str),
        Some("stop")
    );
    assert_eq!(
        stop_envelope
            .get("control_reason_code")
            .and_then(Value::as_str),
        Some("agent_loop_control_stop_terminal_intent")
    );
}

#[test]
fn agent_loop_decision_envelope_schema_drift() {
    const SCHEMA_RAW: &str =
        include_str!("../../../prompts/schemas/agent_loop_decision_envelope.schema.json");
    let schema: Value =
        serde_json::from_str(SCHEMA_RAW).expect("agent_loop_decision_envelope schema json");
    assert_eq!(
        schema.get("additionalProperties").and_then(Value::as_bool),
        Some(false)
    );
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("schema properties");
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .expect("schema required");
    let fields = [
        "schema_version",
        "source",
        "semantic_authority",
        "decision",
        "terminal_intent",
        "control_intent",
        "control_reason_code",
        "reason_code",
        "clarify_reason_code",
        "validation_status",
        "validation_reason_code",
        "confidence",
        "missing_slots",
        "missing_slot",
        "capability_ref",
        "output_contract_ref",
        "required_evidence",
        "evidence_needed",
        "answer_shape",
        "risk_level",
        "delivery_required",
        "language_rendering_policy",
    ];
    for field in fields {
        assert!(
            properties.contains_key(field),
            "schema missing field `{field}`"
        );
        assert!(
            required.iter().any(|value| value.as_str() == Some(field)),
            "schema required missing `{field}`"
        );
    }
    let decisions = properties
        .get("decision")
        .and_then(|value| value.get("enum"))
        .and_then(Value::as_array)
        .expect("decision enum");
    for token in ["respond", "clarify", "call_capability", "synthesize_answer"] {
        assert!(
            decisions.iter().any(|value| value.as_str() == Some(token)),
            "decision enum missing `{token}`"
        );
    }
    let terminal_intents = properties
        .get("terminal_intent")
        .and_then(|value| value.get("enum"))
        .and_then(Value::as_array)
        .expect("terminal_intent enum");
    for token in [
        "answer",
        "clarify",
        "cannot_proceed",
        "needs_confirmation",
        "continue",
    ] {
        assert!(
            terminal_intents
                .iter()
                .any(|value| value.as_str() == Some(token)),
            "terminal_intent enum missing `{token}`"
        );
    }
    let control_intents = properties
        .get("control_intent")
        .and_then(|value| value.get("enum"))
        .and_then(Value::as_array)
        .expect("control_intent enum");
    for token in ["answer", "clarify", "act", "recover", "wait", "stop"] {
        assert!(
            control_intents
                .iter()
                .any(|value| value.as_str() == Some(token)),
            "control_intent enum missing `{token}`"
        );
    }
    let rendering_policies = properties
        .get("language_rendering_policy")
        .and_then(|value| value.get("enum"))
        .and_then(Value::as_array)
        .expect("language rendering enum");
    for token in ["defer_until_observation", "finalizer_llm_i18n"] {
        assert!(
            rendering_policies
                .iter()
                .any(|value| value.as_str() == Some(token)),
            "language_rendering_policy enum missing `{token}`"
        );
    }
    let validation_statuses = properties
        .get("validation_status")
        .and_then(|value| value.get("enum"))
        .and_then(Value::as_array)
        .expect("validation status enum");
    for token in ["valid", "shadow_invalid"] {
        assert!(
            validation_statuses
                .iter()
                .any(|value| value.as_str() == Some(token)),
            "validation_status enum missing `{token}`"
        );
    }
    let semantic_authorities = properties
        .get("semantic_authority")
        .and_then(|value| value.get("enum"))
        .and_then(Value::as_array)
        .expect("semantic authority enum");
    assert!(semantic_authorities
        .iter()
        .any(|value| value.as_str() == Some("planner_loop_shadow")));
    for legacy_field in [
        "initial_hint_ref",
        "initial_gate_ref",
        "fallback_gate_policy",
    ] {
        assert!(
            !properties.contains_key(legacy_field),
            "schema should not expose legacy field `{legacy_field}`"
        );
        assert!(
            !required
                .iter()
                .any(|value| value.as_str() == Some(legacy_field)),
            "schema required should not expose legacy field `{legacy_field}`"
        );
    }
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
fn trace_json_includes_round_source_of_truth_machine_fields() {
    let route = route_for_semantic(crate::OutputSemanticKind::FileNames);
    let plan = crate::PlanResult {
        goal: "inspect workspace".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "call_capability".to_string(),
            skill: "fs.read_text_range".to_string(),
            args: json!({"path": "README.md"}),
            depends_on: Vec::new(),
            why: "read file".to_string(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    };
    let verify = TaskJournalVerifySummary {
        mode: crate::verifier::VerifyMode::ObserveOnly,
        approved: true,
        blocked_reason: None,
        shadow_blocked_reason: Some("missing argument".to_string()),
        permission_decision: json!({"schema_version":1,"owner_layer":"plan_verifier"}),
        needs_confirmation: false,
        issues: vec![TaskJournalVerifyIssue {
            step_id: "step_1".to_string(),
            kind: crate::verifier::VerifyIssueKind::MissingRequiredArg,
            detail: "path".to_string(),
            missing_fields: vec!["path".to_string()],
        }],
    };
    let mut journal = TaskJournal::for_task("task-round-source", "ask", "inspect");
    journal.record_route_result(&route);
    journal.record_rollout_attribution(TaskJournalRolloutAttribution {
        switch_name: "agent_loop_round_context".to_string(),
        event: "round_context_recorded".to_string(),
        outcome: "observed".to_string(),
        budget_profile: Some("fast_read".to_string()),
        boundary_context: Some(json!({"schema_version":1,"owner_layer":"boundary_layer"})),
        ..Default::default()
    });
    journal.record_final_stop_signal("max_tool_calls");
    journal.rounds.push(TaskJournalRoundTrace {
        round_no: 1,
        goal: "inspect workspace".to_string(),
        plan_result: Some(plan),
        verify_result: Some(verify),
        ..Default::default()
    });

    let trace = journal.to_trace_json();
    let round = trace
        .pointer("/rounds/0")
        .expect("round trace should be present");
    assert_eq!(
        round.get("owner_layer").and_then(Value::as_str),
        Some("agent_loop_round")
    );
    assert_eq!(
        round
            .pointer("/boundary_context_summary/owner_layer")
            .and_then(Value::as_str),
        Some("boundary_layer")
    );
    assert_eq!(
        round.get("budget_profile").and_then(Value::as_str),
        Some("fast_read")
    );
    assert_eq!(
        round.get("stop_signal").and_then(Value::as_str),
        Some("max_tool_calls")
    );
    assert_eq!(
        round.get("first_action_decision").and_then(Value::as_str),
        Some("call_capability")
    );
    assert_eq!(
        round
            .get("first_action_capability_ref")
            .and_then(Value::as_str),
        Some("fs.read_text_range")
    );
    assert_eq!(
        round
            .pointer("/capability_resolution_records/0/resolution_source")
            .and_then(Value::as_str),
        Some("capability_resolver")
    );
    assert_eq!(
        round
            .pointer("/repair_signals/0/status_code")
            .and_then(Value::as_str),
        Some("missing_required_arg")
    );
    assert_eq!(
        round
            .pointer("/repair_signals/0/message_key")
            .and_then(Value::as_str),
        Some("clawd.verify.missing_required_arg")
    );
    assert_eq!(
        round
            .pointer("/repair_signals/0/repair_envelope/failed_action_ref")
            .and_then(Value::as_str),
        Some("fs.read_text_range")
    );
    assert_eq!(
        round
            .pointer("/repair_signals/0/missing_fields/0")
            .and_then(Value::as_str),
        Some("path")
    );
    assert_eq!(
        round
            .pointer("/repair_signals/0/repair_envelope/missing_evidence/0")
            .and_then(Value::as_str),
        Some("path")
    );
    let forbidden_repeat = round
        .pointer("/repair_signals/0/forbidden_repeat_fingerprint")
        .and_then(Value::as_str)
        .expect("round repair signal should include plan-step fingerprint");
    assert!(
        forbidden_repeat.contains("read_text_range"),
        "fingerprint should include machine action ref, got {forbidden_repeat}"
    );
    assert!(
        forbidden_repeat.rsplit(':').next().is_some_and(|hash| {
            hash.len() == 16 && hash.chars().all(|ch| ch.is_ascii_hexdigit())
        }),
        "fingerprint should end with stable args hash, got {forbidden_repeat}"
    );
    assert_eq!(
        round
            .pointer("/verify_result/issues/0/forbidden_repeat_fingerprint")
            .and_then(Value::as_str),
        Some(forbidden_repeat)
    );
}

#[test]
fn task_journal_summary_projects_context_budget_report() {
    let mut journal = TaskJournal::for_task("task-context-budget", "ask", "inspect");
    journal.record_context_bundle_summary(
        r#"execution_view=true context_budget_report={"schema_version":1,"budget_tier":"light","included_ref_count":1,"included_refs":[{"ref":"runtime_context","char_count":64}],"excluded_ref_count":1,"excluded_refs":[{"ref":"recent_turns_full","reason":"not_included"}],"char_estimate":64,"token_estimate":16,"truncation_reason":"light_execution_budget","safety_reason":"context_budget_policy","compaction_source":"deterministic_context_builder"}"#,
    );

    let summary = journal.to_summary_json();

    assert_eq!(
        summary
            .pointer("/context_budget_report/budget_tier")
            .and_then(Value::as_str),
        Some("light")
    );
    assert_eq!(
        summary
            .pointer("/context_budget_report/included_refs/0/ref")
            .and_then(Value::as_str),
        Some("runtime_context")
    );
}

fn test_plan_step(step_id: &str, action_type: &str, skill: &str, args: Value) -> crate::PlanStep {
    crate::PlanStep {
        step_id: step_id.to_string(),
        action_type: action_type.to_string(),
        skill: skill.to_string(),
        args,
        depends_on: Vec::new(),
        why: String::new(),
    }
}

fn test_plan(kind: crate::PlanKind, steps: Vec<crate::PlanStep>) -> crate::PlanResult {
    crate::PlanResult {
        goal: String::new(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps,
        planner_notes: String::new(),
        plan_kind: kind,
        raw_plan_text: String::new(),
    }
}

#[test]
fn trace_json_matches_repeated_round_step_ids_by_execution_order_and_skill() {
    let mut journal = TaskJournal::for_task("task-repeat-step-ids", "ask", "ops repair");
    journal.rounds.push(TaskJournalRoundTrace {
        round_no: 1,
        goal: "current_phase=inspect".to_string(),
        execution_recipe_summary: None,
        plan_result: Some(test_plan(
            crate::PlanKind::Single,
            vec![
                test_plan_step(
                    "step_1",
                    "call_tool",
                    "fs_basic",
                    json!({"action": "read_text_range", "path": "index.html"}),
                ),
                test_plan_step(
                    "step_2",
                    "synthesize_answer",
                    "synthesize_answer",
                    json!({}),
                ),
                test_plan_step("step_3", "respond", "respond", json!({})),
            ],
        )),
        verify_result: None,
    });
    journal.rounds.push(TaskJournalRoundTrace {
        round_no: 2,
        goal: "current_phase=apply".to_string(),
        execution_recipe_summary: None,
        plan_result: Some(test_plan(
            crate::PlanKind::Repair,
            vec![
                test_plan_step(
                    "step_1",
                    "call_tool",
                    "http_basic",
                    json!({"action": "get", "url": "http://127.0.0.1:40459/"}),
                ),
                test_plan_step(
                    "step_2",
                    "call_tool",
                    "fs_basic",
                    json!({"action": "write_text", "path": "index.html", "content": "ok"}),
                ),
                test_plan_step(
                    "step_3",
                    "call_tool",
                    "http_basic",
                    json!({"action": "get", "url": "http://127.0.0.1:40459/"}),
                ),
                test_plan_step("step_4", "respond", "respond", json!({})),
            ],
        )),
        verify_result: None,
    });
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"bad"}"#,
    ));
    journal.step_results.push(TaskJournalStepTrace::new(
        "step_2",
        "synthesize_answer",
        crate::executor::StepExecutionStatus::Error,
        None,
        Some("active_recipe_terminal_discussion_before_done".to_string()),
    ));
    journal
        .step_results
        .push(TaskJournalStepTrace::ok("step_3", "fs_basic", "write ok"));
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_4",
        "http_basic",
        r#"{"status_code":200}"#,
    ));

    let trace = journal.to_trace_json();
    let steps = trace
        .get("step_results")
        .and_then(Value::as_array)
        .expect("step_results");
    assert_eq!(
        steps[2].get("requested_action_ref").and_then(Value::as_str),
        Some("fs_basic.write_text")
    );
    assert_eq!(
        steps[2]
            .get("requested_action_type")
            .and_then(Value::as_str),
        Some("call_tool")
    );
    assert_eq!(
        steps[3].get("requested_action_ref").and_then(Value::as_str),
        Some("http_basic.get")
    );
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
            .get("evidence_policy")
            .and_then(|value| value.get("contract_match"))
            .and_then(Value::as_str)
            .is_some(),
        "evidence-policy snapshot should survive trace compaction"
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
            permission_decision: json!({"schema_version":1,"owner_layer":"plan_verifier"}),
            needs_confirmation: false,
            issues: vec![TaskJournalVerifyIssue {
                step_id: "step_1".to_string(),
                kind: crate::verifier::VerifyIssueKind::ContractActionRejected,
                detail: "action rejected".to_string(),
                missing_fields: Vec::new(),
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
        issue.get("reason_code").and_then(Value::as_str),
        Some("verify_contract_action_rejected")
    );
    assert_eq!(
        issue.get("status_code").and_then(Value::as_str),
        Some("contract_action_rejected")
    );
    assert_eq!(
        issue.get("message_key").and_then(Value::as_str),
        Some("clawd.verify.contract_action_rejected")
    );
    assert_eq!(
        issue.get("owner_layer").and_then(Value::as_str),
        Some("plan_verifier")
    );
    assert_eq!(
        issue.get("failure_attribution").and_then(Value::as_str),
        Some("contract_gap")
    );
    let verify = trace
        .pointer("/rounds/0/verify_result")
        .expect("verify result should be present");
    assert_eq!(
        verify.get("blocked_reason_code").and_then(Value::as_str),
        Some("verify_contract_action_rejected")
    );
    assert_eq!(
        verify.get("blocked_status_code").and_then(Value::as_str),
        Some("contract_action_rejected")
    );
    assert_eq!(
        verify.get("blocked_message_key").and_then(Value::as_str),
        Some("clawd.verify.contract_action_rejected")
    );
    assert_eq!(
        verify.get("owner_layer").and_then(Value::as_str),
        Some("plan_verifier")
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
        output: Some(
            json!({"path": ".", "entries": ["README.md", "Cargo.toml"], "count": 2}).to_string(),
        ),
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
    assert_eq!(
        step.get("action_kind").and_then(Value::as_str),
        Some("call_tool")
    );
    assert_eq!(
        step.get("resolved_tool_or_skill").and_then(Value::as_str),
        Some("system_basic")
    );
    assert_eq!(
        step.get("resolution_source").and_then(Value::as_str),
        Some("direct_tool_or_skill_trace")
    );
    assert_eq!(
        step.get("sanitized_args_summary").and_then(Value::as_str),
        Some("system_basic.inventory_dir")
    );
    assert_eq!(
        step.get("sanitized_args_summary_status")
            .and_then(Value::as_str),
        Some("action_ref_only")
    );
    assert!(
        step.get("output_evidence_count")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            >= 1
    );
    assert_eq!(
        step.pointer("/artifact_refs/0/ref").and_then(Value::as_str),
        Some(".")
    );
    assert_eq!(
        step.get("retry_fingerprint_status").and_then(Value::as_str),
        Some("not_recorded_in_step_trace")
    );
}

#[test]
fn trace_json_artifact_refs_ignore_multiline_command_output_strings() {
    let mut journal = TaskJournal::for_task("task-artifact-refs", "ask", "inspect");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "process_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "output_path": "reports/out.txt",
                "extra": {
                    "action": "ps",
                    "output": "exit=0\nPID PPID %CPU %MEM COMM\n1272532 3209 0.7 0.3 clawd",
                    "text": "exit=0\nState Recv-Q Send-Q Local Address:Port"
                },
                "text": "exit=0\nPID PPID %CPU %MEM COMM"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let step = trace
        .pointer("/step_results/0")
        .and_then(Value::as_object)
        .expect("step trace");
    let refs = step
        .get("artifact_refs")
        .and_then(Value::as_array)
        .expect("artifact refs");

    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs[0].get("ref").and_then(Value::as_str),
        Some("reports/out.txt")
    );
    assert!(
        !refs.iter().any(|item| {
            item.get("ref")
                .and_then(Value::as_str)
                .is_some_and(|value| value.contains("PID PPID") || value.contains("State Recv-Q"))
        }),
        "raw command output must not be projected as an artifact ref: {refs:?}"
    );
}

#[test]
fn summary_json_includes_validation_result_machine_shape() {
    let mut journal = TaskJournal::for_task("task-validation", "ask", "validate");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "validation_result": {
                    "status": "passed",
                    "status_code": "config_valid",
                    "message_key": "clawd.validation.config_valid"
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let summary = journal.to_summary_json();
    let trace = journal.to_trace_json();

    assert_eq!(
        summary
            .pointer("/validation_result/validation_step_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        summary
            .pointer("/validation_result/latest_status")
            .and_then(Value::as_str),
        Some("passed")
    );
    assert_eq!(
        trace
            .pointer("/validation_result/signals/0/status_code")
            .and_then(Value::as_str),
        Some("config_valid")
    );
}

#[test]
fn summary_json_counts_unstructured_validation_command_as_observed() {
    let mut journal = TaskJournal::for_task("task-code-validation", "ask", "validate code");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "read_text_range",
                    "path": "/workspace/test_calc_core.py",
                    "resolved_path": "/workspace/test_calc_core.py",
                    "excerpt": "1|from calc_core import safe_div"
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("ALL_TESTS_PASSED".to_string()),
        error: None,
        started_at: 3,
        finished_at: 4,
    });

    let summary = journal.to_summary_json();

    assert_eq!(
        summary
            .pointer("/validation_result/validation_step_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        summary
            .pointer("/validation_result/latest_status")
            .and_then(Value::as_str),
        Some("observed")
    );
    assert_eq!(
        summary
            .pointer("/validation_result/signals/0/status_code")
            .and_then(Value::as_str),
        Some("validation_command_observed")
    );
}

#[test]
fn summary_json_does_not_mark_masked_run_cmd_validation_as_passed() {
    let mut journal = TaskJournal::for_task("task-masked-validation", "ask", "validate code");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "write_text",
                    "path": "/workspace/calc_core.py",
                    "resolved_path": "/workspace/calc_core.py",
                    "content_bytes": 42
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            "VALIDATION_COMMAND_OUTPUT_UNSTRUCTURED\nmasked_shell_exit_after_tail_command\n"
                .to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });

    let summary = journal.to_summary_json();

    assert_eq!(
        summary
            .pointer("/validation_result/validation_step_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        summary
            .pointer("/validation_result/latest_status")
            .and_then(Value::as_str),
        Some("observed")
    );
    assert_eq!(
        summary
            .pointer("/validation_result/signals/0/status_code")
            .and_then(Value::as_str),
        Some("validation_command_observed")
    );
}

#[test]
fn trace_json_compacts_plan_action_ref_to_contract_action() {
    let mut journal = TaskJournal::for_task("task-service", "ask", "check service");
    journal.record_route_result(&crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
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
fn structured_listing_journal_compact_prefers_names_by_kind_over_redundant_names() {
    let names = (0..80)
        .map(|idx| Value::String(format!("file_{idx:02}.md")))
        .collect::<Vec<_>>();
    let files = names.clone();
    let output = json!({
        "text": "raw fallback",
        "extra": {
            "action": "inventory_dir",
            "counts": {"dirs": 0, "files": 80, "hidden": 0, "total": 80},
            "path": ".",
            "resolved_path": "/tmp/repo",
            "names": names,
            "names_by_kind": {
                "dirs": [],
                "files": files,
                "other": []
            },
            "entries": []
        }
    })
    .to_string();

    let compact = super::compact_structured_listing_output_for_journal(&output)
        .expect("structured listing should compact");
    let value: Value = serde_json::from_str(&compact).expect("compact json");
    let extra = value.get("extra").expect("extra");
    assert!(extra.get("names").is_none());
    assert_eq!(
        extra
            .pointer("/names_by_kind/files")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(80)
    );
}

#[test]
fn structured_listing_journal_compact_preserves_entry_size_metadata() {
    let output = json!({
        "action": "inventory_dir",
        "path": "/tmp/logs",
        "resolved_path": "/tmp/logs",
        "names": ["clawd.log"],
        "entries": [
            {
                "name": "clawd.log",
                "kind": "file",
                "size_bytes": 2035,
                "modified_ts": 1780000000
            }
        ]
    })
    .to_string();

    let compact = super::compact_structured_listing_output_for_journal(&output)
        .expect("structured listing should compact");
    let value: Value = serde_json::from_str(&compact).expect("compact json");
    assert_eq!(
        value
            .pointer("/extra/entries/0/size_bytes")
            .and_then(Value::as_u64),
        Some(2035)
    );
    assert_eq!(
        value
            .pointer("/extra/entries/0/modified_ts")
            .and_then(Value::as_u64),
        Some(1780000000)
    );
}

#[test]
fn structured_listing_journal_compact_unwraps_text_json_when_extra_is_missing() {
    let text = json!({
        "action": "inventory_dir",
        "counts": {"dirs": 1, "files": 1, "hidden": 0, "total": 2},
        "path": ".",
        "resolved_path": "/tmp/repo",
        "names_by_kind": {
            "dirs": ["crates"],
            "files": ["README.md"],
            "other": []
        },
        "names": ["crates", "README.md"]
    })
    .to_string();
    let output = json!({ "text": text }).to_string();

    let compact = super::compact_structured_listing_output_for_journal(&output)
        .expect("text json should compact");
    let value: Value = serde_json::from_str(&compact).expect("compact json");
    assert_eq!(
        value
            .pointer("/extra/names_by_kind/dirs/0")
            .and_then(Value::as_str),
        Some("crates")
    );
    assert!(value.pointer("/extra/names").is_none());
}

#[test]
fn step_output_excerpt_compacts_write_text_without_truncating_json() {
    let long_path =
        "/home/guagua/rustclaw/run/nl_eval_tmp/codex_cli_continuous_20260711_new/test_calc_core.py";
    let output = json!({
        "extra": {
            "action": "write_text",
            "append": false,
            "content_bytes": 514,
            "effective_path": long_path,
            "path": long_path,
            "resolved_path": long_path,
            "schema_version": 1,
            "source": "builtin_success_extra"
        },
        "text": format!("written 514 bytes to {long_path}")
    })
    .to_string();

    let excerpt = super::step_output_excerpt_for_journal(&output);
    let value: Value = serde_json::from_str(&excerpt).expect("compact write json");

    assert_eq!(
        value.pointer("/extra/action").and_then(Value::as_str),
        Some("write_text")
    );
    assert_eq!(
        value.pointer("/extra/path").and_then(Value::as_str),
        Some(long_path)
    );
    assert_eq!(
        value
            .pointer("/extra/resolved_path")
            .and_then(Value::as_str),
        Some(long_path)
    );
    assert!(!excerpt.contains("truncated"));
}

#[test]
fn step_output_excerpt_compacts_read_range_as_valid_json() {
    let output = json!({
        "extra": {
            "action": "read_range",
            "path": "/workspace/calc_core.py",
            "resolved_path": "/workspace/calc_core.py",
            "excerpt": "1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b",
            "start_line": 1,
            "end_line": 4,
            "total_lines": 4
        },
        "text": "display text"
    })
    .to_string();

    let excerpt = super::step_output_excerpt_for_journal(&output);
    let value: Value = serde_json::from_str(&excerpt).expect("compact read json");

    assert_eq!(
        value.pointer("/extra/action").and_then(Value::as_str),
        Some("read_range")
    );
    assert!(value
        .pointer("/extra/excerpt")
        .and_then(Value::as_str)
        .is_some_and(|excerpt| excerpt.contains("def add")));
}

#[test]
fn step_output_excerpt_ignores_machine_json_hidden_in_text() {
    let hidden_action = json!({
        "action": "read_range",
        "mode": "tail",
        "requested_n": 2,
        "excerpt": "1|hidden\n2|text"
    })
    .to_string();
    let hidden_listing = json!({
        "action": "list_dir",
        "names": ["hidden.txt"]
    })
    .to_string();

    let action_excerpt = super::step_output_excerpt_for_journal(
        &json!({ "text": hidden_action.clone() }).to_string(),
    );
    let listing_excerpt = super::step_output_excerpt_for_journal(
        &json!({ "text": hidden_listing.clone() }).to_string(),
    );

    assert_eq!(action_excerpt, json!({ "text": hidden_action }).to_string());
    assert_eq!(
        listing_excerpt,
        json!({ "text": hidden_listing }).to_string()
    );
}

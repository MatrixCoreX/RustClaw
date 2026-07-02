use super::*;

#[test]
fn answer_verifier_contract_dry_run_returns_machine_contract_fields() {
    let mut route = base_route_result();
    route.resolved_intent = serde_json::json!({
        "dry_run": true,
        "verifier_contract": "answer_verifier_required_evidence",
        "required_evidence": [
            "required_evidence",
            "missing_evidence_fields",
            "contract_boundary"
        ],
        "missing_evidence_fields": [],
        "contract_boundary": {
            "owner_layer": "answer_verifier",
            "runtime_scope": "agent_loop"
        }
    })
    .to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "Dry run only: answer verifier contract envelope",
        Some(&route),
        &loop_state,
    )
    .expect("answer verifier dry-run contract should be deterministic");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("json response");
    assert_eq!(
        value.get("semantic_kind").and_then(Value::as_str),
        Some("answer_verifier_contract_dry_run")
    );
    assert!(value.get("required_evidence").is_some());
    assert!(value.get("missing_evidence_fields").is_some());
    assert!(value.get("contract_boundary").is_some());
}

#[test]
fn answer_verifier_contract_dry_run_ignores_bare_evidence_words() {
    let mut route = base_route_result();
    route.route_reason =
        "dry_run required_evidence missing_evidence_fields contract_boundary".to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "Dry run only: required_evidence missing_evidence_fields contract_boundary",
        Some(&route),
        &loop_state,
    );

    assert!(
        plan.is_none(),
        "bare verifier evidence words should not preempt planner authority"
    );
}

#[test]
fn async_job_contract_dry_run_exposes_lifecycle_checkpoint_fields() {
    let mut route = base_route_result();
    route.route_reason =
        "dry_run pending_async_job_contract poll_entrypoint=poll_async_job".to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "Dry run only: pending_async_job checkpoint_id poll_ref next_check_after can_cancel",
        Some(&route),
        &loop_state,
    )
    .expect("async job dry-run contract should be deterministic");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("json response");
    assert_eq!(
        value
            .pointer("/task_lifecycle/checkpoint_id")
            .and_then(Value::as_str),
        Some("opaque_checkpoint_id")
    );
    assert_eq!(
        value
            .pointer("/task_lifecycle/poll_ref")
            .and_then(Value::as_str),
        Some("adapter_result.job_id")
    );
    assert_eq!(
        value
            .pointer("/task_lifecycle/can_cancel")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn structured_dry_run_response_emits_async_job_poll_contract() {
    let mut route = base_route_result();
    route.route_reason = "async_job_protocol=version:1 mode=dry_run would_mutate=false".to_string();
    route.resolved_intent =
        "adapter_result_key=async_poll_adapter_result next_step=poll_async_job".to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "dry-run async job protocol",
        Some(&route),
        &loop_state,
    )
    .expect("machine dry-run async tokens should produce structured response");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected structured respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("structured JSON response");
    assert_eq!(
        value.get("semantic_kind").and_then(Value::as_str),
        Some("async_job_poll_contract_dry_run")
    );
    assert_eq!(
        value.get("would_mutate").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        value
            .pointer("/adapter_result/type")
            .and_then(Value::as_str),
        Some("pending_async_job")
    );
    assert_eq!(
        value
            .pointer("/async_timeout_policy/effective_deadline_ts")
            .and_then(Value::as_str),
        Some("min(deadline_ts,max_runtime_deadline_ts)")
    );
    assert_eq!(
        value
            .pointer("/async_timeout_policy/expired_terminal_status")
            .and_then(Value::as_str),
        Some("expired")
    );
    assert_eq!(
        value
            .pointer("/worker_loop/entrypoint")
            .and_then(Value::as_str),
        Some("poll_async_job")
    );
}

#[test]
fn async_timeout_policy_field_tokens_trigger_deterministic_contract() {
    let mut route = base_route_result();
    route.route_reason = "mode=dry_run async_timeout_policy policy_source=async_job_contract fields=effective_deadline_ts,expires_at,remaining_seconds,expired"
        .to_string();
    route.resolved_intent =
        "effective_deadline_ts expires_at remaining_seconds expired dry_run".to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "dry_run effective_deadline_ts expires_at remaining_seconds expired",
        Some(&route),
        &loop_state,
    )
    .expect("async timeout machine fields should produce structured response");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected structured respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("structured JSON response");
    assert_eq!(
        value
            .pointer("/async_timeout_policy/effective_deadline_ts")
            .and_then(Value::as_str),
        Some("min(deadline_ts,max_runtime_deadline_ts)")
    );
    assert_eq!(
        value
            .pointer("/async_timeout_policy/remaining_seconds")
            .and_then(Value::as_str),
        Some("max(effective_deadline_ts-now_ts,0)")
    );
}

#[test]
fn async_job_contract_dry_run_ignores_bare_legacy_async_tokens() {
    let mut route = base_route_result();
    route.route_reason = "dry_run pending_async_job poll_async_job".to_string();
    route.resolved_intent = "pending_async_job poll_async_job dry_run".to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "dry_run pending_async_job poll_async_job",
        Some(&route),
        &loop_state,
    );

    assert!(
        plan.is_none(),
        "bare async job words should not preempt planner authority"
    );
}

#[test]
fn async_job_contract_dry_run_ignores_prompt_only_protocol_envelope() {
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "async_job_protocol=version:1 dry_run adapter_result_key=async_poll_adapter_result",
        None,
        &loop_state,
    );

    assert!(
        plan.is_none(),
        "current user text must not trigger async-job dry-run policy"
    );
}

#[test]
fn async_timeout_policy_dry_run_requires_policy_envelope() {
    let mut route = base_route_result();
    route.route_reason =
        "mode=dry_run fields=effective_deadline_ts,expires_at,remaining_seconds,expired"
            .to_string();
    route.resolved_intent =
        "effective_deadline_ts expires_at remaining_seconds expired dry_run".to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "dry_run effective_deadline_ts expires_at remaining_seconds expired",
        Some(&route),
        &loop_state,
    );

    assert!(
        plan.is_none(),
        "timeout field names need async_timeout_policy + policy_source envelope"
    );
}

#[test]
fn local_process_cancel_dry_run_prefers_local_process_adapter_contract() {
    let mut route = base_route_result();
    route.route_reason =
        "dry_run cancel_ref adapter_kind=local_process_poll status=cancelled terminal_projection"
            .to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "Dry run only: cancel_ref adapter_kind=local_process_poll status=cancelled terminal_projection",
        Some(&route),
        &loop_state,
    )
    .expect("local process cancel dry-run contract should be deterministic");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("json response");
    assert_eq!(
        value.get("adapter_kind").and_then(Value::as_str),
        Some("local_process_poll")
    );
    assert_eq!(
        value.get("status").and_then(Value::as_str),
        Some("cancelled")
    );
    assert_eq!(
        value
            .pointer("/terminal_projection/state")
            .and_then(Value::as_str),
        Some("cancelled")
    );
}

#[test]
fn local_process_cancel_dry_run_ignores_bare_adapter_words() {
    let mut route = base_route_result();
    route.route_reason =
        "dry_run local_process_poll cancel_ref terminal_projection cancelled".to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "dry_run local_process_poll cancel_ref terminal_projection cancelled",
        Some(&route),
        &loop_state,
    );

    assert!(
        plan.is_none(),
        "bare local-process adapter words should not preempt planner authority"
    );
}

#[test]
fn observed_output_projection_dry_run_prefers_projection_contract() {
    let mut route = base_route_result();
    route.resolved_intent = serde_json::json!({
        "dry_run": true,
        "projection_contract": "observed_output_projection",
        "projection_policy": {
            "source": "observed_machine_output"
        },
        "families": [
            "scalar",
            "list",
            "path",
            "json_field",
            "status",
            "artifact_refs"
        ]
    })
    .to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "Dry run only: observed-output projection envelope",
        Some(&route),
        &loop_state,
    )
    .expect("observed-output projection dry-run contract should be deterministic");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("json response");
    assert_eq!(
        value.get("semantic_kind").and_then(Value::as_str),
        Some("observed_output_projection_dry_run")
    );
    let families = value
        .get("families")
        .and_then(Value::as_array)
        .expect("families");
    assert!(families
        .iter()
        .any(|item| item.as_str() == Some("artifact_refs")));
}

#[test]
fn observed_output_projection_dry_run_ignores_bare_projection_words() {
    let mut route = base_route_result();
    route.route_reason =
        "dry_run observed-output scalar list path json field status artifact_refs".to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "Dry run only: return observed-output scalar list path JSON field status artifact_refs",
        Some(&route),
        &loop_state,
    );

    assert!(
        plan.is_none(),
        "bare observed-output projection words should not preempt planner authority"
    );
}

#[test]
fn finalizer_language_policy_dry_run_returns_machine_policy_contract() {
    let mut route = base_route_result();
    route.route_reason = "dry_run message_key=clawd.finalizer.language_policy renderer=finalizer_llm_i18n output_contract=message_key_or_structured_evidence"
        .to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "dry_run finalizer policy envelope",
        Some(&route),
        &loop_state,
    )
    .expect("finalizer language policy dry-run contract should be deterministic");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("json response");
    assert_eq!(
        value.get("semantic_kind").and_then(Value::as_str),
        Some("finalizer_language_policy_dry_run")
    );
    assert_eq!(
        value.get("message_key").and_then(Value::as_str),
        Some("clawd.finalizer.language_policy")
    );
    assert_eq!(
        value
            .pointer("/final_reply_policy/renderer")
            .and_then(Value::as_str),
        Some("finalizer_llm_i18n")
    );
    assert_eq!(
        value
            .pointer("/structured_evidence/output_contract")
            .and_then(Value::as_str),
        Some("message_key_or_structured_evidence")
    );
}

#[test]
fn finalizer_language_policy_dry_run_requires_route_policy_envelope() {
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "dry-run message_key finalizer i18n structured evidence",
        None,
        &loop_state,
    );

    assert!(
        plan.is_none(),
        "current user text must not trigger finalizer dry-run policy"
    );
}

#[test]
fn finalizer_language_policy_dry_run_accepts_json_policy_envelope() {
    let mut route = base_route_result();
    route.resolved_intent = serde_json::json!({
        "dry_run": true,
        "message_key": "clawd.finalizer.language_policy",
        "final_reply_policy": {
            "renderer": "finalizer_llm_i18n"
        },
        "structured_evidence": {
            "output_contract": "message_key_or_structured_evidence"
        }
    })
    .to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "dry-run finalizer policy",
        Some(&route),
        &loop_state,
    )
    .expect("JSON policy envelope should trigger finalizer language dry-run contract");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("json response");
    assert_eq!(
        value.get("semantic_kind").and_then(Value::as_str),
        Some("finalizer_language_policy_dry_run")
    );
}

#[test]
fn finalizer_language_policy_dry_run_ignores_bare_policy_words() {
    let mut route = base_route_result();
    route.route_reason = "dry_run message_key finalizer i18n evidence".to_string();
    route.resolved_intent = "message_key finalizer i18n structured_evidence dry_run".to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "dry-run message_key finalizer i18n evidence",
        Some(&route),
        &loop_state,
    );

    assert!(
        plan.is_none(),
        "bare finalizer/i18n words should not preempt planner authority"
    );
}

#[test]
fn finalizer_language_policy_dry_run_can_preempt_initial_observation_state() {
    let mut route = base_route_result();
    route.route_reason = "dry_run message_key=clawd.finalizer.language_policy renderer=finalizer_llm_i18n output_contract=message_key_or_structured_evidence"
        .to_string();
    let mut loop_state = LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;

    let plan = structured_dry_run_response_deterministic_plan_result(
        "dry-run finalizer policy envelope",
        Some(&route),
        &loop_state,
    )
    .expect("first-round finalizer language dry-run should preempt observation guard");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("json response");
    assert_eq!(
        value.get("message_key").and_then(Value::as_str),
        Some("clawd.finalizer.language_policy")
    );
}

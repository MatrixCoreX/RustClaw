use super::*;

fn normalized_planner_actions(
    route: Option<&RouteResult>,
    loop_state: &LoopState,
    goal: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    normalize_planned_actions_with_original_and_context(
        &test_state(),
        route,
        loop_state,
        goal,
        Some(goal),
        None,
        None,
        actions,
    )
}

fn assert_planner_respond_json_preserved(
    route: &RouteResult,
    loop_state: &LoopState,
    goal: &str,
    content: Value,
) -> Value {
    let normalized = normalized_planner_actions(
        Some(route),
        loop_state,
        goal,
        vec![AgentAction::Respond {
            content: content.to_string(),
        }],
    );
    assert_eq!(
        normalized.len(),
        1,
        "planner-supplied dry-run respond should not expand into extra actions: {normalized:?}"
    );
    let Some(AgentAction::Respond { content }) = normalized.first() else {
        panic!("expected planner-supplied respond action, got {normalized:?}");
    };
    serde_json::from_str(content).expect("structured dry-run response json")
}

fn assert_planner_actions_stay_empty(
    route: Option<&RouteResult>,
    loop_state: &LoopState,
    goal: &str,
) {
    let normalized = normalized_planner_actions(route, loop_state, goal, vec![]);
    assert!(
        normalized.is_empty(),
        "runtime must not inject a dry-run contract plan before the planner: {normalized:?}"
    );
}

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

    let value = assert_planner_respond_json_preserved(
        &route,
        &loop_state,
        "Dry run only: answer verifier contract envelope",
        json!({
            "contract_marker": "answer_verifier_contract_dry_run",
            "required_evidence": ["required_evidence", "missing_evidence_fields", "contract_boundary"],
            "missing_evidence_fields": [],
            "contract_boundary": {"owner_layer": "answer_verifier", "runtime_scope": "agent_loop"}
        }),
    );

    assert_eq!(
        value.get("contract_marker").and_then(Value::as_str),
        Some("answer_verifier_contract_dry_run")
    );
    assert!(value.get("semantic_kind").is_none());
    assert!(value.get("required_evidence").is_some());
    assert!(value.get("missing_evidence_fields").is_some());
    assert!(value.get("contract_boundary").is_some());
}

#[test]
fn answer_verifier_contract_dry_run_ignores_bare_evidence_words() {
    let mut route = base_route_result();
    route.route_reason =
        "dry_run required_evidence missing_evidence_fields contract_boundary".to_string();

    assert_planner_actions_stay_empty(
        Some(&route),
        &LoopState::new(1),
        "Dry run only: required_evidence missing_evidence_fields contract_boundary",
    );
}

#[test]
fn async_job_contract_dry_run_exposes_lifecycle_checkpoint_fields() {
    let mut route = base_route_result();
    route.route_reason =
        "dry_run pending_async_job_contract poll_entrypoint=poll_async_job".to_string();
    let loop_state = LoopState::new(1);

    let value = assert_planner_respond_json_preserved(
        &route,
        &loop_state,
        "Dry run only: pending_async_job checkpoint_id poll_ref next_check_after can_cancel",
        json!({
            "task_lifecycle": {
                "checkpoint_id": "opaque_checkpoint_id",
                "poll_ref": "adapter_result.job_id",
                "can_cancel": true
            }
        }),
    );

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

    let value = assert_planner_respond_json_preserved(
        &route,
        &loop_state,
        "dry-run async job protocol",
        json!({
            "contract_marker": "async_job_poll_contract_dry_run",
            "would_mutate": false,
            "adapter_result": {"type": "pending_async_job"},
            "async_timeout_policy": {
                "effective_deadline_ts": "min(deadline_ts,max_runtime_deadline_ts)",
                "expired_terminal_status": "expired"
            },
            "worker_loop": {"entrypoint": "poll_async_job"}
        }),
    );

    assert_eq!(
        value.get("contract_marker").and_then(Value::as_str),
        Some("async_job_poll_contract_dry_run")
    );
    assert!(value.get("semantic_kind").is_none());
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
fn async_timeout_policy_field_tokens_trigger_planner_supplied_contract() {
    let mut route = base_route_result();
    route.route_reason = "mode=dry_run async_timeout_policy policy_source=async_job_contract fields=effective_deadline_ts,expires_at,remaining_seconds,expired"
        .to_string();
    route.resolved_intent =
        "effective_deadline_ts expires_at remaining_seconds expired dry_run".to_string();
    let loop_state = LoopState::new(1);

    let value = assert_planner_respond_json_preserved(
        &route,
        &loop_state,
        "dry_run effective_deadline_ts expires_at remaining_seconds expired",
        json!({
            "async_timeout_policy": {
                "effective_deadline_ts": "min(deadline_ts,max_runtime_deadline_ts)",
                "remaining_seconds": "max(effective_deadline_ts-now_ts,0)"
            }
        }),
    );

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

    assert_planner_actions_stay_empty(
        Some(&route),
        &LoopState::new(1),
        "dry_run pending_async_job poll_async_job",
    );
}

#[test]
fn async_job_contract_dry_run_ignores_prompt_only_protocol_envelope() {
    assert_planner_actions_stay_empty(
        None,
        &LoopState::new(1),
        "async_job_protocol=version:1 dry_run adapter_result_key=async_poll_adapter_result",
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

    assert_planner_actions_stay_empty(
        Some(&route),
        &LoopState::new(1),
        "dry_run effective_deadline_ts expires_at remaining_seconds expired",
    );
}

#[test]
fn local_process_cancel_dry_run_prefers_local_process_adapter_contract() {
    let mut route = base_route_result();
    route.route_reason =
        "dry_run cancel_ref adapter_kind=local_process_poll status=cancelled terminal_projection"
            .to_string();
    let loop_state = LoopState::new(1);

    let value = assert_planner_respond_json_preserved(
        &route,
        &loop_state,
        "Dry run only: cancel_ref adapter_kind=local_process_poll status=cancelled terminal_projection",
        json!({
            "adapter_kind": "local_process_poll",
            "status": "cancelled",
            "terminal_projection": {"state": "cancelled"}
        }),
    );

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

    assert_planner_actions_stay_empty(
        Some(&route),
        &LoopState::new(1),
        "dry_run local_process_poll cancel_ref terminal_projection cancelled",
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

    let value = assert_planner_respond_json_preserved(
        &route,
        &loop_state,
        "Dry run only: observed-output projection envelope",
        json!({
            "contract_marker": "observed_output_projection_dry_run",
            "families": ["scalar", "list", "path", "json_field", "status", "artifact_refs"]
        }),
    );

    assert_eq!(
        value.get("contract_marker").and_then(Value::as_str),
        Some("observed_output_projection_dry_run")
    );
    assert!(value.get("semantic_kind").is_none());
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

    assert_planner_actions_stay_empty(
        Some(&route),
        &LoopState::new(1),
        "Dry run only: return observed-output scalar list path JSON field status artifact_refs",
    );
}

#[test]
fn finalizer_language_policy_dry_run_returns_machine_policy_contract() {
    let mut route = base_route_result();
    route.route_reason = "dry_run message_key=clawd.finalizer.language_policy renderer=finalizer_llm_i18n output_contract=message_key_or_structured_evidence"
        .to_string();
    let loop_state = LoopState::new(1);

    let value = assert_planner_respond_json_preserved(
        &route,
        &loop_state,
        "dry_run finalizer policy envelope",
        json!({
            "contract_marker": "finalizer_language_policy_dry_run",
            "message_key": "clawd.finalizer.language_policy",
            "final_reply_policy": {"renderer": "finalizer_llm_i18n"},
            "structured_evidence": {"output_contract": "message_key_or_structured_evidence"}
        }),
    );

    assert_eq!(
        value.get("contract_marker").and_then(Value::as_str),
        Some("finalizer_language_policy_dry_run")
    );
    assert!(value.get("semantic_kind").is_none());
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
    assert_planner_actions_stay_empty(
        None,
        &LoopState::new(1),
        "dry-run message_key finalizer i18n structured evidence",
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

    let value = assert_planner_respond_json_preserved(
        &route,
        &loop_state,
        "dry-run finalizer policy",
        json!({
            "contract_marker": "finalizer_language_policy_dry_run"
        }),
    );

    assert_eq!(
        value.get("contract_marker").and_then(Value::as_str),
        Some("finalizer_language_policy_dry_run")
    );
    assert!(value.get("semantic_kind").is_none());
}

#[test]
fn finalizer_language_policy_dry_run_ignores_bare_policy_words() {
    let mut route = base_route_result();
    route.route_reason = "dry_run message_key finalizer i18n evidence".to_string();
    route.resolved_intent = "message_key finalizer i18n structured_evidence dry_run".to_string();

    assert_planner_actions_stay_empty(
        Some(&route),
        &LoopState::new(1),
        "dry-run message_key finalizer i18n evidence",
    );
}

#[test]
fn finalizer_language_policy_dry_run_can_preempt_initial_observation_state() {
    let mut route = base_route_result();
    route.route_reason = "dry_run message_key=clawd.finalizer.language_policy renderer=finalizer_llm_i18n output_contract=message_key_or_structured_evidence"
        .to_string();
    let mut loop_state = LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;

    let value = assert_planner_respond_json_preserved(
        &route,
        &loop_state,
        "dry-run finalizer policy envelope",
        json!({
            "message_key": "clawd.finalizer.language_policy"
        }),
    );

    assert_eq!(
        value.get("message_key").and_then(Value::as_str),
        Some("clawd.finalizer.language_policy")
    );
}

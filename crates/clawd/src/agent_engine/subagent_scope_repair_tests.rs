use super::*;

fn bounded_args() -> Value {
    json!({"children":[{"node_id":"reader","role":"explorer","objective":"inspect input",
        "context_refs":["input.json"],"allowed_capabilities":["filesystem.read_text_range"],
        "capability_argument_constraints":[{"capability":"filesystem.read_text_range","allowed_args":[{
            "type":"object","required":["path","mode","start_line","end_line"],"properties":{
                "path":{"enum":["input.json"]},"mode":{"enum":["range"]},
                "start_line":{"enum":[1]},"end_line":{"enum":[1]}}}]}]}]})
}

fn plan(args: Value) -> crate::PlanResult {
    crate::PlanResult {
        goal: String::new(),
        missing_slots: vec![],
        needs_confirmation: false,
        output_contract: None,
        steps: vec![crate::PlanStep {
            step_id: "step_1".into(),
            action_type: "call_capability".into(),
            skill: "agent.subagent_batch".into(),
            args,
            depends_on: vec![],
            why: String::new(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    }
}

fn task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 1,
        task_id: uuid::Uuid::new_v4().to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: "{}".into(),
    }
}

#[test]
fn rejected_plan_scope_cannot_be_dropped_or_widened_before_enqueue() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_seeded_db_schema()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = task();
    for replacement in [None, Some(json!([1, 2]))] {
        let mut source = bounded_args();
        source["children"][0]["result_contract"] = json!({"required":true});
        let mut loop_state = super::super::LoopState::new();
        remember_rejected_plan(&state, &mut loop_state, &plan(source));
        let mut retry = bounded_args();
        match replacement {
            None => {
                retry["children"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("capability_argument_constraints");
            }
            Some(value) => {
                retry["children"][0]["capability_argument_constraints"][0]["allowed_args"][0]
                    ["properties"]["end_line"]["enum"] = value
            }
        }
        let result = super::super::subagent_runtime::record_durable_readonly_child_task_from_args(
            &state,
            &task,
            &mut loop_state,
            1,
            1,
            &retry,
            &super::super::subagent_runtime::SubagentRuntimeConfig::default(),
        );
        assert_eq!(result.unwrap_err(), "subagent_argument_constraints_invalid");
        let observation = loop_state.task_observations.last().unwrap();
        assert_eq!(
            observation["field_issues"][0]["reason"],
            "prior_argument_scope_must_be_preserved"
        );
        let required = &observation["field_issues"][0]["required_constraint"];
        assert_eq!(
            required["allowed_args"][0]["properties"]["path"]["enum"],
            json!(["input.json"])
        );
        assert_eq!(
            required["allowed_args"][0]["properties"]["end_line"]["enum"],
            json!([1])
        );
        assert_eq!(observation["provider_called"], false);
        assert_eq!(observation["side_effect_applied"], false);
        assert!(loop_state
            .task_budget_slice
            .as_ref()
            .is_none_or(|slice| slice.allocations.is_empty()));
        let count: i64 = state
            .core
            .db
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM child_task_graph_nodes WHERE parent_task_id=?1",
                [&task.task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[test]
fn scope_survives_checkpoint_without_restoring_lossy_plan_arguments() {
    let mut source = super::super::LoopState::new();
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    remember_rejected_plan(&state, &mut source, &plan(bounded_args()));
    let checkpoint = super::super::checkpoint_resume_state::build_checkpoint_resume_state(
        &source,
        super::super::checkpoint_resume_state::AgentCheckpointStage::Planning,
    );
    let mut loop_state = super::super::LoopState::new();
    super::super::checkpoint_resume_state::restore_checkpoint_resume_state(
        &mut loop_state,
        &json!({"agent_loop_resume_state":checkpoint}),
    );
    let mut restored = loop_state.subagent_scope_repairs;
    let mut retry = bounded_args();
    retry["children"][0]
        .as_object_mut()
        .unwrap()
        .remove("capability_argument_constraints");
    assert_eq!(restored.admission_issues(&retry, 0, None).len(), 1);
    assert!(restored
        .admission_issues(&bounded_args(), 0, None)
        .is_empty());
}

#[test]
fn repeated_invalid_proposals_cannot_replace_the_first_valid_scope() {
    let mut guard = ScopeRepairState::default();
    guard.remember(&bounded_args(), 0, None);
    let mut wider = bounded_args();
    wider["children"][0]["capability_argument_constraints"][0]["allowed_args"][0]["properties"]
        ["path"]["enum"] = json!(["another.json"]);
    guard.remember(&wider, 0, None);
    assert_eq!(guard.admission_issues(&wider, 0, None).len(), 1);
    assert!(guard.admission_issues(&bounded_args(), 0, None).is_empty());
}

#[test]
fn ordinary_business_fields_and_malformed_scopes_do_not_create_repair_authority() {
    let mut loop_state = super::super::LoopState::new();
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let mut ordinary = plan(bounded_args());
    ordinary.steps[0].skill = "fixture.inspect".into();
    remember_rejected_plan(&state, &mut loop_state, &ordinary);
    assert!(loop_state.subagent_scope_repairs.nodes.is_empty());
    let mut malformed = bounded_args();
    malformed["children"][0]["capability_argument_constraints"][0]["allowed_args"] = json!([]);
    remember_args(&state, &mut loop_state, &malformed);
    assert!(loop_state.subagent_scope_repairs.nodes.is_empty());
}

#[test]
fn explicit_instruction_revision_starts_a_new_repair_scope() {
    let mut guard = ScopeRepairState::default();
    guard.remember(&bounded_args(), 0, None);
    let mut amended = bounded_args();
    amended["children"][0]
        .as_object_mut()
        .unwrap()
        .remove("capability_argument_constraints");
    assert_eq!(guard.admission_issues(&amended, 0, None).len(), 1);
    assert!(guard.admission_issues(&amended, 1, None).is_empty());
}

#[test]
fn malformed_checkpoint_fails_closed_and_never_silently_discards_scopes() {
    let mut guard = ScopeRepairState::restore(&json!({"nodes":{"reader":[]}}), 7);
    assert_eq!(
        guard.admission_issues(&bounded_args(), 7, None)[0]["reason"],
        "scope_repair_state_unavailable"
    );
}

#[test]
fn equivalent_required_key_order_is_accepted_without_relaxing_values() {
    let mut guard = ScopeRepairState::default();
    guard.remember(&bounded_args(), 0, None);
    let mut reordered = bounded_args();
    reordered["children"][0]["capability_argument_constraints"][0]["allowed_args"][0]["required"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert!(guard.admission_issues(&reordered, 0, None).is_empty());
}

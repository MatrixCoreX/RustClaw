use super::*;
use crate::agent_engine::LoopState;
use claw_core::capability_result::CapabilityResultEnvelope;

fn task(limits: Value) -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 1,
        task_id: "budget-child".into(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: json!({
            "task_role":"subagent_child", "parent_task_id":"parent", "child_task_id":"budget-child",
            "child_task_contract":{"scope":{
                "allowed_capabilities":["filesystem.read_text_range"],
                "capability_call_limits":limits
            }}
        })
        .to_string(),
    }
}

#[test]
fn limits_reject_unknown_duplicate_negative_and_malformed_caps() {
    let allowed = vec!["filesystem.read_text_range".to_string()];
    let cap = json!({"capability":"filesystem.read_text_range","max_calls":2});
    assert!(valid_capability_call_limits(
        &json!([cap.clone()]),
        &allowed
    ));
    for invalid in [
        json!([cap.clone(), cap]),
        json!([{"capability":"process.ps","max_calls":1}]),
        json!([{"capability":"filesystem.read_text_range","max_calls":-1}]),
        json!([{"capability":"filesystem.read_text_range","max_calls":65}]),
        json!([{"capability":"filesystem.read_text_range","max_calls":2,"override":true}]),
        json!({}),
    ] {
        assert!(!valid_capability_call_limits(&invalid, &allowed));
    }
}

#[test]
fn third_call_is_blocked_even_with_larger_total_budget_after_checkpoint_resume() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = task(json!([{"capability":"filesystem.read_text_range","max_calls":2}]));
    let mut original = LoopState::new();
    for _ in 0..2 {
        assert!(capability_budget_error(
            &state,
            &task,
            &original,
            "filesystem.read_text_range",
            "fs_basic"
        )
        .is_none());
        original
            .capability_results
            .push(CapabilityResultEnvelope::ok(
                "filesystem.read_text_range",
                None,
                json!({}),
            ));
    }
    let checkpoint: crate::task_lifecycle::TaskCheckpoint = serde_json::from_value(json!({
        "schema_version":1,"checkpoint_id":"checkpoint-budget","boundary_context":{},
        "observations":[],"capability_results":original.capability_results,
        "evidence_refs":[],"artifact_refs":[],"completed_side_effect_refs":[],
        "budget":{"round":2,"step":2,"llm_calls":2,"tool_calls":2,"elapsed_ms":1},
        "resume_entrypoint":"next_planner_round"
    }))
    .unwrap();
    let mut resumed = LoopState::new();
    crate::agent_engine::seed_loop_state_from_task_checkpoint(&mut resumed, &checkpoint);
    let error = capability_budget_error(
        &state,
        &task,
        &resumed,
        "filesystem.read_text_range",
        "fs_basic",
    )
    .unwrap();
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, "child_capability_budget_exhausted");
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["used_calls"], 2);
    assert_eq!(extra["max_calls"], 2);
    assert_eq!(extra["side_effect_applied"], false);
    assert!(has_scoped_capability_budget(&task));
    let context = planner_budget_context(&state, &task, &resumed).unwrap();
    assert_eq!(context["child_task_id"], "budget-child");
    assert_eq!(context["capability_call_limits"][0]["remaining_calls"], 0);
}

#[test]
fn zero_budget_disables_execution_and_unbounded_sibling_capability_is_unchanged() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = task(json!([{"capability":"filesystem.read_text_range","max_calls":0}]));
    let loop_state = LoopState::new();
    assert!(capability_budget_error(
        &state,
        &task,
        &loop_state,
        "filesystem.read_text_range",
        "fs_basic"
    )
    .is_some());
    assert!(capability_budget_error(
        &state,
        &task,
        &loop_state,
        "filesystem.find_entries",
        "fs_basic"
    )
    .is_none());
    let mut normal = task.clone();
    normal.payload_json = json!({"text":"normal request"}).to_string();
    assert!(capability_budget_error(
        &state,
        &normal,
        &loop_state,
        "filesystem.read_text_range",
        "fs_basic"
    )
    .is_none());
}

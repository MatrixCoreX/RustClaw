use super::*;

#[tokio::test]
async fn finalize_loop_reply_accepts_terminal_machine_json_envelope() {
    let state = test_state();
    let task = claimed_task("task-machine-envelope-terminal");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let envelope = serde_json::json!({
        "output_format": "machine_json",
        "owner_layer": "subagent_boundary_review",
        "boundary": {
            "write_enabled": false,
            "external_publish_enabled": false
        },
        "alignment": {
            "evidence_refs": ["step_1", "step_2", "step_3"]
        }
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "subagent",
        r#"{"owner_layer":"subagent_runtime"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", &envelope));
    loop_state.last_user_visible_respond = Some(envelope.clone());
    loop_state.delivery_messages.push(envelope.clone());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "review boundary",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("machine envelope should finalize");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert_eq!(reply.text.trim(), envelope);
    let summary = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.finalizer_summary.as_ref())
        .expect("finalizer summary");
    assert!(summary.contract_ok);
    assert_eq!(summary.completion_ok, Some(true));
}

#[tokio::test]
async fn finalize_loop_reply_promotes_machine_json_last_respond_to_delivery() {
    let state = test_state();
    let task = claimed_task("task-machine-envelope-last-respond");
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let envelope = serde_json::json!({
        "output_format": "machine_json",
        "owner_layer": "subagent_boundary_review",
        "boundary": {
            "write_enabled": false,
            "external_publish_enabled": false
        },
        "alignment": {
            "evidence_refs": ["step_1", "step_2", "step_3"]
        }
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"path":"AGENTS.md"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", &envelope));
    loop_state.last_user_visible_respond = Some(envelope.clone());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "review boundary",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("machine envelope should be promoted from last respond");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert_eq!(reply.text.trim(), envelope);
    assert_eq!(reply.messages, vec![envelope.clone()]);
    let summary = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.finalizer_summary.as_ref())
        .expect("finalizer summary");
    assert!(summary.contract_ok);
    assert_eq!(summary.completion_ok, Some(true));
}

#[tokio::test]
async fn finalize_loop_reply_promotes_machine_json_step_output_to_delivery() {
    let state = test_state();
    let task = claimed_task("task-machine-envelope-step-output");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let envelope = serde_json::json!({
        "output_format": "machine_json",
        "owner_layer": "subagent_batch_surface",
        "execution_mode": "bounded_parallel_readonly_child_runs",
        "aggregation": {
            "finding_refs": [
                "subagent-batch:1:3:1:explorer",
                "subagent-batch:1:3:2:verifier"
            ]
        }
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"path":"AGENTS.md"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", &envelope));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "subagent batch surface",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("machine envelope should be promoted from step output");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert_eq!(reply.text.trim(), envelope);
    assert_eq!(reply.messages, vec![envelope.clone()]);
    let summary = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.finalizer_summary.as_ref())
        .expect("finalizer summary");
    assert!(summary.contract_ok);
    assert_eq!(summary.completion_ok, Some(true));
}

#[tokio::test]
async fn finalize_loop_reply_prefers_subagent_machine_envelope_over_later_prose() {
    let state = test_state();
    let task = claimed_task("task-machine-envelope-over-prose");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let envelope = serde_json::json!({
        "schema_version": 1,
        "output_format": "machine_json",
        "owner_layer": "subagent_runtime",
        "status": "accepted",
        "execution_mode": "bounded_parallel_readonly_child_runs",
        "aggregation": {
            "status": "completed",
            "finding_refs": [
                "subagent-batch:1:3:1:explorer",
                "subagent-batch:1:3:2:verifier"
            ]
        }
    })
    .to_string();
    let later_prose =
        "execution_mode=inline_readonly_child_run finding_refs=subagent-batch:1:3:1:explorer";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "subagent", &envelope));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        later_prose,
    ));
    loop_state.last_user_visible_respond = Some(later_prose.to_string());
    loop_state.delivery_messages.push(later_prose.to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "subagent batch surface",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("machine envelope should win over prose");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert_eq!(reply.text.trim(), envelope);
    assert_eq!(reply.messages, vec![envelope.clone()]);
}

#[tokio::test]
async fn finalize_loop_reply_projects_subagent_child_model_result_from_runtime_envelope() {
    let state = test_state();
    let task = claimed_task("task-subagent-child-model-result-projection");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let child_result = serde_json::json!({
        "schema_version": 1,
        "owner_layer": "subagent_model_child",
        "output_format": "machine_json",
        "status": "needs_more_evidence",
        "role": "review",
        "findings": [
            {
                "code": "boundary_partial",
                "summary": "plan references available boundaries"
            }
        ],
        "evidence_refs": ["AGENTS.md", "plan/current.md"],
        "confidence": 0.74
    });
    let runtime_envelope = serde_json::json!({
        "schema_version": 1,
        "output_format": "machine_json",
        "owner_layer": "subagent_runtime",
        "status": "accepted",
        "context_evidence": {
            "items": [
                {
                    "path": "AGENTS.md",
                    "content_excerpt": "large internal evidence block"
                }
            ]
        },
        "child_model_result": child_result.clone()
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "subagent", &runtime_envelope));
    loop_state.last_user_visible_respond = Some(runtime_envelope.clone());
    loop_state.delivery_messages.push(runtime_envelope);

    let reply = finalize_loop_reply(
        &state,
        &task,
        "subagent boundary review",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("child model result should be projected");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    let projected: serde_json::Value =
        serde_json::from_str(&reply.text).expect("reply should remain machine json");
    assert_eq!(projected, child_result);
    assert_eq!(reply.messages, vec![child_result.to_string()]);
    assert!(!reply.text.contains("context_evidence"));
}

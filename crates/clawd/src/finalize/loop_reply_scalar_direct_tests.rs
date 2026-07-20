use super::*;
use crate::finalize::loop_reply::{
    enforce_delivery_output_contract, replace_delivery_with_direct_structured_observed_answer,
};

#[tokio::test]
async fn finalize_loop_reply_prefers_observed_raw_scalar_after_synthesis_error() {
    let state = test_state();
    let task = claimed_task("task-raw-scalar-synthesis-error");
    let mut route = scalar_route_result();
    route.configure_exact_command_output();
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"runtime_status","kind":"current_user","value":"guagua","field_value":"guagua","command_output":"guagua"}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "synthesize_answer",
        "synthesis failed",
    ));
    loop_state.delivery_messages.push(
        "获取到的当前用户名是 `guagua`。如果结果不符合预期，请提供更具体的查询条件。".to_string(),
    );
    loop_state.last_publishable_synthesis_output = loop_state.delivery_messages.last().cloned();

    let reply = finalize_loop_reply(
        &state,
        &task,
        "只输出当前用户名，不要解释",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should prefer observed scalar");

    assert_eq!(reply.text, "guagua");
    assert_eq!(reply.messages, vec!["guagua".to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_replaces_scalar_machine_assignment_with_observed_value() {
    let state = test_state();
    let task = claimed_task("task-schema-version-scalar-machine-assignment");
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.requires_content_evidence = true;
    route.delivery_required = false;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"extra":{"action":"schema_version","field_value":{"schema_version":3},"schema_version":3},"text":"{\"columns\":[\"schema_version\"],\"rows\":[{\"schema_version\":3}]}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "schema_version=3",
    ));
    loop_state
        .delivery_messages
        .push("schema_version=3".to_string());
    loop_state.last_user_visible_respond = Some("schema_version=3".to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "read sqlite schema_version and output only the number",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should project bare scalar");

    assert_eq!(reply.text, "3");
    assert_eq!(reply.messages, vec!["3".to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_preserves_publishable_evidence_summary_over_scalar_projection() {
    let state = test_state();
    let task = claimed_task("task-generic-evidence-summary-over-scalar");
    let mut route = free_route_result();
    route.locator_kind = OutputLocatorKind::None;
    route.locator_hint.clear();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Free;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let summary = "Working directory: /home/guagua/rustclaw. A clawd process and a listening port are both visible in the current task evidence.";
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "process_basic",
        r#"{"extra":{"action":"port_list","ports":[8787],"public_ports":[8787],"listeners":[{"port":8787,"process_name":"clawd"}]},"text":"port=8787"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", summary));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_4", "respond", summary));
    loop_state.last_publishable_synthesis_output = Some(summary.to_string());
    loop_state.last_user_visible_respond = Some("port=8787".to_string());

    assert!(
        direct_scalar_observed_answer(Some(&state), &loop_state, Some(&agent_run_context))
            .is_none(),
        "summary-with-evidence routes should not be compressed into one observed scalar"
    );

    let mut staged_loop_state = loop_state.clone();
    backfill_delivery_from_last_outputs(&task, &mut staged_loop_state, Some(&agent_run_context));
    assert_eq!(
        final_answer_text_from_delivery(&staged_loop_state.delivery_messages),
        summary
    );
    enforce_delivery_output_contract(
        &state,
        &task,
        "execute and summarize the observed working directory plus local process evidence",
        &mut staged_loop_state,
        Some(&agent_run_context),
    )
    .await;
    assert_eq!(
        final_answer_text_from_delivery(&staged_loop_state.delivery_messages),
        summary
    );
    let mut staged_finalizer_summary = None;
    assert!(
        !replace_delivery_with_direct_structured_observed_answer(
            &state,
            &task,
            &mut staged_loop_state,
            Some(&agent_run_context),
            &mut staged_finalizer_summary,
        ),
        "direct raw projection should preserve richer publishable summary"
    );
    assert_eq!(
        final_answer_text_from_delivery(&staged_loop_state.delivery_messages),
        summary
    );

    let reply = finalize_loop_reply(
        &state,
        &task,
        "execute and summarize the observed working directory plus local process evidence",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should preserve publishable evidence summary");

    assert_eq!(reply.text, summary);
    assert_eq!(reply.messages, vec![summary.to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn scalar_contract_restores_complete_terminal_multi_machine_record() {
    let state = test_state();
    let task = claimed_task("task-multi-machine-record");
    let mut route = scalar_route_result();
    route.locator_kind = OutputLocatorKind::None;
    route.locator_hint.clear();
    route.requires_content_evidence = false;
    route.response_shape = crate::OutputResponseShape::Scalar;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route),
        ..Default::default()
    };
    let complete = "field_a=0\nfield_b=0\nfield_c=0\nfield_d=0";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "task_control",
        r#"{"action":"list","status":"empty","count":0}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", complete));
    loop_state.last_user_visible_respond = Some("field_d=0".to_string());
    loop_state.delivery_messages = vec!["field_d=0".to_string()];

    enforce_delivery_output_contract(
        &state,
        &task,
        "return the requested machine fields",
        &mut loop_state,
        Some(&agent_run_context),
    )
    .await;

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(complete)
    );
    assert_eq!(loop_state.delivery_messages, vec![complete.to_string()]);
}

#[tokio::test]
async fn scalar_contract_keeps_single_line_publishable_multi_machine_record() {
    let state = test_state();
    let task = claimed_task("task-single-line-machine-record");
    let mut route = scalar_route_result();
    route.locator_kind = OutputLocatorKind::None;
    route.locator_hint.clear();
    route.requires_content_evidence = false;
    route.response_shape = crate::OutputResponseShape::Scalar;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route),
        ..Default::default()
    };
    let complete = "field_a=0, field_b=0, field_c=0, field_d=0";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "task_control",
        r#"{"action":"list","status":"empty","count":0}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", complete));
    loop_state.last_publishable_synthesis_output = Some(complete.to_string());
    loop_state.last_user_visible_respond = Some("field_d".to_string());
    loop_state.delivery_messages = vec!["field_d".to_string()];

    enforce_delivery_output_contract(
        &state,
        &task,
        "return the requested machine fields",
        &mut loop_state,
        Some(&agent_run_context),
    )
    .await;

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(complete)
    );
    assert_eq!(loop_state.delivery_messages, vec![complete.to_string()]);
}

#[tokio::test]
async fn finalize_loop_reply_replaces_wrapped_runtime_status_scalar_delivery() {
    let state = test_state();
    let task = claimed_task("task-wrapped-runtime-status-scalar");
    let mut route = scalar_route_result();
    route.configure_exact_command_output();
    route.locator_kind = OutputLocatorKind::None;
    route.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let wrapped = r#"{"extra":{"action":"runtime_status","command_output":"guagua","field_value":"guagua","kind":"current_user","value":"guagua"},"text":"{\"action\":\"runtime_status\",\"command_output\":\"guagua\",\"field_value\":\"guagua\",\"kind\":\"current_user\",\"value\":\"guagua\"}"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "system_basic", wrapped));
    loop_state
        .capability_results
        .push(crate::capability_result::successful_execution_envelope(
            "system_basic",
            "step_1",
            &serde_json::json!({"action": "runtime_status"}),
            wrapped,
            Some(&serde_json::json!({
                "action": "runtime_status",
                "command_output": "guagua",
                "field_value": "guagua",
                "kind": "current_user",
                "value": "guagua"
            })),
        ));
    loop_state.delivery_messages.push(wrapped.to_string());
    loop_state.last_user_visible_respond = Some(wrapped.to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "只输出当前用户名，不要解释",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should unwrap runtime_status scalar");

    assert_eq!(reply.text, "guagua");
    assert_eq!(reply.messages, vec!["guagua".to_string()]);
    assert!(!reply.text.contains(r#""action":"#));
}

#[tokio::test]
async fn finalize_loop_reply_replaces_wrapped_scalar_path_delivery() {
    let state = test_state();
    let task = claimed_task("task-wrapped-scalar-path");
    let mut route = scalar_route_result();
    route.selection.structured_field_selector = Some("resolved_path".to_string());
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let wrapped = r#"{"extra":{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"dir","path":"","resolved_path":"/home/guagua/rustclaw","size_bytes":4096},"path":"/home/guagua/rustclaw"}],"include_missing":true},"text":"{\"action\":\"path_batch_facts\",\"count\":1,\"facts\":[{\"exists\":true,\"fact\":{\"kind\":\"dir\",\"path\":\"\",\"resolved_path\":\"/home/guagua/rustclaw\",\"size_bytes\":4096},\"path\":\"/home/guagua/rustclaw\"}],\"include_missing\":true}"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "system_basic", wrapped));
    loop_state.delivery_messages.push(wrapped.to_string());
    loop_state.last_user_visible_respond = Some(wrapped.to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "只输出当前工作目录的绝对路径，不要解释",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should unwrap scalar path");

    assert_eq!(reply.text, "/home/guagua/rustclaw");
    assert_eq!(reply.messages, vec!["/home/guagua/rustclaw".to_string()]);
    assert!(!reply.text.contains(r#""action":"#));
}

#[tokio::test]
async fn finalize_loop_reply_replaces_recoverable_scalar_path_candidate_with_observed_path() {
    let state = test_state();
    let task = claimed_task("task-recoverable-scalar-path-dot");
    let mut route = scalar_route_result();
    route.selection.structured_field_selector = Some("resolved_path".to_string());
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let observed = r#"{"extra":{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"dir","path":"","resolved_path":"/home/guagua/rustclaw/.","size_bytes":4096},"path":"."}],"include_missing":true},"text":"{\"action\":\"path_batch_facts\",\"count\":1,\"facts\":[{\"exists\":true,\"fact\":{\"kind\":\"dir\",\"path\":\"\",\"resolved_path\":\"/home/guagua/rustclaw/.\",\"size_bytes\":4096},\"path\":\".\"}],\"include_missing\":true}"}"#;
    let failure_candidate =
        "observed candidate path: /home/guagua/rustclaw; checkpoint_state=waiting";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", observed));
    loop_state
        .delivery_messages
        .push(failure_candidate.to_string());
    loop_state.last_user_visible_respond = Some(failure_candidate.to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "输出当前工作目录的绝对路径，只输出路径或结构化 field_value，不要解释。",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should prefer normalized observed scalar path");

    assert_eq!(reply.text, "/home/guagua/rustclaw");
    assert_eq!(reply.messages, vec!["/home/guagua/rustclaw".to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_preserves_richer_generic_path_facts_delivery() {
    let state = test_state();
    let task = claimed_task("task-existence-summary-same-path-richer");
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.delivery_required = false;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let richer_answer =
        "same_path=false\nservice_notes.md exists=true\nrelease_checklist.md exists=true";
    let mut loop_state = crate::agent_engine::LoopState::new(5);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"path_batch_facts","facts":[{"path":"service_notes.md","exists":true},{"path":"release_checklist.md","exists":true}]}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        richer_answer,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "respond", richer_answer));
    loop_state.delivery_messages.push(richer_answer.to_string());
    loop_state.last_publishable_synthesis_output = Some(richer_answer.to_string());
    loop_state.last_user_visible_respond = Some(richer_answer.to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "return same_path and both exist fields",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should preserve required evidence fields");

    assert_eq!(reply.text, richer_answer);
    assert_eq!(reply.messages, vec![richer_answer.to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_replaces_scalar_field_placeholder_with_observed_path() {
    let state = test_state();
    let task = claimed_task("task-scalar-path-field-placeholder");
    let mut route = scalar_route_result();
    route.selection.structured_field_selector = Some("resolved_path".to_string());
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let observed = r#"{"extra":{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"dir","path":"","resolved_path":"/home/guagua/rustclaw/.","size_bytes":4096},"path":"."}],"include_missing":true},"text":"{\"action\":\"path_batch_facts\",\"count\":1,\"facts\":[{\"exists\":true,\"fact\":{\"kind\":\"dir\",\"path\":\"\",\"resolved_path\":\"/home/guagua/rustclaw/.\",\"size_bytes\":4096},\"path\":\".\"}],\"include_missing\":true}"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", observed));
    loop_state.delivery_messages.push("field_value".to_string());
    loop_state.last_user_visible_respond = Some("field_value".to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "输出当前工作目录的绝对路径，只输出路径。",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should replace scalar field placeholder");

    assert_eq!(reply.text, "/home/guagua/rustclaw");
    assert_eq!(reply.messages, vec!["/home/guagua/rustclaw".to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_replaces_scalar_field_placeholder_with_terminal_path_respond() {
    let state = test_state();
    let task = claimed_task("task-scalar-path-terminal-respond");
    let mut route = scalar_route_result();
    route.selection.structured_field_selector = Some("path".to_string());
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"find_name","count":0,"exact":false,"patterns":["field_value"],"results":[],"root":""},"text":"{\"action\":\"find_name\",\"count\":0,\"exact\":false,\"patterns\":[\"field_value\"],\"results\":[],\"root\":\"\"}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "respond",
        "/home/guagua/rustclaw",
    ));
    loop_state.delivery_messages.push("field_value".to_string());
    loop_state.last_user_visible_respond = Some("field_value".to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "输出当前工作目录的绝对路径，只输出路径或结构化 field_value，不要解释。",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should prefer terminal scalar path respond over field placeholder");

    assert_eq!(reply.text, "/home/guagua/rustclaw");
    assert_eq!(reply.messages, vec!["/home/guagua/rustclaw".to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_projects_file_basename_from_capability_result() {
    let state = test_state();
    let task = claimed_task("task-file-basename-path-facts");
    let mut route = scalar_route_result();
    route.selection.structured_field_selector = Some("basename".to_string());
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
            .to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let extra = serde_json::json!({
        "action": "path_batch_facts",
        "basename": "release_checklist.md",
        "count": 1,
        "facts": [{
            "exists": true,
            "fact": {
                "kind": "file",
                "path": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
                "resolved_path": "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
                "size_bytes": 120
            },
            "path": "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
        }],
        "include_missing": true
    });
    let wrapped = serde_json::json!({
        "extra": extra.clone(),
        "text": "untrusted fallback"
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "system_basic", &wrapped));
    loop_state
        .capability_results
        .push(crate::capability_result::successful_execution_envelope(
            "system_basic",
            "step_1",
            &serde_json::json!({"action": "path_batch_facts"}),
            "untrusted fallback",
            Some(&extra),
        ));
    loop_state.delivery_messages.push(wrapped.clone());
    loop_state.last_user_visible_respond = Some(wrapped);

    let reply = finalize_loop_reply(
        &state,
        &task,
        "return only the selected file basename",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should unwrap file basename");

    assert_eq!(reply.text, "release_checklist.md");
    assert_eq!(reply.messages, vec!["release_checklist.md".to_string()]);
    assert!(!reply.text.contains(r#""action":"#));
}

#[tokio::test]
async fn finalize_loop_reply_projects_selected_scalar_from_wrapped_capability_output() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "crypto"
        enabled = true
        kind = "runner"
        semantic_tags = []
        "#,
        &["crypto"],
    );
    let task = claimed_task("task-wrapped-market-quote-scalar");
    let mut route = scalar_route_result();
    route.selection.structured_field_selector = Some("quote.price_usd".to_string());
    route.locator_kind = OutputLocatorKind::None;
    route.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let wrapped = r#"{"extra":{"action":"quote","content_excerpt":"BTCUSDT | Price sources:\n- BINANCE $67216.010000","quote":{"exchange":"binance","price_usd":67216.01,"source":"binance_api","symbol":"BTCUSDT"}},"text":"BTCUSDT | Price sources:\n- BINANCE $67216.010000"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "crypto", wrapped));
    loop_state.delivery_messages.push(wrapped.to_string());
    loop_state.last_user_visible_respond = Some(wrapped.to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "What is the current BTCUSDT price? Give me the key result only.",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should unwrap market quote scalar");

    assert_eq!(reply.text, "67216.01");
    assert_eq!(reply.messages, vec!["67216.01".to_string()]);
    assert!(!reply.text.contains(r#""quote":"#));
}

#[test]
fn direct_scalar_finalize_uses_structured_extract_field_missing_message() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(scalar_route_result()),
        ..Default::default()
    };
    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("scalar fallback should succeed");
    assert!(answer.contains("message_key=clawd.msg.extract_field_missing"));
    assert!(answer.contains("reason_code=extract_field_missing"));
    assert!(answer.contains("field_path=name"));
    assert!(answer.contains("exists=false"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_scalar_finalize_uses_structured_read_field_missing_message() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"read_field","exists":false,"field_path":"package.name","value_text":"","value":null,"value_type":"null"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(scalar_route_result()),
        ..Default::default()
    };
    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("scalar fallback should succeed");
    assert!(answer.contains("message_key=clawd.msg.extract_field_missing"));
    assert!(answer.contains("reason_code=extract_field_missing"));
    assert!(answer.contains("field_path=package.name"));
    assert!(answer.contains("exists=false"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_structured_observed_answer_skips_multi_evidence_content_routes() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"react-example","value":"react-example","value_type":"string"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd","value":"clawd","value_type":"string"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_3".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("package.name: clawd".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    assert!(
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_structured_observed_answer_preserves_publishable_respond_for_content_routes() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"inventory_dir","names_only":true,"names":["clawd.run.log"],"names_by_kind":{"files":["clawd.run.log"],"dirs":[],"other":[]}}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "respond".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("更像正常启动，没有遇到报错。".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none(),
        "content-evidence routes should preserve the publishable respond instead of projecting a names_only inventory item"
    );
}

#[test]
fn direct_structured_observed_answer_skips_observed_passthrough_for_strict_exact_sentence() {
    let raw_snapshot = "exit=0\nState  Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\nLISTEN 0      4096         0.0.0.0:8787      0.0.0.0:*    users:((\"clawd\",pid=117002,fd=31))";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "process_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(raw_snapshot.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.exact_sentence_count = Some(1);
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_non_builtin_raw_answer_skips_synthesized_delivery_contract() {
    let raw_snapshot = "exit=0\nState  Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\nLISTEN 0      4096         0.0.0.0:8787      0.0.0.0:*    users:((\"clawd\",pid=117002,fd=31))";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "process_basic".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "process_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(raw_snapshot.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.exact_sentence_count = Some(1);
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(direct_non_builtin_skill_raw_answer(
        &test_state(),
        &loop_state,
        Some(&agent_run_context),
    )
    .is_none());
}

#[test]
fn direct_structured_observed_answer_skips_ambiguous_multi_structured_scalars() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"react-example","value":"react-example","value_type":"string"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd","value":"clawd","value_type":"string"}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = false;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    assert!(
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_scalar_finalize_defers_health_check_summary_to_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "health_check".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let route = scalar_route_result();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    assert!(
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none(),
        "health_check scalar summary should be synthesized from observed evidence"
    );
}

#[test]
fn direct_scalar_finalize_reports_missing_path_before_extracting_path_field() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"configs/config_copy"}],"include_missing":true}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = scalar_route_result();
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "configs/config_copy".to_string();
    route.selection.structured_field_selector = Some("count".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_scalar_observed_answer(Some(&state), &loop_state, Some(&agent_run_context))
            .expect("missing path should produce a scalar-compatible failure explanation");

    assert!(answer.contains("configs/config_copy"));
    assert!(answer.contains("exists=false"));
    assert!(answer.contains("final_answer_shape=scalar"));
    assert!(answer.contains("count_available=false"));
    assert_ne!(answer.trim(), "configs/config_copy");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_scalar_finalize_does_not_repair_limited_listing_from_drifted_scalar_count() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"inventory_dir","path":"logs","resolved_path":"/tmp/logs","names_only":true,"sort_by":"mtime_desc","names":["clawd.run.log","model_io.log","act_plan.log"],"counts":{"total":3}}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = scalar_route_result();
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs".to_string();
    route.selection.structured_field_selector = Some("count".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("scalar count fallback should follow the structured contract");
    assert_eq!(answer, "3");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_scalar_finalize_preserves_planned_count_inventory_breakdown() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "count files and directories".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "count_inventory",
                    "path": ".",
                    "count_files": true,
                    "count_dirs": true
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"count_inventory","counts":{"total":66,"files":40,"dirs":26}}"#,
    ));
    let mut route = scalar_route_result();
    route.selection.structured_field_selector = Some("count".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        original_user_request: Some("帮我检查一下当前目录底下有多少个文件和文件夹。".to_string()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("planned component counts should be preserved");

    assert!(answer.contains("40"));
    assert!(answer.contains("26"));
    assert_ne!(answer.trim(), "66");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_scalar_finalize_uses_total_count_without_component_plan() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"count_inventory","counts":{"total":66,"files":40,"dirs":26}}"#,
    ));
    let mut route = scalar_route_result();
    route.selection.structured_field_selector = Some("count".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        original_user_request: Some("当前目录有多少个项目？只回复数字。".to_string()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("total count should be usable directly");

    assert_eq!(answer.trim(), "66");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_scalar_finalize_uses_wrapped_count_inventory_total() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"count_inventory","counts":{"total":66,"files":40,"dirs":26},"path":"logs"},"text":"{\"action\":\"count_inventory\",\"counts\":{\"total\":66,\"files\":40,\"dirs\":26},\"path\":\"logs\"}"}"#,
    ));
    let mut route = scalar_route_result();
    route.selection.structured_field_selector = Some("count".to_string());
    route.locator_hint = "logs".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        original_user_request: Some("count direct entries".to_string()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("wrapped count inventory total should be usable directly");

    assert_eq!(answer.trim(), "66");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn scalar_count_contract_projects_find_ext_count_from_machine_field() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"find_ext","count":1,"ext":"zip","exts":["zip"],"patterns":[],"results":["scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"],"root":"scripts/nl_tests/fixtures/device_local"},"text":"{}"}"#,
    ));
    let mut route = free_route_result();
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Scalar;
    route.selection.structured_field_selector = Some("count".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        original_user_request: Some(
            "count zip files under scripts/nl_tests/fixtures/device_local; output only the number"
                .to_string(),
        ),
        ..Default::default()
    };

    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("scalar count contract should project observed count");

    assert_eq!(answer.trim(), "1");
    assert!(!answer.contains("test_bundle.zip"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn one_sentence_count_waits_for_model_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"count_inventory","counts":{"total":34,"files":32,"dirs":2},"path":"document","recursive":false}"#,
    ));
    let mut route = scalar_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.selection.structured_field_selector = Some("count".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        original_user_request: Some("再数一下 document 目录直接有多少个子项".to_string()),
        ..Default::default()
    };

    assert!(direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none());
}

#[test]
fn direct_structured_finalize_defers_path_inspection_to_model_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/tmp/rustclaw-workspace/rustclaw.service","size_bytes":1190},"path":"/tmp/rustclaw-workspace/rustclaw.service"}],"include_missing":true}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = scalar_route_result();
    route.response_shape = OutputResponseShape::Free;
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint = "rustclaw.service".to_string();
    route.selection.structured_field_selector = Some("exists,path".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    assert!(
        super::direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context))
            .is_none()
    );
}

#[test]
fn direct_non_builtin_finalize_preserves_raw_skill_text() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "crypto".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "crypto".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            "trade_submit order_id=123 status=FILLED binance BTCUSDT buy qty_filled=0.001 avg_price=100000 quote_spent=100 USDT"
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
            .expect("non-builtin fallback should preserve raw text");
    assert_eq!(
        answer,
        "trade_submit order_id=123 status=FILLED binance BTCUSDT buy qty_filled=0.001 avg_price=100000 quote_spent=100 USDT"
    );
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_non_builtin_finalize_skips_structured_machine_output() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "stock".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "stock".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(r#"{"symbol":"AAPL","price":201.32}"#.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };

    assert!(
        direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
            .is_none()
    );
}

#[tokio::test]
async fn direct_publishable_observed_answer_accepts_plain_observation_without_exact_contract() {
    let state = test_state();
    let task = claimed_task("task-no-raw-run-cmd-passthrough");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("/home/guagua/rustclaw\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::OneSentence;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_publishable_observed_answer(&state, &task, &loop_state, Some(&agent_run_context))
            .await
            .expect("publishable plain observation");
    assert_eq!(answer, "/home/guagua/rustclaw");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[tokio::test]
async fn direct_publishable_observed_answer_accepts_exact_plain_observation() {
    let state = test_state();
    let task = claimed_task("task-strict-run-cmd-format");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("/home/guagua/rustclaw\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.configure_exact_command_output();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_publishable_observed_answer(&state, &task, &loop_state, Some(&agent_run_context))
            .await
            .expect("publishable exact observation");
    assert_eq!(answer, "/home/guagua/rustclaw");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn unclassified_output_allows_model_language_fallback() {
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Free;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(agent_context_allows_observed_output_language_fallback(
        Some(&agent_run_context)
    ));
    assert!(agent_context_allows_observed_output_language_fallback(None));
}

#[tokio::test]
async fn unclassified_output_can_publish_grounded_observation() {
    let state = test_state();
    let task = claimed_task("task-matrix-strict-no-raw-publishable");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("README.md\nCargo.toml\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Free;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(direct_publishable_observed_answer(
        &state,
        &task,
        &loop_state,
        Some(&agent_run_context)
    )
    .await
    .is_some());
}

#[test]
fn direct_scalar_finalize_accepts_strict_single_line_observation() {
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("ThinkPad-X1\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.exact_sentence_count = Some(1);
    route.requires_content_evidence = true;
    route.delivery_required = false;
    route.locator_kind = crate::OutputLocatorKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("direct scalar answer");
    assert_eq!(answer, "ThinkPad-X1");
    assert!(summary.contract_ok);
}

#[test]
fn direct_scalar_finalize_skips_strict_exact_observation_output_contract() {
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("ThinkPad-X1\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.exact_sentence_count = Some(1);
    route.requires_content_evidence = true;
    route.configure_exact_command_output();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none());
}

#[test]
fn raw_structured_passthrough_is_dropped_for_scalar_contract() {
    let raw = r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(raw.to_string());
    loop_state.delivery_messages.push(raw.to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(raw.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(scalar_route_result()),
        ..Default::default()
    };
    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            raw
        ),
        Some(true)
    );
}

#[test]
fn structured_user_input_delivery_is_not_dropped_as_observed_passthrough() {
    let message = "Please provide the source directory.";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.pending_user_input_required = true;
    loop_state.last_user_visible_respond = Some(message.to_string());
    loop_state.delivery_messages.push(message.to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "photo_organize".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(message.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(scalar_route_result()),
        ..Default::default()
    };
    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            message
        ),
        None
    );
}

#[test]
fn qualified_scalar_passthrough_is_not_dropped() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some("rustclaw".to_string());
    loop_state.delivery_messages.push("rustclaw".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("rustclaw\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(scalar_route_result()),
        ..Default::default()
    };
    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            "rustclaw"
        ),
        Some(false)
    );
}

#[test]
fn scalar_path_from_write_file_is_not_dropped_as_meta_placeholder() {
    let path = "/home/guagua/rustclaw/document/pwd_line.txt";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(path.to_string());
    loop_state.delivery_messages.push(path.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "write_file",
        "written 48 bytes to /home/guagua/rustclaw/document/pwd_line.txt",
    ));
    loop_state
        .output_vars
        .insert("last_file_path".to_string(), path.to_string());
    loop_state
        .written_file_aliases
        .insert("pwd_line.txt".to_string(), path.to_string());
    let mut route = scalar_route_result();
    route.selection.structured_field_selector = Some("path".to_string());
    route.locator_hint = "pwd_line.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            path
        ),
        Some(false)
    );
}

#[test]
fn direct_scalar_finalize_prefers_presence_plus_path_for_fs_search_presence_queries() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_search".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"find_name","count":1,"results":["rustclaw.service"],"root":""}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = scalar_route_result();
    route.requires_content_evidence = false;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let (answer, summary) =
        direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
            .expect("presence+path fallback should succeed");
    assert!(answer.contains("message_key=clawd.msg.path_fact.observed"));
    assert!(answer.contains("reason_code=path_fact_observed"));
    assert!(answer.contains("exists=true"));
    assert!(answer.contains("path=rustclaw.service"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn archive_exit_zero_passthrough_is_dropped_when_structured_answer_exists() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some("exit=0".to_string());
    loop_state.delivery_messages.push("exit=0".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "archive_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("exit=0\nupdating: tmp/rustclaw-workspace/scripts/skill_calls/\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        locator_hint: "scripts/skill_calls -> tmp/nl_archive_case.zip".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    discard_observed_passthrough_delivery_when_structured_answer_available(
        &claimed_task("task-archive"),
        &mut loop_state,
        Some(&agent_run_context),
    );

    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.last_user_visible_respond.is_none());
}

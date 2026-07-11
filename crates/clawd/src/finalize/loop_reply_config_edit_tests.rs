use super::*;

#[test]
fn direct_config_edit_observed_answer_summarizes_apply_validate_readback() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_edit",
        r#"{"action":"plan_config_change","path":"run/nl_eval_tmp/config_edit_smoke/config.toml","field_path":"skills.skill_switches.config_edit_nl_smoke","new_value":true,"would_change":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_edit",
        r#"{"action":"apply_config_change","applied":true,"path":"run/nl_eval_tmp/config_edit_smoke/config.toml","field_path":"skills.skill_switches.config_edit_nl_smoke","new_value":true,"validated":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "config_edit",
        r#"{"action":"validate_config","path":"run/nl_eval_tmp/config_edit_smoke/config.toml","valid":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_4",
        "config_edit",
        r#"{"action":"read_back","path":"run/nl_eval_tmp/config_edit_smoke/config.toml","field_path":"skills.skill_switches.config_edit_nl_smoke","exists":true,"value":true,"value_text":"true"}"#,
    ));

    let (answer, summary) = direct_config_edit_observed_answer(
        &state,
        "把 config_edit_nl_smoke 开关打开，然后验证并读回",
        &loop_state,
    )
    .expect("config_edit structured answer");

    let payload: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(
        payload
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.config_edit.applied")
    );
    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("config_edit_applied")
    );
    assert_eq!(
        payload
            .pointer("/field_path")
            .and_then(serde_json::Value::as_str),
        Some("skills.skill_switches.config_edit_nl_smoke")
    );
    assert_eq!(
        payload
            .pointer("/validation_passed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        payload
            .pointer("/value")
            .and_then(serde_json::Value::as_str),
        Some("true")
    );
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_config_edit_observed_answer_summarizes_guard_config() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_edit",
        r#"{"action":"guard_config","path":"configs/config.toml","risk_count":2,"risks":["llm.minimax.api_key looks like a real secret","tools.allow_sudo=true"]}"#,
    ));

    let (answer, summary) = direct_config_edit_observed_answer(
        &state,
        "检查 RustClaw 主配置有没有明显风险，不能泄露任何密钥值",
        &loop_state,
    )
    .expect("config_edit guard answer");

    let payload: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(
        payload
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.config_edit.guard")
    );
    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("config_edit_guard_risk_found")
    );
    assert_eq!(
        payload.pointer("/path").and_then(serde_json::Value::as_str),
        Some("configs/config.toml")
    );
    assert_eq!(
        payload
            .pointer("/risk_count")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
    assert_eq!(
        payload
            .pointer("/risks/0")
            .and_then(serde_json::Value::as_str),
        Some("llm.minimax.api_key looks like a real secret")
    );
    assert_eq!(
        payload
            .pointer("/risks/1")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow_sudo=true")
    );
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_config_edit_plan_answer_includes_following_guard_observation() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_edit",
        r#"{"action":"plan_config_change","path":"configs/config.toml","field_path":"llm.selected_vendor","new_value":"minimax","would_change":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_basic",
        r#"{"action":"guard_config","path":"configs/config.toml","risk_count":1,"risks":["tools.allow_sudo=true"]}"#,
    ));

    let (answer, summary) = direct_config_edit_observed_answer(
        &state,
        "preview config change and guard it",
        &loop_state,
    )
    .expect("config_edit plan and guard answer");

    let payload: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(
        payload
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.config_edit.planned")
    );
    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("config_edit_planned")
    );
    assert_eq!(
        payload
            .pointer("/field_path")
            .and_then(serde_json::Value::as_str),
        Some("llm.selected_vendor")
    );
    assert_eq!(
        payload
            .pointer("/value")
            .and_then(serde_json::Value::as_str),
        Some("minimax")
    );
    assert_eq!(
        payload
            .pointer("/risk_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        payload
            .pointer("/risks/0")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow_sudo=true")
    );
    assert_eq!(
        payload
            .pointer("/candidates/0")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow_sudo=true")
    );
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_config_edit_observed_answer_projects_agent_hook_policy_surface() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_fields","count":4,"format":"toml","path":"configs/agent_guard.toml","resolved_path":"/workspace/configs/agent_guard.toml","results":[{"field_path":"agent.hooks.blocked_action_refs","resolved_field_path":"agent.hooks.blocked_action_refs","exists":true,"value":[],"value_text":"[]","value_type":"array"},{"field_path":"agent.hooks.blocked_tools","resolved_field_path":"agent.hooks.blocked_tools","exists":true,"value":["run_cmd"],"value_text":"[\"run_cmd\"]","value_type":"array"},{"field_path":"agent.hooks.require_confirmation_action_refs","resolved_field_path":"agent.hooks.require_confirmation_action_refs","exists":true,"value":[],"value_text":"[]","value_type":"array"},{"field_path":"agent.hooks.background_wait_action_refs","resolved_field_path":"agent.hooks.background_wait_action_refs","exists":true,"value":[],"value_text":"[]","value_type":"array"}]}"#,
    ));

    let (answer, summary) = direct_config_edit_observed_answer(
        &state,
        "只检查当前 hook/permission 配置能否表达 PreToolUse 的机器决策",
        &loop_state,
    )
    .expect("agent hook surface answer");
    let payload: serde_json::Value = serde_json::from_str(&answer).unwrap();

    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("agent_hooks_pre_tool_use_policy_surface")
    );
    assert_eq!(
        payload
            .pointer("/stage")
            .and_then(serde_json::Value::as_str),
        Some("pre_tool_use")
    );
    assert_eq!(
        payload
            .pointer("/decisions/allow/supported")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        payload
            .pointer("/decisions/deny/configured_ref_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        payload
            .pointer("/decisions/require_confirmation/supported")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        payload
            .pointer("/decisions/background_wait/supported")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_config_edit_observed_answer_ignores_visible_text_json_payload() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_edit",
        r#"{"status":"ok","text":"{\"action\":\"guard_config\",\"path\":\"configs/config.toml\",\"risk_count\":2}"}"#,
    ));

    assert!(direct_config_edit_observed_answer(&state, "检查配置风险", &loop_state).is_none());
}

#[test]
fn direct_config_edit_observed_answer_accepts_config_basic_guard_config() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"guard_config","path":"configs/config.toml","risk_count":2,"risks":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"]}"#,
    ));

    let (answer, summary) = direct_config_edit_observed_answer(
        &state,
        "检查 RustClaw 主配置有没有明显风险，不能泄露任何密钥值",
        &loop_state,
    )
    .expect("config_basic guard answer");

    let payload: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("config_edit_guard_risk_found")
    );
    assert_eq!(
        payload.pointer("/path").and_then(serde_json::Value::as_str),
        Some("configs/config.toml")
    );
    assert_eq!(
        payload
            .pointer("/risk_count")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
    assert_eq!(
        payload
            .pointer("/risks/0")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow_sudo=true")
    );
    assert_eq!(
        payload
            .pointer("/risks/1")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow_path_outside_workspace=true")
    );
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_rustclaw_config_risk_answer_uses_structured_field_values() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_fields","path":"/home/guagua/rustclaw/configs/config.toml","resolved_path":"/home/guagua/rustclaw/configs/config.toml","count":6,"results":[{"field_path":"tools.allow","resolved_field_path":"tools.allow","exists":true,"value":["*"],"value_text":"[\"*\"]"},{"field_path":"tools.allow_sudo","resolved_field_path":"tools.allow_sudo","exists":true,"value":true,"value_text":"true"},{"field_path":"tools.allow_path_outside_workspace","resolved_field_path":"tools.allow_path_outside_workspace","exists":true,"value":true,"value_text":"true"},{"field_path":"self_extension.enabled","resolved_field_path":"self_extension.enabled","exists":true,"value":false,"value_text":"false"},{"field_path":"worker.task_timeout_seconds","resolved_field_path":"worker.task_timeout_seconds","exists":true,"value":3600,"value_text":"3600"},{"field_path":"server.listen","resolved_field_path":"server.listen","exists":true,"value":"0.0.0.0:8787","value_text":"0.0.0.0:8787"}]}"#,
    ));

    let (answer, summary) = direct_rustclaw_config_risk_answer(
        &state,
        "configs/config.toml の RustClaw 設定リスクを確認し、重要な点だけ答えて。",
        &loop_state,
    )
    .expect("structured config risk answer");

    let payload: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(
        payload
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.config_risk.summary")
    );
    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("config_risk_found")
    );
    assert_eq!(
        payload
            .pointer("/risk_count")
            .and_then(serde_json::Value::as_u64),
        Some(4)
    );
    assert_eq!(
        payload
            .pointer("/risks/0")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow=[\"*\"]")
    );
    assert_eq!(
        payload
            .pointer("/risks/1")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow_sudo=true")
    );
    assert_eq!(
        payload
            .pointer("/risks/2")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow_path_outside_workspace=true")
    );
    assert_eq!(
        payload
            .pointer("/risks/3")
            .and_then(serde_json::Value::as_str),
        Some("server.listen=\"0.0.0.0:8787\"")
    );
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn rustclaw_config_risk_replacement_drops_ungrounded_synthesis() {
    let state = test_state();
    let task = claimed_task("task-config-risk-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("self_extension.enabled=true and worker.task_timeout_seconds=86400".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_fields","path":"configs/config.toml","resolved_path":"/home/guagua/rustclaw/configs/config.toml","results":[{"field_path":"tools.allow_sudo","resolved_field_path":"tools.allow_sudo","exists":true,"value":true},{"field_path":"tools.allow_path_outside_workspace","resolved_field_path":"tools.allow_path_outside_workspace","exists":true,"value":true},{"field_path":"self_extension.enabled","resolved_field_path":"self_extension.enabled","exists":true,"value":false},{"field_path":"worker.task_timeout_seconds","resolved_field_path":"worker.task_timeout_seconds","exists":true,"value":3600}]}"#,
    ));
    let mut summary = None;

    assert!(replace_delivery_with_deterministic_rustclaw_config_risk_answer(
        &state,
        &task,
        "Check configs/config.toml for RustClaw configuration risks and list only important findings.",
        &mut loop_state,
        &mut summary,
    ));

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("tools.allow_sudo=true"));
    assert!(answer.contains("tools.allow_path_outside_workspace=true"));
    assert!(!answer.contains("self_extension.enabled=true"));
    assert!(!answer.contains("86400"));
    assert!(summary.is_some());
}

#[tokio::test]
async fn finalize_loop_reply_uses_config_edit_observed_answer_after_synthesis_failure() {
    let state = test_state();
    let task = claimed_task("task-config-edit-fallback");
    let mut loop_state = crate::agent_engine::LoopState::new(5);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_edit",
        r#"{"action":"apply_config_change","applied":true,"path":"run/nl_eval_tmp/config_edit_smoke/config.toml","field_path":"skills.skill_switches.config_edit_nl_smoke","new_value":true,"validated":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_edit",
        r#"{"action":"validate_config","path":"run/nl_eval_tmp/config_edit_smoke/config.toml","valid":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "config_edit",
        r#"{"action":"read_back","path":"run/nl_eval_tmp/config_edit_smoke/config.toml","field_path":"skills.skill_switches.config_edit_nl_smoke","exists":true,"value":true,"value_text":"true"}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_4",
        "synthesize_answer",
        "synthesis failed",
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "把 config_edit_nl_smoke 开关打开，然后验证并读回",
        loop_state,
        None,
    )
    .await
    .expect("finalize should succeed");

    assert!(!reply.should_fail_task);
    let payload: serde_json::Value = serde_json::from_str(&reply.text).unwrap();
    assert_eq!(
        payload
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.config_edit.applied")
    );
    assert_eq!(
        payload
            .pointer("/validation_passed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert!(!reply.text.contains("没能整理成可靠结论"));
}

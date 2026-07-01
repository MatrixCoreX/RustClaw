use super::*;

#[test]
fn missing_file_search_evidence_detects_zero_match_fs_search() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_search".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "find_name",
                "count": 0,
                "results": [],
                "root": ""
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    assert!(has_missing_file_search_evidence(&loop_state));
}

#[test]
fn missing_file_search_evidence_detects_missing_path_facts() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "path_batch_facts",
                "count": 1,
                "facts": [{
                    "exists": false,
                    "path": "/tmp/definitely-missing.txt",
                    "error": "not found"
                }],
                "include_missing": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    assert!(has_missing_file_search_evidence(&loop_state));
}

#[test]
fn missing_file_search_evidence_detects_missing_path_facts_from_machine_extra() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "status": "ok",
                "text": "path facts",
                "extra": {
                    "action": "path_batch_facts",
                    "facts": [{
                        "exists": false,
                        "path": "/tmp/definitely-missing.txt"
                    }],
                    "include_missing": true
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    assert!(has_missing_file_search_evidence(&loop_state));
}

#[test]
fn missing_file_search_evidence_ignores_json_hidden_in_visible_text() {
    let hidden_payload = serde_json::json!({
        "action": "path_batch_facts",
        "facts": [{
            "exists": false,
            "path": "/tmp/definitely-missing.txt"
        }],
        "include_missing": true
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "status": "ok",
                "text": hidden_payload
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    assert!(!has_missing_file_search_evidence(&loop_state));
}

#[test]
fn latest_file_delivery_observation_treats_missing_path_facts_as_terminal_missing() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "path_batch_facts",
                "count": 1,
                "facts": [{
                    "exists": false,
                    "path": "/tmp/definitely-missing.txt",
                    "error": "not found"
                }],
                "include_missing": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_publishable_synthesis_output =
        Some("文件 /tmp/definitely-missing.txt 不存在，无法发送。".to_string());
    loop_state.last_user_visible_respond = loop_state.last_publishable_synthesis_output.clone();
    loop_state.delivery_messages = vec![loop_state
        .last_publishable_synthesis_output
        .clone()
        .unwrap()];

    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(latest_file_delivery_observation_is_missing(&loop_state));
    assert!(should_return_missing_file_delivery_reply(
        &loop_state,
        Some(&agent_run_context)
    ));
}

#[test]
fn missing_file_search_evidence_detects_not_found_probe_output() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("NOT_FOUND\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    assert!(has_missing_file_search_evidence(&loop_state));
}

#[test]
fn missing_file_search_evidence_detects_system_basic_find_path_zero_matches() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "find_path",
                "count": 0,
                "matches": [],
                "query": "missing.md",
                "target_kind": "file"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    assert!(has_missing_file_search_evidence(&loop_state));
}

#[test]
fn missing_file_search_evidence_detects_wrapped_fs_basic_find_name_zero_matches() {
    let output = serde_json::json!({
        "extra": {
            "action": "find_name",
            "count": 0,
            "exact": false,
            "patterns": ["definitely_missing_text_match_case_001.txt"],
            "results": [],
            "root": "document"
        },
        "text": "{\"action\":\"find_name\",\"count\":0,\"exact\":false,\"patterns\":[\"definitely_missing_text_match_case_001.txt\"],\"results\":[],\"root\":\"document\"}"
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(output.clone()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    assert!(has_missing_file_search_evidence(&loop_state));
    assert!(latest_file_delivery_observation_is_missing(&loop_state));
    assert_eq!(
        missing_file_path_from_output(&output).as_deref(),
        Some("document/definitely_missing_text_match_case_001.txt")
    );
}

#[test]
fn missing_file_path_from_output_ignores_json_hidden_in_visible_text() {
    let hidden_payload = serde_json::json!({
        "action": "find_name",
        "count": 0,
        "exact": false,
        "patterns": ["definitely_missing_text_match_case_001.txt"],
        "results": [],
        "root": "document"
    })
    .to_string();
    let output = serde_json::json!({
        "status": "ok",
        "text": hidden_payload
    })
    .to_string();

    assert_eq!(missing_file_path_from_output(&output), None);
}

#[tokio::test]
async fn finalize_loop_reply_returns_not_found_for_missing_file_delivery() {
    let state = test_state();
    let task = claimed_task("task-missing-file-delivery");
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "definitely_missing_named_file.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_search".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "find_name",
                "count": 0,
                "results": [],
                "root": ""
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "把 definitely_missing_named_file.txt 发给我",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-file answer");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert!(reply
        .messages
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
    assert_missing_file_delivery_text(&reply.text);
    assert!(reply.text.contains("definitely_missing_named_file.txt"));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn finalize_loop_reply_inherits_language_for_missing_file_delivery_path_reply() {
    let state = test_state();
    let task = claimed_task("task-missing-file-delivery-path-reply-language");
    let mut route = scalar_route_result();
    route.resolved_intent =
        "继续上一轮请求：把缺失路径对应的文件发给用户，不要贴内容。".to_string();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint =
        "/home/guagua/rustclaw/definitely_missing_named_file.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some("把那个文件发给我，不要贴内容".to_string()),
        user_request: Some("/home/guagua/rustclaw/definitely_missing_named_file.txt".to_string()),
        ..Default::default()
    };

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_search".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "find_name",
                "count": 0,
                "results": [],
                "root": "/home/guagua/rustclaw"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "/home/guagua/rustclaw/definitely_missing_named_file.txt",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-file answer");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert!(reply.text.contains("definitely_missing_named_file.txt"));
    assert_missing_file_delivery_text(&reply.text);
}

#[tokio::test]
async fn finalize_loop_reply_returns_not_found_for_wrapped_fs_basic_missing_file_delivery() {
    let state = test_state();
    let task = claimed_task("task-wrapped-missing-file-delivery");
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint =
        "document/definitely_missing_text_match_case_001.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(
            "__RC_SKILL_ERROR__:{\"error_kind\":\"contract_action_rejected\",\"error_text\":\"action `system_basic.path_batch_facts` is rejected by contract `file_paths`\",\"skill\":\"system_basic\"}"
                .to_string(),
        ),
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "extra": {
                    "action": "find_name",
                    "count": 0,
                    "exact": false,
                    "patterns": ["definitely_missing_text_match_case_001.txt"],
                    "results": [],
                    "root": "document"
                },
                "text": "{\"action\":\"find_name\",\"count\":0,\"exact\":false,\"patterns\":[\"definitely_missing_text_match_case_001.txt\"],\"results\":[],\"root\":\"document\"}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "把 document/definitely_missing_text_match_case_001.txt 发给我",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-file answer");

    assert!(!reply.should_fail_task);
    assert!(reply
        .text
        .contains("document/definitely_missing_text_match_case_001.txt"));
    assert_missing_file_delivery_text(&reply.text);
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn finalize_loop_reply_returns_not_found_for_run_cmd_not_found_delivery() {
    let state = test_state();
    let task = claimed_task("task-missing-file-delivery-run-cmd");
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/tmp/definitely-missing.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("NOT_FOUND\n".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "把 /tmp/definitely-missing.txt 发给我",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-file answer");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert_missing_file_delivery_text(&reply.text);
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn finalize_loop_reply_returns_not_found_for_missing_path_facts_delivery() {
    let state = test_state();
    let task = claimed_task("task-missing-file-delivery-path-facts");
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/tmp/definitely-missing.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "path_batch_facts",
                "count": 1,
                "facts": [{
                    "exists": false,
                    "path": "/tmp/definitely-missing.txt",
                    "error": "not found"
                }],
                "include_missing": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_user_visible_respond = Some("FILE:/tmp/definitely-missing.txt".to_string());
    loop_state.delivery_messages = vec!["FILE:/tmp/definitely-missing.txt".to_string()];

    let reply = finalize_loop_reply(
        &state,
        &task,
        "把 /tmp/definitely-missing.txt 发给我",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-file answer");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert!(reply
        .messages
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
    assert_missing_file_delivery_text(&reply.text);
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn finalize_loop_reply_keeps_missing_file_delivery_when_synthesis_is_non_token() {
    let state = test_state();
    let task = claimed_task("task-missing-file-delivery-synthesis");
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/tmp/definitely-missing.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "path_batch_facts",
                "count": 1,
                "facts": [{
                    "exists": false,
                    "path": "/tmp/definitely-missing.txt",
                    "error": "not found"
                }],
                "include_missing": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_publishable_synthesis_output =
        Some("文件 /tmp/definitely-missing.txt 不存在，无法发送。".to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "把 /tmp/definitely-missing.txt 发给我，不要猜内容",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-file answer");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert!(reply.text.contains("/tmp/definitely-missing.txt"));
    assert_missing_file_delivery_text(&reply.text);
    assert!(reply
        .messages
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
}

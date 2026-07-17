use super::*;
use crate::finalize::loop_reply::replace_delivery_with_direct_structured_observed_answer;
use crate::finalize::loop_reply::shell_stdout_redirect_target_path;
use crate::finalize::raw_command_machine_field_projection_from_journal;

#[test]
fn raw_command_chatact_prefers_exact_observed_output_over_planned_extra_content() {
    let state = test_state();
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/workspace/project\n",
    ));
    let planned = "/workspace/project\n\nworkspace ready".to_string();
    loop_state.last_user_visible_respond = Some(planned.clone());
    let mut delivery_messages = vec![planned.clone()];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-raw-command-chatact-planned",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec!["/workspace/project".to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("/workspace/project")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn raw_command_multiline_output_replaces_reordered_synthesis() {
    let state = test_state();
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "version_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh\n",
    ));
    let planned = "verify_task_termination.sh\nversion_info.sh\ntest_qwen_api.sh".to_string();
    loop_state.last_user_visible_respond = Some(planned.clone());
    let mut delivery_messages = vec![planned];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-raw-command-reordered",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec!["version_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh".to_string()]
    );
    assert!(finalizer_summary.is_some());
}

#[tokio::test]
async fn finalize_loop_reply_replaces_drifted_raw_command_short_list_synthesis() {
    let state = test_state();
    let task = claimed_task("task-raw-command-short-list-drift");
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let observed =
        "version_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh\ntest_qwen_5_channels.py\ntest_minimax_curl.sh";
    let drifted =
        "version_info.sh\nversion_info.sh\nverify_task_termination.sh\nverify_task_termination.sh\ntest_qwen_api.sh";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", observed));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", drifted));
    loop_state.last_publishable_synthesis_output = Some(drifted.to_string());
    loop_state.last_user_visible_respond = Some(drifted.to_string());
    loop_state.delivery_messages.push(drifted.to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "执行 ls scripts，把结果按字母倒序排，只输出前 5 个",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return observed raw command output");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert_eq!(reply.text.trim(), observed);
}

#[test]
fn raw_command_projection_plan_replaces_drifted_projected_answer() {
    let state = test_state();
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Untitled\n__pycache__\nauth-key.sh\ncheck-secrets.sh\nversion_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh\n",
    ));
    push_raw_plan_text(
        &mut loop_state,
        r#"{"steps":[{"type":"call_tool","tool":"fs_basic","args":{"action":"list_dir","path":"/home/guagua/rustclaw/scripts","names_only":true,"sort_by":"name_desc","max_entries":5}}]}"#,
    );
    let projected =
        "version_info.sh\nverify_task_termination.sh\nUntitled\ntest_qwen_api.sh\ntest_qwen_5_channels.py"
            .to_string();
    loop_state.last_user_visible_respond = Some(projected.clone());
    let mut delivery_messages = vec![projected.clone()];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-raw-command-structural-projection",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec![
            "version_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh\ncheck-secrets.sh\nauth-key.sh"
                .to_string()
        ]
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn raw_command_projection_plan_replaces_unprojected_listing_output() {
    let state = test_state();
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = OutputResponseShape::Free;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let raw_output = "Untitled\n__pycache__\nauth-key.sh\ncheck-secrets.sh\nversion_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh\ntest_qwen_5_channels.py\ntest_minimax_curl.sh\n".to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", &raw_output));
    push_raw_plan_text(
        &mut loop_state,
        r#"{"steps":[{"type":"call_tool","tool":"fs_basic","args":{"action":"list_dir","path":"/home/guagua/rustclaw/scripts","names_only":true,"sort_by":"name_desc","max_entries":5}}]}"#,
    );
    let mut delivery_messages = vec![raw_output];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-raw-command-structural-projection-free",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec![
            "version_info.sh\nverify_task_termination.sh\ntest_qwen_api.sh\ntest_qwen_5_channels.py\ntest_minimax_curl.sh"
                .to_string()
        ]
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn shell_stdout_redirect_target_path_parses_quoted_and_unquoted_targets() {
    assert_eq!(
        shell_stdout_redirect_target_path("printf '%s' 'note' > /tmp/rustclaw-workspace-note.txt",),
        Some(PathBuf::from("/tmp/rustclaw-workspace-note.txt"))
    );
    assert_eq!(
        shell_stdout_redirect_target_path(r#"printf note 1> "/tmp/rustclaw note.txt""#),
        Some(PathBuf::from("/tmp/rustclaw note.txt"))
    );
    assert_eq!(
        shell_stdout_redirect_target_path("printf note 2> /tmp/stderr.txt"),
        None
    );
}

#[test]
fn raw_command_redirect_projection_returns_existing_workspace_file_path() {
    let mut state = test_state();
    let tmp = TempDirGuard::new("raw_command_redirect_projection");
    state.skill_rt.workspace_root = tmp.path().to_path_buf();
    let output_path = tmp.path().join("workspace_note.txt");
    fs::write(&output_path, "workspace note").expect("write output file");
    let mut route = scalar_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.locator_kind = crate::OutputLocatorKind::None;
    route.locator_hint.clear();
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let raw_snapshot = format!(
        "exit=0 command=printf '%s' 'RustClaw workspace note' > {}",
        output_path.display()
    );
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "run_cmd", &raw_snapshot));
    let mut delivery_messages = vec![format!("/home/guagua/rustclaw\n{raw_snapshot}")];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-raw-command-redirect-path",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec![output_path.canonicalize().unwrap().display().to_string()]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(
            output_path
                .canonicalize()
                .unwrap()
                .display()
                .to_string()
                .as_str()
        )
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn raw_command_requested_stdout_path_uses_observed_stdout_path_value() {
    let state = test_state();
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("exit_code,stdout_path".to_string());
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "respond",
        r#"{"capability":"run_cmd","reason_code":"verify_capability_unavailable"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "respond",
        r#"{"capability":"run_cmd","reason_code":"verify_capability_unavailable"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_4",
        "run_cmd",
        "exit=0 command=pwd > /tmp/pwd_stdout.txt",
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_5", "run_cmd", "EXIT=0\n"));

    let (answer, summary) = direct_raw_command_output_projection(&state, &route, &loop_state)
        .expect("raw command projection");

    assert_eq!(answer, "exit_code=0\nstdout_path=/home/guagua/rustclaw");
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn raw_command_requested_stdout_uses_observed_stdout_value() {
    let state = test_state();
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("exit_code,stdout".to_string());
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "RUSTCLAW_NL_100"));

    let (answer, summary) = direct_raw_command_output_projection(&state, &route, &loop_state)
        .expect("raw command projection");

    assert_eq!(answer, "exit_code=0\nstdout=RUSTCLAW_NL_100");
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn raw_command_requested_stdout_path_ignores_temp_stdout_file_wrapper() {
    let state = test_state();
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("exit_code,stdout_path".to_string());
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "run_cmd",
        "exit_code=0\nstdout_path=/tmp/pwd_stdout.txt\n---\n/home/guagua/rustclaw\n",
    ));

    let (answer, summary) = direct_raw_command_output_projection(&state, &route, &loop_state)
        .expect("raw command projection");

    assert_eq!(answer, "exit_code=0\nstdout_path=/home/guagua/rustclaw");
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn raw_command_journal_projection_prefers_real_stdout_over_wrapper_metadata() {
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("exit_code,stdout_path".to_string());
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-raw-command-journal", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "run_cmd",
            "/home/guagua/rustclaw\n",
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "run_cmd",
            "exit_code=0\nstdout_path=/tmp/pwd_stdout.txt\n",
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "respond",
            "exit_code=0\nstdout_path=/tmp/pwd_stdout.txt stdout_path",
        ));

    let answer = raw_command_machine_field_projection_from_journal(&route, &journal)
        .expect("raw command journal projection");

    assert_eq!(answer, "exit_code=0\nstdout_path=/home/guagua/rustclaw");
}

#[tokio::test]
async fn finalize_loop_reply_keeps_requested_raw_command_machine_fields_exact() {
    let state = test_state();
    let task = claimed_task("task-raw-command-machine-fields-exact");
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("exit_code,stdout_path".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "run_cmd",
        "exit_code=0\nstdout_path=/tmp/pwd_stdout.txt\n---\n/home/guagua/rustclaw\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "respond",
        "exit_code=0\nstdout_path=/home/guagua/rustclaw stdout_path",
    ));
    loop_state
        .delivery_messages
        .push("exit_code=0\nstdout_path=/home/guagua/rustclaw stdout_path".to_string());
    loop_state.last_user_visible_respond =
        Some("exit_code=0\nstdout_path=/home/guagua/rustclaw stdout_path".to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "Run pwd and return only exit_code and stdout_path.",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should keep exact machine fields");

    assert_eq!(
        reply.text.trim(),
        "exit_code=0\nstdout_path=/home/guagua/rustclaw"
    );
    assert!(!reply.text.contains(" stdout_path"));
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_returns_redirected_file_path_for_scalar_raw_command_contract() {
    let mut state = test_state();
    let tmp = TempDirGuard::new("finalize_raw_command_redirect_path");
    state.skill_rt.workspace_root = tmp.path().to_path_buf();
    let output_path = tmp.path().join("workspace_note.txt");
    fs::write(&output_path, "RustClaw workspace note").expect("write output file");
    let raw_snapshot = format!(
        "exit=0 command=printf '%s' 'RustClaw workspace note' > {}",
        output_path.display()
    );
    let task = claimed_task("task-finalize-raw-command-redirect-path");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "run_cmd", &raw_snapshot));
    loop_state.last_user_visible_respond = Some(format!("/home/guagua/rustclaw\n{raw_snapshot}"));
    loop_state
        .delivery_messages
        .push(format!("/home/guagua/rustclaw\n{raw_snapshot}"));
    let mut route = scalar_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.locator_kind = crate::OutputLocatorKind::None;
    route.locator_hint.clear();
    route.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "write workspace note and return only its absolute path",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should succeed");

    let expected = output_path.canonicalize().unwrap().display().to_string();
    assert_eq!(reply.text, expected);
    assert!(!reply.should_fail_task);
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some(expected.as_str())
    );
    assert!(!reply.text.contains("exit=0"));
}

#[test]
fn backfill_suppresses_raw_run_cmd_when_plan_declares_projection() {
    let task = claimed_task("task-backfill-raw-projection");
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Untitled\n__pycache__\nauth-key.sh\ncheck-secrets.sh\n",
    ));
    loop_state.last_user_visible_respond =
        Some("Untitled\n__pycache__\nauth-key.sh\ncheck-secrets.sh\n".to_string());
    push_raw_plan_text(
        &mut loop_state,
        r#"{"steps":[{"type":"call_tool","tool":"fs_basic","args":{"action":"list_dir","path":"/home/guagua/rustclaw/scripts","sort_by":"name_desc","max_entries":5}}]}"#,
    );

    backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&agent_run_context));

    assert!(loop_state.delivery_messages.is_empty());
}

#[test]
fn raw_command_free_contract_keeps_publishable_synthesis_over_raw_projection() {
    let state = test_state();
    let task = claimed_task("task-raw-command-synthesis-over-raw");
    let raw_df = "Filesystem      Size  Used Avail Use% Mounted on\n/dev/nvme0n1p6  146G  125G   15G  90% /\n";
    let synthesis = "最需要关注的是 `/` 这一项：根分区已经使用 90%，只剩约 15G 可用。";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", raw_df));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", synthesis));
    loop_state.delivery_messages.push(raw_df.trim().to_string());
    loop_state.last_user_visible_respond = Some(raw_df.trim().to_string());
    loop_state.last_publishable_synthesis_output = Some(synthesis.to_string());
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Free;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(replace_raw_observation_delivery_with_synthesis(
        &task,
        &mut loop_state,
        Some(&agent_run_context),
    ));
    assert_eq!(loop_state.delivery_messages, vec![synthesis]);

    let mut delivery_messages = loop_state.delivery_messages.clone();
    let mut finalizer_summary = None;
    prefer_observed_answer_for_exact_contract(
        &state,
        "task-raw-command-synthesis-over-raw",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![synthesis]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(synthesis)
    );
}

#[test]
fn raw_command_projection_collapses_identical_repeated_outputs() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "ThinkPad-X1\n"));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "run_cmd", "ThinkPad-X1\n"));
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;

    let (answer, summary) = direct_raw_command_output_projection(&state, &route, &loop_state)
        .expect("raw command projection");

    assert_eq!(answer, "ThinkPad-X1");
    assert_eq!(summary.used_evidence_ids_count, 1);
}

#[test]
fn raw_command_projection_uses_latest_contiguous_run_cmd_group() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(5);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "run_cmd", "guagua\n"));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "run_cmd", "ThinkPad-X1\n"));
    loop_state.executed_step_results.push(ok_step_result(
        "step_4",
        "synthesize_answer",
        "old synthesis",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_5",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_6", "run_cmd", "guagua\n"));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_7", "run_cmd", "ThinkPad-X1\n"));
    loop_state.executed_step_results.push(ok_step_result(
        "step_8",
        "respond",
        "/home/guagua/rustclaw\nguagua\nThinkPad-X1",
    ));
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;

    let (answer, summary) = direct_raw_command_output_projection(&state, &route, &loop_state)
        .expect("raw command projection");

    assert_eq!(answer, "/home/guagua/rustclaw\nguagua\nThinkPad-X1");
    assert_eq!(summary.used_evidence_ids_count, 3);
}

#[test]
fn raw_command_direct_structured_replacement_keeps_multi_step_projection() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(5);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "run_cmd", "guagua\n"));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "run_cmd", "ThinkPad-X1\n"));
    loop_state.delivery_messages.push("ThinkPad-X1".to_string());
    loop_state.last_user_visible_respond = Some("ThinkPad-X1".to_string());
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_direct_structured_observed_answer(
        &state,
        &claimed_task("task-raw-command-multi-step-replace"),
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.delivery_messages,
        vec!["/home/guagua/rustclaw\nguagua\nThinkPad-X1".to_string()]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("/home/guagua/rustclaw\nguagua\nThinkPad-X1")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn raw_command_projection_accepts_wrapped_read_range_output() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let read_range_output = serde_json::json!({
        "extra": {
            "action": "read_range",
            "mode": "tail",
            "requested_n": 2,
            "path": "/tmp/clawd-dev.log",
            "resolved_path": "/tmp/clawd-dev.log",
            "excerpt": "98|first observed line\n99|second observed line"
        },
        "text": "{\"action\":\"read_range\",\"excerpt\":\"98|first observed line\\n99|second observed line\"}"
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", &read_range_output));
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;

    let (answer, summary) = direct_raw_command_output_projection(&state, &route, &loop_state)
        .expect("read range raw command projection");

    assert_eq!(answer, "first observed line\nsecond observed line");
    assert_eq!(summary.used_evidence_ids_count, 1);
}

#[tokio::test]
async fn finalize_loop_reply_uses_raw_read_projection_when_delivery_empty() {
    let state = test_state();
    let task = claimed_task("task-raw-read-projection-empty-delivery");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let read_range_output = serde_json::json!({
        "extra": {
            "action": "read_range",
            "mode": "tail",
            "requested_n": 2,
            "path": "/tmp/clawd-dev.log",
            "resolved_path": "/tmp/clawd-dev.log",
            "excerpt": "98|first observed line\n99|second observed line"
        },
        "text": "{\"action\":\"read_range\",\"excerpt\":\"98|first observed line\\n99|second observed line\"}"
    })
    .to_string();
    let synthesis = "planned fallback text";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", &read_range_output));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", synthesis));
    loop_state.last_publishable_synthesis_output = Some(synthesis.to_string());
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "show the last two lines exactly",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should project read_range output directly");

    assert_eq!(reply.text, "first observed line\nsecond observed line");
    assert_eq!(
        reply.messages,
        vec!["first observed line\nsecond observed line".to_string()]
    );
    assert!(!reply.should_fail_task);
}

#[test]
fn direct_structured_observed_answer_accepts_wrapped_raw_read_range() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let read_range_output = serde_json::json!({
        "extra": {
            "action": "read_range",
            "mode": "tail",
            "requested_n": 2,
            "path": "/tmp/clawd-dev.log",
            "resolved_path": "/tmp/clawd-dev.log",
            "excerpt": "98|first observed line\n99|second observed line"
        },
        "text": "{\"action\":\"read_range\",\"excerpt\":\"98|first observed line\\n99|second observed line\"}"
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", &read_range_output));
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) =
        direct_structured_observed_answer(Some(&state), &loop_state, Some(&agent_run_context))
            .expect("direct structured raw read range answer");

    assert_eq!(answer, "first observed line\nsecond observed line");
    assert_eq!(summary.used_evidence_ids_count, 1);
}

#[test]
fn raw_read_range_observed_answer_replaces_planned_error_interpretation() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let read_range_output = serde_json::json!({
        "extra": {
            "action": "read_range",
            "mode": "tail",
            "requested_n": 2,
            "path": "/tmp/clawd-dev.log",
            "resolved_path": "/tmp/clawd-dev.log",
            "excerpt": "98|first observed line\n99|second observed line"
        },
        "text": "{\"action\":\"read_range\",\"excerpt\":\"98|first observed line\\n99|second observed line\"}"
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", &read_range_output));
    loop_state
        .delivery_messages
        .push("planned interpretation of old log error".to_string());
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_direct_structured_observed_answer(
        &state,
        &claimed_task("task-raw-read-range-replace"),
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.delivery_messages,
        vec!["first observed line\nsecond observed line".to_string()]
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn raw_publishable_guard_rejects_structured_json_payloads() {
    assert!(looks_like_structured_machine_output(
        r#"{"hostname":"rustclaw-test-host.local","cwd":"/tmp/rustclaw-workspace"}"#
    ));
    assert!(looks_like_structured_machine_output(
        r#"[{"name":"README.md"},{"name":"Cargo.toml"}]"#
    ));
    assert!(!looks_like_structured_machine_output(
        "rustclaw-test-host.local"
    ));
    assert!(!looks_like_structured_machine_output(
        "package_manager=brew"
    ));
    assert!(looks_like_structured_machine_output(
        "count[0].target=docs\ncount[0].total=3\ncount[1].target=logs\ncount[1].total=2"
    ));
    assert!(looks_like_structured_machine_output(
        "git.branch=main\ngit.clean=true"
    ));
    assert!(looks_like_structured_machine_output(concat!(
        "dry_run=true\n",
        "provider=minimax\n",
        "model=image-01\n",
        "model_kind=dry_run\n",
        "output_path=/home/guagua/rustclaw/document/media_dry_run/image_status_card.png\n",
        "planned_outputs=[{\"path\":\"/home/guagua/rustclaw/document/media_dry_run/image_status_card.png\",\"type\":\"image_file\"}]"
    )));
}

#[test]
fn raw_publishable_guard_rejects_multi_line_command_snapshots() {
    assert!(looks_like_raw_command_snapshot(
        "exit=0\nCOMMAND PID USER\nclawd 4498 testuser TCP *:8787 (LISTEN)\n"
    ));
    assert!(!looks_like_raw_command_snapshot("testuser"));
}

use super::*;
use crate::finalize::loop_reply::enforce_delivery_output_contract;
use crate::finalize::loop_reply::file_delivery::async_poll_result_report_from_value;

#[test]
fn file_delivery_fallback_uses_ranked_inventory_after_placeholder_plan() {
    let dir = TempDirGuard::new("ranked_inventory_file_delivery");
    let newest = dir.path().join("newest.txt");
    let older = dir.path().join("older.txt");
    fs::write(&newest, "new").expect("write newest");
    fs::write(&older, "old").expect("write older");

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "deliver selected file from directory".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "list_dir",
                        "path": dir.path().display().to_string(),
                        "names_only": true,
                        "sort_by": "mtime_desc"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: serde_json::json!({
                        "content": format!("FILE:{}/{{{{last_output}}}}", dir.path().display())
                    }),
                    depends_on: vec!["step_1".to_string()],
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "inventory_dir",
            "resolved_path": dir.path().display().to_string(),
            "names_only": true,
            "sort_by": "mtime_desc",
            "names": ["newest.txt", "older.txt"],
            "counts": {"files": 2, "dirs": 0, "total": 2}
        })
        .to_string(),
    ));
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = dir.path().display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (token, summary) = direct_file_token_from_observed_inventory(&loop_state, Some(&ctx))
        .expect("ranked inventory should recover file token");

    assert_eq!(token, format!("FILE:{}", newest.display()));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn file_delivery_fallback_uses_single_find_entries_result() {
    let dir = TempDirGuard::new("find_entries_file_delivery");
    let file = dir.path().join("patches/open-lark/CHANGELOG.md");
    fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
    fs::write(&file, "release notes").expect("write changelog");
    let mut state = test_state();
    state.skill_rt.workspace_root = dir.path().to_path_buf();

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "find_name",
            "count": 1,
            "exact": false,
            "patterns": ["changelog"],
            "results": ["patches/open-lark/CHANGELOG.md"],
            "root": ""
        })
        .to_string(),
    ));
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (token, summary) =
        direct_file_token_from_observed_find_entries(&state, &loop_state, Some(&ctx))
            .expect("single find_entries result should recover file token");

    assert_eq!(
        token,
        format!("FILE:{}", file.canonicalize().unwrap_or(file).display())
    );
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn compound_content_file_delivery_appends_token_after_summary() {
    let dir = TempDirGuard::new("compound_content_file_delivery");
    let file = dir.path().join("config.toml");
    fs::write(&file, "answer = true\n").expect("write config");
    let mut state = test_state();
    state.skill_rt.workspace_root = dir.path().to_path_buf();

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .delivery_messages
        .push("observed summary".to_string());
    loop_state.last_user_visible_respond = Some("observed summary".to_string());
    let mut route = scalar_route_result();
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = file.display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(append_compound_file_delivery_token_from_route(
        &state,
        &claimed_task("compound-content-delivery"),
        &mut loop_state,
        Some(&ctx),
    ));

    assert_eq!(loop_state.delivery_messages.len(), 2);
    assert_eq!(loop_state.delivery_messages[0], "observed summary");
    assert_eq!(
        loop_state.delivery_messages[1],
        format!("FILE:{}", file.canonicalize().unwrap_or(file).display())
    );
}

#[tokio::test]
async fn compound_content_file_delivery_enforce_preserves_synthesis_before_token_append() {
    let dir = TempDirGuard::new("compound_content_delivery_enforce");
    let file = dir.path().join("config.toml");
    fs::write(&file, "answer = true\n").expect("write config");
    let mut state = test_state();
    state.skill_rt.workspace_root = dir.path().to_path_buf();
    let task = claimed_task("compound-content-enforce");
    let synthesis = "observed summary";

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_publishable_synthesis_output = Some(synthesis.to_string());
    loop_state.last_user_visible_respond = Some(synthesis.to_string());
    loop_state.delivery_messages.push(synthesis.to_string());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "synthesize_answer", synthesis));
    let mut route = scalar_route_result();
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = file.display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    enforce_delivery_output_contract(
        &state,
        &task,
        "deliver content summary and file",
        &mut loop_state,
        Some(&ctx),
    )
    .await;

    assert_eq!(loop_state.delivery_messages, vec![synthesis.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(synthesis)
    );
}

#[tokio::test]
async fn generated_delivery_existing_file_content_synthesis_enforce_preserves_summary_and_token() {
    let dir = TempDirGuard::new("generated_delivery_existing_file_content_synthesis");
    let file = dir.path().join("config.toml");
    fs::write(&file, "answer = true\n").expect("write config");
    let canonical = file.canonicalize().unwrap_or_else(|_| file.clone());
    let canonical_text = canonical.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = dir.path().to_path_buf();
    let task = claimed_task("generated-delivery-content-synthesis");
    let summary = "observed summary";

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_publishable_synthesis_output = Some(summary.to_string());
    loop_state.last_user_visible_respond = Some(summary.to_string());
    loop_state.delivery_messages.push(summary.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "read_range",
            "resolved_path": &canonical_text,
            "path": &canonical_text,
            "excerpt": "1|answer = true"
        })
        .to_string(),
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", summary));

    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = canonical_text.clone();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    enforce_delivery_output_contract(
        &state,
        &task,
        "deliver existing config with synthesized content summary",
        &mut loop_state,
        Some(&ctx),
    )
    .await;

    assert_eq!(
        loop_state.delivery_messages,
        vec![summary.to_string(), format!("FILE:{}", canonical_text)]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(summary)
    );
}

#[test]
fn generated_delivery_existing_file_content_synthesis_ignores_write_plans() {
    let dir = TempDirGuard::new("generated_delivery_with_write");
    let file = dir.path().join("config.toml");
    fs::write(&file, "answer = true\n").expect("write config");
    let canonical = file.canonicalize().unwrap_or_else(|_| file.clone());
    let canonical_text = canonical.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = dir.path().to_path_buf();
    let summary = "observed summary";

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_publishable_synthesis_output = Some(summary.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "read_range",
            "resolved_path": &canonical_text,
            "path": &canonical_text,
            "excerpt": "1|answer = true"
        })
        .to_string(),
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        &serde_json::json!({
            "action": "write_text",
            "resolved_path": &canonical_text,
            "path": &canonical_text
        })
        .to_string(),
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", summary));

    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = canonical_text;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(generated_delivery_existing_file_content_synthesis_token(
        &state,
        &loop_state,
        Some(&ctx),
    )
    .is_none());
}

#[test]
fn file_delivery_fallback_uses_last_inventory_selection_from_placeholder_plan() {
    let dir = TempDirGuard::new("last_inventory_file_delivery");
    let first = dir.path().join("alpha.txt");
    let last = dir.path().join("zeta.txt");
    fs::write(&first, "first").expect("write first");
    fs::write(&last, "last").expect("write last");

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "deliver selected file from directory".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "list_dir",
                        "path": dir.path().display().to_string(),
                        "names_only": true,
                        "sort_by": "name"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: serde_json::json!({
                        "content": format!(
                            "FILE:{}/{{{{last_output.lines().last().unwrap()}}}}",
                            dir.path().display()
                        )
                    }),
                    depends_on: vec!["step_1".to_string()],
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "inventory_dir",
            "resolved_path": dir.path().display().to_string(),
            "names_only": true,
            "sort_by": "name",
            "names": ["alpha.txt", "zeta.txt"],
            "counts": {"files": 2, "dirs": 0, "total": 2}
        })
        .to_string(),
    ));
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = dir.path().display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (token, summary) = direct_file_token_from_observed_inventory(&loop_state, Some(&ctx))
        .expect("explicit last selection over deterministic inventory should recover token");

    assert_eq!(token, format!("FILE:{}", last.display()));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn file_delivery_fallback_defers_ambiguous_unranked_inventory() {
    let dir = TempDirGuard::new("ambiguous_inventory_file_delivery");
    fs::write(dir.path().join("a.txt"), "a").expect("write a");
    fs::write(dir.path().join("b.txt"), "b").expect("write b");

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "inventory_dir",
            "resolved_path": dir.path().display().to_string(),
            "names_only": true,
            "sort_by": "name",
            "names": ["a.txt", "b.txt"],
            "counts": {"files": 2, "dirs": 0, "total": 2}
        })
        .to_string(),
    ));
    let mut route = scalar_route_result();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_file_token_from_observed_inventory(&loop_state, Some(&ctx)).is_none());
}

#[test]
fn file_token_auto_locator_wraps_bare_filename_under_directory() {
    let temp = TempDirGuard::new("file_token_dir");
    let file_path = temp.path().join("report.txt");
    fs::write(&file_path, "hello").expect("write");
    let expected = format!(
        "FILE:{}",
        file_path
            .canonicalize()
            .unwrap_or(file_path.clone())
            .display()
    );
    assert_eq!(
        resolve_file_token_from_auto_locator_answer(
            "report.txt",
            Some(temp.path().to_string_lossy().as_ref())
        )
        .as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn file_token_auto_locator_normalizes_delivery_messages() {
    let temp = TempDirGuard::new("file_token_messages");
    let file_path = temp.path().join("report.txt");
    fs::write(&file_path, "hello").expect("write");
    let expected = format!(
        "FILE:{}",
        file_path
            .canonicalize()
            .unwrap_or(file_path.clone())
            .display()
    );
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.last_user_visible_respond = Some("report.txt".to_string());
    loop_state.delivery_messages.push("report.txt".to_string());

    let mut route = scalar_route_result();
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        auto_locator_path: Some(temp.path().to_string_lossy().to_string()),
        ..Default::default()
    };

    normalize_file_token_delivery_from_auto_locator(&mut loop_state, Some(&agent_run_context));

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(loop_state.delivery_messages, vec![expected]);
}

#[test]
fn file_token_auto_locator_recovers_from_observed_bare_filename() {
    let temp = TempDirGuard::new("file_token_observed_bare_filename");
    let file_path = temp.path().join("report.txt");
    fs::write(&file_path, "hello").expect("write");
    let expected = format!(
        "FILE:{}",
        file_path
            .canonicalize()
            .unwrap_or(file_path.clone())
            .display()
    );
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "report.txt\n"));

    let mut route = scalar_route_result();
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        auto_locator_path: Some(temp.path().to_string_lossy().to_string()),
        ..Default::default()
    };

    let (token, summary) = direct_file_token_from_observed_auto_locator_filename(
        &loop_state,
        Some(&agent_run_context),
    )
    .expect("bare filename observation under auto locator should recover file token");

    assert_eq!(token, expected);
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn file_token_observed_path_normalizes_bare_filename_delivery() {
    let temp = TempDirGuard::new("file_token_observed_path");
    let file_path = temp.path().join("document/report.txt");
    fs::create_dir_all(file_path.parent().expect("parent")).expect("mkdir");
    fs::write(&file_path, "hello").expect("write");
    let expected = format!(
        "FILE:{}",
        file_path
            .canonicalize()
            .unwrap_or(file_path.clone())
            .display()
    );
    let mut state = test_state();
    state.skill_rt.workspace_root = temp.path().to_path_buf();
    state.skill_rt.default_locator_search_dir = temp.path().to_path_buf();

    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.last_user_visible_respond = Some("FILE:report.txt".to_string());
    loop_state
        .delivery_messages
        .push("FILE:report.txt".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "entries": [
                    {"name": "report.txt", "path": "document/report.txt"}
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let mut route = scalar_route_result();
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    normalize_file_token_delivery_from_observed_paths(
        &state,
        &mut loop_state,
        Some(&agent_run_context),
    );

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(loop_state.delivery_messages, vec![expected]);
}

#[tokio::test]
async fn finalize_loop_reply_returns_file_token_from_path_batch_after_read_rejections() {
    let state = test_state();
    let task = claimed_task("task-file-delivery-after-read-rejections");
    let tmp = TempDirGuard::new("file_delivery_path_batch_after_reject");
    let file = tmp.path().join("release_checklist.md");
    std::fs::write(&file, "release checklist").expect("write temp file");

    let mut route = scalar_route_result();
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.wants_file_delivery = false;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = file.display().to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let contract_error = "__RC_SKILL_ERROR__:{\"error_kind\":\"contract_action_rejected\",\"error_text\":\"action `system_basic.read_range` is rejected by contract `generic_delivery` (rejected_not_allowed)\",\"extra\":{\"action\":\"system_basic.read_range\",\"contract_match\":\"generic_delivery\",\"decision\":\"rejected_not_allowed\"},\"skill\":\"system_basic\"}";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(contract_error.to_string()),
        started_at: 1,
        finished_at: 2,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(contract_error.to_string()),
        started_at: 3,
        finished_at: 4,
    });
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "fs_basic",
        &serde_json::json!({
            "action": "path_batch_facts",
            "count": 1,
            "facts": [{
                "exists": true,
                "fact": {
                    "kind": "file",
                    "path": file.display().to_string(),
                    "resolved_path": file.display().to_string(),
                    "size_bytes": 17
                },
                "path": file.display().to_string()
            }],
            "include_missing": true
        })
        .to_string(),
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return file token");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, format!("FILE:{}", file.display()));
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[test]
fn async_poll_result_report_projects_requested_machine_fields() {
    let value = serde_json::json!({
        "extra": {
            "task_id": "image-task-001",
            "job_id": "image-job-001",
            "status": "succeeded",
            "async_poll_adapter_result": {
                "schema_version": 1,
                "adapter_kind": "media_job_poll",
                "job_id": "image-job-001",
                "status": "succeeded",
                "final_result_json": {
                    "task_id": "image-task-001",
                    "dry_run": true
                }
            }
        },
        "text": "IMAGE_TASK:image-task-001"
    });

    let rendered = async_poll_result_report_from_value(&value).expect("async_poll_result_render");

    assert!(rendered.contains("task_id=image-task-001"));
    assert!(rendered.contains("job_id=image-job-001"));
    assert!(rendered.contains("status=succeeded"));
    assert!(rendered.contains("async_poll_adapter_result={"));
}

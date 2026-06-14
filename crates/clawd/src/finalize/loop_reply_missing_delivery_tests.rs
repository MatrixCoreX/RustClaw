use super::*;

#[tokio::test]
async fn observed_execution_without_delivery_reply_attaches_raw_summary() {
    let state = test_state();
    let task = claimed_task("task-missing-delivery-observed");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "list recent logs".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command": "ls -t logs | head -2"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "model_io.log\nact_plan.log\n",
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "列出 logs 最近两个文件，再判断类型",
        &loop_state,
        Some(&ctx),
        None,
        "no publishable final answer was produced",
    )
    .await
    .expect("observed execution reply");

    assert!(reply.should_fail_task);
    assert_eq!(reply.messages.len(), 2);
    assert!(reply.messages[0].contains("**执行过程**"));
    assert!(reply.messages[0].contains("命令 `ls -t logs | head -2`"));
    assert!(reply.messages[0].contains("model_io.log"));
    assert!(reply.messages[0].contains("act_plan.log"));
    assert!(!reply.text.contains("你最想看的是哪一项"));
}

#[tokio::test]
async fn observed_execution_without_delivery_prefers_finalizer_clarify_question() {
    let state = test_state();
    let task = claimed_task("task-missing-delivery-clarify");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","path":"/tmp","entries":[]}"#,
    ));
    let mut route = free_route_result();
    route.needs_clarify = true;
    route.clarify_question = "请提供压缩包路径。".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let summary = crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
        needs_clarify: Some(true),
        ..Default::default()
    };

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "把那个压缩包解压到 /tmp/unpack_case 然后告诉我结果",
        &loop_state,
        Some(&ctx),
        Some(summary),
        "stage=observed_generic, needs_clarify=true",
    )
    .await
    .expect("observed execution clarify reply");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, "请提供压缩包路径。");
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Clarify)
    );
}
#[tokio::test]
async fn observed_execution_without_delivery_skips_summary_for_extract_field_result() {
    let state = test_state();
    let task = claimed_task("task-missing-field-observed");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "read package name".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "extract_field",
                    "path": "package.json",
                    "field_path": "name"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"extract_field","exists":false,"field_path":"name","format":"json","path":"package.json","resolved_path":"/tmp/package.json","value":null,"value_text":"","value_type":"null"}"#,
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(free_route_result()),
        ..Default::default()
    };

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "读取 package.json 里的 name 字段，只输出值",
        &loop_state,
        Some(&ctx),
        None,
        "no publishable final answer was produced",
    )
    .await
    .expect("observed execution reply");

    assert_eq!(reply.messages.len(), 2);
    assert!(reply.messages[0].contains("**执行过程**"));
    assert!(reply.messages[0].contains("system_basic"));
}

#[tokio::test]
async fn observed_execution_without_delivery_uses_structured_container_summary() {
    let state = test_state();
    let task = claimed_task("task-structured-container-summary");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"scripts","format":"json","path":"package.json","resolved_field_path":"scripts","value":{"build":"echo build","dev":"echo dev","lint":"echo lint"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\",\"lint\":\"echo lint\"}","value_type":"object"}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "Read the scripts field from package.json and summarize it briefly.",
        &loop_state,
        Some(&ctx),
        None,
        "no publishable final answer was produced",
    )
    .await
    .expect("observed execution reply");

    assert!(!reply.should_fail_task);
    assert_eq!(
        reply.text,
        "`scripts` contains 3 entries: build=echo build, dev=echo dev, lint=echo lint."
    );
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn observed_execution_without_delivery_uses_matrix_grouped_name_answer() {
    let state = test_state();
    let task = claimed_task("task-matrix-grouped-no-delivery");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "workspace".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":2,"files":1,"total":3},"names_by_kind":{"files":["README.md"],"dirs":["configs","docs"],"other":[]},"path":"workspace"}"#,
    ));

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "list direct children grouped by kind",
        &loop_state,
        Some(&ctx),
        None,
        "no publishable final answer was produced",
    )
    .await
    .expect("observed execution reply");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, "dirs:\n- configs\n- docs\nfiles:\n- README.md");
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.finalizer_summary.as_ref())
            .and_then(|summary| summary.format_ok),
        Some(true)
    );
}

#[tokio::test]
async fn observed_execution_without_delivery_uses_matrix_hidden_entries_answer() {
    let state = test_state();
    let task = claimed_task("task-matrix-hidden-no-delivery");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":1,"files":2,"hidden":2,"total":3},"entries":[{"hidden":true,"kind":"dir","name":".git","path":".git"},{"hidden":true,"kind":"file","name":".gitignore","path":".gitignore"},{"hidden":false,"kind":"file","name":"README.md","path":"README.md"}],"include_hidden":true,"names":[".git",".gitignore","README.md"],"path":"."}"#,
    ));

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "check hidden entries",
        &loop_state,
        Some(&ctx),
        None,
        "no publishable final answer was produced",
    )
    .await
    .expect("observed execution reply");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, ".git\n.gitignore");
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn observed_execution_without_delivery_uses_docker_image_observation() {
    let state = test_state();
    let task = claimed_task("task-matrix-docker-images-no-delivery");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DockerImages;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "bash: line 1: docker: command not found\n",
    ));

    let reply = observed_execution_without_publishable_delivery_reply(
        &state,
        &task,
        "list Docker images",
        &loop_state,
        Some(&ctx),
        None,
        "no publishable final answer was produced",
    )
    .await
    .expect("observed execution reply");

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, "bash: line 1: docker: command not found");
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}
#[test]
fn exact_file_names_contract_prefers_observed_list_over_synthesized_sentence() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "document".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "list_dir",
        "alpha.md\nbeta.md\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "document 目录下有 alpha.md 和 beta.md。",
    ));
    loop_state.last_publishable_synthesis_output =
        Some("document 目录下有 alpha.md 和 beta.md。".to_string());
    loop_state.last_user_visible_respond =
        Some("document 目录下有 alpha.md 和 beta.md。".to_string());
    let mut delivery = vec!["document 目录下有 alpha.md 和 beta.md。".to_string()];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task_test",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut finalizer_summary,
    );

    assert_eq!(delivery, vec!["alpha.md\nbeta.md"]);
    assert!(finalizer_summary.is_some());
}

#[test]
fn exact_directory_names_contract_replaces_file_list_synthesis_with_parent_dirs() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryNames;
    route.resolved_intent = "Find directories containing .sh files".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"find_ext","count":4,"ext":"sh","results":["build-all.sh","component_start/start-clawd.sh","scripts/check.sh","component_start/start-feishud.sh"],"root":""}"#,
    ));
    let file_list =
        "1. build-all.sh\n2. component_start/start-clawd.sh\n3. scripts/check.sh".to_string();
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        &file_list,
    ));
    loop_state.last_user_visible_respond = Some(file_list.clone());
    loop_state.last_publishable_synthesis_output = Some(file_list.clone());
    let mut delivery = vec![file_list];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task_test",
        &mut loop_state,
        Some(&ctx),
        &mut delivery,
        &mut finalizer_summary,
    );

    assert_eq!(delivery, vec![".\ncomponent_start\nscripts"]);
    assert!(finalizer_summary.is_some());
}
#[test]
fn preferred_route_clarify_question_only_uses_explicit_route_clarify() {
    let mut route = scalar_route_result();
    route.needs_clarify = true;
    route.clarify_question = "请确认要读取哪个文件？".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    assert_eq!(
        super::preferred_route_clarify_question(Some(&ctx)),
        Some("请确认要读取哪个文件？")
    );

    let mut route = scalar_route_result();
    route.clarify_question = "不会被复用".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    assert_eq!(super::preferred_route_clarify_question(Some(&ctx)), None);
}

#[test]
fn finalize_structured_clarify_context_uses_route_reason_code() {
    let mut route = scalar_route_result();
    route.needs_clarify = true;
    route.route_reason =
        "semantic_contract_requires_evidence; clarify_reason_code:missing_read_target".to_string();
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let context = super::route_structured_clarify_context(Some(&ctx)).expect("structured context");
    assert!(context.contains("clarify_case: missing_read_target"));
    assert!(context.contains("locator_kind: path"));
    assert!(context.contains("response_shape: scalar"));
}

#[test]
fn confirmation_resume_requires_enforce_mode() {
    let mut verify = verify_summary(crate::verifier::VerifyMode::ObserveOnly);
    assert!(!verify_summary_requires_resume_confirmation(&verify));

    verify.mode = crate::verifier::VerifyMode::Enforce;
    assert!(verify_summary_requires_resume_confirmation(&verify));

    verify.approved = false;
    assert!(!verify_summary_requires_resume_confirmation(&verify));
}

#[test]
fn content_evidence_routes_require_clarify_without_qualified_completion() {
    assert!(finalizer_requires_clarify(None, true, false));
    assert!(!finalizer_requires_clarify(None, true, true));

    let allow_fallback = finalizer_summary(crate::finalize::FinalizerDisposition::AllowFallback);
    assert!(finalizer_requires_clarify(
        Some(&allow_fallback),
        true,
        false
    ));
    assert!(!finalizer_requires_clarify(
        Some(&allow_fallback),
        true,
        true
    ));

    let qualified = finalizer_summary(crate::finalize::FinalizerDisposition::QualifiedCompletion);
    assert!(!finalizer_requires_clarify(Some(&qualified), true, false));
    assert!(!finalizer_requires_clarify(None, false, false));
}

#[test]
fn missing_publishable_delivery_can_finish_as_clarify() {
    let summary = crate::task_journal::TaskJournalFinalizerSummary {
        needs_clarify: Some(true),
        ..Default::default()
    };

    let (status, should_fail) =
        observed_execution_without_publishable_delivery_outcome(false, Some(&summary));
    assert_eq!(status, crate::task_journal::TaskJournalFinalStatus::Clarify);
    assert!(!should_fail);

    let (status, should_fail) =
        observed_execution_without_publishable_delivery_outcome(true, Some(&summary));
    assert_eq!(status, crate::task_journal::TaskJournalFinalStatus::Success);
    assert!(!should_fail);

    let (status, should_fail) =
        observed_execution_without_publishable_delivery_outcome(false, None);
    assert_eq!(status, crate::task_journal::TaskJournalFinalStatus::Failure);
    assert!(should_fail);
}

#[test]
fn successful_delivery_can_preserve_structured_user_input_clarify() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    assert_eq!(
        successful_delivery_final_status(&loop_state, None),
        crate::task_journal::TaskJournalFinalStatus::Success
    );

    loop_state.pending_user_input_required = true;
    assert_eq!(
        successful_delivery_final_status(&loop_state, None),
        crate::task_journal::TaskJournalFinalStatus::Clarify
    );
}

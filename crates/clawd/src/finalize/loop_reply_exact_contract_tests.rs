use super::*;

#[test]
fn exact_contract_keeps_rich_content_evidence_delivery_over_short_observed_projection() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "README.md; scripts/nl_tests/fixtures/device_local/docs; configs/skills_registry.toml"
            .to_string();
    let delivery = "以下为本次只读巡检结果：\n\n| 检查项 | 结果 |\n|---|---|\n| README.md 是否存在 | 存在 |\n| docs 文件名 | release_checklist.md、service_notes.md |\n| fs_basic.planner_kind | tool |";

    assert!(
        super::super::exact_contract::should_keep_planned_delivery_over_observed_answer(
            &route,
            delivery,
            "fs_basic planner_kind"
        )
    );
}

#[test]
fn exact_contract_does_not_keep_rich_delivery_when_exact_delivery_is_required() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.delivery_required = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    let delivery = "result:\n- README.md\n- configs/skills_registry.toml";

    assert!(
        !super::super::exact_contract::should_keep_planned_delivery_over_observed_answer(
            &route,
            delivery,
            "README.md"
        )
    );
}

#[test]
fn exact_contract_keeps_latest_terminal_table_over_short_observed_projection() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(6);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"path_batch_facts","facts":[{"exists":true,"path":"README.md"}]}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","names":["archive","release_checklist.md","service_notes.md"],"path":"scripts/nl_tests/fixtures/device_local/docs"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "fs_basic",
        r#"{"extra":{"action":"count_entries","count":2,"path":"scripts/nl_tests/fixtures/device_local/logs"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_4",
        "config_basic",
        r#"{"extra":{"action":"extract_field","field_path":"skills[?(@.name=='fs_basic')].planner_kind","path":"configs/skills_registry.toml","value":"tool","value_text":"tool"}}"#,
    ));
    let table = "```markdown\n| check | result |\n|---|---|\n| README.md | exists |\n| docs names | archive, release_checklist.md, service_notes.md |\n| logs child count | 2 |\n| fs_basic planner_kind | `tool` |\n```\n\nreadonly evidence summary";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_5", "synthesize_answer", table));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_6", "respond", table));
    loop_state.last_user_visible_respond = Some(table.to_string());
    loop_state.last_publishable_synthesis_output = Some(table.to_string());
    let mut delivery_messages = vec![table.to_string()];
    let mut route = free_route_result();
    route.resolved_intent = "multi target readonly inspection".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "README.md; scripts/nl_tests/fixtures/device_local/docs; scripts/nl_tests/fixtures/device_local/logs; configs/skills_registry.toml".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-terminal-table-over-short-observed",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![table]);
    assert_eq!(loop_state.last_user_visible_respond.as_deref(), Some(table));
}

#[test]
fn exact_contract_replaces_incomplete_directory_groups_synthesis_with_observed_groups() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":2,"files":3,"total":5},"names_by_kind":{"files":["README.md","LICENSE","RustClaw.png"],"dirs":["configs","docs"],"other":[]},"path":"workspace"}"#,
    ));
    let incomplete = "文件夹：configs、docs\n文件：README.md";
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        incomplete,
    ));
    loop_state.last_user_visible_respond = Some(incomplete.to_string());
    loop_state.last_publishable_synthesis_output = Some(incomplete.to_string());
    let mut delivery_messages = vec![incomplete.to_string()];
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-directory-groups-incomplete-synthesis",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec!["dirs:\n- configs\n- docs\nfiles:\n- LICENSE\n- README.md\n- RustClaw.png"]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("dirs:\n- configs\n- docs\nfiles:\n- LICENSE\n- README.md\n- RustClaw.png")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn exact_contract_keeps_mixed_directory_content_synthesis_with_read_evidence() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":1,"files":1,"total":2},"names_by_kind":{"files":["package.json"],"dirs":["UI"],"other":[]},"path":"workspace"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","path":"UI/package.json","excerpt":"1|{\"name\":\"react-example\"}"}"#,
    ));
    let answer = "UI 目录像独立前端，因为它有 package.json 且 name 为 react-example。";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", answer));
    loop_state.last_user_visible_respond = Some(answer.to_string());
    loop_state.last_publishable_synthesis_output = Some(answer.to_string());
    let mut delivery_messages = vec![answer.to_string()];
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-directory-groups-mixed-synthesis",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![answer]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(answer)
    );
}

#[test]
fn exact_contract_keeps_publishable_synthesis_over_raw_observed_inventory() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"inventory_dir","counts":{"dirs":1,"files":1,"total":2},"ext_filter":["md"],"names":["regression_llm_first","垃圾代码端分析报告.md"],"names_only":true}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("垃圾代码端分析报告.md".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_user_visible_respond = Some("垃圾代码端分析报告.md".to_string());
    loop_state.last_publishable_synthesis_output = Some("垃圾代码端分析报告.md".to_string());
    let mut delivery_messages = vec!["垃圾代码端分析报告.md".to_string()];
    let mut route = scalar_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.locator_hint = "document".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-synth-file-names",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec!["垃圾代码端分析报告.md"]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("垃圾代码端分析报告.md")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn exact_contract_keeps_model_language_verdict_over_observed_scalar() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","exists":false,"kind":"missing","path":"/tmp/rustclaw-missing-ja.txt"}],"include_missing":true}"#,
    ));
    let planned = "ファイルは存在しません。".to_string();
    loop_state.last_user_visible_respond = Some(planned.clone());
    let mut delivery_messages = vec![planned.clone()];
    let mut route = scalar_route_result();
    route.resolved_intent =
        "Check if /tmp/rustclaw-missing-ja.txt exists; if not, respond briefly in Japanese"
            .to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/rustclaw-missing-ja.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-ja-existence-verdict",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![planned.clone()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(planned.as_str())
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn archive_pack_exact_contract_prefers_observed_archive_path_over_exit_code_respond() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "archive_basic",
        r#"{"extra":{"action":"pack","archive":"/home/guagua/rustclaw/tmp/nl_archive_case.zip","format":"zip","output":"exit=0\nupdating: scripts/skill_calls/"},"text":"archive_path=/home/guagua/rustclaw/tmp/nl_archive_case.zip\nexit=0"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", "0"));
    loop_state.last_user_visible_respond = Some("0".to_string());
    let mut delivery_messages = vec!["0".to_string()];
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchivePack;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case.zip".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-archive-pack-path",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec!["/home/guagua/rustclaw/tmp/nl_archive_case.zip".to_string()]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("/home/guagua/rustclaw/tmp/nl_archive_case.zip")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn archive_pack_capability_ref_prefers_observed_archive_path_without_semantic_kind() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "archive_basic",
        r#"{"extra":{"action":"pack","archive":"/home/guagua/rustclaw/tmp/nl_archive_case.zip","format":"zip","output":"exit=0\nupdating: scripts/skill_calls/"},"text":"archive_path=/home/guagua/rustclaw/tmp/nl_archive_case.zip\nexit=0"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", "0"));
    loop_state.last_user_visible_respond = Some("0".to_string());
    let mut delivery_messages = vec!["0".to_string()];
    let mut route = free_route_result();
    route.route_reason = "capability_ref=archive.pack".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case.zip".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-archive-pack-capability-ref-path",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec!["/home/guagua/rustclaw/tmp/nl_archive_case.zip".to_string()]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("/home/guagua/rustclaw/tmp/nl_archive_case.zip")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn archive_pack_exact_contract_keeps_later_terminal_respond() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "archive_basic",
        r#"{"extra":{"action":"pack","archive":"/home/guagua/rustclaw/tmp/nl_archive_case.zip","format":"zip","output":"exit=0\nupdating: scripts/skill_calls/"},"text":"archive_path=/home/guagua/rustclaw/tmp/nl_archive_case.zip\nexit=0"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "respond",
        "needs_user_confirmation",
    ));
    loop_state.last_user_visible_respond = Some("needs_user_confirmation".to_string());
    let mut delivery_messages = vec!["needs_user_confirmation".to_string()];
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchivePack;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case.zip".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-archive-pack-terminal-respond",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec!["needs_user_confirmation"]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("needs_user_confirmation")
    );
}

#[test]
fn exact_contract_keeps_planned_subset_over_raw_observed_file_paths() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"find_ext","count":4,"ext":"toml","results":["Cargo.toml","configs/config.toml","configs/skills_registry.toml","crates/clawd/Cargo.toml"]}"#,
    ));
    let planned = "Cargo.toml\nconfigs/config.toml\nconfigs/skills_registry.toml".to_string();
    loop_state.last_user_visible_respond = Some(planned.clone());
    let mut delivery_messages = vec![planned.clone()];
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-planned-subset-file-paths",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![planned]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("Cargo.toml\nconfigs/config.toml\nconfigs/skills_registry.toml")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn exact_contract_keeps_explicit_json_delivery_over_observed_phrase() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"README.md","resolved_path":"/home/guagua/rustclaw/README.md","size_bytes":24929},"path":"/home/guagua/rustclaw/README.md"}],"fields":["exists","size"],"include_missing":true}"#
                .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_user_visible_respond =
        Some(r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#.to_string());
    let mut delivery_messages =
        vec![r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#.to_string()];
    let mut route = scalar_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = "README.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-strict-json-delivery",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec![r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#)
    );
    assert!(finalizer_summary.is_none());
}

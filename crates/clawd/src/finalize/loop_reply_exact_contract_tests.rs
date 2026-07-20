use super::*;

#[test]
fn exact_contract_keeps_rich_content_evidence_delivery_over_short_observed_projection() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint =
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
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.delivery_required = true;
    route.locator_kind = crate::OutputLocatorKind::Path;
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
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.locator_hint = "README.md; scripts/nl_tests/fixtures/device_local/docs; scripts/nl_tests/fixtures/device_local/logs; configs/skills_registry.toml".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
fn unclassified_inventory_keeps_model_synthesis_with_read_evidence() {
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
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.semantic_kind = crate::OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.locator_hint = "document".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "/tmp/rustclaw-missing-ja.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::FilePaths;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.locator_hint = "README.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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

use super::*;
use std::collections::BTreeSet;

#[test]
fn archive_read_contract_recovers_explicit_archive_path_when_locator_hint_is_empty() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let request = format!("读取 {archive} 里的 notes.txt 内容片段，并简短总结。");
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.resolved_intent = request.clone();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint.clear();
    let loop_state = LoopState::new(1);

    let plan = archive_read_deterministic_plan_result(
        "read archive member",
        &state,
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw/tmp/contract_matrix_unpacked/notes.txt"),
        &request,
    )
    .expect("archive read plan should recover explicit archive path");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "archive_basic", "read");
    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

#[test]
fn archive_read_contract_prefers_complete_request_path_over_basename_locator_hint() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let request = format!("读取 {archive} 里的 notes.txt 内容片段，并简短总结。");
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.resolved_intent = request.clone();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = "test_bundle.zip | notes.txt".to_string();
    let loop_state = LoopState::new(1);

    let plan = archive_read_deterministic_plan_result(
        "read archive member",
        &state,
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw/tmp/contract_matrix_unpacked/notes.txt"),
        &request,
    )
    .expect("archive read plan should restore full archive path");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "archive_basic", "read");
    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

#[test]
fn archive_read_structural_member_target_plans_direct_read_without_semantic_label() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.resolved_intent =
        format!("Read the notes.txt content from archive {archive} and output only it");
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_hint = archive.to_string();
    let loop_state = LoopState::new(1);

    let plan = archive_read_deterministic_plan_result(
        "read archive member",
        &state,
        Some(&route),
        &loop_state,
        Some(archive),
        &format!("Read {archive} member notes.txt"),
    )
    .expect("archive read plan from structural member target");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "archive_basic", "read");
    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

#[test]
fn archive_read_contract_rejects_unsafe_member_locator() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | ../secret.txt".to_string();
    let loop_state = LoopState::new(1);

    assert!(archive_read_deterministic_plan_result(
        "read archive member",
        &state,
        Some(&route),
        &loop_state,
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"),
        "Read member ../secret.txt from scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
    )
    .is_none());
}

#[test]
fn archive_database_aggregate_uses_structured_skills_for_compound_archive_list_route() {
    let state = test_state_with_enabled_skills(&["archive_basic", "db_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let db_path = "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite";
    let request = format!("列出 {archive} 的成员并读取 notes.txt；再查看 {db_path} 的表列表。");
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.resolved_intent = format!(
        "archive.list archive.read database.list_tables archive={archive} member=notes.txt db_path={db_path}"
    );
    route.route_reason = "machine_plan: archive.list archive.read database.list_tables".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveList;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = format!("{archive} | {db_path}");
    let loop_state = LoopState::new(1);

    let plan = archive_database_aggregate_deterministic_plan_result(
        &state,
        "archive plus sqlite aggregate",
        Some(&route),
        &loop_state,
        &request,
        None,
    )
    .expect("multi-source archive/sqlite request should use structured tools");

    assert_eq!(plan.steps.len(), 5);
    let list_action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&list_action, "archive_basic", "list");
    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
    let read_action = plan.steps[1].to_agent_action().expect("agent action");
    let args = expect_planned_call(&read_action, "archive_basic", "read");
    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
    let db_action = plan.steps[2].to_agent_action().expect("agent action");
    let args = expect_planned_call(&db_action, "db_basic", "list_tables");
    assert_eq!(args.get("db_path").and_then(Value::as_str), Some(db_path));
    assert_eq!(plan.steps[3].action_type, "synthesize_answer");
    assert_eq!(plan.steps[4].action_type, "respond");
}

#[test]
fn archive_database_aggregate_handles_content_excerpt_fallback_route() {
    let state = test_state_with_enabled_skills(&["archive_basic", "db_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let db_path = "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite";
    let request = format!("列出 {archive} 的成员并读取 notes.txt；再查看 {db_path} 的表列表。");
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.resolved_intent =
        "llm_failed_existing_path_observation_fallback; explicit_existing_path_observation"
            .to_string();
    route.route_reason = "auto_locator_suppressed_multiple_explicit_paths".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_hint = format!("{archive} | {db_path}");
    let loop_state = LoopState::new(1);

    let plan = archive_database_aggregate_deterministic_plan_result(
        &state,
        "archive plus sqlite fallback aggregate",
        Some(&route),
        &loop_state,
        &request,
        None,
    )
    .expect("fallback content excerpt route should preserve compound structured observations");

    assert_eq!(plan.steps.len(), 5);
    let read_action = plan.steps[1].to_agent_action().expect("agent action");
    let args = expect_planned_call(&read_action, "archive_basic", "read");
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
    let db_action = plan.steps[2].to_agent_action().expect("agent action");
    let args = expect_planned_call(&db_action, "db_basic", "list_tables");
    assert_eq!(args.get("db_path").and_then(Value::as_str), Some(db_path));
}

#[test]
fn transform_action_alias_and_sort_args_normalize_to_transform_data_ops() {
    let actions = vec![AgentAction::CallTool {
        tool: "transform".to_string(),
        args: json!({
            "action": "transform",
            "data": [
                {"name": "alpha", "score": 7},
                {"name": "beta", "score": 12}
            ],
            "sort_by": "score",
            "order": "desc",
            "output_format": "md_table"
        }),
    }];

    let normalized = normalize_transform_schema_aliases(actions);

    let args = expect_planned_call(&normalized[0], "transform", "transform_data");
    assert_eq!(
        args.get("output_format").and_then(Value::as_str),
        Some("md_table")
    );
    let ops = args
        .get("ops")
        .and_then(Value::as_array)
        .expect("ops array");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].get("op").and_then(Value::as_str), Some("sort"));
    assert_eq!(ops[0].get("by").and_then(Value::as_str), Some("score"));
    assert_eq!(ops[0].get("order").and_then(Value::as_str), Some("desc"));
    assert!(args.get("sort_by").is_none());
}

#[test]
fn inline_json_transform_deterministic_plan_uses_current_payload() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"{"action":"transform_data","data":[{"name":"alpha","score":7},{"name":"beta","score":12}],"ops":[{"op":"filter","where":{"field":"score","gte":7}}]}"#;
    let goal = r#"older context: {"action":"transform_data","data":[{"stale":true}],"ops":[{"op":"project","fields":["stale"]}]}"#;

    let plan =
        inline_json_transform_deterministic_plan_result(goal, &state, &loop_state, current, None)
            .expect("inline transform should produce deterministic plan");

    assert_eq!(plan.steps.len(), 1);
    let step = &plan.steps[0];
    assert_eq!(step.action_type, "call_skill");
    assert_eq!(step.skill, "transform");
    assert_eq!(
        step.args.get("action").and_then(Value::as_str),
        Some("transform_data")
    );
    assert_eq!(
        step.args
            .get("data")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("name"))
            .and_then(Value::as_str),
        Some("alpha")
    );
    let op = step
        .args
        .get("ops")
        .and_then(Value::as_array)
        .and_then(|ops| ops.first())
        .and_then(Value::as_object)
        .expect("normalized filter op");
    assert_eq!(op.get("field").and_then(Value::as_str), Some("score"));
    assert_eq!(op.get("cmp").and_then(Value::as_str), Some("gte"));
    assert_eq!(op.get("value").and_then(Value::as_i64), Some(7));
}

#[test]
fn inline_json_transform_does_not_derive_group_sum_from_answer_candidate() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"对这个 JSON 数组按 team 分组求 amount 总和，只输出 JSON：[{"team":"A","amount":3},{"team":"A","amount":4},{"team":"B","amount":2}]"#;
    let mut route = base_route_result();
    route.route_reason = "inline_json_transform_structured_execute".to_string();
    route.resolved_intent =
            "group inline records\nanswer_candidate: [{\"team\":\"A\",\"amount\":7},{\"team\":\"B\",\"amount\":2}]".to_string();

    let plan = inline_json_transform_deterministic_plan_result(
        current,
        &state,
        &loop_state,
        current,
        Some(&route),
    );

    assert!(plan.is_none());
}

#[test]
fn contextual_inline_payload_does_not_guess_default_numeric_sort_table() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"[{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let mut route = base_route_result();
    route.route_reason = "inline_structured_payload_context_execute:test".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;

    let plan = inline_json_transform_deterministic_plan_result(
        current,
        &state,
        &loop_state,
        current,
        Some(&route),
    );

    assert!(plan.is_none());
}

#[test]
fn repaired_inline_transform_contract_does_not_guess_default_numeric_sort_table() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"Sort this JSON array by score descending and output only a markdown table: [{"name":"alpha","score":7},{"name":"beta","score":12},{"name":"gamma","score":9}]"#;
    let mut route = base_route_result();
    route.route_reason = "inline_structured_transform_contract_repair".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::None;

    let plan = inline_json_transform_deterministic_plan_result(
        current,
        &state,
        &loop_state,
        current,
        Some(&route),
    );

    assert!(plan.is_none());
}

#[test]
fn inline_json_transform_derives_single_object_rename_after_context_json() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"把这个 JSON 对象里的 old_name 改成 new_name，只输出 JSON：{"old_name":"alpha","count":2}"#;
    let goal = format!(
        r#"background example: {{"kind":"ask","payload":{{"text":"hello"}}}}

Structured inline transform request:
{current}"#
    );
    let mut route = base_route_result();
    route.route_reason = "inline_json_transform_structured_execute".to_string();
    route.resolved_intent = r#"rename inline object
answer_candidate: {"new_name":"alpha","count":2}"#
        .to_string();

    let plan = inline_json_transform_deterministic_plan_result(
        &goal,
        &state,
        &loop_state,
        "",
        Some(&route),
    )
    .expect("context JSON should not steal inline object transform");

    let step = &plan.steps[0];
    assert_eq!(step.skill, "transform");
    assert_eq!(
        step.args.get("result_shape").and_then(Value::as_str),
        Some("single_object")
    );
    assert!(step.args.get("data").is_some_and(Value::is_object));
}

#[test]
fn inline_json_transform_derives_single_object_rename_without_answer_candidate() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let req = r#"把这个 JSON 对象里的 old_name 改成 new_name，只输出 JSON：{"old_name":"alpha","count":2}"#;

    let plan = inline_json_transform_deterministic_plan_result(req, &state, &loop_state, req, None)
        .expect("single object rename should produce deterministic plan");

    let step = &plan.steps[0];
    assert_eq!(step.skill, "transform");
    assert!(step.args.get("data").is_some_and(Value::is_object));
    assert_eq!(
        step.args.get("result_shape").and_then(Value::as_str),
        Some("single_object")
    );
    assert_eq!(
        step.args
            .get("ops")
            .and_then(Value::as_array)
            .and_then(|ops| ops.first())
            .and_then(|op| op.get("op"))
            .and_then(Value::as_str),
        Some("rename")
    );
}

#[test]
fn inline_json_transform_does_not_derive_scalar_sum_from_answer_candidate() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"计算这个 JSON 数组里 value 的总和，只输出数字：[ {"value": 4}, {"value": 6}, {"value": 5} ]"#;
    let goal = format!(
        r#"background example: {{"kind":"ask","payload":{{"text":"hello"}}}}

Structured inline transform request:
{current}"#
    );
    let mut route = base_route_result();
    route.route_reason = "inline_json_transform_structured_execute".to_string();
    route.resolved_intent = "sum inline records\nanswer_candidate: 15".to_string();

    let plan = inline_json_transform_deterministic_plan_result(
        &goal,
        &state,
        &loop_state,
        "",
        Some(&route),
    );

    assert!(plan.is_none());
}

#[test]
fn inline_json_transform_derives_count_from_scalar_count_contract() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = r#"统计这个 JSON 数组中对象数量，只输出数字：[{"x":1},{"x":2},{"x":3},{"x":4}]"#;
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;

    let plan = inline_json_transform_deterministic_plan_result(
        current,
        &state,
        &loop_state,
        current,
        Some(&route),
    )
    .expect("inline scalar count should produce deterministic transform");

    let step = &plan.steps[0];
    assert_eq!(step.skill, "transform");
    assert_eq!(
        step.args.get("result_shape").and_then(Value::as_str),
        Some("scalar")
    );
    let agg = step
        .args
        .get("ops")
        .and_then(Value::as_array)
        .and_then(|ops| ops.first())
        .and_then(|op| op.get("aggregations"))
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .expect("count aggregation");
    assert_eq!(agg.get("op").and_then(Value::as_str), Some("count"));
}

#[test]
fn inline_csv_transform_derives_markdown_table_from_escaped_newlines() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let loop_state = LoopState::new(1);
    let current = "把这个 CSV 转成 markdown 表格：name,score\\nalpha,7\\nbeta,9";
    let mut route = base_route_result();
    route.resolved_intent =
            "render inline records\nanswer_candidate: | name | score |\n|------|-------|\n| alpha | 7 |\n| beta | 9 |".to_string();

    let plan = inline_json_transform_deterministic_plan_result(
        current,
        &state,
        &loop_state,
        current,
        Some(&route),
    )
    .expect("escaped newline CSV should produce deterministic transform");

    assert_eq!(plan.steps.len(), 1);
    let step = &plan.steps[0];
    assert_eq!(step.skill, "transform");
    assert_eq!(
        step.args.get("csv_text").and_then(Value::as_str),
        Some("name,score\nalpha,7\nbeta,9")
    );
    assert_eq!(
        step.args.get("output_format").and_then(Value::as_str),
        Some("md_table")
    );
}

#[test]
fn lightweight_prompt_mentions_archive_basic_for_archive_contracts() {
    let state = test_state_with_enabled_skills(&[
        "archive_basic",
        "docker_basic",
        "config_guard",
        "doc_parse",
        "transform",
        "browser_web",
    ])
    .with_prompt_layers_installed();
    let task = test_task();
    let quick_index = build_lightweight_skill_quick_index_text(&state, &task, None);
    let playbooks = build_lightweight_skill_playbooks_text(&state, &task, None);
    assert!(quick_index.contains("archive_basic"));
    assert!(playbooks.contains("archive_basic"));
    assert!(playbooks.contains("`pack`") || playbooks.contains("packing"));
    assert!(quick_index.contains("docker_basic"));
    assert!(playbooks.contains("docker_basic"));
    assert!(quick_index.contains("config_guard"));
    assert!(playbooks.contains("config_guard"));
    assert!(quick_index.contains("doc_parse"));
    assert!(playbooks.contains("doc_parse"));
    assert!(quick_index.contains("transform"));
    assert!(playbooks.contains("transform"));
    assert!(quick_index.contains("browser_web"));
    assert!(playbooks.contains("browser_web"));
}

#[test]
fn lightweight_prompt_includes_registry_planner_metadata() {
    let state = test_state_with_registry();
    let registry = state.get_skills_registry().expect("registry loaded");
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(HashSet::from([
            "archive_basic".to_string(),
            "service_control".to_string(),
        ])),
    });
    let state = state.with_prompt_layers_installed();
    let task = test_task();
    let quick_index = build_lightweight_skill_quick_index_text(&state, &task, None);
    let playbooks = build_lightweight_skill_playbooks_text(&state, &task, None);
    assert!(quick_index.contains("archive_basic"));
    assert!(quick_index.contains("planner_kind: tool"));
    assert!(quick_index.contains("semantic_tags: archive_list"));
    assert!(quick_index.contains("preferred_over_run_cmd: true"));
    assert!(quick_index.contains("validation_actions: list"));
    assert!(quick_index.contains("planner_capabilities: archive.list"));
    assert!(quick_index.contains("optional=format"));
    assert!(quick_index.contains("risk=high"));
    assert!(quick_index.contains("output_contract: kind=text"));
    assert!(quick_index.contains("required=text"));
    assert!(playbooks.contains("### archive_basic"));
    assert!(playbooks.contains("Registry metadata: planner_kind: tool"));
    assert!(playbooks.contains("semantic_tags: archive_list"));
    assert!(playbooks.contains("preferred_over_run_cmd: true"));
    assert!(playbooks.contains("validation_actions: list"));
    assert!(playbooks.contains("planner_capabilities: archive.list"));
    assert!(playbooks.contains("output_contract: kind=text"));
    assert!(playbooks.contains("### service_control"));
    assert!(playbooks.contains("semantic_tags: service_status"));
}

#[test]
fn lightweight_prompt_respects_contract_skill_scope() {
    let state = test_state_with_enabled_skills(&["fs_basic", "archive_basic", "docker_basic"])
        .with_prompt_layers_installed();
    let task = test_task();
    let scope = BTreeSet::from(["fs_basic".to_string()]);

    let quick_index = build_lightweight_skill_quick_index_text(&state, &task, Some(&scope));
    let playbooks = build_lightweight_skill_playbooks_text(&state, &task, Some(&scope));

    assert!(quick_index.contains("fs_basic"));
    assert!(playbooks.contains("fs_basic"));
    assert!(!quick_index.contains("archive_basic"));
    assert!(!playbooks.contains("archive_basic"));
    assert!(!quick_index.contains("docker_basic"));
    assert!(!playbooks.contains("docker_basic"));
}

#[test]
fn lightweight_skill_playbook_keeps_config_entry_points() {
    let prompt = r#"
## Capability Summary
- Converts audio to text.

## Config Entry Points
- Main STT config: `configs/audio.toml` -> `[audio_transcribe]`.
- Local provider uses `audio_transcribe.providers.custom`.

## Parameter Contract
- `path` is optional here.
"#;
    let compact = compact_skill_playbook_from_prompt("audio_transcribe", prompt);
    assert!(compact.contains("configs/audio.toml"));
    assert!(compact.contains("audio_transcribe.providers.custom"));
}

#[test]
fn lightweight_tool_spec_includes_route_task_contract() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;

    let spec = build_lightweight_tool_spec(Some(&route), None);

    assert!(spec.contains("task_contract"));
    assert!(spec.contains("route_gate_kind=execute"));
    assert!(!spec.contains("ask_mode="));
    assert!(!spec.contains("derived_route_label="));
    assert!(spec.contains("intent_kind=planner_execute"));
    assert!(spec.contains("target_object=directory"));
    assert!(spec.contains("operation=list"));
    assert!(spec.contains("required_evidence_fields=candidates"));
    assert!(spec.contains("failure_policy=retry_with_alternatives"));
}

#[test]
fn planner_prompt_contract_guard_allows_present_compact_contract_block() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let contract_line =
        crate::contract_matrix::compact_prompt_line_for_route(&route).expect("contract line");
    let prompt = format!("System\n{contract_line}\nUser");

    ensure_required_contract_block_present(Some(&route), &prompt).expect("contract present");
}

#[test]
fn planner_prompt_contract_guard_fails_closed_when_compact_contract_block_missing() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;

    let err = ensure_required_contract_block_present(Some(&route), "System\nUser")
        .expect_err("missing contract block should fail closed");

    assert!(err.contains("prompt_budget_error"));
    assert!(err.contains("contract_line_hash="));
}

#[test]
fn planning_prompt_class_uses_lightweight_execution_for_scalar_contract() {
    let mut route = base_route_result();
    route.route_reason = "llm_contract:generic_filename_scalar_extract".to_string();
    route.resolved_intent = "读取 UI/package.json 里的 name 字段，只输出值".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &LoopState::default())
            .as_str(),
        "lightweight_execution"
    );
}

#[test]
fn planning_prompt_class_uses_lightweight_execution_for_generic_scalar_path_read() {
    let mut route = base_route_result();
    route.resolved_intent =
            "读取 /home/guagua/rustclaw/configs/config.toml 中的 tools.allow_sudo 配置项的值，并仅输出该值"
                .to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/configs/config.toml".to_string();
    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &LoopState::default())
            .as_str(),
        "lightweight_execution"
    );
}

#[test]
fn planning_prompt_class_uses_lightweight_execution_for_pwd_only_route() {
    let mut route = base_route_result();
    route.route_reason = "llm_contract:scalar_path_only".to_string();
    route.resolved_intent = "只输出当前工作目录的绝对路径，不要解释".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &LoopState::default())
            .as_str(),
        "lightweight_execution"
    );
}

#[test]
fn planning_prompt_class_uses_lightweight_execution_for_content_evidence_reads() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.route_reason = "llm_contract:generic_filename_read_range".to_string();
    route.resolved_intent = "先读一下 README.md 前 4 行".to_string();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &LoopState::default())
            .as_str(),
        "lightweight_execution"
    );
}

#[test]
fn planning_prompt_class_uses_lightweight_for_concrete_path_content_excerpt() {
    let mut route = base_route_result();
    route.route_reason = "llm_contract:content_excerpt_summary_path_read".to_string();
    route.resolved_intent =
        "读取 /home/guagua/rustclaw/README.md 前 20 行并回答结构化摘要".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/README.md".to_string();
    let mut round2 = LoopState::default();
    round2.round_no = 2;
    round2.has_tool_or_skill_output = true;

    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &round2).as_str(),
        "lightweight_execution"
    );
}

#[test]
fn planning_prompt_class_keeps_open_for_unbounded_chat_wrapped_but_light_for_later_rounds() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.resolved_intent = "比较这两个文件大小，然后一句话总结".to_string();
    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &LoopState::default())
            .as_str(),
        "open_planning"
    );

    let mut scalar = base_route_result();
    scalar.route_reason = "llm_contract:generic_filename_scalar_extract".to_string();
    scalar.resolved_intent = "读取 UI/package.json 里的 name 字段，只输出值".to_string();
    scalar.output_contract.response_shape = OutputResponseShape::Scalar;
    scalar.output_contract.requires_content_evidence = true;
    scalar.output_contract.locator_kind = OutputLocatorKind::Filename;
    scalar.output_contract.locator_hint = "package.json".to_string();
    let mut round2 = LoopState::default();
    round2.round_no = 2;
    assert_eq!(
        classify_planning_prompt_class(Some(&scalar), &scalar.resolved_intent, &round2).as_str(),
        "lightweight_execution"
    );
}

#[test]
fn planning_prompt_class_uses_lightweight_for_bounded_observation_summary_later_round() {
    let mut route = base_route_result();
    route.resolved_intent =
        "Run pwd, inspect clawd process and listening ports, then summarize observed results."
            .to_string();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let mut round2 = LoopState::default();
    round2.round_no = 2;
    round2.has_tool_or_skill_output = true;

    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &round2).as_str(),
        "lightweight_execution"
    );
}

#[test]
fn planning_prompt_class_keeps_open_planning_for_current_workspace_drafting() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.resolved_intent =
        "Write a short RustClaw setup note for the current workspace project".to_string();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint = "rustclaw workspace".to_string();

    assert_eq!(
        classify_planning_prompt_class(Some(&route), &route.resolved_intent, &LoopState::default())
            .as_str(),
        "open_planning"
    );
}

#[test]
fn round1_prompt_spec_switches_to_lightweight_prompt_for_light_class() {
    assert_eq!(
        round1_prompt_spec_for_class(PlanningPromptClass::OpenPlanning),
        (
            "single_plan_execution_prompt",
            "prompts/single_plan_execution_prompt.md",
        )
    );
    assert_eq!(
        round1_prompt_spec_for_class(PlanningPromptClass::LightweightExecution),
        (
            "lightweight_execution_prompt",
            "prompts/lightweight_execution_prompt.md",
        )
    );
}

#[test]
fn incremental_prompt_spec_switches_to_lightweight_prompt_for_light_class() {
    assert_eq!(
        incremental_prompt_spec_for_class(PlanningPromptClass::OpenPlanning),
        (
            "loop_incremental_plan_prompt",
            "prompts/loop_incremental_plan_prompt.md",
        )
    );
    assert_eq!(
        incremental_prompt_spec_for_class(PlanningPromptClass::LightweightExecution),
        (
            "lightweight_incremental_plan_prompt",
            "prompts/lightweight_incremental_plan_prompt.md",
        )
    );
}

#[test]
fn lightweight_incremental_goal_context_omits_background_memory_sections() {
    let goal = "\
### MEMORY_USE_POLICY
profile: planner_scoped
reason: test

### PLANNER_MEMORY_CONTEXT (BACKGROUND ONLY)
#### STABLE_FACTS
- stale artifact /tmp/old-output.txt

### CURRENT_REQUEST
Run `pwd`, then inspect process status.

### RECENT_EXECUTION_CONTEXT
old step output that should not override this task

### RUNTIME_CONTEXT
current_process_cwd: /home/guagua/rustclaw
workspace_root: /home/guagua/rustclaw";

    let compact = compact_lightweight_incremental_goal_context(goal);

    assert!(compact.contains("LIGHTWEIGHT_INCREMENTAL_CONTEXT_BUDGET"));
    assert!(compact.contains(
        "omitted_sections=memory_use_policy,planner_memory_context,recent_execution_context"
    ));
    assert!(!compact.contains("stale artifact /tmp/old-output.txt"));
    assert!(!compact.contains("old step output that should not override this task"));
    assert!(compact.contains("### CURRENT_REQUEST"));
    assert!(compact.contains("Run `pwd`, then inspect process status."));
    assert!(compact.contains("current_process_cwd: /home/guagua/rustclaw"));
}

#[test]
fn lightweight_incremental_goal_context_truncates_middle_and_preserves_tail() {
    let goal = format!(
        "### CURRENT_REQUEST\n{}\n\n### RUNTIME_CONTEXT\ncurrent_process_cwd: /repo\nworkspace_root: /repo",
        "large_context_line\n".repeat(1400)
    );

    let compact = compact_lightweight_incremental_goal_context(&goal);

    assert!(compact.contains("truncated_middle=true"));
    assert!(compact.len() < goal.len());
    assert!(compact.contains("### CURRENT_REQUEST"));
    assert!(compact.contains("workspace_root: /repo"));
}

#[test]
fn lightweight_tool_spec_includes_contract_and_auto_locator() {
    let mut route = base_route_result();
    route.route_reason = "llm_contract:generic_explicit_path_scalar_extract".to_string();
    route.resolved_intent = "读取 UI/package.json 里的 name 字段，只输出值".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_hint = "UI/package.json".to_string();
    let rendered = build_lightweight_tool_spec(Some(&route), Some("/tmp/UI/package.json"));
    assert!(rendered.contains("planning_class=lightweight_execution"));
    assert!(rendered.contains("route_gate_kind=execute"));
    assert!(rendered.contains("response_shape=scalar"));
    assert!(rendered.contains("locator_hint=UI/package.json"));
    assert!(rendered.contains("auto_locator_path=/tmp/UI/package.json"));
}

#[test]
fn rewrite_extract_field_field_alias_to_field_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": "/tmp/config.toml",
            "field": "tools.allow_sudo"
        }),
    }];
    let out = rewrite_extract_field_alias_args(actions);
    match &out[0] {
        AgentAction::CallSkill { args, .. } => {
            assert_eq!(
                args.get("field_path").and_then(|value| value.as_str()),
                Some("tools.allow_sudo")
            );
            assert!(args.get("field").is_none());
        }
        other => panic!("expected call_skill, got {other:?}"),
    }
}

#[test]
fn rewrite_extract_field_keeps_existing_field_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": "/tmp/config.toml",
            "field": "tools.allow_sudo",
            "field_path": "tools.allow_path_outside_workspace"
        }),
    }];
    let out = rewrite_extract_field_alias_args(actions);
    match &out[0] {
        AgentAction::CallSkill { args, .. } => {
            assert_eq!(
                args.get("field_path").and_then(|value| value.as_str()),
                Some("tools.allow_path_outside_workspace")
            );
            assert_eq!(
                args.get("field").and_then(|value| value.as_str()),
                Some("tools.allow_sudo")
            );
        }
        other => panic!("expected call_skill, got {other:?}"),
    }
}

#[test]
fn rewrite_extract_field_file_path_alias_to_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "file_path": "/tmp/config.toml",
            "field_path": "tools.allow_sudo"
        }),
    }];
    let out = rewrite_extract_field_alias_args(actions);
    match &out[0] {
        AgentAction::CallSkill { args, .. } => {
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("/tmp/config.toml")
            );
            assert!(args.get("file_path").is_none());
        }
        other => panic!("expected call_skill, got {other:?}"),
    }
}

#[test]
fn rewrite_extract_field_target_alias_to_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "target": "/tmp/config.toml",
            "field_path": "tools.allow_sudo"
        }),
    }];
    let out = rewrite_extract_field_alias_args(actions);
    match &out[0] {
        AgentAction::CallSkill { args, .. } => {
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("/tmp/config.toml")
            );
            assert!(args.get("target").is_none());
        }
        other => panic!("expected call_skill, got {other:?}"),
    }
}

#[test]
fn extract_field_rewrites_bare_manifest_to_shallow_candidate_with_field() {
    let root = TempDirGuard::new("structured_manifest_candidate");
    fs::write(
        root.path.join("package.json"),
        r#"{"dependencies":{"left-pad":"1.0.0"}}"#,
    )
    .expect("write root package");
    fs::create_dir_all(root.path.join("UI")).expect("create ui");
    fs::write(
        root.path.join("UI/package.json"),
        r#"{"name":"react-example"}"#,
    )
    .expect("write ui package");
    fs::create_dir_all(root.path.join("services/wa-web-bridge")).expect("create service");
    fs::write(
        root.path.join("services/wa-web-bridge/package.json"),
        r#"{"name":"wa-web-bridge"}"#,
    )
    .expect("write service package");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let root_package = root.path.join("package.json");
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": root_package.display().to_string(),
            "field_path": "name"
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "读取 package.json 里的 name 字段",
        None,
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root.path.join("UI/package.json").to_string_lossy().as_ref())
    );
}

#[test]
fn extract_field_rewrites_workspace_cargo_package_field_to_current_package_manifest() {
    let root = TempDirGuard::new("workspace_cargo_candidate");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/other", "crates/clawd"]
"#,
    )
    .expect("write workspace cargo");
    fs::create_dir_all(root.path.join("crates/other")).expect("create other");
    fs::write(
        root.path.join("crates/other/Cargo.toml"),
        r#"[package]
name = "other"
"#,
    )
    .expect("write other cargo");
    fs::create_dir_all(root.path.join("crates/clawd")).expect("create clawd");
    fs::write(
        root.path.join("crates/clawd/Cargo.toml"),
        r#"[package]
name = "clawd"
"#,
    )
    .expect("write clawd cargo");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let root_cargo = root.path.join("Cargo.toml");
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": root_cargo.display().to_string(),
            "field_path": "package.name"
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "读取 Cargo.toml 的 package.name",
        None,
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(
            root.path
                .join("crates/clawd/Cargo.toml")
                .to_string_lossy()
                .as_ref()
        )
    );
}

#[test]
fn extract_field_keeps_root_manifest_when_auto_locator_is_workspace_root_scope() {
    let root = TempDirGuard::new("root_scope_manifest_binding");
    let root_package = root.path.join("package.json");
    fs::write(
        &root_package,
        r#"{"dependencies":{"@xdevplatform/xurl":"^1.0.3"}}"#,
    )
    .expect("write root package");
    fs::create_dir_all(root.path.join("UI")).expect("create ui");
    fs::write(
        root.path.join("UI/package.json"),
        r#"{"name":"react-example"}"#,
    )
    .expect("write ui package");
    let root_cargo = root.path.join("Cargo.toml");
    fs::write(
        &root_cargo,
        r#"[workspace]
members = ["crates/clawd"]

[workspace.package]
version = "0.1.7"

[workspace.dependencies]
toml = "0.8"
reqwest = { version = "0.12" }
"#,
    )
    .expect("write workspace cargo");
    fs::create_dir_all(root.path.join("crates/clawd")).expect("create clawd");
    fs::write(
        root.path.join("crates/clawd/Cargo.toml"),
        r#"[package]
name = "clawd"
"#,
    )
    .expect("write member cargo");

    let mut state = test_state_with_enabled_skills(&["system_basic", "config_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.delivery_required = false;
    route.resolved_intent =
        "Read root package.json name and root Cargo.toml package.name".to_string();
    let root_scope = root.path.display().to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "path": "package.json",
                "field_path": "name",
                "format": "json",
            }),
        },
        AgentAction::CallTool {
            tool: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "path": "Cargo.toml",
                "field_path": "package.name",
                "format": "toml",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read root package.json name and root Cargo.toml package.name",
        Some(&root_scope),
        actions,
    );
    let read_paths = normalized
        .iter()
        .filter_map(|action| {
            let args = match action {
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill == "config_basic"
                        && args.get("action").and_then(Value::as_str) == Some("read_field") =>
                {
                    args
                }
                _ => return None,
            };
            args.get("path").and_then(Value::as_str).map(|raw| {
                let path = Path::new(raw);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    root.path.join(path)
                }
            })
        })
        .collect::<Vec<_>>();

    assert_eq!(read_paths.len(), 2, "normalized actions: {normalized:?}");
    assert_eq!(read_paths[0], root_package);
    assert_eq!(read_paths[1], root_cargo);
}

#[test]
fn extract_field_rewrites_workspace_cargo_package_version_to_workspace_package_version() {
    let root = TempDirGuard::new("workspace_cargo_version");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/clawd"]

[workspace.package]
version = "0.1.7"

[workspace.dependencies]
toml = "0.8"
reqwest = { version = "0.12" }
"#,
    )
    .expect("write workspace cargo");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let root_cargo = root.path.join("Cargo.toml");
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": root_cargo.display().to_string(),
            "field_path": "package.version"
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read workspace package version from Cargo.toml",
        Some(root_cargo.to_string_lossy().as_ref()),
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_cargo.to_string_lossy().as_ref())
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("workspace.package.version")
    );
}

#[test]
fn config_basic_read_field_rewrites_workspace_cargo_package_version_to_workspace_package_version() {
    let root = TempDirGuard::new("config_workspace_cargo_version");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/clawd"]

[workspace.package]
version = "0.1.7"
"#,
    )
    .expect("write workspace cargo");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let root_cargo = root.path.join("Cargo.toml");
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_field",
            "path": root_cargo.display().to_string(),
            "field_path": "package.version"
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read Cargo.toml version and answer as `version=<value>` only.",
        Some(root_cargo.to_string_lossy().as_ref()),
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_cargo.to_string_lossy().as_ref())
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("workspace.package.version")
    );
}

#[test]
fn active_clarify_scalar_field_followup_rewrites_text_read_to_read_field() {
    let root = TempDirGuard::new("active_clarify_scalar_field_followup");
    let package = root.path.join("package.json");
    fs::write(&package, r#"{"name":"rustclaw","version":"0.1.7"}"#).expect("write package");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.resolved_intent =
        "Continue the previous request that was waiting for clarification: 读一下那个文件里的名字字段，只输出值\n[RESOLVED_INTENT]\n读取指定文件中的名字字段（name），仅输出该字段的值\nUser now provides the missing target or content: package.json"
            .to_string();
    route.route_reason =
        "active_clarify_locator_reply_fast_path; preserve_active_clarify_output_contract"
            .to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.output_contract.requires_content_evidence = true;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": package.display().to_string(),
            "mode": "head",
            "n": 120
        }),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(1),
        "package.json",
        Some("package.json"),
        None,
        Some(package.to_string_lossy().as_ref()),
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(package.to_string_lossy().as_ref())
    );
    assert_eq!(args.get("field_path").and_then(Value::as_str), Some("name"));
}

#[test]
fn active_clarify_scalar_candidate_respond_rewrites_to_read_field_evidence() {
    let root = TempDirGuard::new("active_clarify_scalar_candidate_respond");
    let package = root.path.join("package.json");
    fs::write(&package, r#"{"name":"rustclaw","private":true}"#).expect("write package");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.route_reason =
        "active_clarify_locator_reply_fast_path; active_clarify_fast_path_scalar_field_value_contract_repair"
            .to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.output_contract.requires_content_evidence = true;
    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let actions = vec![AgentAction::Respond {
        content: "rustclaw".to_string(),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "package.json",
        Some("package.json"),
        None,
        Some(package.to_string_lossy().as_ref()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(package.to_string_lossy().as_ref())
    );
    assert_eq!(args.get("field_path").and_then(Value::as_str), Some("name"));
}

#[test]
fn active_clarify_scalar_candidate_respond_keeps_ambiguous_value() {
    let root = TempDirGuard::new("active_clarify_scalar_candidate_ambiguous");
    let package = root.path.join("package.json");
    fs::write(&package, r#"{"name":"rustclaw","alias":"rustclaw"}"#).expect("write package");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.route_reason =
        "active_clarify_locator_reply_fast_path; active_clarify_fast_path_scalar_field_value_contract_repair"
            .to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.output_contract.requires_content_evidence = true;
    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let actions = vec![AgentAction::Respond {
        content: "rustclaw".to_string(),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "package.json",
        Some("package.json"),
        None,
        Some(package.to_string_lossy().as_ref()),
        actions,
    );

    assert!(matches!(
        normalized.as_slice(),
        [AgentAction::Respond { content }] if content == "rustclaw"
    ));
}

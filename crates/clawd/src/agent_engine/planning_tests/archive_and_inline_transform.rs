use super::*;
use std::collections::BTreeSet;

#[test]
fn archive_read_capability_ref_allows_planner_supplied_member_args() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.resolved_intent = "capability_ref=archive.read".to_string();
    route.route_reason = "capability_ref=archive.read".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = archive.to_string();

    let policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({
            "action": "read",
            "archive": archive,
            "member": "notes.txt",
        }),
    )
    .expect("archive.read capability ref should expose archive_basic.read");
    assert!(policy.is_allowed(), "{policy:?}");
    assert!(policy.action_matches_preferred(), "{policy:?}");
}

#[test]
fn archive_read_capability_ref_uses_policy_not_archive_read_semantic_kind() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.resolved_intent = "capability_ref=archive.read".to_string();
    route.route_reason = "capability_ref=archive.read".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = "test_bundle.zip | notes.txt".to_string();

    let policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({
            "action": "read",
            "archive": archive,
            "member": "notes.txt",
        }),
    )
    .expect("archive.read capability ref should work without ArchiveRead semantic kind");
    assert!(policy.is_allowed(), "{policy:?}");
    assert!(policy.action_matches_preferred(), "{policy:?}");
}

#[test]
fn archive_read_semantic_kind_without_capability_ref_does_not_expose_action_refs() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = format!("{archive} | notes.txt");

    assert_eq!(
        crate::evidence_policy::capability_ref_action_refs_for_route(&route, false).len(),
        0,
        "ArchiveRead output marker alone must not choose archive.read before the planner"
    );
}

#[test]
fn archive_read_structural_member_target_waits_for_planner_capability_ref() {
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

    assert!(
        crate::evidence_policy::capability_ref_action_refs_for_route(&route, false).is_empty(),
        "structural archive/member text without a machine capability_ref must be left to the planner"
    );
}

#[test]
fn archive_read_contract_rejects_unsafe_member_locator() {
    assert!(!super::super::directory_unique_entry::archive_member_path_is_safe("../secret.txt"));
    assert!(!super::super::directory_unique_entry::archive_member_path_is_safe("/tmp/secret.txt"));
    assert!(super::super::directory_unique_entry::archive_member_path_is_safe("notes.txt"));
}

#[test]
fn archive_database_aggregate_capability_refs_allow_structured_observation_actions() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let db_path = "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.resolved_intent =
        "capability_ref=archive.list capability_ref=archive.read capability_ref=database.list_tables"
            .to_string();
    route.route_reason =
        "capability_ref=archive.list capability_ref=archive.read capability_ref=database.list_tables"
            .to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = format!("{archive} | {db_path}");

    let list_policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({"action": "list", "archive": archive}),
    )
    .expect("archive.list capability ref should expose archive_basic.list");
    assert!(list_policy.is_allowed(), "{list_policy:?}");
    assert!(list_policy.action_matches_preferred(), "{list_policy:?}");

    let read_policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({"action": "read", "archive": archive, "member": "notes.txt"}),
    )
    .expect("archive.read capability ref should expose archive_basic.read");
    assert!(read_policy.is_allowed(), "{read_policy:?}");
    assert!(read_policy.action_matches_preferred(), "{read_policy:?}");

    let db_policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "db_basic",
        &json!({"action": "list_tables", "db_path": db_path}),
    )
    .expect("database.list_tables capability ref should expose db_basic.list_tables");
    assert!(db_policy.is_allowed(), "{db_policy:?}");
    assert!(db_policy.action_matches_preferred(), "{db_policy:?}");
}

#[test]
fn archive_database_aggregate_without_capability_refs_does_not_expose_action_refs() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let db_path = "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite";
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

    assert!(
        crate::evidence_policy::capability_ref_action_refs_for_route(&route, false).is_empty(),
        "content-excerpt fallback route must not choose compound archive/database skills without capability refs"
    );
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

#[tokio::test]
async fn inline_json_transform_reaches_planner_path() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let request = r#"{"action":"transform_data","data":[{"name":"alpha","score":7},{"name":"beta","score":12}],"ops":[{"op":"filter","where":{"field":"score","gte":7}}]}"#;
    let task = ClaimedTask {
        task_id: "inline-transform-plan-round".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({ "text": request }).to_string(),
    };
    let mut route = base_route_result();
    route.resolved_intent = request.to_string();
    route.route_reason = "capability_ref=transform.transform_data".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    let loop_state = LoopState::new(1);
    let policy = super::super::super::support::load_agent_loop_guard_policy(&state);

    let err = super::super::plan_round_actions(
        &state,
        &task,
        request,
        request,
        &policy,
        &loop_state,
        None,
        None,
        Some(&route),
        None,
    )
    .await
    .expect_err("inline transform should reach planner instead of pre-LLM transform plan");
    assert!(
        err.contains("required prompt missing"),
        "expected missing planner prompt after deterministic shortcut removal, got: {err}"
    );
    assert!(
        !err.contains("plan_deterministic_inline_json_transform"),
        "old inline transform deterministic fallback leaked into planner error: {err}"
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
fn lightweight_tool_spec_includes_route_evidence_policy_context() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;

    let spec = build_lightweight_tool_spec(Some(&route), None);

    assert!(spec.contains("evidence_policy_context"));
    assert!(spec.contains("boundary_contract_hint"));
    assert!(!spec.contains("route_gate_kind="));
    assert!(spec.lines().any(|line| {
        line.split_whitespace()
            .any(|part| part == "contract_marker=file_paths")
    }));
    assert!(!spec.lines().any(|line| {
        line.split_whitespace()
            .any(|part| part.starts_with("semantic_kind="))
    }));
    assert!(!spec.contains("ask_mode="));
    assert!(!spec.contains("derived_route_label="));
    assert!(!spec.contains("intent_kind="));
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
        crate::evidence_policy::compact_prompt_line_for_route(&route).expect("contract line");
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
    assert!(rendered.contains("boundary_contract_hint"));
    assert!(!rendered.contains("route_gate_kind="));
    assert!(rendered.contains("response_shape=scalar"));
    assert!(rendered.lines().any(|line| {
        line.split_whitespace()
            .any(|part| part == "contract_marker=scalar_path_only")
    }));
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

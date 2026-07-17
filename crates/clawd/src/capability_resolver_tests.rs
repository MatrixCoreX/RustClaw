use serde_json::json;

use super::*;

fn state_with_workspace_registry() -> crate::AppState {
    state_with_workspace_registry_excluding(&[])
}

fn state_with_workspace_registry_excluding(disabled: &[&str]) -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load workspace skills registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
        .filter(|skill| !disabled.iter().any(|disabled| skill.as_str() == *disabled))
        .collect::<std::collections::HashSet<_>>();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = std::sync::Arc::new(crate::SkillViewsSnapshot {
        registry: Some(std::sync::Arc::new(registry)),
        skills_list: std::sync::Arc::new(enabled),
    });
    state
}

fn state_with_registry_toml(toml: &str) -> crate::AppState {
    let path = std::env::temp_dir().join(format!(
        "rustclaw-capability-resolver-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos()
    ));
    std::fs::write(&path, toml).expect("write registry fixture");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&path)
        .expect("load registry fixture");
    let _ = std::fs::remove_file(path);
    let enabled = registry
        .enabled_names()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let state = crate::AppState::test_default_with_fixture_provider();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = std::sync::Arc::new(crate::SkillViewsSnapshot {
        registry: Some(std::sync::Arc::new(registry)),
        skills_list: std::sync::Arc::new(enabled),
    });
    state
}

#[test]
fn resolver_candidate_rank_prefers_dedicated_low_risk_tool_before_run_cmd() {
    let mut candidates = vec![
        ResolverCandidate {
            skill: "run_cmd".to_string(),
            action: None,
            planner_kind: PlannerCapabilityKind::Tool,
            preferred: true,
            risk_level: SkillRiskLevel::High,
            required_args: Vec::new(),
            optional_args: Vec::new(),
            input_schema: None,
        },
        ResolverCandidate {
            skill: "fs_basic".to_string(),
            action: Some("list_dir".to_string()),
            planner_kind: PlannerCapabilityKind::Tool,
            preferred: true,
            risk_level: SkillRiskLevel::Low,
            required_args: Vec::new(),
            optional_args: Vec::new(),
            input_schema: None,
        },
    ];
    candidates.sort_by_key(resolver_candidate_rank);
    assert_eq!(candidates[0].skill, "fs_basic");
}

#[test]
fn optional_enum_arg_outside_registry_schema_is_dropped_before_skill_call() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "extension.assess_gap",
        json!({
            "request": "Add a reusable local CSV statistics capability",
            "mode_hint": "read_only_csv_stats"
        }),
    );

    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(
        record.resolved_ref.as_deref(),
        Some("skill:extension_manager")
    );
    let Some(AgentAction::CallSkill { skill, args }) = action else {
        panic!("expected extension_manager skill action, got {action:?}");
    };
    assert_eq!(skill, "extension_manager");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("assess_gap")
    );
    assert_eq!(
        args.get("request").and_then(Value::as_str),
        Some("Add a reusable local CSV statistics capability")
    );
    assert!(
        args.get("mode_hint").is_none(),
        "invalid optional enum value should be removed so the skill can use its default"
    );
}

#[test]
fn valid_optional_enum_arg_is_preserved_before_skill_call() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "extension.assess_gap",
        json!({
            "request": "Add a reusable local CSV statistics capability",
            "mode_hint": "permanent_extension"
        }),
    );

    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    let Some(AgentAction::CallSkill { args, .. }) = action else {
        panic!("expected extension_manager skill action, got {action:?}");
    };
    assert_eq!(
        args.get("mode_hint").and_then(Value::as_str),
        Some("permanent_extension")
    );
}

#[test]
fn capability_resolution_record_covers_resolved_mapping() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "filesystem.list_entries",
        json!({"path": "."}),
    );
    let action = action.expect("registry filesystem capability should resolve");
    match action {
        AgentAction::CallTool { tool, .. } => assert_eq!(tool, "fs_basic"),
        AgentAction::CallSkill { skill, .. } => assert_eq!(skill, "fs_basic"),
        other => panic!("unexpected resolved action: {other:?}"),
    }
    assert_eq!(record.owner_layer, "capability_resolver");
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.outcome, "resolved");
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "filesystem.list_entries");
    assert!(matches!(
        record.resolved_ref.as_deref(),
        Some("tool:fs_basic") | Some("skill:fs_basic")
    ));
    assert!(record.planner_kind.is_some());
}

#[test]
fn filesystem_write_text_capability_normalizes_write_mode_alias() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "filesystem.write_text",
        json!({
            "path": "notes/memo.txt",
            "content": "hello\n",
            "write_mode": "overwrite"
        }),
    );
    let action = action.expect("filesystem.write_text should resolve");
    let AgentAction::CallTool { tool, args } = action else {
        panic!("expected fs_basic tool action, got {action:?}");
    };
    assert_eq!(tool, "fs_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("write_text")
    );
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("notes/memo.txt")
    );
    assert_eq!(args.get("content").and_then(Value::as_str), Some("hello\n"));
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("overwrite"));
    assert!(args.get("write_mode").is_none());
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
}

#[test]
fn workspace_registry_requires_explicit_bare_capability_action() {
    let state = state_with_workspace_registry();
    let (action, record) =
        resolve_capability_action_with_record_for_state(&state, "config_basic", json!({}));

    assert!(action.is_none());
    assert_eq!(record.reason_code, "capability_unavailable");
    assert_eq!(record.source, "none");
    assert_eq!(record.capability_ref, "config_basic");
}

#[test]
fn registry_resolves_crypto_positions_capability() {
    let state = state_with_workspace_registry();
    let (action, record) =
        resolve_capability_action_with_record_for_state(&state, "crypto.positions", json!({}));
    let action = action.expect("registry crypto.positions capability should resolve");
    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "crypto");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("positions")
            );
        }
        other => panic!("unexpected resolved action: {other:?}"),
    }
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "crypto.positions");
}

#[test]
fn registry_resolves_bare_skill_capability_by_machine_action() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "task_control",
        json!({"action": "list", "limit": 5}),
    );
    let action = action.expect("bare task_control with machine action should resolve");
    match action {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "task_control");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list"));
            assert_eq!(args.get("limit").and_then(Value::as_i64), Some(5));
        }
        other => panic!("unexpected resolved action: {other:?}"),
    }
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "task_control");
    assert_eq!(record.resolved_ref.as_deref(), Some("tool:task_control"));
}

#[test]
fn config_read_fields_capability_normalizes_machine_field_aliases() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "config_basic.read_fields",
        json!({
            "config_path": "configs/agent_guard.toml",
            "fields": [
                "agent.hooks.handlers",
                "agent.subagents.allowed_roles",
                "agent.subagents.max_parallel_readonly",
                "agent.loop_guard.max_rounds"
            ]
        }),
    );
    let action = action.expect("config_basic.read_fields capability should resolve");
    let AgentAction::CallTool { tool, args } = action else {
        panic!("expected config_basic tool action, got {action:?}");
    };
    assert_eq!(tool, "config_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("read_fields")
    );
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/agent_guard.toml")
    );
    assert!(args.get("fields").is_none());
    assert!(args.get("config_path").is_none());
    let field_paths = args
        .get("field_paths")
        .and_then(Value::as_array)
        .expect("field_paths array");
    assert_eq!(field_paths.len(), 4);
    assert!(field_paths
        .iter()
        .any(|value| value.as_str() == Some("agent.hooks.handlers")));
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "config_basic.read_fields");
}

#[test]
fn registry_resolves_fully_qualified_skill_action_capability() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "browser_web.open_extract",
        json!({"url": "https://example.com"}),
    );
    let action = action.expect("registry skill.action capability should resolve");
    match action {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "browser_web");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("open_extract")
            );
            assert_eq!(
                args.get("url").and_then(Value::as_str),
                Some("https://example.com")
            );
        }
        other => panic!("unexpected resolved action: {other:?}"),
    }
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "browser_web.open_extract");
    assert_eq!(record.resolved_ref.as_deref(), Some("tool:browser_web"));
}

#[test]
fn registry_resolves_doc_parse_bare_capability() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "doc_parse",
        json!({"path": "/tmp/example.md"}),
    );
    let action = action.expect("registry doc_parse capability should resolve");
    match action {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "doc_parse");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("parse_doc")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some("/tmp/example.md")
            );
        }
        other => panic!("unexpected resolved action: {other:?}"),
    }
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "doc_parse");
}

#[test]
fn registry_metadata_adds_ordinary_skill_without_static_branch() {
    let state = state_with_registry_toml(
        r#"
[[skills]]
name = "custom_translate"
enabled = true
kind = "runner"
planner_kind = "skill"
aliases = ["translate"]
capabilities = ["llm"]
planner_capabilities = [
  { name = "text.translate", action = "translate", effect = "external", required = ["text"], optional = ["target_locale"], risk_level = "medium", preferred = true }
]
"#,
    );

    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "text.translate",
        json!({"text": "hello", "target_locale": "fr"}),
    );
    let action = action.expect("registry-only ordinary skill should resolve");
    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "custom_translate");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("translate")
            );
            assert_eq!(
                args.get("target_locale").and_then(Value::as_str),
                Some("fr")
            );
        }
        other => panic!("unexpected resolved action: {other:?}"),
    }
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "text.translate");
    assert_eq!(
        record.resolved_ref.as_deref(),
        Some("skill:custom_translate")
    );
    assert_eq!(record.planner_kind, Some("skill"));
}

#[test]
fn registry_resolves_terminal_layer_representative_capabilities() {
    let state = state_with_workspace_registry();
    let cases = [
        (
            "filesystem.list_entries",
            json!({"path": "."}),
            "tool:fs_basic",
        ),
        (
            "system.run_command",
            json!({"command": "pwd"}),
            "skill:run_cmd",
        ),
        (
            "system.shell_run",
            json!({"shell_command": "pwd"}),
            "skill:run_cmd",
        ),
        ("git.status", json!({}), "tool:git_basic"),
        (
            "web.search_results",
            json!({"query": "rustclaw"}),
            "tool:web_search_extract",
        ),
        (
            "config.read_field",
            json!({"path": "configs/config.toml", "field_path": "skills.registry_path"}),
            "tool:config_basic",
        ),
        ("process.ps", json!({}), "tool:process_basic"),
        (
            "service.status",
            json!({"target": "clawd"}),
            "tool:service_control",
        ),
        (
            "task_control.list",
            json!({"limit": 5}),
            "tool:task_control",
        ),
        (
            "image_vision.describe",
            json!({"images": ["fixtures/image.png"]}),
            "skill:image_vision",
        ),
        (
            "audio.transcribe",
            json!({"audio_path": "fixtures/audio.wav"}),
            "skill:audio_transcribe",
        ),
        (
            "video.generate",
            json!({"prompt": "test"}),
            "skill:video_generate",
        ),
        (
            "music.generate",
            json!({"prompt": "test"}),
            "skill:music_generate",
        ),
    ];

    for (capability, args, expected_ref) in cases {
        let (action, record) =
            resolve_capability_action_with_record_for_state(&state, capability, args);
        assert!(action.is_some(), "{capability} should resolve");
        assert_eq!(
            record.reason_code, "capability_resolver_registry_mapping_resolved",
            "{capability} should resolve through registry"
        );
        assert_eq!(record.source, "registry");
        assert_eq!(record.capability_ref, capability);
        assert_eq!(record.resolved_ref.as_deref(), Some(expected_ref));
    }
}

#[test]
fn registry_resolution_preserves_media_poll_action_arg() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "image.poll",
        json!({
            "task_id": "image-task-001",
            "job_id": "image-job-001",
            "output_path": "document/media_dry_run/image_status_card.png",
            "dry_run": true,
            "mock_status": "succeeded",
        }),
    );

    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.resolved_ref.as_deref(), Some("skill:image_generate"));
    let Some(AgentAction::CallSkill { skill, args }) = action else {
        panic!("expected image_generate skill action, got {action:?}");
    };
    assert_eq!(skill, "image_generate");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("poll"));
    assert_eq!(
        args.get("task_id").and_then(Value::as_str),
        Some("image-task-001")
    );
    assert_eq!(args.get("dry_run").and_then(Value::as_bool), Some(true));
}

#[test]
fn command_like_runtime_status_rewrites_to_run_cmd_capability() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "system.runtime_status",
        json!({
            "kind": "run_cmd",
            "shell_command": "python3 test_calc_core.py",
            "cwd": "/tmp/project"
        }),
    );

    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.capability_ref, "system.run_command");
    assert_eq!(record.resolved_ref.as_deref(), Some("skill:run_cmd"));
    let Some(AgentAction::CallSkill { skill, args }) = action else {
        panic!("expected run_cmd skill action, got {action:?}");
    };
    assert_eq!(skill, "run_cmd");
    assert_eq!(
        args.get("command").and_then(Value::as_str),
        Some("python3 test_calc_core.py")
    );
    assert!(args.get("kind").is_none());
    assert!(args.get("shell_command").is_none());
}

#[test]
fn task_queue_runtime_status_rewrites_to_task_control_list() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "system.runtime_status",
        json!({
            "kind": "task_queue_status",
            "limit": 5
        }),
    );

    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.capability_ref, "task_control.list");
    assert_eq!(record.resolved_ref.as_deref(), Some("tool:task_control"));
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected task_control tool action, got {action:?}");
    };
    assert_eq!(tool, "task_control");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("list"));
    assert_eq!(args.get("limit").and_then(Value::as_i64), Some(5));
    assert!(args.get("kind").is_none());
}

#[test]
fn registry_resolves_legacy_machine_capability_aliases_without_static_fallback() {
    let state = state_with_workspace_registry();
    let cases = [
        ("system.run_cmd", json!({"command": "pwd"}), "skill:run_cmd"),
        (
            "system.shell_run",
            json!({"shell_command": "pwd"}),
            "skill:run_cmd",
        ),
        ("run_cmd", json!({"command": "pwd"}), "skill:run_cmd"),
        (
            "filesystem.stat_path",
            json!({"path": "."}),
            "tool:fs_basic",
        ),
        ("filesystem.list_dir", json!({"path": "."}), "tool:fs_basic"),
        (
            "filesystem.read_file",
            json!({"path": "README.md"}),
            "tool:fs_basic",
        ),
        (
            "fs_basic.read_text",
            json!({"path": "README.md"}),
            "tool:fs_basic",
        ),
        (
            "fs_basic.read_range",
            json!({"path": "README.md"}),
            "tool:fs_basic",
        ),
        (
            "fs_basic.read_file",
            json!({"path": "README.md"}),
            "tool:fs_basic",
        ),
        (
            "filesystem.find_files",
            json!({"root": ".", "pattern": "*.rs"}),
            "tool:fs_basic",
        ),
        (
            "filesystem.search_text",
            json!({"root": ".", "query": "TaskJournal"}),
            "tool:fs_basic",
        ),
        (
            "filesystem.create_dir",
            json!({"path": "/tmp/rustclaw-test"}),
            "tool:fs_basic",
        ),
        (
            "filesystem.delete_path",
            json!({"path": "/tmp/rustclaw-test"}),
            "tool:fs_basic",
        ),
        (
            "config.plan_config_change",
            json!({"field_path": "llm.default_vendor", "value": "minimax"}),
            "tool:config_edit",
        ),
        (
            "config.guard_config",
            json!({"path": "configs/config.toml"}),
            "tool:config_edit",
        ),
        (
            "system_basic.extract_field",
            json!({"path": "configs/config.toml", "field_path": "server.listen"}),
            "tool:system_basic",
        ),
        (
            "system_basic.read_text_range",
            json!({"path": "README.md"}),
            "tool:system_basic",
        ),
        (
            "transform",
            json!({"records": [{"score": 1}], "ops": [{"op": "sort", "by": "score"}]}),
            "tool:transform",
        ),
        (
            "data.transform_records",
            json!({"records": [{"score": 1}], "ops": [{"op": "sort", "by": "score"}]}),
            "tool:transform",
        ),
    ];

    for (capability, args, expected_ref) in cases {
        let (action, record) =
            resolve_capability_action_with_record_for_state(&state, capability, args);
        assert!(action.is_some(), "{capability} should resolve");
        assert_eq!(
            record.reason_code, "capability_resolver_registry_mapping_resolved",
            "{capability} should resolve through registry without static fallback"
        );
        assert_eq!(record.source, "registry");
        assert_eq!(record.capability_ref, capability);
        assert_eq!(record.resolved_ref.as_deref(), Some(expected_ref));
    }
}

#[test]
fn capability_resolution_record_covers_unresolved_mapping() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let (action, record) =
        resolve_capability_action_with_record_for_state(&state, "unknown.example", json!({}));
    assert!(action.is_none());
    assert_eq!(record.owner_layer, "capability_resolver");
    assert_eq!(record.reason_code, "capability_unavailable");
    assert_eq!(record.outcome, "unresolved");
    assert_eq!(record.source, "none");
    assert_eq!(record.capability_ref, "unknown.example");
    assert!(record.resolved_ref.is_none());
    assert!(record.planner_kind.is_none());
}

#[test]
fn disabled_registry_capability_returns_machine_disabled_record_without_static_fallback() {
    let state = state_with_workspace_registry_excluding(&["fs_basic"]);
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "filesystem.list_entries",
        json!({"path": "."}),
    );

    assert!(
        action.is_none(),
        "disabled registry capability must not fall back to static compat"
    );
    assert_eq!(record.owner_layer, "capability_resolver");
    assert_eq!(record.reason_code, "capability_disabled");
    assert_eq!(record.outcome, "blocked");
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "filesystem.list_entries");
    assert_eq!(record.resolved_ref.as_deref(), Some("tool:fs_basic"));
    assert_eq!(record.planner_kind, Some("tool"));
}

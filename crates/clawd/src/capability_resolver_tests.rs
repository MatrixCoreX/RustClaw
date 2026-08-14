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
        .all_names()
        .into_iter()
        .filter(|skill| !disabled.iter().any(|disabled| skill.as_str() == *disabled))
        .collect::<std::collections::HashSet<_>>();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = std::sync::Arc::new(crate::SkillViewsSnapshot {
        binding: Default::default(),
        registry: Some(std::sync::Arc::new(registry)),
        skills_list: std::sync::Arc::new(enabled),
    });
    state
}

fn state_with_registry_toml(toml: &str) -> crate::AppState {
    let path = std::env::temp_dir().join(format!(
        "agent-runtime-capability-resolver-{}-{}.toml",
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
        binding: Default::default(),
        registry: Some(std::sync::Arc::new(registry)),
        skills_list: std::sync::Arc::new(enabled),
    });
    state
}

#[test]
fn nni_capabilities_resolve_through_one_read_only_registry_surface() {
    let state = state_with_workspace_registry();
    let cases = [
        ("nni.status", "status", json!({})),
        ("nni.device_status", "device_status", json!({})),
        ("nni.heartbeat_status", "heartbeat_status", json!({})),
        ("nni.heartbeat_enable", "heartbeat_enable", json!({})),
        ("nni.heartbeat_disable", "heartbeat_disable", json!({})),
        ("nni.heartbeat_now", "heartbeat_now", json!({})),
        ("nni.network_stats", "network_stats", json!({"limit": 5})),
        ("nni.my_rewards", "my_rewards", json!({"limit": 5})),
        ("nni.bancor_market", "bancor_market", json!({})),
        ("nni.bancor_account", "bancor_account", json!({"limit": 5})),
        (
            "nni.bancor_market_trades",
            "bancor_market_trades",
            json!({"limit": 10}),
        ),
        (
            "nni.bancor_candles",
            "bancor_candles",
            json!({"interval": "15m", "limit": 12}),
        ),
        (
            "nni.bancor_quote",
            "bancor_quote",
            json!({"side": "buy", "pay_amount": "25"}),
        ),
    ];

    for (capability, expected_action, args) in cases {
        let (action, record) =
            resolve_capability_action_with_record_for_state(&state, capability, args);
        let Some(AgentAction::CallSkill { skill, args }) = action else {
            panic!("expected NNI skill action for {capability}");
        };
        assert_eq!(skill, "nni");
        assert_eq!(args["action"], expected_action);
        assert_eq!(
            record.reason_code,
            "capability_resolver_registry_mapping_resolved"
        );
    }

    let registry = state.get_skills_registry().expect("skills registry");
    let capabilities = registry.planner_capabilities("nni");
    assert_eq!(capabilities.len(), 13);
    assert!(capabilities.iter().all(|mapping| !matches!(
        mapping.name.as_str(),
        "nni.buy" | "nni.sell" | "nni.trade" | "nni.bancor_trade"
    )));
    assert!(!crate::schema_contract::executable_enum_violations(
        &state,
        "nni",
        &json!({"action": "trade", "pay_amount": "25"}),
    )
    .is_empty());
}

#[test]
fn media_download_routes_raw_share_text_to_autonomous_download() {
    let state = state_with_workspace_registry();
    assert_eq!(
        state
            .get_skills_registry()
            .and_then(|registry| registry.get("media_download").cloned())
            .map(|entry| entry.enabled),
        Some(false),
        "the resolver fixture must prove runtime enablement overrides the on-demand base default"
    );
    let share = "复制这条消息，打开快手看看 https://v.kuaishou.com/AbCdEf 更多内容";
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "media_download.download",
        json!({"share": share}),
    );
    let Some(AgentAction::CallSkill { skill, args }) = action else {
        panic!("expected media download skill action");
    };

    assert_eq!(skill, "media_download");
    assert_eq!(args["action"], "download");
    assert_eq!(args["share"], share);
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );

    let manifest = state
        .skill_manifest("media_download")
        .expect("media download manifest");
    assert_eq!(manifest.auto_invocable, Some(true));
    assert_eq!(manifest.requires_confirmation, Some(true));
    assert!(!state.skill_invocation_requires_confirmation_policy(
        "media_download",
        Some(&json!({"action": "download", "share": share})),
    ));
    assert!(state.skill_invocation_requires_confirmation_policy(
        "media_download",
        Some(&json!({"action": "transcribe", "input_path": "input.mp4"})),
    ));

    let download = state
        .get_skills_registry()
        .expect("skills registry")
        .planner_capabilities("media_download")
        .iter()
        .find(|mapping| mapping.name == "media_download.download")
        .cloned()
        .expect("download capability");
    assert_eq!(
        download.effect.map(|effect| effect.as_token()),
        Some("mutate")
    );
    assert_eq!(download.risk_level, Some(SkillRiskLevel::Medium));
    assert_eq!(download.external_publish, Some(false));
    assert!(download
        .optional
        .iter()
        .any(|name| name == "deliver_to_user"));
}

#[test]
fn registry_resolution_observes_required_companion_capabilities() {
    let state = state_with_workspace_registry();
    let (_, record) = resolve_capability_action_with_record_for_state(
        &state,
        "web.search_results",
        json!({"query": "current finance news"}),
    );
    assert_eq!(
        record.required_companions,
        vec!["rss.list_categories", "rss.latest_news"]
    );
    assert_eq!(
        record.dispatch_observation(1, 1, 1)["required_companions"],
        json!(["rss.list_categories", "rss.latest_news"])
    );
}

#[test]
fn memory_forget_is_action_scoped_non_auto_invocable_and_confirmation_protected() {
    let state = state_with_workspace_registry();
    let registry = state.get_skills_registry().expect("skills registry");
    assert_eq!(
        registry.resolved_auto_invocable("memory_store", Some("forget")),
        Some(false)
    );
    assert_eq!(
        registry.resolved_requires_confirmation("memory_store", Some("forget")),
        Some(true)
    );
    assert!(state.skill_invocation_requires_confirmation_policy(
        "memory_store",
        Some(&json!({
            "action": "forget",
            "scope": "current_principal",
            "memory_id": "memory_fixture_opaque",
            "expected_revision": 1
        })),
    ));
    assert!(!state.skill_invocation_requires_confirmation_policy(
        "memory_store",
        Some(&json!({"action": "search", "query": "fixture"})),
    ));
}

#[test]
fn map_optional_coordinates_remain_nullable_in_registry_schema() {
    let state = state_with_workspace_registry();
    let registry = state.get_skills_registry().expect("skills registry");
    let manifest = registry.manifest("map_merchant").expect("map manifest");
    let schema = manifest.input_schema.expect("map input schema");

    for field in ["latitude", "longitude"] {
        let property = &schema["properties"][field];
        let variants = property["anyOf"]
            .as_array()
            .unwrap_or_else(|| panic!("{field} should use a nullable union"));
        assert!(
            variants
                .iter()
                .any(|variant| variant["type"].as_str() == Some("number")),
            "{field} should accept numbers"
        );
        assert!(
            variants
                .iter()
                .any(|variant| variant["type"].as_str() == Some("null")),
            "{field} should accept null"
        );
        let description = property["description"]
            .as_str()
            .unwrap_or_else(|| panic!("{field} should explain absence semantics"));
        assert!(description.contains("never invent a placeholder coordinate"));
    }
}

#[test]
fn photo_grouping_schema_uses_machine_array_tokens() {
    let state = state_with_workspace_registry();
    let registry = state.get_skills_registry().expect("skills registry");
    let manifest = registry
        .manifest("photo_organize")
        .expect("photo organize manifest");
    let schema = manifest.input_schema.expect("photo input schema");
    let group_by = &schema["properties"]["group_by"];

    assert_eq!(group_by["type"], "array");
    assert_eq!(group_by["uniqueItems"], true);
    assert_eq!(
        group_by["items"]["enum"],
        json!([
            "brand",
            "model",
            "lens",
            "focal_length",
            "year",
            "year_month",
            "date"
        ])
    );
}

#[test]
fn resolver_candidate_rank_prefers_dedicated_low_risk_tool_before_run_cmd() {
    let mut candidates = vec![
        ResolverCandidate {
            skill: "run_cmd".to_string(),
            capability: "system.run_command".to_string(),
            action: None,
            planner_kind: PlannerCapabilityKind::Tool,
            preferred: true,
            risk_level: SkillRiskLevel::High,
            required_companions: Vec::new(),
        },
        ResolverCandidate {
            skill: "fs_basic".to_string(),
            capability: "filesystem.list_entries".to_string(),
            action: Some("list_dir".to_string()),
            planner_kind: PlannerCapabilityKind::Tool,
            preferred: true,
            risk_level: SkillRiskLevel::Low,
            required_companions: Vec::new(),
        },
    ];
    candidates.sort_by_key(resolver_candidate_rank);
    assert_eq!(candidates[0].skill, "fs_basic");
}

#[test]
fn schedule_preview_resolves_through_registry_contract() {
    let state = state_with_workspace_registry();
    let (_, _record) = resolve_capability_action_with_record_for_state(
        &state,
        "schedule.preview",
        json!({"text": "language-neutral schedule input"}),
    );
}

#[test]
fn package_detection_resolves_through_registry_contract() {
    let state = state_with_workspace_registry();
    let (action, _record) = resolve_capability_action_with_record_for_state(
        &state,
        "package.detect_manager",
        json!({}),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected package manager tool action");
    };
    assert_eq!(tool, "package_manager");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("detect"));
}

#[test]
fn office_capability_preserves_registry_action_namespace() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "word.create",
        json!({
            "output_path": "tmp/report.docx",
            "operations": [{"op": "add_paragraph", "text": "fixture"}],
        }),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected office workspace tool action");
    };

    assert_eq!(
        record.canonical_capability_ref.as_deref(),
        Some("word.create")
    );
    assert_eq!(tool, "office_workspace");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("word.create")
    );
    assert!(crate::schema_contract::executable_enum_violations(&state, &tool, &args).is_empty());
}

#[test]
fn config_key_listing_resolves_through_registry_contract() {
    let state = state_with_workspace_registry();
    let (action, _record) = resolve_capability_action_with_record_for_state(
        &state,
        "config.list_keys",
        json!({"path": "configs/config.toml"}),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected config tool action");
    };
    assert_eq!(tool, "config_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("list_keys")
    );
}

#[test]
fn compatibility_capability_token_resolves_with_canonical_audit_reference() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "filesystem.read_file",
        json!({"path": "README.md"}),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected filesystem tool action");
    };

    assert_eq!(tool, "fs_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("read_text_range")
    );
    assert_eq!(
        record.canonical_capability_ref.as_deref(),
        Some("filesystem.read_text_range")
    );
}

#[test]
fn config_read_resolves_through_registry_contract() {
    let state = state_with_workspace_registry();
    let (action, _record) = resolve_capability_action_with_record_for_state(
        &state,
        "config.read_field",
        json!({
            "path": "configs/config.toml",
            "field_path": "llm.selected_vendor",
        }),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected config tool action");
    };
    assert_eq!(tool, "config_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("read_field")
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("llm.selected_vendor")
    );
}

#[test]
fn config_risk_resolves_without_domain_output_contract() {
    let state = state_with_workspace_registry();
    let (action, _record) = resolve_capability_action_with_record_for_state(
        &state,
        "config.risk",
        json!({"path": "configs/config.toml"}),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected config risk tool action");
    };
    assert_eq!(tool, "config_edit");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("guard_config")
    );
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
}

#[test]
fn config_mutation_resolves_without_domain_output_contract() {
    let state = state_with_workspace_registry();
    let (action, _record) = resolve_capability_action_with_record_for_state(
        &state,
        "config.apply_change",
        json!({
            "path": "configs/config.toml",
            "field_path": "skills.skill_switches.example",
            "value": true,
        }),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected config mutation tool action");
    };
    assert_eq!(tool, "config_edit");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("apply_config_change")
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("skills.skill_switches.example")
    );
}

#[test]
fn config_validation_resolves_without_domain_output_contract() {
    let state = state_with_workspace_registry();
    let (action, _record) = resolve_capability_action_with_record_for_state(
        &state,
        "config.validate",
        json!({"path": "configs/config.toml"}),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected config validation tool action");
    };
    assert_eq!(tool, "config_basic");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("validate"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
}

#[test]
fn config_guard_resolves_to_dedicated_machine_action() {
    let state = state_with_workspace_registry();
    let (action, _record) = resolve_capability_action_with_record_for_state(
        &state,
        "config.guard_config",
        json!({"path": "configs/config.toml"}),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected config guard tool action");
    };
    assert_eq!(tool, "config_edit");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("guard_config")
    );
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
}

#[test]
fn filesystem_grep_resolver_preserves_planner_query_without_semantic_contract() {
    let state = state_with_workspace_registry();
    let (action, _record) = resolve_capability_action_with_record_for_state(
        &state,
        "filesystem.grep_text",
        json!({
            "root": "docs",
            "query": "release",
            "multiline": true,
            "context_before": 2,
            "context_after": 1,
            "max_file_bytes": 1048576,
            "max_scan_bytes": 8388608,
            "max_results": 8,
        }),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected filesystem tool action");
    };
    assert_eq!(tool, "fs_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("grep_text")
    );
    assert_eq!(args.get("root").and_then(Value::as_str), Some("docs"));
    assert_eq!(args.get("query").and_then(Value::as_str), Some("release"));
    assert_eq!(args.get("multiline").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("context_before").and_then(Value::as_u64), Some(2));
    assert_eq!(args.get("context_after").and_then(Value::as_u64), Some(1));
    assert_eq!(
        args.get("max_file_bytes").and_then(Value::as_u64),
        Some(1_048_576)
    );
    assert_eq!(
        args.get("max_scan_bytes").and_then(Value::as_u64),
        Some(8_388_608)
    );
    assert_eq!(args.get("max_results").and_then(Value::as_u64), Some(8));
}

#[test]
fn filesystem_find_images_resolves_to_bounded_virtual_action() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "filesystem.find_images",
        json!({
            "root": "assets",
            "exts": ["png", "jpg"],
            "max_results": 20,
            "cursor": 5,
            "max_dirs": 8
        }),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected filesystem tool action");
    };
    assert_eq!(tool, "fs_basic");
    assert_eq!(args["action"], "find_images");
    assert_eq!(args["root"], "assets");
    assert_eq!(args["exts"], json!(["png", "jpg"]));
    assert_eq!(args["max_results"], 20);
    assert_eq!(args["cursor"], 5);
    assert_eq!(args["max_dirs"], 8);
    assert_eq!(record.capability_ref, "filesystem.find_images");
}

#[test]
fn filesystem_read_resolver_preserves_planner_range_without_semantic_contract() {
    let state = state_with_workspace_registry();
    let (action, _record) = resolve_capability_action_with_record_for_state(
        &state,
        "filesystem.read_text_range",
        json!({
            "path": "docs/release_checklist.md",
            "mode": "head",
            "n": 20,
        }),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected filesystem tool action");
    };
    assert_eq!(tool, "fs_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("read_text_range")
    );
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("docs/release_checklist.md")
    );
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("head"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(20));
}

#[test]
fn docker_capabilities_resolve_through_registry_contracts() {
    let state = state_with_workspace_registry();
    for (capability, expected_action, args) in [
        ("docker.list_containers", "ps", json!({})),
        ("docker.list_images", "images", json!({})),
        (
            "docker.read_logs",
            "logs",
            json!({"container": "agent-runtime-test", "tail": 20}),
        ),
        (
            "docker.restart_container",
            "restart",
            json!({"container": "agent-runtime-test"}),
        ),
    ] {
        let (action, _record) =
            resolve_capability_action_with_record_for_state(&state, capability, args);
        let Some(AgentAction::CallTool { tool, args }) = action else {
            panic!("expected docker tool action for {capability}");
        };
        assert_eq!(tool, "docker_basic", "{capability}");
        assert_eq!(
            args.get("action").and_then(Value::as_str),
            Some(expected_action),
            "{capability}"
        );
    }
}

#[test]
fn database_capabilities_resolve_through_registry_contracts() {
    let state = state_with_workspace_registry();
    for (capability, expected_action, args) in [
        (
            "database.query",
            "sqlite_query",
            json!({"db_path": "data/app.db", "sql": "SELECT 1"}),
        ),
        (
            "database.list_tables",
            "list_tables",
            json!({"db_path": "data/app.db"}),
        ),
        (
            "database.schema_version",
            "schema_version",
            json!({"db_path": "data/app.db"}),
        ),
        (
            "database.user_version",
            "user_version",
            json!({"db_path": "data/app.db"}),
        ),
    ] {
        let (action, _record) =
            resolve_capability_action_with_record_for_state(&state, capability, args);
        let Some(AgentAction::CallTool { tool, args }) = action else {
            panic!("expected database tool action for {capability}");
        };
        assert_eq!(tool, "db_basic", "{capability}");
        assert_eq!(
            args.get("action").and_then(Value::as_str),
            Some(expected_action),
            "{capability}"
        );
    }
}

#[test]
fn archive_capabilities_resolve_through_registry_contracts() {
    let state = state_with_workspace_registry();
    for (capability, expected_action, args) in [
        ("archive.list", "list", json!({"archive": "tmp/bundle.zip"})),
        (
            "archive.read",
            "read",
            json!({"archive": "tmp/bundle.zip", "member": "notes.txt"}),
        ),
        (
            "archive.pack",
            "pack",
            json!({"source": "reports", "archive": "tmp/reports.zip"}),
        ),
        (
            "archive.unpack",
            "unpack",
            json!({"archive": "tmp/bundle.zip", "dest": "tmp/unpacked"}),
        ),
    ] {
        let (action, _record) =
            resolve_capability_action_with_record_for_state(&state, capability, args);
        let Some(AgentAction::CallTool { tool, args }) = action else {
            panic!("expected archive tool action for {capability}");
        };
        assert_eq!(tool, "archive_basic", "{capability}");
        assert_eq!(
            args.get("action").and_then(Value::as_str),
            Some(expected_action),
            "{capability}"
        );
    }
}

#[test]
fn git_capabilities_resolve_through_registry_contracts() {
    let state = state_with_workspace_registry();
    for (capability, expected_action, args) in [
        ("git.status", "status", json!({})),
        ("git.current_branch", "current_branch", json!({})),
        ("git.log", "log", json!({"limit": 3})),
    ] {
        let (action, _record) =
            resolve_capability_action_with_record_for_state(&state, capability, args);
        let Some(AgentAction::CallTool { tool, args }) = action else {
            panic!("expected git tool action for {capability}");
        };
        assert_eq!(tool, "git_basic", "{capability}");
        assert_eq!(
            args.get("action").and_then(Value::as_str),
            Some(expected_action),
            "{capability}"
        );
    }
}

#[test]
fn weather_capability_resolves_through_registry_contract() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "weather.current",
        json!({"city": "Beijing", "display_location": "北京"}),
    );

    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    let Some(AgentAction::CallSkill { skill, args }) = action else {
        panic!("expected weather skill action, got {action:?}");
    };
    assert_eq!(skill, "weather");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("query"));
    assert_eq!(
        args.get("display_location").and_then(Value::as_str),
        Some("北京")
    );
}

#[test]
fn rss_capability_resolves_through_registry_contract() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "rss.latest_news",
        json!({"category": "general", "limit": 3}),
    );

    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    let Some(AgentAction::CallSkill { skill, args }) = action else {
        panic!("expected rss_fetch skill action, got {action:?}");
    };
    assert_eq!(skill, "rss_fetch");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("latest"));
    assert_eq!(args.get("limit").and_then(Value::as_i64), Some(3));
}

#[test]
fn media_photo_and_publish_preview_resolve_through_registry_contracts() {
    let state = state_with_workspace_registry();
    let cases = [
        (
            "image_vision.describe",
            json!({"image": "https://example.invalid/image.png"}),
            "image_vision",
            "describe",
        ),
        (
            "photo.prepare_source_candidates",
            json!({}),
            "photo_organize",
            "prepare",
        ),
        (
            "x.draft_preview",
            json!({"text": "release notes", "dry_run": true}),
            "x",
            "preview",
        ),
    ];

    for (capability, args, expected_skill, expected_action) in cases {
        let (action, record) =
            resolve_capability_action_with_record_for_state(&state, capability, args);
        assert_eq!(
            record.reason_code,
            "capability_resolver_registry_mapping_resolved"
        );
        let Some(AgentAction::CallSkill { skill, args }) = action else {
            panic!("expected skill action for {capability}, got {action:?}");
        };
        assert_eq!(skill, expected_skill);
        assert_eq!(
            args.get("action").and_then(Value::as_str),
            Some(expected_action)
        );
    }
}

#[test]
fn web_search_capability_resolves_through_registry_contract() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "web.search_results",
        json!({"query": "rust async", "top_k": 3}),
    );

    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    let Some(AgentAction::CallTool { tool: skill, args }) = action else {
        panic!("expected web_search_extract action, got {action:?}");
    };
    assert_eq!(skill, "web_search_extract");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("search_extract")
    );
    assert_eq!(
        args.get("query").and_then(Value::as_str),
        Some("rust async")
    );
}

#[test]
fn planner_output_contract_is_preserved_without_registry_rewrite() {
    let mut output_contract = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        ..Default::default()
    };
    output_contract.selection.structured_field_selector = Some(
        "checkpoint,diff,failed_verification,repair_attempt,passing_verification,rewind_references"
            .to_string(),
    );
    let plan_result = crate::PlanResult {
        goal: "preview repair".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: Some(output_contract),
        steps: vec![crate::plan_step_from_agent_action(
            &AgentAction::CallCapability {
                capability: "coding_workflow.preview_repair".to_string(),
                args: json!({}),
            },
            "step_1".to_string(),
            Vec::new(),
            "preview repair".to_string(),
        )],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: "{}".to_string(),
    };

    let preserved = plan_result
        .output_contract
        .expect("existing planner contract must remain available");
    assert_eq!(preserved.response_shape, crate::OutputResponseShape::Strict);
    assert_eq!(
        preserved.selection.structured_field_selector.as_deref(),
        Some(
            "checkpoint,diff,failed_verification,repair_attempt,passing_verification,rewind_references"
        )
    );
}

#[test]
fn optional_enum_arg_outside_registry_schema_is_preserved_for_verifier_repair() {
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
    assert_eq!(
        args.get("mode_hint").and_then(Value::as_str),
        Some("read_only_csv_stats"),
        "resolver must not silently replace model output with a skill default"
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
    assert_eq!(
        record.canonical_capability_ref.as_deref(),
        Some("filesystem.list_entries")
    );
    assert!(matches!(
        record.resolved_ref.as_deref(),
        Some("tool:fs_basic") | Some("skill:fs_basic")
    ));
    assert!(record.planner_kind.is_some());
}

#[test]
fn canonical_capability_resolution_records_registry_identity() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "coding_workflow.preview_repair",
        json!({}),
    );

    let (executable, args) = match action {
        Some(AgentAction::CallTool { tool, args }) => (tool, args),
        Some(AgentAction::CallSkill { skill, args }) => (skill, args),
        other => panic!("expected task_control executable action, got {other:?}"),
    };
    assert_eq!(executable, "task_control");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("preview_coding_repair")
    );
    assert_eq!(record.capability_ref, "coding_workflow.preview_repair");
    assert_eq!(
        record.canonical_capability_ref.as_deref(),
        Some("coding_workflow.preview_repair")
    );
}

#[test]
fn real_registry_resolves_inline_and_persistent_subagent_capabilities() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();

    let (inline, inline_record) = resolve_capability_action_with_record_for_state(
        &state,
        "agent.subagent",
        json!({
            "role": "review",
            "objective": "inspect_runtime_boundary",
            "context_refs": ["AGENTS.md"],
            "isolation_profile": "danger_full",
            "network_access": true
        }),
    );
    let Some(AgentAction::CallTool { tool, args }) = inline else {
        panic!("inline subagent capability must resolve to an internal tool");
    };
    assert_eq!(tool, "subagent");
    assert_eq!(args["role"], "review");
    assert_eq!(args["action"], "inline_readonly");
    assert!(args.get("isolation_profile").is_none());
    assert!(args.get("network_access").is_none());
    assert_eq!(inline_record.source, "registry");
    assert_eq!(
        inline_record.canonical_capability_ref.as_deref(),
        Some("agent.subagent")
    );

    let (batch, batch_record) = resolve_capability_action_with_record_for_state(
        &state,
        "agent.subagent_batch",
        json!({
            "children": [
                {"role": "review", "objective": "inspect_runtime_boundary"},
                {"role": "test", "objective": "inspect_test_boundary"}
            ]
        }),
    );
    let Some(AgentAction::CallTool { tool, args }) = batch else {
        panic!("batch subagent capability must resolve to an internal tool");
    };
    assert_eq!(tool, "subagent");
    assert_eq!(args["action"], "bounded_parallel_readonly");
    assert_eq!(args["children"].as_array().map(Vec::len), Some(2));
    assert_eq!(batch_record.source, "registry");
    assert_eq!(
        batch_record.canonical_capability_ref.as_deref(),
        Some("agent.subagent_batch")
    );

    let (persistent, persistent_record) = resolve_capability_action_with_record_for_state(
        &state,
        "agent.subagent_persistent",
        json!({
            "role": "reviewer",
            "objective": "inspect_runtime_boundary",
            "allowed_capabilities": ["filesystem.read_text"]
        }),
    );
    let Some(AgentAction::CallTool { tool, args }) = persistent else {
        panic!("persistent subagent capability must resolve to an internal tool");
    };
    assert_eq!(tool, "subagent");
    assert_eq!(args["action"], "persistent_child_task");
    assert_eq!(persistent_record.source, "registry");
    assert_eq!(
        persistent_record.canonical_capability_ref.as_deref(),
        Some("agent.subagent_persistent")
    );
}

#[test]
fn real_registry_resolves_revisioned_task_plan_capabilities() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "task.plan_update",
        json!({
            "plan_revision": 3,
            "updates": [{"step_id":"verify","status":"in_progress"}]
        }),
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("task plan capability must resolve to a host tool");
    };
    assert_eq!(tool, "task_plan");
    assert_eq!(args["action"], "update_steps");
    assert_eq!(args["plan_revision"], 3);
    assert_eq!(args["updates"][0]["step_id"], "verify");
    assert_eq!(record.source, "registry");
    assert_eq!(
        record.canonical_capability_ref.as_deref(),
        Some("task.plan_update")
    );
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
fn filesystem_apply_patch_alias_resolves_to_canonical_workspace_patch() {
    let state = state_with_workspace_registry();
    let patch = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "filesystem.apply_patch",
        json!({"patch": patch}),
    );

    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected fs_basic tool action");
    };
    assert_eq!(tool, "fs_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("apply_patch")
    );
    assert_eq!(args.get("patch").and_then(Value::as_str), Some(patch));
    assert_eq!(record.capability_ref, "filesystem.apply_patch");
    assert_eq!(
        record.canonical_capability_ref.as_deref(),
        Some("workspace.apply_patch")
    );
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
}

#[test]
fn workspace_replace_text_resolves_with_exact_machine_arguments() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "workspace.replace_text",
        json!({
            "path": "src/lib.rs",
            "old_text": "old_value",
            "new_text": "new_value",
            "expected_occurrences": 1,
            "expected_sha256": "sha256:abcd",
        }),
    );

    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected fs_basic tool action");
    };
    assert_eq!(tool, "fs_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("replace_text")
    );
    assert_eq!(
        args.get("expected_occurrences").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(record.capability_ref, "workspace.replace_text");
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
}

#[test]
fn workspace_edit_text_resolves_batch_without_a_second_runtime_action() {
    let state = state_with_workspace_registry();
    let edits = json!([
        {
            "old_text": "old_value",
            "new_text": "middle_value",
        },
        {
            "old_text": "middle_value",
            "new_text": "new_value",
            "replace_all": true,
            "expected_occurrences": 2,
        }
    ]);
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "workspace.edit_text",
        json!({"path": "src/lib.rs", "edits": edits}),
    );

    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected fs_basic tool action");
    };
    assert_eq!(tool, "fs_basic");
    assert_eq!(args["action"], "replace_text");
    assert_eq!(args["edits"], edits);
    assert_eq!(record.capability_ref, "workspace.edit_text");
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
}

#[test]
fn local_and_remote_git_capabilities_resolve_to_separate_registry_skills() {
    let state = state_with_workspace_registry();
    for (capability, action_name, input) in [
        (
            "git.stage",
            "stage",
            json!({"paths": ["src/lib.rs", "docs/说明 file.md"]}),
        ),
        (
            "git.commit",
            "commit",
            json!({"message": "record local changes"}),
        ),
        (
            "git.create_branch",
            "create_branch",
            json!({"branch_name": "feature/local"}),
        ),
        (
            "git.checkout_branch",
            "checkout_branch",
            json!({"branch_name": "feature/local"}),
        ),
    ] {
        let (resolved, record) =
            resolve_capability_action_with_record_for_state(&state, capability, input);
        let Some(AgentAction::CallTool { tool, args }) = resolved else {
            panic!("expected registry tool action for {capability}");
        };
        assert_eq!(tool, "git_basic");
        assert_eq!(
            args.get("action").and_then(Value::as_str),
            Some(action_name)
        );
        assert_eq!(record.source, "registry");
        assert_eq!(record.capability_ref, capability);
    }

    let (push, record) =
        resolve_capability_action_with_record_for_state(&state, "git.push", json!({}));
    let Some(AgentAction::CallSkill { skill, args }) = push else {
        panic!("expected registry skill action for git.push");
    };
    assert_eq!(skill, "git_remote_publish");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("push"));
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "git.push");
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
fn registry_does_not_expose_crypto_trading_or_order_actions_to_planner() {
    let state = state_with_workspace_registry();
    for capability in [
        "crypto.trade_preview",
        "crypto.trade_submit",
        "crypto.order_status",
        "crypto.cancel_order",
        "crypto.cancel_all_orders",
        "crypto.open_orders",
        "crypto.trade_history",
    ] {
        let (action, record) = resolve_capability_action_with_record_for_state(
            &state,
            capability,
            json!({"action": capability.trim_start_matches("crypto.")}),
        );
        assert!(action.is_none(), "{capability} must stay direct/admin-only");
        assert_eq!(record.reason_code, "capability_unavailable", "{capability}");
        assert_eq!(record.source, "none", "{capability}");
        assert_eq!(record.capability_ref, capability);
        assert!(record.resolved_ref.is_none(), "{capability}");
    }
}

#[test]
fn crypto_registry_separates_read_capability_policy_from_complete_runner_risk() {
    let state = state_with_workspace_registry();
    let manifest = state.skill_manifest("crypto").expect("crypto manifest");
    assert_eq!(
        manifest.risk_level,
        Some(claw_core::skill_registry::SkillRiskLevel::High)
    );
    assert_eq!(manifest.requires_confirmation, Some(true));
    assert_eq!(manifest.side_effect, Some(true));

    for action in ["quote", "multi_quote", "positions"] {
        assert!(
            !state.skill_invocation_requires_confirmation_policy(
                "crypto",
                Some(&json!({"action": action}))
            ),
            "{action} should remain a read-only planner path"
        );
    }
    for action in ["trade_submit", "cancel_order", "cancel_all_orders"] {
        assert!(
            state.skill_invocation_requires_confirmation_policy(
                "crypto",
                Some(&json!({"action": action}))
            ),
            "{action} must inherit complete-runner confirmation policy"
        );
    }

    let registry = state.get_skills_registry().expect("skills registry");
    for capability in ["crypto.quote", "crypto.multi_quote"] {
        let mapping = registry
            .planner_capabilities("crypto")
            .iter()
            .find(|mapping| mapping.name == capability)
            .unwrap_or_else(|| panic!("missing {capability}"));
        assert_eq!(mapping.network_access, Some(true), "{capability}");
        assert_eq!(mapping.credential_access, Some(false), "{capability}");
    }
    let positions = registry
        .planner_capabilities("crypto")
        .iter()
        .find(|mapping| mapping.name == "crypto.positions")
        .expect("crypto.positions");
    assert_eq!(positions.network_access, Some(true));
    assert_eq!(positions.credential_access, Some(true));
}

#[test]
fn registry_resolves_market_quote_capabilities_without_domain_contracts() {
    let state = state_with_workspace_registry();
    for (capability, expected_skill, symbol) in [
        ("crypto.quote", "crypto", "BTCUSDT"),
        ("stock.quote", "stock", "600519"),
    ] {
        let (action, record) = resolve_capability_action_with_record_for_state(
            &state,
            capability,
            json!({"symbol": symbol}),
        );
        let action = action.unwrap_or_else(|| panic!("{capability} should resolve"));
        let AgentAction::CallSkill { skill, args } = action else {
            panic!("unexpected resolved action for {capability}: {action:?}");
        };
        assert_eq!(skill, expected_skill);
        assert_eq!(args.get("action").and_then(Value::as_str), Some("quote"));
        assert_eq!(args.get("symbol").and_then(Value::as_str), Some(symbol));
        assert_eq!(
            record.reason_code,
            "capability_resolver_registry_mapping_resolved"
        );
        assert_eq!(record.source, "registry");
        assert_eq!(record.capability_ref, capability);
    }
}

#[test]
fn registry_rejects_bare_skill_capability_even_with_machine_action() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "task_control",
        json!({"action": "list", "limit": 5}),
    );
    assert!(action.is_none());
    assert_eq!(record.reason_code, "capability_unavailable");
    assert_eq!(record.source, "none");
    assert_eq!(record.capability_ref, "task_control");
    assert!(record.resolved_ref.is_none());
}

#[test]
fn registry_rejects_bare_virtual_tool_even_with_registered_action_alias() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "fs_basic",
        json!({
            "action": "create_directory",
            "path": "run/example",
            "create_parents": true
        }),
    );

    assert!(action.is_none());
    assert_eq!(record.reason_code, "capability_unavailable");
    assert_eq!(record.source, "none");
    assert_eq!(record.capability_ref, "fs_basic");
}

#[test]
fn selected_registry_capability_cannot_be_rewritten_by_args_action() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "filesystem.write_text",
        json!({
            "action": "remove_path",
            "path": "notes/memo.txt",
            "content": "safe"
        }),
    );

    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected fs_basic tool action, got {action:?}");
    };
    assert_eq!(tool, "fs_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("write_text")
    );
}

#[test]
fn bare_skill_capability_rejects_unregistered_action_alias() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "fs_basic",
        json!({"action": "invent_directory", "path": "run/example"}),
    );

    assert!(action.is_none());
    assert_eq!(record.reason_code, "capability_unavailable");
}

#[test]
fn config_read_fields_capability_normalizes_machine_field_aliases() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "config.read_fields",
        json!({
            "config_path": "configs/agent_guard.toml",
            "fields": [
                "agent.hooks.handlers",
                "agent.subagents.allowed_roles",
                "agent.subagents.max_parallel_readonly",
                "agent.task_budget.admin_max_model_turns"
            ]
        }),
    );
    let action = action.expect("config.read_fields capability should resolve");
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
    assert_eq!(record.capability_ref, "config.read_fields");
}

#[test]
fn registry_resolves_declared_browser_capability() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "browser.open_extract",
        json!({"url": "https://example.com"}),
    );
    let action = action.expect("declared browser capability should resolve");
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
    assert_eq!(record.capability_ref, "browser.open_extract");
    assert_eq!(record.resolved_ref.as_deref(), Some("tool:browser_web"));
}

#[test]
fn registry_rejects_undeclared_skill_action_capability() {
    let state = state_with_workspace_registry();
    let (action, record) = resolve_capability_action_with_record_for_state(
        &state,
        "image_generate.preview_generate",
        json!({"prompt": "abstract geometric study"}),
    );

    assert!(action.is_none());
    assert_eq!(record.reason_code, "capability_unavailable");
    assert_eq!(record.outcome, "unresolved");
    assert_eq!(record.source, "none");
    assert_eq!(record.capability_ref, "image_generate.preview_generate");
    assert!(record.canonical_capability_ref.is_none());
    assert!(record.resolved_ref.is_none());
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
input_schema = { type = "object", properties = { text = { type = "string" }, target_locale = { type = "string" } } }
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
            json!({"query": "agent-runtime"}),
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
fn command_like_runtime_status_does_not_cross_capability_boundary() {
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
    assert_eq!(record.capability_ref, "system.runtime_status");
    assert_eq!(record.resolved_ref.as_deref(), Some("tool:system_basic"));
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected system_basic tool action, got {action:?}");
    };
    assert_eq!(tool, "system_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("runtime_status")
    );
    assert_eq!(args.get("kind").and_then(Value::as_str), Some("run_cmd"));
    assert_eq!(
        args.get("shell_command").and_then(Value::as_str),
        Some("python3 test_calc_core.py")
    );
}

#[test]
fn task_queue_runtime_status_does_not_cross_capability_boundary() {
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
    assert_eq!(record.capability_ref, "system.runtime_status");
    assert_eq!(record.resolved_ref.as_deref(), Some("tool:system_basic"));
    let Some(AgentAction::CallTool { tool, args }) = action else {
        panic!("expected system_basic tool action, got {action:?}");
    };
    assert_eq!(tool, "system_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("runtime_status")
    );
    assert_eq!(args.get("limit").and_then(Value::as_i64), Some(5));
    assert_eq!(
        args.get("kind").and_then(Value::as_str),
        Some("task_queue_status")
    );
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
            "filesystem.read_range",
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
            json!({"path": "/tmp/agent-runtime-test"}),
            "tool:fs_basic",
        ),
        (
            "filesystem.delete_path",
            json!({"path": "/tmp/agent-runtime-test"}),
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
            json!({"path": "configs/config.toml", "field_path": "webd.listen"}),
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

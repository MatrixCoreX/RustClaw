use super::*;

/// Registry stores `prompt_file` as a logical path
/// (e.g. prompts/skills/run_cmd.md).
/// Runtime in clawd assembles skill prompts from
/// prompts/layers/generated/skills/<name>.md plus optional
/// prompts/layers/vendor_patches/<vendor>/skills/common.md and
/// prompts/layers/vendor_patches/<vendor>/skills/<name>.md.
#[test]
fn test_registry_resolve_and_timeout() {
    let toml = r#"
[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"
aliases = ["shell", "exec"]
timeout_seconds = 60
prompt_file = "prompts/skills/run_cmd.md"
output_kind = "text"

[[skills]]
name = "image_vision"
enabled = true
kind = "runner"
aliases = ["vision"]
timeout_seconds = 90
prompt_file = "prompts/skills/image_vision.md"
output_kind = "image"
"#;
    let path = std::env::temp_dir().join("test_skills_registry.toml");
    std::fs::write(&path, toml).unwrap();
    let reg = SkillsRegistry::load_from_path(&path).unwrap();
    assert_eq!(reg.resolve_canonical("run_cmd"), Some("run_cmd"));
    assert_eq!(reg.resolve_canonical("shell"), Some("run_cmd"));
    assert_eq!(reg.resolve_canonical("exec"), Some("run_cmd"));
    assert_eq!(reg.resolve_canonical("image_vision"), Some("image_vision"));
    assert_eq!(reg.resolve_canonical("vision"), Some("image_vision"));
    assert_eq!(reg.timeout_seconds("run_cmd"), 60);
    assert_eq!(reg.timeout_seconds("image_vision"), 90);
    assert!(reg.enabled_names().contains(&"run_cmd".to_string()));
    assert!(reg.enabled_names().contains(&"image_vision".to_string()));
    let _ = std::fs::remove_file(path);
}

#[test]
fn planner_visible_defaults_true_and_can_hide_runtime_backing_tools() {
    let toml = r#"
[[skills]]
name = "config_basic"
enabled = true
kind = "builtin"

[[skills]]
name = "config_guard"
enabled = true
planner_visible = false
kind = "runner"
"#;
    let path = std::env::temp_dir().join("test_registry_planner_visible.toml");
    std::fs::write(&path, toml).unwrap();
    let reg = SkillsRegistry::load_from_path(&path).unwrap();

    assert!(reg.enabled_names().contains(&"config_guard".to_string()));
    assert!(reg.is_planner_visible("config_basic"));
    assert!(!reg.is_planner_visible("config_guard"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn registry_projects_fixed_and_initial_core_policy_separately() {
    let registry = SkillsRegistry::load_from_str(
        r#"
[[skills]]
name = "run_cmd"
fixed_on = true
planner_eager_load = true

[[skills]]
name = "schedule"
fixed_on = true
planner_eager_load = false

[[skills]]
name = "weather"
fixed_on = false
planner_eager_load = false
"#,
    )
    .expect("registry");

    assert_eq!(
        registry.fixed_on_names(),
        vec!["run_cmd".to_string(), "schedule".to_string()]
    );
    assert_eq!(registry.initial_core_names(), vec!["run_cmd".to_string()]);
    assert_eq!(
        registry.deferred_names(),
        vec!["schedule".to_string(), "weather".to_string()]
    );
    assert!(registry.is_fixed_on("schedule"));
    assert!(!registry.is_fixed_on("weather"));
}

#[test]
fn planner_capability_aliases_are_hidden_but_resolve_to_canonical_policy() {
    let registry = SkillsRegistry::load_from_str(
        r#"
[[skills]]
name = "fs_basic"
planner_capability_aliases = { "filesystem.read_file" = "filesystem.read_text_range" }
planner_capabilities = [
  { name = "filesystem.read_text_range", action = "read_text_range", effect = "observe", required = ["path"], optional = ["start_line"], risk_level = "low", idempotent = true, dedup_scope = "args" },
  { name = "filesystem.read_file", action = "read_text_range", effect = "observe", required = ["path"], risk_level = "low", idempotent = true, dedup_scope = "args" },
]
"#,
    )
    .expect("load alias registry");

    assert_eq!(registry.planner_capabilities("fs_basic").len(), 2);
    assert_eq!(
        registry
            .planner_exposed_capabilities("fs_basic")
            .into_iter()
            .map(|mapping| mapping.name.as_str())
            .collect::<Vec<_>>(),
        vec!["filesystem.read_text_range"]
    );
    assert_eq!(
        registry.canonical_planner_capability_name("filesystem.read_file"),
        Some("filesystem.read_text_range")
    );
    assert_eq!(
        registry
            .manifest("fs_basic")
            .expect("manifest")
            .planner_capabilities
            .len(),
        1
    );
}

#[test]
fn planner_capability_alias_policy_drift_is_rejected() {
    let error = SkillsRegistry::load_from_str(
        r#"
[[skills]]
name = "fs_basic"
planner_capability_aliases = { "filesystem.read_file" = "filesystem.read_text_range" }
planner_capabilities = [
  { name = "filesystem.read_text_range", action = "read_text_range", effect = "observe", required = ["path"], risk_level = "low", idempotent = true, dedup_scope = "args" },
  { name = "filesystem.read_file", action = "read_text_range", effect = "mutate", required = ["path"], risk_level = "high", idempotent = false, dedup_scope = "action" },
]
"#,
    )
    .expect_err("policy drift must fail");

    assert!(error.contains("planner capability alias policy mismatch"));
}

#[test]
fn duplicate_cross_skill_planner_capability_alias_is_rejected() {
    let error = SkillsRegistry::load_from_str(
        r#"
[[skills]]
name = "first"
planner_capability_aliases = { "legacy.read" = "first.read" }
planner_capabilities = [
  { name = "first.read", action = "read", effect = "observe", risk_level = "low", idempotent = true, dedup_scope = "args" },
  { name = "legacy.read", action = "read", effect = "observe", risk_level = "low", idempotent = true, dedup_scope = "args" },
]

[[skills]]
name = "second"
planner_capability_aliases = { "legacy.read" = "second.read" }
planner_capabilities = [
  { name = "second.read", action = "read", effect = "observe", risk_level = "low", idempotent = true, dedup_scope = "args" },
  { name = "legacy.read", action = "read", effect = "observe", risk_level = "low", idempotent = true, dedup_scope = "args" },
]
"#,
    )
    .expect_err("duplicate aliases must fail");

    assert!(error.contains("duplicate planner capability alias"));
}

#[test]
fn config_guard_ownership_leads_the_compact_registry_description() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&path).expect("load workspace registry");
    let description = registry
        .get("config_basic")
        .and_then(|entry| entry.description.as_deref())
        .expect("config_basic description");

    assert!(description.starts_with("config.guard_rustclaw_config owns"));
    assert!(description.contains("config.validate parses syntax only"));
    assert!(
        description.len() <= 180,
        "compact summary must remain visible"
    );
}

#[test]
fn integrity_report_flags_missing_and_wrong_kind() {
    let toml = r#"
[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"

[[skills]]
name = "read_file"
enabled = true
kind = "runner"   # 故意写错 kind，应该被 wrong_kind 抓到
"#;
    let path = std::env::temp_dir().join("test_registry_integrity_report.toml");
    std::fs::write(&path, toml).unwrap();
    let reg = SkillsRegistry::load_from_path(&path).unwrap();
    let report = reg.integrity_report();

    assert!(!report.is_clean());
    assert!(
        report.missing.contains(&"write_file".to_string()),
        "missing should include uncovered builtins, got {:?}",
        report.missing
    );
    assert_eq!(report.wrong_kind, vec!["read_file".to_string()]);

    let human = report.into_human_message().unwrap();
    assert!(human.contains("missing builtins"));
    assert!(human.contains("wrong kind"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn registry_load_rejects_duplicate_aliases_across_skills() {
    let toml = r#"
[[skills]]
name = "system_basic"
enabled = true
kind = "runner"
aliases = ["system"]

[[skills]]
name = "service_control"
enabled = true
kind = "runner"
aliases = ["system"]
"#;
    let path = std::env::temp_dir().join("test_registry_duplicate_alias.toml");
    std::fs::write(&path, toml).unwrap();
    let err = SkillsRegistry::load_from_path(&path)
        .err()
        .expect("duplicate aliases must fail registry load");
    assert!(err.contains("duplicate skill alias `system`"), "{err}");
    assert!(err.contains("system_basic"), "{err}");
    assert!(err.contains("service_control"), "{err}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn registry_load_rejects_alias_colliding_with_later_skill_name() {
    let toml = r#"
[[skills]]
name = "system_basic"
enabled = true
kind = "runner"
aliases = ["service_control"]

[[skills]]
name = "service_control"
enabled = true
kind = "runner"
"#;
    let path = std::env::temp_dir().join("test_registry_alias_name_collision.toml");
    std::fs::write(&path, toml).unwrap();
    let err = SkillsRegistry::load_from_path(&path)
        .err()
        .expect("alias/name collisions must fail registry load");
    assert!(
        err.contains("duplicate skill alias/name `service_control`"),
        "{err}"
    );
    assert!(err.contains("system_basic"), "{err}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn capability_parses_closed_set_and_secrets_namespace() {
    // 注意：`secrets.<name>` 用 `<用途>_<vendor>_api_key` 命名，避免把
    // image / chat / vision 三套独立 LLM 配置的 key 串到一起。
    for token in [
        "llm",
        "net",
        "fs.read",
        "fs.write",
        "exec",
        "exec.sudo",
        "secrets.image_generation_minimax_api_key",
        "secrets.text_openai_api_key",
    ] {
        let cap = Capability::parse(token).unwrap_or_else(|e| {
            panic!("expected `{token}` to parse, got error: {e}");
        });
        assert_eq!(cap.as_token(), token);
    }
}

#[test]
fn capability_parse_is_case_insensitive_but_normalizes_to_lowercase() {
    assert_eq!(Capability::parse("LLM").unwrap(), Capability::Llm);
    assert_eq!(Capability::parse("FS.Write").unwrap(), Capability::FsWrite);
    assert_eq!(
        Capability::parse("Secrets.Image_Generation_MiniMax_Api_Key").unwrap(),
        Capability::Secrets("image_generation_minimax_api_key".to_string())
    );
}

#[test]
fn capability_rejects_unknown_tokens_and_bad_secret_names() {
    // 完全未知词
    assert!(Capability::parse("disk").is_err());
    // 空 token
    assert!(Capability::parse("").is_err());
    // secrets 但名字非法（含点、空、过长）
    assert!(Capability::parse("secrets.").is_err());
    assert!(Capability::parse("secrets.bad-name").is_err());
    assert!(Capability::parse("secrets.has space").is_err());
    let too_long = format!("secrets.{}", "a".repeat(65));
    assert!(Capability::parse(&too_long).is_err());
}

#[test]
fn capability_rejects_bare_vendor_secret_naming() {
    // 反模式：会让 image / text / vision / chat 共用 key —— 必须拒。
    for token in [
        "secrets.openai_api_key",
        "secrets.gemini_api_key",
        "secrets.anthropic_api_key",
        "secrets.claude_api_key",
        "secrets.qwen_api_key",
        "secrets.minimax_api_key",
        // 极端反模式：连 _api_key 都不带，纯 vendor 名
        "secrets.openai",
        "secrets.minimax",
    ] {
        let err = Capability::parse(token)
            .err()
            .unwrap_or_else(|| panic!("expected `{token}` to be rejected as bare-vendor naming"));
        assert!(
            err.contains("bare vendor naming"),
            "unexpected error for `{token}`: {err}"
        );
        assert!(
            err.contains("<usage>_<vendor>_api_key"),
            "error should hint the canonical naming pattern: {err}"
        );
    }

    // 正模式：带用途前缀必须放行。
    for token in [
        "secrets.image_generation_minimax_api_key",
        "secrets.image_edit_qwen_api_key",
        "secrets.image_vision_openai_api_key",
        "secrets.text_openai_api_key",
        "secrets.chat_minimax_api_key",
    ] {
        assert!(
            Capability::parse(token).is_ok(),
            "expected `{token}` to be accepted"
        );
    }
}

#[test]
fn registry_load_resolves_capabilities_and_dedups() {
    // 故意写两次 "llm" 验证 dedup；secret 用 image_generation 命名空间，
    // 与 chat/规划用的 [llm] 配置完全分离，杜绝把 image 的 key 注入到 text 链路。
    let toml = r#"
[[skills]]
name = "image_generate"
enabled = true
kind = "runner"
side_effect = true
capabilities = ["llm", "net", "fs.write", "llm", "secrets.image_generation_minimax_api_key"]
"#;
    let path = std::env::temp_dir().join("test_registry_capabilities_ok.toml");
    std::fs::write(&path, toml).unwrap();
    let reg = SkillsRegistry::load_from_path(&path).unwrap();
    let caps = reg.capabilities("image_generate");
    let tokens: Vec<String> = caps.iter().map(Capability::as_token).collect();
    // 已 dedup（"llm" 出现两次）+ 已按 as_token 排序
    assert_eq!(
        tokens,
        vec![
            "fs.write".to_string(),
            "llm".to_string(),
            "net".to_string(),
            "secrets.image_generation_minimax_api_key".to_string(),
        ]
    );
    assert!(reg.has_capability("image_generate", &Capability::Llm));
    assert!(reg.has_capability(
        "image_generate",
        &Capability::Secrets("image_generation_minimax_api_key".to_string())
    ));
    // 关键：image_generation 的 key 不应被 chat/text 链路误命中
    assert!(!reg.has_capability(
        "image_generate",
        &Capability::Secrets("text_minimax_api_key".to_string())
    ));
    assert!(!reg.has_capability("image_generate", &Capability::ExecSudo));
    // manifest 视图也带上
    let manifest = reg.manifest("image_generate").unwrap();
    assert_eq!(manifest.capabilities.len(), 4);
    let _ = std::fs::remove_file(path);
}

#[test]
fn registry_manifest_exposes_planner_metadata() {
    let toml = r#"
	[[skills]]
	name = "db_basic"
	enabled = true
	kind = "runner"
	description = "Use structured SQLite actions."
	semantic_tags = ["sqlite_query", "SQLite_Query", "sqlite_table_listing", ""]
	preferred_over_run_cmd = true
	validation_actions = ["sqlite_query", "SQLITE_QUERY"]
	once_per_task = true
	dedup_scope = "action"
	idempotent = false
		runtime_skill = "system_basic"
		runtime_action = "Inventory-Dir"
		runtime_default_args = { names_only = true }
		runtime_rewrite_arg_keys = ["Include_Hidden", "include-hidden", ""]
		supported_os = ["Linux", "macOS", ""]
	required_bins = ["sqlite3", "SQLite3"]
	optional_bins = ["file", "FILE"]
	platform_notes = ["SQLite file access is pure Rust in the runner.", "SQLite file access is pure Rust in the runner.", ""]
	planner_capabilities = [
		  { name = "Database::List-Tables", action = "List-Tables", effect = "observe", required = ["DB-Path"], optional = ["Limit"], preferred = true, risk_level = "low", once_per_task = false, dedup_scope = "args", idempotent = true, execution_mode = "async_preferred", async_adapter_kind = "HTTP-Job-Poll", isolation_profile = "read_only", network_access = false, filesystem_write = false, external_publish = false, credential_access = false, final_answer_shape = "Table-Listing" },
	  { name = "database::list-tables", action = "duplicate-ignored" }
	]
	matrix_admission = { eligible = true, declared_actions = ["List-Tables"], evidence_sources = ["structured-json"], required_extra_fields = ["extra.tables", "extra.count", "extra.tables"], extractor_kind = "Structured-Json", admission_version = "external-v1" }
	"#;
    let path = std::env::temp_dir().join("test_registry_planner_metadata.toml");
    std::fs::write(&path, toml).unwrap();
    let reg = SkillsRegistry::load_from_path(&path).unwrap();
    let manifest = reg.manifest("db_basic").unwrap();
    assert_eq!(
        manifest.description.as_deref(),
        Some("Use structured SQLite actions.")
    );
    assert_eq!(
        manifest.semantic_tags,
        vec![
            "sqlite_query".to_string(),
            "sqlite_table_listing".to_string()
        ]
    );
    assert!(reg.has_semantic_tag("db_basic", "sqlite_query"));
    assert!(reg.has_semantic_tag("DB_BASIC", "SQLite_Query"));
    assert!(!reg.has_semantic_tag("db_basic", "missing_tag"));
    assert!(manifest.preferred_over_run_cmd);
    assert_eq!(manifest.planner_kind, PlannerCapabilityKind::Tool);
    assert_eq!(
        manifest.validation_actions,
        vec!["sqlite_query".to_string()]
    );
    assert_eq!(manifest.runtime_skill.as_deref(), Some("system_basic"));
    assert_eq!(manifest.runtime_action.as_deref(), Some("inventory_dir"));
    assert_eq!(
        manifest
            .runtime_default_args
            .as_ref()
            .and_then(|value| value.get("names_only"))
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        manifest.runtime_rewrite_arg_keys,
        vec!["include_hidden".to_string()]
    );
    assert_eq!(
        manifest.supported_os,
        vec!["linux".to_string(), "macos".to_string()]
    );
    assert_eq!(manifest.required_bins, vec!["sqlite3".to_string()]);
    assert_eq!(manifest.optional_bins, vec!["file".to_string()]);
    assert_eq!(
        manifest.platform_notes,
        vec!["SQLite file access is pure Rust in the runner.".to_string()]
    );
    assert_eq!(manifest.planner_capabilities.len(), 1);
    let capability = &manifest.planner_capabilities[0];
    assert_eq!(capability.name, "database.list_tables");
    assert_eq!(capability.action.as_deref(), Some("list_tables"));
    assert_eq!(capability.effect, Some(PlannerCapabilityEffect::Observe));
    assert_eq!(capability.required, vec!["db_path".to_string()]);
    assert_eq!(capability.optional, vec!["limit".to_string()]);
    assert_eq!(capability.risk_level, Some(SkillRiskLevel::Low));
    assert!(capability.preferred);
    assert_eq!(capability.once_per_task, Some(false));
    assert_eq!(capability.dedup_scope, Some(RegistryDedupScope::Args));
    assert_eq!(capability.idempotent, Some(true));
    assert_eq!(
        capability.execution_mode,
        Some(CapabilityExecutionMode::AsyncPreferred)
    );
    assert_eq!(
        capability.async_adapter_kind.as_deref(),
        Some("http_job_poll")
    );
    assert_eq!(
        capability.isolation_profile,
        Some(CapabilityIsolationProfile::ReadOnly)
    );
    assert_eq!(capability.network_access, Some(false));
    assert_eq!(capability.filesystem_write, Some(false));
    assert_eq!(capability.external_publish, Some(false));
    assert_eq!(capability.credential_access, Some(false));
    assert_eq!(
        capability.final_answer_shape.as_deref(),
        Some("table_listing")
    );
    assert_eq!(manifest.once_per_task, Some(true));
    assert_eq!(manifest.dedup_scope, Some(RegistryDedupScope::Action));
    assert_eq!(manifest.idempotent, Some(false));
    let entry = reg.get("db_basic").unwrap();
    assert_eq!(entry.semantic_tags, manifest.semantic_tags);
    assert_eq!(entry.validation_actions, manifest.validation_actions);
    assert_eq!(entry.supported_os, manifest.supported_os);
    assert_eq!(
        reg.planner_capabilities("db_basic"),
        manifest.planner_capabilities.as_slice()
    );
    let admission = reg
        .matrix_admission("db_basic")
        .expect("matrix admission metadata should load");
    assert!(admission.eligible);
    assert_eq!(admission.declared_actions, vec!["list_tables".to_string()]);
    assert_eq!(
        admission.evidence_sources,
        vec!["structured_json".to_string()]
    );
    assert_eq!(
        admission.required_extra_fields,
        vec!["extra.tables".to_string(), "extra.count".to_string()]
    );
    assert_eq!(admission.extractor_kind.as_deref(), Some("structured_json"));
    assert_eq!(admission.admission_version.as_deref(), Some("external-v1"));
    assert!(reg.matrix_admission_eligible("db_basic", Some("list-tables")));
    assert!(!reg.matrix_admission_eligible("db_basic", Some("query")));
    let _ = std::fs::remove_file(path);
}

#[test]
fn planner_capabilities_default_isolation_from_effect() {
    let toml = r#"
[[skills]]
name = "default_policy_tool"
enabled = true
kind = "runner"
planner_capabilities = [
  { name = "default.observe", effect = "observe" },
  { name = "default.mutate", effect = "mutate" },
  { name = "default.external", effect = "external" }
]
"#;
    let path = std::env::temp_dir().join("test_registry_default_isolation.toml");
    std::fs::write(&path, toml).unwrap();
    let reg = SkillsRegistry::load_from_path(&path).unwrap();
    let caps = reg.planner_capabilities("default_policy_tool");

    assert_eq!(
        caps[0].isolation_profile,
        Some(CapabilityIsolationProfile::ReadOnly)
    );
    assert_eq!(caps[0].network_access, Some(false));
    assert_eq!(caps[0].filesystem_write, Some(false));
    assert_eq!(caps[0].external_publish, Some(false));
    assert_eq!(caps[0].credential_access, Some(false));
    assert_eq!(
        caps[1].isolation_profile,
        Some(CapabilityIsolationProfile::LocalCurrentWorkspace)
    );
    assert_eq!(caps[1].network_access, Some(false));
    assert_eq!(caps[1].filesystem_write, Some(true));
    assert_eq!(caps[1].external_publish, Some(false));
    assert_eq!(
        caps[2].isolation_profile,
        Some(CapabilityIsolationProfile::RemoteExecutor)
    );
    assert_eq!(caps[2].network_access, Some(true));
    assert_eq!(caps[2].filesystem_write, Some(false));
    assert_eq!(caps[2].external_publish, Some(true));

    let _ = std::fs::remove_file(path);
}

#[test]
fn registry_resolves_action_governance_from_explicit_fields_effects_and_legacy_defaults() {
    let toml = r#"
	[[skills]]
	name = "config_edit"
	enabled = true
	kind = "runner"
	risk_level = "high"
	requires_confirmation = true
	side_effect = true
	once_per_task = true
	dedup_scope = "action"
	idempotent = false
	planner_capabilities = [
	  { name = "config.plan", action = "plan", effect = "observe" },
	  { name = "config.apply", action = "apply", effect = "mutate" },
	  { name = "config.preview", action = "preview", effect = "mutate", optional = ["path"], once_per_task = false, dedup_scope = "resource", dedup_fields = ["path"], idempotent = true }
	]

	[[skills]]
	name = "legacy_status"
	enabled = true
	kind = "runner"
	side_effect = false

	[[skills]]
	name = "legacy_mutate"
	enabled = true
	kind = "runner"
	risk_level = "high"
	requires_confirmation = true
	side_effect = true
	"#;
    let path = std::env::temp_dir().join("test_registry_action_governance.toml");
    std::fs::write(&path, toml).unwrap();
    let reg = SkillsRegistry::load_from_path(&path).unwrap();

    assert!(!reg.resolved_once_per_task("config_edit", Some("plan")));
    assert_eq!(
        reg.resolved_dedup_scope("config_edit", Some("plan")),
        RegistryDedupScope::Args
    );
    assert!(reg.resolved_idempotent("config_edit", Some("plan")));

    assert!(reg.resolved_once_per_task("config_edit", Some("apply")));
    assert_eq!(
        reg.resolved_dedup_scope("config_edit", Some("apply")),
        RegistryDedupScope::Action
    );
    assert!(!reg.resolved_idempotent("config_edit", Some("apply")));

    assert!(!reg.resolved_once_per_task("config_edit", Some("preview")));
    assert_eq!(
        reg.resolved_dedup_scope("config_edit", Some("preview")),
        RegistryDedupScope::Resource
    );
    assert_eq!(
        reg.resolved_dedup_fields("config_edit", Some("preview")),
        vec!["path"]
    );
    assert!(reg.resolved_idempotent("config_edit", Some("preview")));

    assert!(reg.resolved_once_per_task("config_edit", Some("unknown_action")));
    assert_eq!(
        reg.resolved_dedup_scope("config_edit", Some("unknown_action")),
        RegistryDedupScope::Action
    );
    assert!(!reg.resolved_idempotent("config_edit", Some("unknown_action")));

    assert!(!reg.resolved_once_per_task("legacy_status", None));
    assert_eq!(
        reg.resolved_dedup_scope("legacy_status", None),
        RegistryDedupScope::Args
    );
    assert!(reg.resolved_idempotent("legacy_status", None));

    assert!(reg.resolved_once_per_task("legacy_mutate", None));
    assert_eq!(
        reg.resolved_dedup_scope("legacy_mutate", None),
        RegistryDedupScope::Action
    );
    assert!(!reg.resolved_idempotent("legacy_mutate", None));
    assert!(!reg.resolved_once_per_task("missing", None));
    assert_eq!(
        reg.resolved_dedup_scope("missing", None),
        RegistryDedupScope::Args
    );
    assert!(!reg.resolved_idempotent("missing", None));
    let _ = std::fs::remove_file(path);
}

#[test]
fn registry_selects_preferred_actionless_capability_for_direct_invocation() {
    let toml = r#"
[[skills]]
name = "direct_runner"
enabled = true
kind = "runner"
once_per_task = false
idempotent = true
planner_capabilities = [
  { name = "runner.execute", effect = "external", preferred = true, once_per_task = true, idempotent = false, external_publish = true, credential_access = true },
  { name = "runner.alias", effect = "external", once_per_task = true, idempotent = false, external_publish = true, credential_access = true }
]
"#;
    let path = std::env::temp_dir().join(format!(
        "test_registry_direct_capability_{}_{}.toml",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, toml).unwrap();
    let registry = SkillsRegistry::load_from_path(&path).unwrap();

    let selected =
        select_planner_capability_mapping(registry.planner_capabilities("direct_runner"), None)
            .expect("preferred direct capability");
    assert_eq!(selected.name, "runner.execute");
    assert_eq!(selected.external_publish, Some(true));
    assert_eq!(selected.credential_access, Some(true));
    assert!(select_planner_capability_mapping(
        registry.planner_capabilities("direct_runner"),
        Some("unknown_action")
    )
    .is_none());
    assert!(registry.resolved_once_per_task("direct_runner", None));
    assert!(!registry.resolved_idempotent("direct_runner", None));

    let _ = std::fs::remove_file(path);
}

#[test]
fn registry_manifest_infers_and_allows_planner_kind_override() {
    let toml = r#"
[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"

[[skills]]
name = "image_vision"
enabled = true
kind = "runner"

[[skills]]
name = "extension_manager"
enabled = true
kind = "runner"
planner_kind = "workflow"

[[skills]]
name = "custom_bundle"
enabled = true
kind = "runner"
planner_kind = "tool"
"#;
    let path = std::env::temp_dir().join("test_registry_planner_kind.toml");
    std::fs::write(&path, toml).unwrap();
    let reg = SkillsRegistry::load_from_path(&path).unwrap();

    assert_eq!(
        reg.manifest("run_cmd").unwrap().planner_kind,
        PlannerCapabilityKind::Tool
    );
    assert_eq!(
        reg.manifest("image_vision").unwrap().planner_kind,
        PlannerCapabilityKind::Skill
    );
    assert_eq!(
        reg.manifest("extension_manager").unwrap().planner_kind,
        PlannerCapabilityKind::Workflow
    );
    assert_eq!(
        reg.manifest("custom_bundle").unwrap().planner_kind,
        PlannerCapabilityKind::Tool
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn shape_consistency_rejects_exec_sudo_without_confirmation() {
    let toml = r#"
[[skills]]
name = "danger_skill"
enabled = true
kind = "runner"
risk_level = "high"
side_effect = true
capabilities = ["exec.sudo"]
# 故意没设 requires_confirmation
"#;
    let path = std::env::temp_dir().join("test_shape_exec_sudo_no_confirm.toml");
    std::fs::write(&path, toml).unwrap();
    let err = SkillsRegistry::load_from_path(&path)
        .err()
        .expect("expected load to fail when exec.sudo lacks requires_confirmation");
    assert!(err.contains("`danger_skill`"), "{err}");
    assert!(err.contains("requires_confirmation"), "{err}");
    assert!(err.contains("R1"), "{err}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn shape_consistency_rejects_exec_sudo_without_high_risk() {
    let toml = r#"
[[skills]]
name = "danger_skill"
enabled = true
kind = "runner"
requires_confirmation = true
side_effect = true
risk_level = "medium"
capabilities = ["exec.sudo"]
"#;
    let path = std::env::temp_dir().join("test_shape_exec_sudo_medium_risk.toml");
    std::fs::write(&path, toml).unwrap();
    let err = SkillsRegistry::load_from_path(&path)
        .err()
        .expect("expected load to fail when exec.sudo is not high risk");
    assert!(err.contains("risk_level"), "{err}");
    assert!(err.contains("R2"), "{err}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn shape_consistency_rejects_explicit_side_effect_false_with_write_cap() {
    let toml = r#"
[[skills]]
name = "lying_skill"
enabled = true
kind = "runner"
side_effect = false
capabilities = ["fs.write"]
"#;
    let path = std::env::temp_dir().join("test_shape_fs_write_no_side_effect.toml");
    std::fs::write(&path, toml).unwrap();
    let err = SkillsRegistry::load_from_path(&path)
        .err()
        .expect("expected load to fail when fs.write declared with side_effect=false");
    assert!(err.contains("`lying_skill`"), "{err}");
    assert!(err.contains("side_effect"), "{err}");
    assert!(err.contains("R3"), "{err}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn shape_consistency_passes_with_proper_declarations() {
    // 完全合规：exec.sudo + confirm + high + side_effect=true
    let toml = r#"
[[skills]]
name = "safe_sudo"
enabled = true
kind = "runner"
requires_confirmation = true
risk_level = "high"
side_effect = true
capabilities = ["exec.sudo"]

[[skills]]
name = "writer"
enabled = true
kind = "runner"
side_effect = true
capabilities = ["fs.write"]

[[skills]]
name = "writer_unspecified_side_effect"
enabled = true
kind = "runner"
# 没设 side_effect — 容忍 None，但禁止显式 false
capabilities = ["fs.write"]
"#;
    let path = std::env::temp_dir().join("test_shape_consistency_clean.toml");
    std::fs::write(&path, toml).unwrap();
    let reg =
        SkillsRegistry::load_from_path(&path).expect("registry with proper shape should load");
    assert!(reg.validate_shape_consistency().is_empty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn registry_load_rejects_unknown_capability_token() {
    let toml = r#"
[[skills]]
name = "foo"
enabled = true
kind = "runner"
capabilities = ["llm", "wifi"]
"#;
    let path = std::env::temp_dir().join("test_registry_capabilities_bad.toml");
    std::fs::write(&path, toml).unwrap();
    let err = SkillsRegistry::load_from_path(&path)
        .err()
        .expect("expected load to fail on unknown capability token");
    assert!(
        err.contains("`foo`"),
        "error should mention skill name: {err}"
    );
    assert!(
        err.contains("wifi"),
        "error should mention bad token: {err}"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn registry_loads_structured_skill_memory_policy() {
    let toml = r#"
[[skills]]
name = "photo_organize"
enabled = true
kind = "runner"
memory_policy = { profile = "skill_scoped", include = ["preferences", "relevant-facts", "knowledge_docs"], exclude = ["assistant_results"], max_chars = 900, reason = "photo_organize_structured_args_only" }
"#;
    let path = std::env::temp_dir().join("test_registry_memory_policy.toml");
    std::fs::write(&path, toml).unwrap();
    let reg = SkillsRegistry::load_from_path(&path).unwrap();
    let policy = reg.memory_policy("photo_organize").expect("memory policy");
    assert_eq!(policy.profile, SkillMemoryPolicyProfile::SkillScoped);
    assert_eq!(
        policy.include,
        vec![
            "preferences".to_string(),
            "relevant_facts".to_string(),
            "knowledge_docs".to_string()
        ]
    );
    assert_eq!(policy.exclude, vec!["assistant_results".to_string()]);
    assert_eq!(policy.max_chars, Some(900));
    assert_eq!(
        policy.reason.as_deref(),
        Some("photo_organize_structured_args_only")
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn registry_rejects_unknown_skill_memory_policy_source() {
    let toml = r#"
[[skills]]
name = "photo_organize"
enabled = true
kind = "runner"
memory_policy = { include = ["preferences", "recent_chat_magic"] }
"#;
    let path = std::env::temp_dir().join("test_registry_memory_policy_bad.toml");
    std::fs::write(&path, toml).unwrap();
    let err = SkillsRegistry::load_from_path(&path)
        .err()
        .expect("expected load to fail on unknown memory source");
    assert!(
        err.contains("photo_organize"),
        "error should mention skill name: {err}"
    );
    assert!(
        err.contains("recent_chat_magic"),
        "error should mention bad memory source token: {err}"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn integrity_report_clean_when_all_required_builtins_present() {
    let mut toml = String::new();
    for name in REQUIRED_BUILTIN_SKILLS {
        toml.push_str(&format!(
            "[[skills]]\nname = \"{name}\"\nenabled = true\nkind = \"builtin\"\n\n"
        ));
    }
    let path = std::env::temp_dir().join("test_registry_integrity_clean.toml");
    std::fs::write(&path, toml).unwrap();
    let reg = SkillsRegistry::load_from_path(&path).unwrap();
    let report = reg.integrity_report();
    assert!(report.is_clean(), "expected clean report, got {report:?}");
    assert!(report.into_human_message().is_none());
    let _ = std::fs::remove_file(path);
}

#[test]
fn test_registry_manifest_view() {
    let toml = r#"
[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"
prompt_file = "prompts/skills/run_cmd.md"
description = "Run a shell command"
risk_level = "high"
auto_invocable = false
requires_confirmation = true
side_effect = true
retryable = true
	group = "shell"
	primary_fallback_role = "primary"
	supported_os = ["linux", "macos"]
	required_bins = ["bash"]
	optional_bins = ["sudo"]
	platform_notes = ["Uses bash for command execution."]
	input_schema = { type = "object", required = ["command"] }
output_schema = { type = "object", properties = { text = { type = "string" } } }
"#;
    let path = std::env::temp_dir().join("test_skills_manifest_registry.toml");
    std::fs::write(&path, toml).unwrap();
    let reg = SkillsRegistry::load_from_path(&path).unwrap();
    let manifest = reg.manifest("run_cmd").unwrap();
    assert_eq!(manifest.name, "run_cmd");
    assert_eq!(manifest.planner_kind, PlannerCapabilityKind::Tool);
    assert_eq!(manifest.description.as_deref(), Some("Run a shell command"));
    assert_eq!(
        manifest.prompt_file.as_deref(),
        Some("prompts/skills/run_cmd.md")
    );
    assert_eq!(manifest.risk_level, Some(SkillRiskLevel::High));
    assert_eq!(manifest.auto_invocable, Some(false));
    assert_eq!(manifest.requires_confirmation, Some(true));
    assert_eq!(manifest.side_effect, Some(true));
    assert_eq!(manifest.retryable, Some(true));
    assert_eq!(manifest.group.as_deref(), Some("shell"));
    assert_eq!(
        manifest.primary_fallback_role,
        Some(PrimaryFallbackRole::Primary)
    );
    assert_eq!(
        manifest.supported_os,
        vec!["linux".to_string(), "macos".to_string()]
    );
    assert_eq!(manifest.required_bins, vec!["bash".to_string()]);
    assert_eq!(manifest.optional_bins, vec!["sudo".to_string()]);
    assert_eq!(
        manifest.platform_notes,
        vec!["Uses bash for command execution.".to_string()]
    );
    assert_eq!(
        manifest
            .input_schema
            .as_ref()
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str()),
        Some("object")
    );
    assert_eq!(
        manifest
            .output_schema
            .as_ref()
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str()),
        Some("object")
    );
    let _ = std::fs::remove_file(path);
}

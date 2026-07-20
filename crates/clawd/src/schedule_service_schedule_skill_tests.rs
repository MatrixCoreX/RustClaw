use super::*;
use crate::runtime::types::{ScheduleIntentSchedule, ScheduleIntentTask};
use crate::ClaimedTask;
use serde_json::json;

fn test_registry() -> SkillsRegistry {
    let toml = r#"
[[skills]]
name = "rss_fetch"
enabled = true
kind = "runner"
aliases = ["rss", "rss_reader", "rss_fetcher", "news", "news_fetcher"]
timeout_seconds = 30
prompt_file = "prompts/skills/rss_fetch.md"
output_kind = "text"

[[skills]]
name = "crypto"
enabled = true
kind = "runner"
aliases = []
timeout_seconds = 30
prompt_file = "prompts/skills/crypto.md"
output_kind = "text"

[[skills]]
name = "demo_runner"
enabled = true
kind = "runner"
aliases = ["demo"]
timeout_seconds = 30
prompt_file = "prompts/skills/rss_fetch.md"
output_kind = "text"

[[skills]]
name = "health_check"
enabled = false
kind = "runner"
aliases = []
timeout_seconds = 30
prompt_file = "prompts/skills/health_check.md"
output_kind = "text"
"#;
    let path = std::env::temp_dir().join(format!(
        "schedule_skill_test_registry_{}_{}.toml",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, toml).unwrap();
    let reg = SkillsRegistry::load_from_path(&path).unwrap();
    let _ = std::fs::remove_file(path);
    reg
}

fn claimed_task_with_payload(channel: &str, payload: serde_json::Value) -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 0,
        task_id: "task-1".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("rk-test".to_string()),
        channel: channel.to_string(),
        external_user_id: Some("external-user".to_string()),
        external_chat_id: Some("external-chat".to_string()),
        kind: "ask".to_string(),
        payload_json: payload.to_string(),
    }
}

#[test]
fn schedule_payload_inherits_wechat_context_token_from_source_task() {
    let task = claimed_task_with_payload(
        "wechat",
        json!({
            "context_token": "ctx-123",
            "text": "45分的时候，发条消息给我"
        }),
    );
    let payload = json!({
        "text": "今晚记得点外卖",
        "schedule_task_mode": "direct_text"
    });

    let merged = inherit_schedule_delivery_context(&task, payload);

    assert_eq!(
        merged.get("context_token").and_then(|v| v.as_str()),
        Some("ctx-123")
    );
}

#[test]
fn schedule_payload_keeps_existing_context_token() {
    let task = claimed_task_with_payload(
        "wechat",
        json!({
            "context_token": "ctx-from-source"
        }),
    );
    let payload = json!({
        "text": "今晚记得点外卖",
        "context_token": "ctx-existing"
    });

    let merged = inherit_schedule_delivery_context(&task, payload);

    assert_eq!(
        merged.get("context_token").and_then(|v| v.as_str()),
        Some("ctx-existing")
    );
}

#[test]
fn schedule_needs_more_info_fallback_returns_machine_message_key() {
    let state = AppState::test_default_with_fixture_provider();
    let task = claimed_task_with_payload("api", json!({"text": "placeholder"}));

    for prompt in [
        "Remind me tomorrow to check deployment",
        "明天提醒我检查部署",
    ] {
        let value = serde_json::from_str::<serde_json::Value>(
            &schedule_needs_more_info_fallback_text(&state, &task, prompt),
        )
        .expect("machine fallback payload");
        assert_eq!(
            value.get("message_key").and_then(serde_json::Value::as_str),
            Some("schedule.msg.create_needs_more_info")
        );
        assert_eq!(
            value.get("reason_code").and_then(serde_json::Value::as_str),
            Some("schedule_needs_more_info")
        );
    }
}

#[test]
fn schedule_payload_does_not_inherit_context_token_for_non_wechat_channel() {
    let task = claimed_task_with_payload(
        "telegram",
        json!({
            "context_token": "ctx-123"
        }),
    );
    let payload = json!({
        "text": "今晚记得点外卖"
    });

    let merged = inherit_schedule_delivery_context(&task, payload);

    assert_eq!(merged.get("context_token"), None);
}

#[tokio::test]
async fn schedule_compile_only_create_returns_preview_without_insert() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = claimed_task_with_payload(
        "ui",
        json!({
            "text": "schedule parser dry-run"
        }),
    );
    let intent = ScheduleIntentOutput {
        kind: "create".to_string(),
        timezone: "Asia/Shanghai".to_string(),
        mode: "compile_only".to_string(),
        schedule: ScheduleIntentSchedule {
            r#type: "once".to_string(),
            run_at: "2099-01-01 09:00:00".to_string(),
            ..Default::default()
        },
        task: ScheduleIntentTask {
            kind: "ask".to_string(),
            payload: json!({"text": "check service"}),
        },
        confidence: 0.99,
        ..Default::default()
    };

    let reply =
        try_handle_schedule_request(&state, &task, "schedule parser dry-run", Some(&intent))
            .await
            .expect("schedule handler")
            .expect("preview reply");
    assert!(serde_json::from_str::<serde_json::Value>(&reply).is_err());
    assert!(reply.contains("message_key=schedule.intent.preview"));
    assert!(reply.contains("final_answer_shape=summary_with_evidence"));
    assert!(reply.contains("status=ok"));
    assert!(reply.contains("mode=compile_only"));
    assert!(reply.contains("kind=create"));
    assert!(reply.contains("dry_run=true"));
    assert!(reply.contains("preview_only=true"));
    assert!(reply.contains("would_mutate=false"));
    assert!(reply.contains("timezone=Asia/Shanghai"));
    assert!(reply.contains("datetime=2099-01-01 09:00:00"));
    assert!(reply.contains("schedule.type=once"));
    assert!(reply.contains("schedule.run_at=2099-01-01 09:00:00"));
    assert!(reply.contains("task_content=check service"));
    assert!(reply.contains("title=check service"));
    assert!(!reply.contains("contract_marker"));
    assert!(!reply.contains("semantic_kind"));
    let db = state.core.db.get().expect("db");
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM scheduled_jobs", [], |row| row.get(0))
        .expect("count scheduled jobs");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn schedule_compile_only_once_preview_omits_stale_weekday() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = claimed_task_with_payload(
        "ui",
        json!({
            "text": "schedule parser dry-run"
        }),
    );
    let intent = ScheduleIntentOutput {
        kind: "create".to_string(),
        timezone: "Asia/Shanghai".to_string(),
        mode: "compile_only".to_string(),
        schedule: ScheduleIntentSchedule {
            r#type: "once".to_string(),
            run_at: "2099-01-01 09:00:00".to_string(),
            weekday: 1,
            ..Default::default()
        },
        task: ScheduleIntentTask {
            kind: "ask".to_string(),
            payload: json!({"text": "check service"}),
        },
        confidence: 0.99,
        ..Default::default()
    };

    let reply =
        try_handle_schedule_request(&state, &task, "schedule parser dry-run", Some(&intent))
            .await
            .expect("schedule handler")
            .expect("preview reply");

    assert!(reply.contains("schedule.type=once"));
    assert!(reply.contains("schedule.run_at=2099-01-01 09:00:00"));
    assert!(!reply.contains("schedule.weekday="));
}

#[tokio::test]
async fn schedule_compile_only_preview_strips_internal_context_from_ask_payload() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = claimed_task_with_payload(
        "ui",
        json!({
            "text": "schedule parser dry-run"
        }),
    );
    let intent = ScheduleIntentOutput {
        kind: "create".to_string(),
        timezone: "Asia/Shanghai".to_string(),
        mode: "compile_only".to_string(),
        schedule: ScheduleIntentSchedule {
            r#type: "once".to_string(),
            run_at: "2099-01-01 09:00:00".to_string(),
            ..Default::default()
        },
        task: ScheduleIntentTask {
            kind: "ask".to_string(),
            payload: json!({}),
        },
        confidence: 0.99,
        ..Default::default()
    };
    let prompt_with_internal_context =
        "check service\n\n### ACTIVE_EXECUTION_ANCHOR\nfollowup_bound_target: /tmp/runtime.log";

    let reply =
        try_handle_schedule_request(&state, &task, prompt_with_internal_context, Some(&intent))
            .await
            .expect("schedule handler")
            .expect("preview reply");
    assert!(serde_json::from_str::<serde_json::Value>(&reply).is_err());
    assert!(reply.contains("task_content=check service"));
    assert!(!reply.contains("ACTIVE_EXECUTION_ANCHOR"));
    assert!(!reply.contains("followup_bound_target"));
}

#[test]
fn parse_local_datetime_accepts_t_separator_and_offset_forms() {
    let tz = parse_timezone("Asia/Shanghai");
    let base = parse_local_datetime("2099-01-01 09:00:00", tz).expect("space datetime");
    assert_eq!(parse_local_datetime("2099-01-01T09:00:00", tz), Some(base));
    assert_eq!(
        parse_local_datetime("2099-01-01 09:00:00 +08:00", tz),
        Some(base)
    );
    assert_eq!(
        parse_local_datetime("2099-01-01T09:00:00+08:00", tz),
        Some(base)
    );
}

#[test]
fn schedule_intent_alias_fields_normalize_to_canonical_payload() {
    let mut intent: ScheduleIntentOutput = serde_json::from_value(json!({
        "kind": "create",
        "dry_run": true,
        "timezone": "",
        "schedule": {
            "type": "once",
            "trigger_at": "2099-01-01T09:00:00+08:00",
            "timezone": "Asia/Shanghai",
            "content": "check service"
        },
        "task": {
            "kind": "",
            "payload": {}
        },
        "confidence": 0.99
    }))
    .expect("schedule intent aliases");

    normalize_schedule_intent_alias_fields(&mut intent);

    assert_eq!(intent.mode, "compile_only");
    assert_eq!(intent.timezone, "Asia/Shanghai");
    assert_eq!(intent.schedule.run_at, "2099-01-01T09:00:00+08:00");
    assert_eq!(intent.task.kind, "ask");
    assert_eq!(
        intent
            .task
            .payload
            .get("text")
            .and_then(|value| value.as_str()),
        Some("check service")
    );
}

#[test]
fn build_schedule_skill_catalog_from_registry_includes_enabled_skills() {
    let reg = test_registry();
    let catalog = build_schedule_skill_catalog_from_registry(&reg);
    assert!(catalog.contains("rss_fetch"));
    assert!(catalog.contains("demo_runner"));
    assert!(catalog.contains("rss") || catalog.contains("aliases"));
    assert!(catalog.contains("[enabled]"));
    assert!(catalog.contains("health_check"));
    assert!(catalog.contains("do NOT schedule") || catalog.contains("disabled"));
}

#[test]
fn schedule_prompt_template_substitutes_skill_catalog_placeholders() {
    let cat = "- rss_fetch (aliases: rss) [enabled] — runner";
    let tpl = "SKILL=__SKILL_CATALOG__\nLEGACY=__SKILLS_CATALOG__";
    let out = crate::render_prompt_template(
        tpl,
        &[("__SKILL_CATALOG__", cat), ("__SKILLS_CATALOG__", cat)],
    );
    assert!(!out.contains("__SKILL_CATALOG__"));
    assert!(!out.contains("__SKILLS_CATALOG__"));
    assert!(out.contains("rss_fetch"));
}

#[test]
fn validate_schedule_run_skill_news_fetcher_alias_resolves_to_rss_fetch() {
    let reg = test_registry();
    let payload = json!({
        "skill_name": "news_fetcher",
        "args": { "action": "latest", "category": "world" }
    });
    let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
    assert_eq!(
        out.get("skill_name").and_then(|v| v.as_str()),
        Some("rss_fetch")
    );
}

#[test]
fn validate_schedule_run_skill_unknown_skill_fails() {
    let reg = test_registry();
    let payload = json!({
        "skill_name": "totally_fake_skill_xyz",
        "args": { "action": "latest", "category": "tech" }
    });
    let out = validate_schedule_run_skill_with_registry(&reg, &payload);
    assert!(out.is_err());
    assert!(out.unwrap_err().contains("unknown skill"));
}

#[test]
fn validate_schedule_run_skill_alias_resolved_to_canonical() {
    let reg = test_registry();
    let payload = json!({
        "skill_name": "rss",
        "args": { "action": "latest", "category": "tech" }
    });
    let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
    assert_eq!(
        out.get("skill_name").and_then(|v| v.as_str()),
        Some("rss_fetch")
    );
}

/// Schedule does not rewrite `rss_fetch` args; `fetch_feed` is handled inside the skill.
#[test]
fn validate_schedule_run_skill_rss_fetch_fetch_feed_passes_through() {
    let reg = test_registry();
    let payload = json!({
        "skill_name": "rss_fetch",
        "args": { "action": "fetch_feed", "category": "tech" }
    });
    let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
    let args = out.get("args").and_then(|v| v.as_object()).unwrap();
    assert_eq!(
        args.get("action").and_then(|v| v.as_str()),
        Some("fetch_feed")
    );
    assert_eq!(args.get("category").and_then(|v| v.as_str()), Some("tech"));
}

#[test]
fn validate_schedule_run_skill_args_must_be_object() {
    let reg = test_registry();
    let payload = json!({
        "skill_name": "rss_fetch",
        "args": "not_an_object"
    });
    let out = validate_schedule_run_skill_with_registry(&reg, &payload);
    assert!(out.is_err());
    assert!(out.unwrap_err().contains("args"));
}

#[test]
fn validate_schedule_run_skill_disabled_skill_fails() {
    let reg = test_registry();
    let payload = json!({
        "skill_name": "health_check",
        "args": {}
    });
    let out = validate_schedule_run_skill_with_registry(&reg, &payload);
    assert!(out.is_err());
    assert!(out.unwrap_err().contains("disabled"));
}

#[test]
fn validate_schedule_run_skill_valid_rss_fetch_kept() {
    let reg = test_registry();
    let payload = json!({
        "skill_name": "rss_fetch",
        "args": { "action": "latest", "category": "science" }
    });
    let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
    assert_eq!(
        out.get("skill_name").and_then(|v| v.as_str()),
        Some("rss_fetch")
    );
    let args = out.get("args").and_then(|v| v.as_object()).unwrap();
    assert_eq!(args.get("action").and_then(|v| v.as_str()), Some("latest"));
    assert_eq!(
        args.get("category").and_then(|v| v.as_str()),
        Some("science")
    );
}

#[test]
fn validate_schedule_run_skill_rss_fetch_legacy_action_not_rewritten() {
    let reg = test_registry();
    let payload = json!({
        "skill_name": "rss_fetch",
        "args": { "action": "fetch_crypto_news" }
    });
    let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
    let args = out.get("args").and_then(|v| v.as_object()).unwrap();
    assert_eq!(
        args.get("action").and_then(|v| v.as_str()),
        Some("fetch_crypto_news")
    );
    assert!(!args.contains_key("category"));
}

#[test]
fn validate_schedule_run_skill_rss_fetch_unknown_action_passes_through() {
    let reg = test_registry();
    let payload = json!({
        "skill_name": "rss_fetch",
        "args": { "action": "totally_fake_action" }
    });
    let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
    assert_eq!(
        out.get("args")
            .and_then(|v| v.as_object())
            .and_then(|a| a.get("action"))
            .and_then(|v| v.as_str()),
        Some("totally_fake_action")
    );
}

/// Schedule does not validate `crypto` actions; bogus actions pass through for the skill to reject at runtime.
#[test]
fn validate_schedule_run_skill_crypto_action_passes_through_unvalidated() {
    let reg = test_registry();
    let payload = json!({
        "skill_name": "crypto",
        "args": { "action": "totally_bogus_crypto_action_xyz", "symbol": "BTCUSDT" }
    });
    let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
    assert_eq!(
        out.get("skill_name").and_then(|v| v.as_str()),
        Some("crypto")
    );
    let args = out.get("args").and_then(|v| v.as_object()).unwrap();
    assert_eq!(
        args.get("action").and_then(|v| v.as_str()),
        Some("totally_bogus_crypto_action_xyz")
    );
}

/// Production must not embed rss_fetch legacy action normalization (skill owns aliases).
#[test]
fn schedule_production_has_no_rss_legacy_action_strings_in_validation() {
    const SRC: &str = include_str!("schedule_service.rs");
    let prod = SRC.split("#[cfg(test)]").next().unwrap_or(SRC);
    for needle in ["fetch_crypto_news", "fetch_tech_news", "fetch_news"] {
        assert!(
                !prod.contains(needle),
                "schedule layer must not embed rss legacy alias `{needle}` (handled in rss_fetch skill)"
            );
    }
}

/// Generic `run_skill` validation must not rewrite args (no per-skill merge paths or symbol subprocesses).
#[test]
fn validate_schedule_run_skill_args_pass_through_for_any_enabled_skill() {
    let reg = test_registry();
    let payload = json!({
        "skill_name": "demo_runner",
        "args": { "action": "noop", "symbol": "TEST" }
    });
    let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
    assert_eq!(
        out.get("skill_name").and_then(|v| v.as_str()),
        Some("demo_runner")
    );
    let args = out.get("args").and_then(|v| v.as_object()).unwrap();
    assert_eq!(args.get("action").and_then(|v| v.as_str()), Some("noop"));
    assert_eq!(args.get("symbol").and_then(|v| v.as_str()), Some("TEST"));
    assert!(!args.contains_key("window_minutes"));
}

/// Schedule layer must not invoke arbitrary skills (only `schedule` compile via adapter).
#[test]
fn schedule_service_only_uses_adapter_for_schedule_compile_skill() {
    const SRC: &str = include_str!("schedule_service.rs");
    let prod = SRC.split("#[cfg(test)]").next().unwrap_or(SRC);
    let n = prod.matches("execution_adapters::run_skill").count();
    assert_eq!(
        n, 1,
        "schedule_service must only call execution_adapters::run_skill for schedule intent compile"
    );
    assert!(
        prod.contains("run_skill(state, task, \"schedule\", compile_args)"),
        "adapter target must remain the schedule skill only"
    );
}

/// Production must stay free of legacy coin-monitoring business logic (profiles, merge, extract).
#[test]
fn schedule_service_production_source_has_no_legacy_coin_markers() {
    const SRC: &str = include_str!("schedule_service.rs");
    let prod = SRC.split("#[cfg(test)]").next().unwrap_or(SRC);
    let markers = [
        concat!("Cr", "ypto", "Price", "Alert", "Profile"),
        concat!("Existing", "Cr", "ypto", "Price", "Alert", "Job"),
        concat!("extract_", "cryp", "to", "_price", "_alert", "_profile"),
        concat!("load_", "existing", "_cryp", "to", "_price", "_alert", "_jobs"),
        concat!("schedule_", "content", "_matches"),
        concat!("normalize_", "direction"),
        concat!("normalize_", "threshold", "_pct"),
        concat!("create_", "exists", "_same"),
        concat!("update_", "existing", "_ok"),
    ];
    for m in markers {
        assert!(
            !prod.contains(m),
            "production source must not contain legacy coin-monitoring marker `{m}`"
        );
    }
}

fn try_handle_schedule_create_arm_source() -> &'static str {
    const SRC: &str = include_str!("schedule_service.rs");
    let start = SRC
        .find("\"create\" => {")
        .expect("schedule match arm create");
    let tail = &SRC[start..];
    let end_rel = tail
        .find("\n        _ => Ok(None),")
        .expect("end of schedule match (before catch-all arm)");
    &tail[..end_rel]
}

/// `create` arm: generic job insert only — no removed coin-monitoring helpers, preflight, or VIP update paths.
#[test]
fn schedule_create_arm_inserts_job_without_coin_business_branches() {
    let create_arm = try_handle_schedule_create_arm_source();
    assert!(
        create_arm.contains("INSERT INTO scheduled_jobs"),
        "create must persist via scheduled_jobs insert"
    );
    assert_eq!(
        create_arm.matches("INSERT INTO scheduled_jobs").count(),
        1,
        "create must perform a single insert (no alternate merge/update path)"
    );
    assert!(
        create_arm.contains("schedule.msg.create_ok"),
        "create success path must still use generic create_ok message key"
    );

    let forbidden = [
        concat!("cryp", "to"),
        concat!("price_", "alert", "_check"),
        concat!("price_", "monitor"),
        concat!("monitor_", "price"),
        concat!("volatility", "_alert"),
        concat!("binance", "_symbol", "_check"),
        concat!("de", "dupe"),
        concat!("Pro", "file"),
        concat!("Cr", "ypto", "Price", "Alert", "Profile"),
        concat!("Existing", "Cr", "ypto", "Price", "Alert", "Job"),
        concat!("extract_", "cryp", "to", "_price", "_alert", "_profile"),
        concat!("schedule_", "content", "_matches"),
        concat!("load_", "existing", "_cryp", "to", "_price", "_alert", "_jobs"),
        concat!("normalize_", "direction"),
        concat!("normalize_", "threshold", "_pct"),
    ];
    for needle in forbidden {
        assert!(
            !create_arm.contains(needle),
            "create arm must not contain coin-specific marker `{needle}`"
        );
    }
}

/// Root schedule intent few-shots must not embed built-in monitoring defaults (belong in target skills).
#[test]
fn schedule_intent_prompt_root_avoids_builtin_monitoring_defaults_in_examples() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (s, resolved_path) = claw_core::prompt_layers::load_prompt_template_for_vendor(
        &workspace_root,
        "default",
        "prompts/schedule_intent_prompt.md",
        "",
    );
    assert!(
        !s.trim().is_empty(),
        "resolved prompt should not be empty: {}",
        resolved_path
    );
    assert!(
        !s.contains("\"window_minutes\":15"),
        "schedule_intent_prompt must not spell default window 15 in examples"
    );
    assert!(
        !s.contains("\"threshold_pct\":5"),
        "schedule_intent_prompt must not spell default threshold 5 in examples"
    );
    assert!(
        !s.contains("\"direction\":\"both\""),
        "schedule_intent_prompt must not spell default direction both in examples"
    );
}

#[test]
fn schedule_intent_schema_drift() {
    const SCHEMA_RAW: &str = include_str!("../../../prompts/schemas/schedule_intent.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA_RAW).expect("schedule_intent.schema.json must be valid JSON");
    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("schema.properties must be an object");
    for field in [
        "kind",
        "timezone",
        "schedule",
        "task",
        "target_job_id",
        "raw",
        "mode",
        "confidence",
        "reason",
        "needs_clarify",
        "clarify_question",
    ] {
        assert!(
                properties.contains_key(field),
                "schema missing parser field `{field}` under properties — sync prompts/schemas/schedule_intent.schema.json with ScheduleIntentOutput",
            );
    }

    let schedule_props = properties
        .get("schedule")
        .and_then(|v| v.get("properties"))
        .and_then(|v| v.as_object())
        .expect("schedule.properties must be an object");
    for field in [
        "type",
        "run_at",
        "time",
        "weekday",
        "every_minutes",
        "cron",
        "timezone",
        "content",
    ] {
        assert!(
            schedule_props.contains_key(field),
            "schema missing nested schedule field `{field}`",
        );
    }

    let task_props = properties
        .get("task")
        .and_then(|v| v.get("properties"))
        .and_then(|v| v.as_object())
        .expect("task.properties must be an object");
    for field in ["kind", "payload"] {
        assert!(
            task_props.contains_key(field),
            "schema missing nested task field `{field}`",
        );
    }
    let task_payload_props = task_props
        .get("payload")
        .and_then(|v| v.get("properties"))
        .and_then(|v| v.as_object())
        .expect("task.payload.properties must be an object");
    assert!(
        task_payload_props.contains_key("text"),
        "schema missing canonical task.payload.text field",
    );

    let probe = serde_json::json!({
        "kind": "create",
        "timezone": "Asia/Shanghai",
        "schedule": {
            "type": "daily",
            "run_at": "",
            "time": "08:00",
            "weekday": 1,
            "every_minutes": 0,
            "cron": ""
        },
        "task": {
            "kind": "run_skill",
            "payload": {
                "skill_name": "weather",
                "args": {}
            }
        },
        "target_job_id": "",
        "raw": "每天 8 点看天气",
        "mode": "execute",
        "confidence": 0.9,
        "reason": "daily weather schedule",
        "needs_clarify": false,
        "clarify_question": ""
    });
    let validated = crate::prompt_utils::validate_against_schema::<serde_json::Value>(
        &probe.to_string(),
        crate::prompt_utils::PromptSchemaId::ScheduleIntent,
    )
    .expect("schedule intent probe should validate");
    assert_eq!(
        validated
            .value
            .pointer("/task/kind")
            .and_then(|v| v.as_str()),
        Some("run_skill")
    );
}

#[test]
fn schedule_invocation_metadata_contains_required_keys() {
    let meta = schedule_invocation_metadata("job_abc123", "run_001");
    let keys: std::collections::HashSet<_> = meta.iter().map(|(k, _)| k.as_str()).collect();
    assert!(keys.contains("schedule_triggered"));
    assert!(keys.contains("schedule_job_id"));
    assert!(keys.contains("automation_ref"));
    assert!(keys.contains("automation_kind"));
    assert!(keys.contains("invocation_source"));
    assert!(keys.contains("resume_trigger"));
    assert!(keys.contains("resume_directive"));
    assert!(keys.contains("thread_resume"));
    assert!(keys.contains("thread_resume_source"));
    assert!(keys.contains("automation_checkpoint_required"));
    assert!(keys.contains("automation_run_id"));
    assert!(keys.contains("automation_thread_ref"));
    assert!(keys.contains("thread_ref"));
    assert!(keys.contains("scheduled_run_schema_version"));
    assert!(keys.contains("scheduled"));
    let job_id = meta
        .iter()
        .find(|(k, _)| *k == "schedule_job_id")
        .map(|(_, v)| v);
    assert_eq!(job_id.and_then(|v| v.as_str()), Some("job_abc123"));
    let src = meta
        .iter()
        .find(|(k, _)| *k == "invocation_source")
        .map(|(_, v)| v);
    assert_eq!(src.and_then(|v| v.as_str()), Some("schedule"));
    let automation_ref = meta
        .iter()
        .find(|(k, _)| *k == "automation_ref")
        .map(|(_, v)| v);
    assert_eq!(automation_ref.and_then(|v| v.as_str()), Some("job_abc123"));
    let automation_kind = meta
        .iter()
        .find(|(k, _)| *k == "automation_kind")
        .map(|(_, v)| v);
    assert_eq!(
        automation_kind.and_then(|v| v.as_str()),
        Some("scheduled_job")
    );
    let resume_trigger = meta
        .iter()
        .find(|(k, _)| *k == "resume_trigger")
        .map(|(_, v)| v);
    assert_eq!(
        resume_trigger.and_then(|v| v.as_str()),
        Some("scheduled_wakeup")
    );
    let thread_resume = meta
        .iter()
        .find(|(k, _)| *k == "thread_resume")
        .map(|(_, v)| v);
    assert_eq!(thread_resume.and_then(|v| v.as_bool()), Some(true));
    let run_id = meta
        .iter()
        .find(|(k, _)| *k == "automation_run_id")
        .map(|(_, v)| v);
    assert_eq!(run_id.and_then(|v| v.as_str()), Some("run_001"));
    let thread_ref = meta
        .iter()
        .find(|(k, _)| *k == "thread_ref")
        .map(|(_, v)| v);
    assert_eq!(
        thread_ref.and_then(|v| v.as_str()),
        Some("scheduled_job:job_abc123")
    );
}

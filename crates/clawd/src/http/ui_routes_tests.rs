use super::*;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_workspace_root() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("rustclaw-ui-routes-{unique}"));
    std::fs::create_dir_all(&path).expect("create temp dir");
    path
}

#[test]
fn write_workspace_and_mounted_file_writes_both_copies() {
    let root = temp_workspace_root();
    let relative = "configs/config.toml";
    let raw = "[llm]\nprovider = \"minimax\"\n";

    write_workspace_and_mounted_file(&root, relative, raw).expect("write config");

    let active = std::fs::read_to_string(root.join(relative)).expect("read active");
    let mounted =
        std::fs::read_to_string(root.join("docker/config/config.toml")).expect("read mounted");
    assert_eq!(active, raw);
    assert_eq!(mounted, raw);
}

#[test]
fn write_workspace_and_mounted_file_writes_channel_copy_to_mounted_channels_dir() {
    let root = temp_workspace_root();
    let relative = "configs/channels/wechat.toml";
    let raw = "[wechat]\nenabled = true\n";

    write_workspace_and_mounted_file(&root, relative, raw).expect("write config");

    let active = std::fs::read_to_string(root.join(relative)).expect("read active");
    let mounted = std::fs::read_to_string(root.join("docker/config/channels/wechat.toml"))
        .expect("read mounted");
    assert_eq!(active, raw);
    assert_eq!(mounted, raw);
}

#[test]
fn service_control_error_response_uses_machine_fields() {
    let (status, Json(body)) = service_control_error_response(
        StatusCode::BAD_REQUEST,
        "feishud",
        "start",
        ServiceControlFailure::new("service_disabled"),
    );

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!body.ok);
    assert_eq!(body.error.as_deref(), Some("service_disabled"));
    let data = body.data.expect("service control error data");
    assert_eq!(
        data.get("owner_layer").and_then(serde_json::Value::as_str),
        Some("ui_service_control")
    );
    assert_eq!(
        data.get("error_code").and_then(serde_json::Value::as_str),
        Some("service_disabled")
    );
    assert_eq!(
        data.get("message_key").and_then(serde_json::Value::as_str),
        Some("clawd.ui.service_control.service_disabled")
    );
}

#[test]
fn workspace_update_api_error_uses_machine_token() {
    let status_snapshot = WorkspaceUpdateStatus {
        status: "running".to_string(),
        step: "building_clawd".to_string(),
        ..WorkspaceUpdateStatus::default()
    };

    let (status, Json(body)) = workspace_update_api_error(
        StatusCode::CONFLICT,
        "workspace_update_already_running",
        Some(status_snapshot),
    );

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!body.ok);
    assert_eq!(
        body.error.as_deref(),
        Some("workspace_update_already_running")
    );
    assert_eq!(
        body.data.as_ref().map(|data| data.status.as_str()),
        Some("running")
    );
}

#[test]
fn update_feishu_config_raw_preserves_template_comments_and_updates_only_keys() {
    let output = update_feishu_config_raw_preserving_format(
        FEISHU_CONFIG_TEMPLATE,
        "cli_test_app",
        "secret_test",
    );
    assert!(output.contains("# Feishu（中国站）应用机器人通道配置"));
    assert!(output.contains("# 入站模式：webhook | long_connection"));
    assert!(output.contains("enabled = true"));
    assert!(output.contains("app_id = \"cli_test_app\""));
    assert!(output.contains("app_secret = \"secret_test\""));
    assert!(output.contains("image_inbox_dir = \"data/feishud/image\""));
}

#[test]
fn update_feishu_config_raw_keeps_unrelated_lines_when_updating_existing_file() {
    let raw = r#"# header
[feishu]
# before
app_id = ""
app_secret = ""
enabled = false
custom_keep = "yes"
"#;
    let output =
        update_feishu_config_raw_preserving_format(raw, "cli_keep_format", "secret_keep_format");
    assert!(output.contains("# before"));
    assert!(output.contains("custom_keep = \"yes\""));
    assert!(output.contains("app_id = \"cli_keep_format\""));
    assert!(output.contains("app_secret = \"secret_keep_format\""));
    assert!(output.contains("enabled = true"));
}

#[test]
fn llm_runtime_differs_when_only_api_key_changes() {
    assert!(llm_runtime_differs(
        "minimax",
        "MiniMax-M2.7",
        "openai_compat",
        "https://api.minimaxi.com/v1",
        "old-key",
        "minimax",
        "MiniMax-M2.7",
        "openai_compat",
        "https://api.minimaxi.com/v1",
        "new-key",
    ));
}

#[test]
fn llm_runtime_differs_when_only_base_url_changes() {
    assert!(llm_runtime_differs(
        "minimax",
        "MiniMax-M2.7",
        "openai_compat",
        "https://api.minimaxi.com/v1",
        "same-key",
        "minimax",
        "MiniMax-M2.7",
        "openai_compat",
        "https://api.minimax.cn/v1",
        "same-key",
    ));
}

#[test]
fn llm_runtime_differs_is_false_when_runtime_matches_saved_config() {
    assert!(!llm_runtime_differs(
        "minimax",
        "MiniMax-M2.7",
        "openai_compat",
        "https://api.minimaxi.com/v1",
        "same-key",
        "minimax",
        "MiniMax-M2.7",
        "openai_compat",
        "https://api.minimaxi.com/v1",
        "same-key",
    ));
}

#[test]
fn llm_runtime_differs_when_only_minimax_provider_type_changes() {
    assert!(llm_runtime_differs(
        "minimax",
        "MiniMax-M2.7",
        "anthropic_claude",
        "https://api.minimaxi.com/v1",
        "same-key",
        "minimax",
        "MiniMax-M2.7",
        "openai_compat",
        "https://api.minimaxi.com/v1",
        "same-key",
    ));
}

#[test]
fn llm_runtime_differs_when_only_mimo_provider_type_changes() {
    assert!(llm_runtime_differs(
        "mimo",
        "mimo-v2.5-pro",
        "anthropic_claude",
        "https://token-plan-sgp.xiaomimimo.com/v1",
        "same-key",
        "mimo",
        "mimo-v2.5-pro",
        "openai_compat",
        "https://token-plan-sgp.xiaomimimo.com/v1",
        "same-key",
    ));
}

#[test]
fn collect_llm_vendor_info_defaults_minimax_api_format_to_openai() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M2.7"

[llm.minimax]
api_key = ""
base_url = "https://api.minimaxi.com/v1"
model = "MiniMax-M2.7"
models = ["MiniMax-M2.7"]
"#,
    )
    .expect("parse");

    let vendors = collect_llm_vendor_info(&parsed);
    let minimax = vendors
        .iter()
        .find(|vendor| vendor.get("name").and_then(|v| v.as_str()) == Some("minimax"))
        .expect("minimax vendor");

    assert_eq!(
        minimax.get("api_format").and_then(|v| v.as_str()),
        Some("openai_compat")
    );
}

#[test]
fn collect_llm_vendor_info_defaults_mimo_api_format_to_openai() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[llm]
selected_vendor = "mimo"
selected_model = "mimo-v2.5-pro"

[llm.mimo]
api_key = ""
base_url = "https://token-plan-sgp.xiaomimimo.com/v1"
model = "mimo-v2.5-pro"
models = ["mimo-v2.5-pro"]
"#,
    )
    .expect("parse");

    let vendors = collect_llm_vendor_info(&parsed);
    let mimo = vendors
        .iter()
        .find(|vendor| vendor.get("name").and_then(|v| v.as_str()) == Some("mimo"))
        .expect("mimo vendor");

    assert_eq!(
        mimo.get("api_format").and_then(|v| v.as_str()),
        Some("openai_compat")
    );
}

#[test]
fn model_provider_keys_include_video_and_music_sections() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[video_generation.providers.minimax]
api_key = "video-secret"

[music_generation.providers.minimax]
api_key = "music-secret"
"#,
    )
    .expect("parse");

    let video = read_module_provider_keys(&parsed, &["video_generation"]);
    let music = read_module_provider_keys(&parsed, &["music_generation"]);

    assert_eq!(
        video
            .get("video_generation")
            .and_then(|vendors| vendors.get("minimax"))
            .map(String::as_str),
        Some("vide****cret")
    );
    assert_eq!(
        music
            .get("music_generation")
            .and_then(|vendors| vendors.get("minimax"))
            .map(String::as_str),
        Some("musi****cret")
    );
}

#[test]
fn upsert_model_section_updates_video_and_music_model_items() {
    let mut video = toml::Value::Table(toml::map::Map::new());
    let mut music = toml::Value::Table(toml::map::Map::new());
    let video_item = ModelConfigItem {
        vendor: "minimax".to_string(),
        model: "video-01".to_string(),
        base_url: Some("https://api.minimaxi.com/v1".to_string()),
        api_key: Some("video-secret".to_string()),
        ..default_model_item()
    };
    let music_item = ModelConfigItem {
        vendor: "minimax".to_string(),
        model: "music-2.6".to_string(),
        base_url: Some("https://api.minimaxi.com/v1".to_string()),
        api_key: Some("music-secret".to_string()),
        ..default_model_item()
    };

    upsert_model_section(&mut video, "video_generation", &video_item).unwrap();
    upsert_model_section(&mut music, "music_generation", &music_item).unwrap();

    assert_eq!(
        read_model_section(&video, "video_generation").model,
        "video-01"
    );
    assert_eq!(
        read_model_section(&music, "music_generation").model,
        "music-2.6"
    );
    assert_eq!(
        read_model_section(&video, "video_generation").api_key_configured,
        Some(true)
    );
    assert_eq!(
        read_model_section(&music, "music_generation").api_key_configured,
        Some(true)
    );
}

#[test]
fn model_sections_include_capability_metadata_and_model_cache() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[video_generation]
default_vendor = "minimax"
default_model = "video-01"
models = ["video-01", "video-01", "video-02"]

[video_generation.providers.minimax]
api_key = "video-secret"
"#,
    )
    .expect("parse");

    let item = read_model_section(&parsed, "video_generation");

    assert_eq!(item.capabilities, vec!["video.generate"]);
    assert_eq!(item.capability_family.as_deref(), Some("video"));
    assert_eq!(
        item.input_modalities,
        vec!["text".to_string(), "image".to_string(), "video".to_string()]
    );
    assert_eq!(item.output_modalities, vec!["video".to_string()]);
    assert_eq!(item.available_models, vec!["video-01", "video-02"]);
    assert_eq!(item.async_job_supported, Some(true));
    assert_eq!(
        item.shared_quota_group.as_deref(),
        Some("provider_account:minimax")
    );
    assert_eq!(
        item.shared_quota_note_key.as_deref(),
        Some("provider_account_shared_quota")
    );
    assert_eq!(item.model_list_source.as_deref(), Some("static_config"));
    assert_eq!(item.capability_source.as_deref(), Some("static_metadata"));
    assert_eq!(item.risk_level.as_deref(), Some("high"));
    assert_eq!(item.dry_run_supported, Some(true));
    assert_eq!(item.external_provider, Some(true));
    assert_eq!(item.provider_supported, Some(true));
    assert_eq!(item.unsupported_reason, None);
}

#[test]
fn llm_context_window_metadata_reads_selected_vendor_static_config() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M3"

[llm.minimax]
context_window_tokens = 1000000
models = ["MiniMax-M3"]
        "#,
    )
    .expect("parse");

    assert_eq!(
        read_llm_context_window_tokens(&parsed, "minimax"),
        Some(1_000_000)
    );
}

#[test]
fn model_sections_mark_cached_model_mismatch_with_machine_reason() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[video_generation]
default_vendor = "minimax"
default_model = "video-missing"
models = ["video-01", "video-02"]
        "#,
    )
    .expect("parse");

    let item = read_model_section(&parsed, "video_generation");

    assert_eq!(item.provider_supported, Some(false));
    assert_eq!(
        item.unsupported_reason.as_deref(),
        Some("model_not_in_available_models")
    );
}

#[test]
fn capability_items_flatten_skill_metadata_for_cli_and_ui() {
    let skill = SkillListItem {
        name: "video_generate".to_string(),
        description: None,
        kind: Some("builtin".to_string()),
        planner_kind: Some("capability".to_string()),
        adapter_category: Some("external_api_adapter".to_string()),
        background_job_capable: Some(true),
        group: Some("media".to_string()),
        risk_level: Some("high".to_string()),
        auto_invocable: Some(false),
        requires_confirmation: Some(true),
        side_effect: Some(true),
        retryable: Some(true),
        output_kind: Some("mixed".to_string()),
        enabled: Some(true),
        runtime_available: Some(true),
        unavailable_reason: None,
        current_os: Some("linux".to_string()),
        unsupported_os: None,
        missing_required_bins: None,
        missing_optional_bins: None,
        supported_os: None,
        required_bins: None,
        optional_bins: None,
        platform_notes: None,
        planner_capabilities: Some(vec!["video.generate".to_string()]),
        planner_capability_policies: Some(vec![PlannerCapabilityPolicyItem {
            capability: "video.generate".to_string(),
            isolation_profile: Some("remote_executor".to_string()),
            network_access: Some(true),
            filesystem_write: Some(false),
            external_publish: Some(true),
            credential_access: Some(true),
        }]),
        capabilities: Some(vec!["media.video".to_string()]),
    };

    let items = capability_items_from_skill_items(&[skill]);

    assert_eq!(items.len(), 2);
    assert!(items.iter().any(|item| {
        item.skill_name == "video_generate"
            && item.capability == "video.generate"
            && item.capability_kind == "planner_capability"
            && item.adapter_category.as_deref() == Some("external_api_adapter")
            && item.background_job_capable == Some(true)
            && item.enabled == Some(true)
            && item.risk_level.as_deref() == Some("high")
            && item.runtime_available == Some(true)
            && item.isolation_profile.as_deref() == Some("remote_executor")
            && item.network_access == Some(true)
            && item.filesystem_write == Some(false)
            && item.external_publish == Some(true)
            && item.credential_access == Some(true)
    }));
    assert!(items.iter().any(|item| {
        item.skill_name == "video_generate"
            && item.capability == "media.video"
            && item.capability_kind == "runtime_capability"
            && item.output_kind.as_deref() == Some("mixed")
    }));
}

#[test]
fn capability_items_include_disabled_machine_reason() {
    let skill = SkillListItem {
        name: "fs_basic".to_string(),
        description: None,
        kind: Some("builtin".to_string()),
        planner_kind: Some("tool".to_string()),
        adapter_category: Some("local_tool_adapter".to_string()),
        background_job_capable: None,
        group: Some("filesystem".to_string()),
        risk_level: Some("high".to_string()),
        auto_invocable: Some(false),
        requires_confirmation: Some(true),
        side_effect: Some(true),
        retryable: Some(false),
        output_kind: Some("text".to_string()),
        enabled: Some(false),
        runtime_available: Some(false),
        unavailable_reason: Some("skill_disabled".to_string()),
        current_os: Some("linux".to_string()),
        unsupported_os: None,
        missing_required_bins: None,
        missing_optional_bins: None,
        supported_os: None,
        required_bins: None,
        optional_bins: None,
        platform_notes: None,
        planner_capabilities: Some(vec!["filesystem.list_entries".to_string()]),
        planner_capability_policies: None,
        capabilities: None,
    };

    let items = capability_items_from_skill_items(&[skill]);

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].skill_name, "fs_basic");
    assert_eq!(items[0].capability, "filesystem.list_entries");
    assert_eq!(items[0].enabled, Some(false));
    assert_eq!(items[0].runtime_available, Some(false));
    assert_eq!(
        items[0].adapter_category.as_deref(),
        Some("local_tool_adapter")
    );
    assert_eq!(
        items[0].unavailable_reason.as_deref(),
        Some("skill_disabled")
    );
}

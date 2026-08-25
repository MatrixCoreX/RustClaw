use super::*;
use axum::http::HeaderValue;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const UI_ROUTE_TEST_USER_KEY: &str = "ui-route-test-key";

fn temp_workspace_root() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("agent-runtime-ui-routes-{unique}"));
    std::fs::create_dir_all(&path).expect("create temp dir");
    path
}

fn insert_ui_route_auth_key(state: &AppState) {
    state.seed_test_auth_identity(UI_ROUTE_TEST_USER_KEY, "admin");
}

fn ui_route_auth_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-agent-key",
        HeaderValue::from_static(UI_ROUTE_TEST_USER_KEY),
    );
    headers
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
fn wechat_service_start_can_enable_a_saved_channel_config() {
    let root = temp_workspace_root();
    std::fs::create_dir_all(root.join("configs/channels")).expect("channel config dir");
    std::fs::write(
        root.join("configs/channels/wechat.toml"),
        r#"
[wechat]
enabled = false
listen = "127.0.0.1:8792"
clawd_base_url = "http://127.0.0.1:8787"
api_base_url = "https://ilinkai.weixin.qq.com"
"#,
    )
    .expect("write wechat config");
    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root;

    validate_service_start_readiness(&state, "wechatd")
        .expect("start action persists enabled=true before spawning WeChat");
}

#[test]
fn communication_service_stop_persists_disabled_state_to_both_config_copies() {
    let root = temp_workspace_root();
    std::fs::create_dir_all(root.join("configs/channels")).expect("channel config dir");
    std::fs::write(
        root.join("configs/channels/wechat.toml"),
        "[wechat]\nenabled = true\nbot_token = \"saved\"\n",
    )
    .expect("write config");
    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    persist_channel_service_enabled(&state, "wechatd", false).expect("disable service");

    for path in [
        root.join("configs/channels/wechat.toml"),
        root.join("docker/config/channels/wechat.toml"),
    ] {
        let raw = std::fs::read_to_string(path).expect("read persisted config");
        let value = toml::from_str::<toml::Value>(&raw).expect("parse config");
        assert_eq!(
            value
                .get("wechat")
                .and_then(|section| section.get("enabled"))
                .and_then(toml::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            value
                .get("wechat")
                .and_then(|section| section.get("bot_token"))
                .and_then(toml::Value::as_str),
            Some("saved")
        );
    }

    persist_channel_service_enabled(&state, "wechatd", true).expect("enable service");
    for path in [
        root.join("configs/channels/wechat.toml"),
        root.join("docker/config/channels/wechat.toml"),
    ] {
        let raw = std::fs::read_to_string(path).expect("read re-enabled config");
        let value = toml::from_str::<toml::Value>(&raw).expect("parse config");
        assert_eq!(
            value
                .get("wechat")
                .and_then(|section| section.get("enabled"))
                .and_then(toml::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value
                .get("wechat")
                .and_then(|section| section.get("bot_token"))
                .and_then(toml::Value::as_str),
            Some("saved")
        );
    }
}

#[test]
fn communication_service_reset_clears_only_the_selected_channel_credentials() {
    let root = temp_workspace_root();
    std::fs::create_dir_all(root.join("configs/channels")).expect("channel config dir");
    std::fs::write(
        root.join("configs/channels/telegram.toml"),
        "[telegram]\nbot_token = \"secret\"\nbots = [{ name = \"extra\", bot_token = \"other\" }]\nbindings = [{ external_user_id = \"u\" }]\n\n[telegram_bot]\nenabled = true\n",
    )
    .expect("write config");
    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    reset_channel_service_config(&state, "telegramd").expect("reset telegram");

    let raw = std::fs::read_to_string(root.join("configs/channels/telegram.toml"))
        .expect("read reset config");
    let value = toml::from_str::<toml::Value>(&raw).expect("parse reset config");
    assert_eq!(
        value
            .get("telegram_bot")
            .and_then(|section| section.get("enabled"))
            .and_then(toml::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        value
            .get("telegram")
            .and_then(|section| section.get("bot_token"))
            .and_then(toml::Value::as_str),
        Some("")
    );
    assert!(value
        .get("telegram")
        .and_then(|section| section.get("bots"))
        .and_then(toml::Value::as_array)
        .is_some_and(Vec::is_empty));
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
fn workspace_release_version_prefers_release_tag_and_falls_back_to_version() {
    let root = temp_workspace_root();
    std::fs::write(root.join("VERSION"), "0.1.8\n").expect("write version");
    assert_eq!(workspace_release_version(&root).as_deref(), Some("0.1.8"));

    std::fs::write(root.join(".release-tag"), "ubuntu-x86_64-20260727-1\n")
        .expect("write release tag");
    assert_eq!(
        workspace_release_version(&root).as_deref(),
        Some("ubuntu-x86_64-20260727-1")
    );
    std::fs::remove_dir_all(root).expect("remove release version fixture");
}

#[tokio::test]
async fn workspace_release_tag_fallback_uses_version_order_for_the_requested_platform() {
    let root = temp_workspace_root();
    run_workspace_update_test_git(&root, &["init"]);
    run_workspace_update_test_git(&root, &["config", "user.name", "Agent Runtime Test"]);
    run_workspace_update_test_git(&root, &["config", "user.email", "test@agent-runtime.local"]);
    std::fs::write(root.join("README.md"), "fixture\n").expect("write release tag fixture");
    run_workspace_update_test_git(&root, &["add", "README.md"]);
    run_workspace_update_test_git(&root, &["commit", "-m", "fixture"]);
    run_workspace_update_test_git(&root, &["tag", "ubuntu-x86_64-20260728-9"]);
    run_workspace_update_test_git(&root, &["tag", "ubuntu-x86_64-20260728-10"]);
    run_workspace_update_test_git(&root, &["tag", "pi-aarch64-20260729-1"]);

    assert_eq!(
        resolve_latest_workspace_release_tag_for(&root, "ubuntu-x86_64-")
            .await
            .as_deref(),
        Some("ubuntu-x86_64-20260728-10")
    );
    std::fs::remove_dir_all(root).expect("remove release tag fallback fixture");
}

#[tokio::test]
async fn workspace_update_refresh_reports_release_version_without_git_fields() {
    let root = temp_workspace_root();
    std::fs::write(root.join("VERSION"), "0.1.8\n").expect("write version");
    std::fs::write(root.join(".release-tag"), "ubuntu-x86_64-20260727-1\n")
        .expect("write release tag");
    let shared = Arc::new(Mutex::new(WorkspaceUpdateStatus {
        old_commit: Some("stale-local".to_string()),
        new_commit: Some("stale-new".to_string()),
        remote_commit: Some("stale-remote".to_string()),
        latest_release_tag: Some("ubuntu-x86_64-20260727-2".to_string()),
        latest_release_check_status: "available".to_string(),
        latest_release_checked_ts: Some(current_unix_ts()),
        ..WorkspaceUpdateStatus::default()
    }));

    let status = refresh_workspace_update_versions(&root, shared, false).await;

    assert_eq!(status.installation_kind, "release_package");
    assert_eq!(
        status.current_release_version.as_deref(),
        Some("ubuntu-x86_64-20260727-1")
    );
    assert_eq!(status.old_commit, None);
    assert_eq!(status.new_commit, None);
    assert_eq!(status.remote_commit, None);
    std::fs::remove_dir_all(root).expect("remove release status fixture");
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
fn update_lark_config_raw_uses_international_endpoints_and_preserves_custom_lines() {
    let raw = r#"# Lark custom header
[lark]
enabled = false
app_id = ""
app_secret = ""
custom_keep = "yes"
"#;
    let output = update_lark_config_raw_preserving_format(raw, "cli_lark_test", "secret_lark_test");
    assert!(output.contains("# Lark custom header"));
    assert!(output.contains("custom_keep = \"yes\""));
    assert!(output.contains("enabled = true"));
    assert!(output.contains("app_id = \"cli_lark_test\""));
    assert!(output.contains("app_secret = \"secret_lark_test\""));
    assert!(output.contains("api_base_url = \"https://open.larksuite.com\""));
    assert!(output.contains("listen = \"0.0.0.0:8790\""));
}

#[test]
fn lark_app_entry_url_uses_larksuite_applink() {
    assert_eq!(
        agent_app_entry_url_for_app_id(AgentAppChannel::Lark, "cli_test").as_deref(),
        Some("https://applink.larksuite.com/client/bot/open?appId=cli_test")
    );
    assert_eq!(
        agent_app_entry_url_for_app_id(AgentAppChannel::Lark, ""),
        None
    );
}

#[test]
fn llm_runtime_differs_when_only_api_key_changes() {
    assert!(llm_runtime_differs(
        "minimax",
        "MiniMax-M3",
        "openai_compat",
        "https://api.minimaxi.com/v1",
        "old-key",
        "minimax",
        "MiniMax-M3",
        "openai_compat",
        "https://api.minimaxi.com/v1",
        "new-key",
    ));
}

#[test]
fn llm_runtime_differs_when_only_base_url_changes() {
    assert!(llm_runtime_differs(
        "minimax",
        "MiniMax-M3",
        "openai_compat",
        "https://api.minimaxi.com/v1",
        "same-key",
        "minimax",
        "MiniMax-M3",
        "openai_compat",
        "https://proxy.example/minimax/v1",
        "same-key",
    ));
}

#[test]
fn llm_runtime_differs_is_false_when_runtime_matches_saved_config() {
    assert!(!llm_runtime_differs(
        "minimax",
        "MiniMax-M3",
        "openai_compat",
        "https://api.minimaxi.com/v1",
        "same-key",
        "minimax",
        "MiniMax-M3",
        "openai_compat",
        "https://api.minimaxi.com/v1",
        "same-key",
    ));
}

#[test]
fn llm_runtime_differs_is_false_when_environment_key_matches_runtime() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M3"

[llm.minimax]
api_key = "legacy-config-key"
base_url = "https://api.minimaxi.com/v1"
model = "MiniMax-M3"
"#,
    )
    .expect("parse");
    let (base_url, api_key, provider_type) =
        saved_llm_vendor_runtime_fields_with_env(&parsed, "minimax", |name| {
            (name == "MINIMAX_API_KEY").then(|| "environment-key".to_string())
        });

    assert!(!llm_runtime_differs(
        "minimax",
        "MiniMax-M3",
        "openai_compat",
        "https://api.minimaxi.com/v1",
        "environment-key",
        "minimax",
        "MiniMax-M3",
        &provider_type,
        &base_url,
        &api_key,
    ));
}

#[test]
fn effective_saved_llm_key_uses_runtime_environment_precedence() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[llm.mimo]
api_key = "config-key"
base_url = "https://token-plan-cn.xiaomimimo.com/v1"
model = "mimo-v2.5-pro"
"#,
    )
    .expect("parse");
    let (_, api_key, _) =
        saved_llm_vendor_runtime_fields_with_env(&parsed, "mimo", |name| match name {
            "XIAOMI_API_KEY" => Some("xiaomi-key".to_string()),
            "MIMO_API_KEY" => Some("mimo-key".to_string()),
            _ => None,
        });

    assert_eq!(api_key, "mimo-key");
}

#[test]
fn effective_saved_llm_key_ignores_legacy_config_value_without_environment() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[llm.minimax]
api_key = "legacy-config-key"
base_url = "https://api.minimaxi.com/v1"
model = "MiniMax-M3"
"#,
    )
    .expect("parse");
    let (_, api_key, _) = saved_llm_vendor_runtime_fields_with_env(&parsed, "minimax", |_| None);

    assert!(api_key.is_empty());
}

#[test]
fn llm_runtime_differs_when_only_minimax_provider_type_changes() {
    assert!(llm_runtime_differs(
        "minimax",
        "MiniMax-M3",
        "anthropic_claude",
        "https://api.minimaxi.com/v1",
        "same-key",
        "minimax",
        "MiniMax-M3",
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
        "https://token-plan-cn.xiaomimimo.com/v1",
        "same-key",
        "mimo",
        "mimo-v2.5-pro",
        "openai_compat",
        "https://token-plan-cn.xiaomimimo.com/v1",
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
fn collect_llm_vendor_info_reports_environment_credentials_without_exposing_them() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M3"

[llm.minimax]
api_key = ""
base_url = "https://api.minimaxi.com/v1"
model = "MiniMax-M3"
"#,
    )
    .expect("parse");

    let vendors = collect_llm_vendor_info_with_env(&parsed, |name| {
        (name == "MINIMAX_API_KEY").then(|| "environment-secret".to_string())
    });
    let minimax = vendors
        .iter()
        .find(|vendor| vendor.get("name").and_then(|value| value.as_str()) == Some("minimax"))
        .expect("minimax vendor");

    assert_eq!(
        minimax
            .get("api_key_configured")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        minimax.get("api_key").and_then(|value| value.as_str()),
        Some("")
    );
    assert_eq!(
        minimax
            .get("api_key_source")
            .and_then(|value| value.as_str()),
        Some("environment")
    );
    assert_eq!(
        minimax
            .get("api_key_env_names")
            .and_then(|value| value.as_array())
            .and_then(|values| values.first())
            .and_then(|value| value.as_str()),
        Some("MINIMAX_API_KEY")
    );
    assert!(!minimax.to_string().contains("environment-secret"));
}

#[test]
fn collect_llm_vendor_info_does_not_treat_toml_key_as_configured() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[llm.minimax]
api_key = "legacy-config-secret"
base_url = "https://api.minimaxi.com/v1"
model = "MiniMax-M3"
"#,
    )
    .expect("parse");

    let vendors = collect_llm_vendor_info_with_env(&parsed, |_| None);
    let minimax = vendors
        .iter()
        .find(|vendor| vendor.get("name").and_then(|value| value.as_str()) == Some("minimax"))
        .expect("minimax vendor");

    assert_eq!(
        minimax
            .get("api_key_configured")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    assert!(!minimax.to_string().contains("legacy-config-secret"));
}

#[test]
fn collect_llm_vendor_info_reports_hosted_relay_device_enrollment() {
    let root = temp_workspace_root();
    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root;
    let parsed = toml::from_str::<toml::Value>(
        r#"
[llm]
selected_vendor = "custom"
selected_model = "minimax"

[llm.hosted_relay]
enabled = true
vendor = "custom"
model = "minimax"
base_url = "https://llm.example.test/v1"

[llm.custom]
api_key = ""
base_url = "https://llm.example.test/v1"
model = "minimax"
models = ["minimax"]
"#,
    )
    .expect("parse hosted relay config");

    let vendors = collect_llm_vendor_info_for_state(&parsed, &state);
    let relay = vendors
        .iter()
        .find(|vendor| vendor.get("name").and_then(Value::as_str) == Some("custom"))
        .expect("custom relay vendor");

    assert_eq!(
        relay.get("api_key_configured").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        relay.get("api_key_source").and_then(Value::as_str),
        Some("device_enrollment")
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
base_url = "https://token-plan-cn.xiaomimimo.com/v1"
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
fn ensure_string_array_contains_in_section_appends_future_model() {
    let raw = r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M3"

[llm.minimax]
model = "MiniMax-M3"
models = [
    "MiniMax-M3",
    "MiniMax-M2.7",
]
"#;

    let updated = ensure_string_array_contains_in_section(
        raw,
        "llm.minimax",
        "models",
        &["MiniMax-M3".to_string(), "MiniMax-M2.7".to_string()],
        "MiniMax-M4",
    );
    let parsed = toml::from_str::<toml::Value>(&updated).expect("updated toml parses");
    let models = parsed
        .get("llm")
        .and_then(|llm| llm.get("minimax"))
        .and_then(|minimax| minimax.get("models"))
        .and_then(|models| models.as_array())
        .expect("models array")
        .iter()
        .filter_map(|item| item.as_str())
        .collect::<Vec<_>>();

    assert_eq!(models, vec!["MiniMax-M3", "MiniMax-M2.7", "MiniMax-M4"]);
}

#[tokio::test]
async fn update_llm_config_saves_future_model_into_provider_pool() {
    let root = temp_workspace_root();
    std::fs::create_dir_all(root.join("configs")).expect("configs dir");
    std::fs::write(
        root.join("configs/config.toml"),
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M3"

[llm.minimax]
api_key = "legacy-config-key"
base_url = "https://api.minimaxi.com/v1"
model = "MiniMax-M3"
models = [
    "MiniMax-M3",
    "MiniMax-M2.7",
]
input_modalities = ["text", "image", "video"]
output_modalities = ["text"]
timeout_seconds = 180
"#,
    )
    .expect("write config");
    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    insert_ui_route_auth_key(&state);

    let (status, Json(body)) = update_llm_config(
        State(state),
        ui_route_auth_headers(),
        Json(UpdateLlmConfigRequest {
            selected_vendor: "minimax".to_string(),
            selected_model: "MiniMax-M4".to_string(),
            vendor_base_url: Some("https://api.minimaxi.com/v1".to_string()),
            vendor_api_key: Some(String::new()),
            vendor_api_format: Some("openai_compat".to_string()),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.ok);
    let updated = std::fs::read_to_string(root.join("configs/config.toml")).expect("read config");
    let parsed = toml::from_str::<toml::Value>(&updated).expect("updated toml parses");
    let llm = parsed.get("llm").expect("llm");
    let minimax = llm.get("minimax").expect("llm.minimax");
    let models = minimax
        .get("models")
        .and_then(|models| models.as_array())
        .expect("models")
        .iter()
        .filter_map(|item| item.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        llm.get("selected_model").and_then(|v| v.as_str()),
        Some("MiniMax-M4")
    );
    assert_eq!(
        minimax.get("model").and_then(|v| v.as_str()),
        Some("MiniMax-M4")
    );
    assert_eq!(minimax.get("api_key").and_then(|v| v.as_str()), Some(""));
    assert_eq!(models, vec!["MiniMax-M3", "MiniMax-M2.7", "MiniMax-M4"]);
}

#[tokio::test]
async fn test_llm_config_rejects_inline_api_key_for_direct_provider() {
    let root = temp_workspace_root();
    std::fs::create_dir_all(root.join("configs")).expect("configs dir");
    std::fs::write(
        root.join("configs/config.toml"),
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M3"

[llm.minimax]
api_key = ""
base_url = "https://api.minimaxi.com/v1"
model = "MiniMax-M3"
models = ["MiniMax-M3", "MiniMax-M2.7"]
"#,
    )
    .expect("write config");
    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root;
    insert_ui_route_auth_key(&state);

    let (status, Json(body)) = test_llm_config(
        State(state),
        ui_route_auth_headers(),
        Json(UpdateLlmConfigRequest {
            selected_vendor: "minimax".to_string(),
            selected_model: "MiniMax-M4".to_string(),
            vendor_base_url: Some("http://127.0.0.1:9/v1".to_string()),
            vendor_api_key: Some("test-key".to_string()),
            vendor_api_format: Some("openai_compat".to_string()),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!body.ok);
    assert_eq!(
        body.error.as_deref(),
        Some("vendor_api_key_private_store_not_allowed")
    );
}

#[tokio::test]
async fn hosted_relay_uses_automatic_device_key_enrollment() {
    let root = temp_workspace_root();
    std::fs::create_dir_all(root.join("configs")).expect("configs dir");
    std::fs::write(
        root.join("configs/config.toml"),
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M3"

[llm.hosted_relay]
enabled = true
vendor = "custom"
model = "minimax"
base_url = "https://llm.example.test/v1"
daily_request_limit = 100

[llm.model_classes.default]
provider = "minimax"
model = "MiniMax-M3"

[llm.model_classes.fast]
provider = "minimax"
model = "MiniMax-M3"

[llm.model_classes.reasoning]
provider = "minimax"
model = "MiniMax-M3"

[llm.custom]
api_key = ""
base_url = "https://api.example.test/v1"
model = "custom-model"
models = ["custom-model"]
"#,
    )
    .expect("write config");
    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    insert_ui_route_auth_key(&state);

    let (status, Json(body)) = update_llm_config(
        State(state),
        ui_route_auth_headers(),
        Json(UpdateLlmConfigRequest {
            selected_vendor: "custom".to_string(),
            selected_model: "minimax".to_string(),
            vendor_base_url: Some("https://llm.example.test/v1".to_string()),
            vendor_api_key: None,
            vendor_api_format: None,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "response: {body:?}");
    let raw = std::fs::read_to_string(root.join("configs/config.toml")).expect("read config");
    let parsed = toml::from_str::<toml::Value>(&raw).expect("parse config");
    assert_eq!(
        parsed["llm"]["model_classes"]["default"]["provider"].as_str(),
        Some("custom")
    );
    assert_eq!(
        parsed["llm"]["model_classes"]["reasoning"]["model"].as_str(),
        Some("minimax")
    );
    let credential_path = claw_core::git_remote_config::git_credential_store_path(&root);
    assert!(!claw_core::secrets::file_secret_is_configured(
        &credential_path,
        "text_custom_api_key"
    )
    .expect("credential status"));
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
fn model_sections_include_selected_vendor_model_cache() {
    let parsed = toml::from_str::<toml::Value>(
        r#"
[image_vision]
default_vendor = "minimax"
default_model = "MiniMax-M3"
models = ["qwen-vl-max"]
minimax_models = ["MiniMax-M3", "MiniMax-M3"]
qwen_models = ["qwen-vl-max", "qwen-vl-plus"]
        "#,
    )
    .expect("parse");

    let item = read_model_section(&parsed, "image_vision");

    assert_eq!(item.available_models, vec!["MiniMax-M3"]);
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
fn model_catalog_guard_status_reads_latest_gate_artifact() {
    let root = temp_workspace_root();
    let older = root.join("logs/agent_parity_gate/older");
    let newer = root.join("logs/agent_parity_gate/newer");
    std::fs::create_dir_all(&older).expect("older dir");
    std::fs::create_dir_all(&newer).expect("newer dir");
    std::fs::write(
        older.join("chinese_model_catalog.json"),
        r#"{"status":"error","finding_count":2}"#,
    )
    .expect("older guard");
    std::thread::sleep(std::time::Duration::from_millis(2));
    std::fs::write(
        newer.join("chinese_model_catalog.json"),
        r#"{"status":"ok","finding_count":0}"#,
    )
    .expect("newer guard");

    let status = read_model_catalog_guard_status(&root);

    assert_eq!(status["available"], true);
    assert_eq!(status["status"], "ok");
    assert_eq!(status["finding_count"], 0);
    assert_eq!(
        status["path"],
        "logs/agent_parity_gate/newer/chinese_model_catalog.json"
    );
}

#[tokio::test]
async fn model_catalog_api_handler_returns_secret_free_catalog() {
    let root = temp_workspace_root();
    std::fs::create_dir_all(root.join("configs")).expect("configs dir");
    std::fs::write(
        root.join("configs/config.toml"),
        r#"
[llm]
selected_vendor = "minimax"
selected_model = "MiniMax-M3"

[llm.minimax]
api_format = "openai_compat"
base_url = "https://api.minimaxi.com/v1"
api_key = "catalog-secret"
model = "MiniMax-M3"
models = ["MiniMax-M3", "MiniMax-M2.7"]
input_modalities = ["text", "image", "video"]
output_modalities = ["text"]
context_window_tokens = 1000000
timeout_seconds = 60
"#,
    )
    .expect("write config");
    std::fs::write(
        root.join("configs/image.toml"),
        r#"
[image_vision]
minimax_models = ["MiniMax-M3"]
"#,
    )
    .expect("write image config");
    std::fs::write(
        root.join("configs/video.toml"),
        r#"
[video_generation]
minimax_models = ["MiniMax-Hailuo-2.3"]
"#,
    )
    .expect("write video config");
    std::fs::write(
        root.join("configs/music.toml"),
        r#"
[music_generation]
minimax_models = ["music-2.6"]
"#,
    )
    .expect("write music config");
    let gate = root.join("logs/agent_parity_gate/catalog");
    std::fs::create_dir_all(&gate).expect("guard dir");
    std::fs::write(
        gate.join("chinese_model_catalog.json"),
        r#"{"status":"ok","finding_count":0}"#,
    )
    .expect("guard status");

    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root;
    insert_ui_route_auth_key(&state);

    let (status, Json(body)) = get_model_catalog(State(state), ui_route_auth_headers()).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.ok);
    assert_eq!(body.error, None);
    let data = body.data.expect("catalog data");
    assert_eq!(data["schema_version"], 2);
    assert_eq!(data["selected_provider"], "minimax");
    assert_eq!(data["selected_model"], "MiniMax-M3");
    assert_eq!(data["last_guard_status"]["available"], true);
    assert_eq!(data["last_guard_status"]["status"], "ok");
    let entries = data["entries"].as_array().expect("catalog entries");
    let minimax = entries
        .iter()
        .find(|entry| entry["provider"] == "minimax")
        .expect("minimax entry");
    assert_eq!(minimax["model"], "MiniMax-M3");
    assert_eq!(
        minimax["input_modalities"],
        json!(["text", "image", "video"])
    );
    assert_eq!(minimax["output_modalities"], json!(["text"]));
    assert_eq!(minimax["supports_text"], true);
    assert_eq!(minimax["supports_image_input"], true);
    assert_eq!(minimax["supports_video_input"], true);
    assert_eq!(minimax["supports_image_understanding"], true);
    assert_eq!(minimax["supports_video_generation"], false);
    assert_eq!(minimax["supports_music_generation"], false);
    assert_eq!(minimax["dry_run_supported"], false);
    assert_eq!(minimax["active_text_provider"], true);
    assert!(
        !data.to_string().contains("catalog-secret"),
        "catalog response must not expose provider secrets"
    );
}

#[test]
fn capability_items_flatten_skill_metadata_for_cli_and_ui() {
    let skill = SkillListItem {
        name: "video_generate".to_string(),
        description: None,
        description_zh: None,
        semantic_tags: None,
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
        fixed_on: Some(false),
        initial_core: Some(false),
        deferred: Some(true),
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
        config_files: None,
        planner_capabilities: Some(vec!["video.generate".to_string()]),
        planner_capability_details: None,
        planner_capability_policies: Some(vec![PlannerCapabilityPolicyItem {
            capability: "video.generate".to_string(),
            isolation_profile: Some("remote_executor".to_string()),
            network_access: Some(true),
            filesystem_write: Some(false),
            external_publish: Some(true),
            credential_access: Some(true),
            subprocess: Some(false),
            package_install: Some(false),
            privilege_escalation: Some(false),
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
            && item.subprocess == Some(false)
            && item.package_install == Some(false)
            && item.privilege_escalation == Some(false)
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
        description_zh: None,
        semantic_tags: None,
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
        fixed_on: Some(true),
        initial_core: Some(true),
        deferred: Some(false),
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
        config_files: None,
        planner_capabilities: Some(vec!["filesystem.list_entries".to_string()]),
        planner_capability_details: None,
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

#[test]
fn channel_capability_payload_exposes_auditable_provenance_without_secret_fields() {
    let payload = channel_capabilities_payload();
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["policy_version"], "channel-capability-policy-v1");
    assert_eq!(payload["verified_at"], "2026-07-31");

    let capabilities = payload["capabilities"].as_array().expect("capabilities");
    assert!(capabilities.len() >= 20);
    for source_kind in [
        "official_contract",
        "local_safety_policy",
        "experimental_inference",
    ] {
        assert!(capabilities
            .iter()
            .any(|record| record["source_kind"] == source_kind));
    }
    assert!(!payload.to_string().to_ascii_lowercase().contains("api_key"));
}

#[test]
fn skill_items_expose_registry_owned_instruction_metadata() {
    let skill = SkillListItem {
        name: "weather".to_string(),
        description: Some("Query weather observations.".to_string()),
        description_zh: Some("查询天气观测数据。".to_string()),
        semantic_tags: Some(vec!["weather.current".to_string()]),
        kind: Some("runner".to_string()),
        planner_kind: Some("skill".to_string()),
        adapter_category: Some("external_api_adapter".to_string()),
        background_job_capable: None,
        group: Some("news/web".to_string()),
        risk_level: Some("low".to_string()),
        auto_invocable: Some(true),
        requires_confirmation: Some(false),
        side_effect: Some(false),
        retryable: Some(true),
        output_kind: Some("text".to_string()),
        enabled: Some(true),
        fixed_on: Some(false),
        initial_core: Some(false),
        deferred: Some(true),
        runtime_available: Some(true),
        unavailable_reason: None,
        current_os: Some("linux".to_string()),
        unsupported_os: None,
        missing_required_bins: None,
        missing_optional_bins: None,
        supported_os: Some(vec!["linux".to_string(), "macos".to_string()]),
        required_bins: Some(vec!["curl".to_string()]),
        optional_bins: None,
        platform_notes: Some(vec!["Uses a public API.".to_string()]),
        config_files: Some(vec!["configs/weather.toml".to_string()]),
        planner_capabilities: Some(vec!["weather.current".to_string()]),
        planner_capability_details: Some(vec![PlannerCapabilityDisplayItem {
            capability: "weather.current".to_string(),
            action: Some("query".to_string()),
            description: Some("Read current weather.".to_string()),
            effect: Some("observe".to_string()),
            required: vec!["city|latitude+longitude".to_string()],
            optional: vec!["locale".to_string()],
        }]),
        planner_capability_policies: None,
        capabilities: Some(vec!["net".to_string()]),
    };

    let value = serde_json::to_value(skill).expect("serialize skill list item");

    assert_eq!(value["description"], "Query weather observations.");
    assert_eq!(value["description_zh"], "查询天气观测数据。");
    assert_eq!(value["config_files"][0], "configs/weather.toml");
    assert_eq!(
        value["planner_capability_details"][0]["capability"],
        "weather.current"
    );
    assert_eq!(
        value["planner_capability_details"][0]["required"][0],
        "city|latitude+longitude"
    );
    assert_eq!(value["deferred"], true);
}

#[test]
fn workspace_update_git_path_parser_is_nul_delimited_and_not_log_limited() {
    let mut raw = Vec::new();
    for index in 0..2_000 {
        raw.extend_from_slice(format!("generated/path_{index:04}.txt").as_bytes());
        raw.push(0);
    }
    assert!(raw.len() > WORKSPACE_UPDATE_LOG_MAX_CHARS);
    let paths = parse_git_name_list_bytes(&raw).expect("parse complete git path list");
    assert_eq!(paths.len(), 2_000);
    assert_eq!(
        paths.first().map(String::as_str),
        Some("generated/path_0000.txt")
    );
    assert_eq!(
        paths.last().map(String::as_str),
        Some("generated/path_1999.txt")
    );
}

#[test]
fn workspace_update_git_path_parser_preserves_spaces_and_rejects_escape_paths() {
    let paths = parse_git_name_list_bytes(b"dir/file with spaces.txt\0README.md\0").unwrap();
    assert_eq!(paths, vec!["dir/file with spaces.txt", "README.md"]);
    assert!(parse_git_name_list_bytes(b"../outside.txt\0").is_err());
    assert!(parse_git_name_list_bytes(b"/absolute.txt\0").is_err());
}

fn run_workspace_update_test_git(root: &Path, args: &[&str]) -> String {
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git command");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn workspace_update_upstream_candidate_prefers_origin_and_rejects_ambiguity() {
    assert_eq!(
        workspace_update_upstream_candidate("main", "backup/main\norigin/main\nupstream/main\n",)
            .as_deref(),
        Some("origin/main")
    );
    assert_eq!(
        workspace_update_upstream_candidate("stable", "company/stable\n").as_deref(),
        Some("company/stable")
    );
    assert_eq!(
        workspace_update_upstream_candidate("main", "backup/main\nupstream/main\n"),
        None
    );
}

#[tokio::test]
async fn workspace_update_resolves_missing_upstream_from_matching_origin_branch() {
    let root = temp_workspace_root();
    run_workspace_update_test_git(&root, &["init"]);
    run_workspace_update_test_git(&root, &["config", "user.name", "Agent Runtime Test"]);
    run_workspace_update_test_git(&root, &["config", "user.email", "test@agent-runtime.local"]);
    std::fs::write(root.join("README.md"), "fixture\n").expect("write fixture");
    run_workspace_update_test_git(&root, &["add", "README.md"]);
    run_workspace_update_test_git(&root, &["commit", "-m", "fixture"]);

    let branch = run_workspace_update_test_git(&root, &["branch", "--show-current"]);
    let commit = run_workspace_update_test_git(&root, &["rev-parse", "--short", "HEAD"]);
    let remote_ref = format!("refs/remotes/origin/{branch}");
    run_workspace_update_test_git(&root, &["update-ref", &remote_ref, "HEAD"]);
    run_workspace_update_test_git(&root, &["config", "remote.origin.url", "."]);
    run_workspace_update_test_git(
        &root,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );

    let resolved = resolve_workspace_update_remote_commit(&root)
        .await
        .expect("resolve upstream");
    assert_eq!(resolved.as_deref(), Some(commit.as_str()));
    assert_eq!(
        run_workspace_update_test_git(
            &root,
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}"
            ],
        ),
        format!("origin/{branch}")
    );
    std::fs::remove_dir_all(root).expect("remove temp repo");
}

#[tokio::test]
async fn workspace_update_conflict_detection_ignores_large_unrelated_file_lists() {
    let root = temp_workspace_root();
    run_workspace_update_test_git(&root, &["init"]);
    run_workspace_update_test_git(&root, &["config", "user.name", "Agent Runtime Test"]);
    run_workspace_update_test_git(&root, &["config", "user.email", "test@agent-runtime.local"]);

    std::fs::write(root.join("tracked.txt"), "base\n").expect("write base file");
    std::fs::write(root.join("already_remote.txt"), "base\n").expect("write generated base file");
    run_workspace_update_test_git(&root, &["add", "tracked.txt", "already_remote.txt"]);
    run_workspace_update_test_git(&root, &["commit", "-m", "base"]);
    let base_commit = run_workspace_update_test_git(&root, &["rev-parse", "HEAD"]);
    let branch = run_workspace_update_test_git(&root, &["branch", "--show-current"]);

    std::fs::write(root.join("tracked.txt"), "remote\n").expect("write remote change");
    std::fs::write(root.join("already_remote.txt"), "remote\n")
        .expect("write generated remote change");
    std::fs::write(root.join("new_remote.txt"), "remote\n").expect("write remote file");
    run_workspace_update_test_git(
        &root,
        &["add", "tracked.txt", "already_remote.txt", "new_remote.txt"],
    );
    run_workspace_update_test_git(&root, &["commit", "-m", "remote"]);
    let remote_commit = run_workspace_update_test_git(&root, &["rev-parse", "HEAD"]);

    run_workspace_update_test_git(&root, &["reset", "--hard", &base_commit]);
    let upstream_ref = format!("refs/remotes/origin/{branch}");
    run_workspace_update_test_git(&root, &["update-ref", &upstream_ref, &remote_commit]);
    run_workspace_update_test_git(&root, &["config", "remote.origin.url", "."]);
    run_workspace_update_test_git(
        &root,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    let branch_remote_key = format!("branch.{branch}.remote");
    let branch_merge_key = format!("branch.{branch}.merge");
    let branch_merge_ref = format!("refs/heads/{branch}");
    run_workspace_update_test_git(&root, &["config", &branch_remote_key, "origin"]);
    run_workspace_update_test_git(&root, &["config", &branch_merge_key, &branch_merge_ref]);

    std::fs::write(root.join("tracked.txt"), "local\n").expect("write local change");
    std::fs::write(root.join("already_remote.txt"), "remote\n")
        .expect("materialize incoming generated content");
    std::fs::write(root.join("new_remote.txt"), "local\n").expect("write local conflict");
    let unrelated = root.join("unrelated");
    std::fs::create_dir_all(&unrelated).expect("create unrelated directory");
    for index in 0..600 {
        std::fs::write(
            unrelated.join(format!("generated_path_{index:04}.txt")),
            b"x",
        )
        .expect("write unrelated path");
    }

    let conflicts = detect_workspace_update_conflict_paths(&root)
        .await
        .expect("detect conflicts");
    assert_eq!(conflicts.tracked, vec!["tracked.txt"]);
    assert_eq!(conflicts.untracked, vec!["new_remote.txt"]);

    std::fs::remove_dir_all(root).expect("remove test repository");
}

#[tokio::test]
async fn workspace_update_snapshots_and_restores_only_runtime_config_conflicts() {
    let root = temp_workspace_root();
    run_workspace_update_test_git(&root, &["init"]);
    run_workspace_update_test_git(&root, &["config", "user.name", "Agent Runtime Test"]);
    run_workspace_update_test_git(&root, &["config", "user.email", "test@agent-runtime.local"]);
    std::fs::create_dir_all(root.join("configs")).expect("create configs directory");
    std::fs::write(root.join("configs/config.toml"), "value = 'base'\n")
        .expect("write base config");
    run_workspace_update_test_git(&root, &["add", "configs/config.toml"]);
    run_workspace_update_test_git(&root, &["commit", "-m", "base"]);
    std::fs::write(root.join("configs/config.toml"), "value = 'local'\n")
        .expect("write local config");

    let paths = WorkspaceUpdateConflictPaths {
        tracked: vec!["configs/config.toml".to_string()],
        untracked: Vec::new(),
    };
    assert!(!workspace_update_has_non_config_conflicts(&paths));
    let snapshot = snapshot_workspace_update_config_conflicts(&root, &paths)
        .await
        .expect("snapshot local config");
    prepare_workspace_update_config_paths_for_pull(&root, &paths)
        .await
        .expect("prepare config for pull");
    assert_eq!(
        std::fs::read_to_string(root.join("configs/config.toml")).unwrap(),
        "value = 'base'\n"
    );
    std::fs::write(root.join("configs/config.toml"), "value = 'remote'\n")
        .expect("write remote config");
    restore_workspace_update_config_snapshot(&root, &snapshot)
        .await
        .expect("restore local config");
    assert_eq!(
        std::fs::read_to_string(root.join("configs/config.toml")).unwrap(),
        "value = 'local'\n"
    );

    let source_conflict = WorkspaceUpdateConflictPaths {
        tracked: vec!["crates/clawd/src/main.rs".to_string()],
        untracked: Vec::new(),
    };
    assert!(workspace_update_has_non_config_conflicts(&source_conflict));
    std::fs::remove_dir_all(root).expect("remove test repository");
}

#[tokio::test]
async fn workspace_update_refresh_clears_resolved_failure_when_upstream_matches() {
    let root = temp_workspace_root();
    run_workspace_update_test_git(&root, &["init"]);
    run_workspace_update_test_git(&root, &["config", "user.name", "Agent Runtime Test"]);
    run_workspace_update_test_git(&root, &["config", "user.email", "test@agent-runtime.local"]);

    std::fs::write(root.join("tracked.txt"), "base\n").expect("write base file");
    run_workspace_update_test_git(&root, &["add", "tracked.txt"]);
    run_workspace_update_test_git(&root, &["commit", "-m", "base"]);
    let branch = run_workspace_update_test_git(&root, &["branch", "--show-current"]);
    run_workspace_update_test_git(&root, &["config", "remote.origin.url", "."]);
    run_workspace_update_test_git(
        &root,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    let branch_remote_key = format!("branch.{branch}.remote");
    let branch_merge_key = format!("branch.{branch}.merge");
    let branch_merge_ref = format!("refs/heads/{branch}");
    run_workspace_update_test_git(&root, &["config", &branch_remote_key, "origin"]);
    run_workspace_update_test_git(&root, &["config", &branch_merge_key, &branch_merge_ref]);
    run_workspace_update_test_git(&root, &["fetch", "--quiet"]);

    let shared = Arc::new(Mutex::new(WorkspaceUpdateStatus {
        status: "failed".to_string(),
        step: "pulling_latest_code".to_string(),
        exit_code: Some(1),
        stdout_tail: "stale stdout".to_string(),
        stderr_tail: "stale stderr".to_string(),
        error: Some("workspace_update_git_path_list_item_limit_exceeded".to_string()),
        next_step_key: Some("workspace_update.pull_conflict_detection_failed".to_string()),
        ..WorkspaceUpdateStatus::default()
    }));

    let status = refresh_workspace_update_versions(&root, shared, false).await;

    assert_eq!(status.status, "up_to_date");
    assert_eq!(status.step, "already_latest");
    assert_eq!(status.exit_code, None);
    assert!(status.stdout_tail.is_empty());
    assert!(status.stderr_tail.is_empty());
    assert_eq!(status.error, None);
    assert_eq!(status.next_step_key, None);

    std::fs::remove_dir_all(root).expect("remove test repository");
}

#[test]
fn workspace_update_release_selector_requires_matching_platform_asset() {
    let artifact_id = claw_core::product_identity::product_identity().release_artifact_id();
    let ubuntu_asset_prefix = format!("{artifact_id}-ubuntu-x86_64-");
    let pi_asset_prefix = format!("{artifact_id}-pi-aarch64-");
    let releases = vec![
        json!({
            "tag_name": "ubuntu-x86_64-20260724-2",
            "draft": true,
            "assets": [{"name": format!("{ubuntu_asset_prefix}20260724-2.tar.gz")}]
        }),
        json!({
            "tag_name": "pi-aarch64-20260724-2",
            "assets": [{"name": format!("{pi_asset_prefix}20260724-2.tar.gz")}]
        }),
        json!({
            "tag_name": "ubuntu-x86_64-20260724-prerelease",
            "prerelease": true,
            "assets": [{"name": format!("{ubuntu_asset_prefix}20260724-prerelease.tar.gz")}]
        }),
        json!({
            "tag_name": "ubuntu-x86_64-20260724-1",
            "assets": [{"name": format!("{ubuntu_asset_prefix}20260724-1.tar.gz")}]
        }),
    ];
    assert_eq!(
        select_latest_compatible_release_tag(&releases, "ubuntu-x86_64-", &ubuntu_asset_prefix)
            .as_deref(),
        Some("ubuntu-x86_64-20260724-1")
    );
    assert_eq!(
        select_latest_compatible_release_tag(&releases, "pi-aarch64-", &pi_asset_prefix).as_deref(),
        Some("pi-aarch64-20260724-2")
    );
}

#[test]
fn workspace_update_release_platform_rejects_macos_and_unknown_architectures() {
    assert_eq!(
        release_platform_prefixes_for("linux", "x86_64"),
        Some("ubuntu-x86_64-")
    );
    assert_eq!(
        release_platform_prefixes_for("linux", "aarch64"),
        Some("pi-aarch64-")
    );
    assert_eq!(release_platform_prefixes_for("macos", "aarch64"), None);
    assert_eq!(release_platform_prefixes_for("linux", "riscv64"), None);
}

#[test]
fn workspace_update_release_check_retries_failures_and_honors_forced_refresh() {
    let mut status = WorkspaceUpdateStatus {
        latest_release_checked_ts: Some(1_000),
        ..WorkspaceUpdateStatus::default()
    };

    assert!(!latest_release_check_due(&status, false, 1_299));
    assert!(latest_release_check_due(&status, false, 1_300));
    assert!(latest_release_check_due(&status, true, 1_001));

    status.latest_release_check_error = Some("release_lookup_timed_out".to_string());
    assert!(!latest_release_check_due(&status, false, 1_029));
    assert!(latest_release_check_due(&status, false, 1_030));
}

#[tokio::test]
async fn workspace_update_release_install_skips_git_and_reports_installation_kind() {
    let root = temp_workspace_root();
    std::fs::write(root.join(".release-tag"), "ubuntu-x86_64-test\n")
        .expect("write release marker");
    let shared = Arc::new(Mutex::new(WorkspaceUpdateStatus {
        latest_release_checked_ts: Some(current_unix_ts()),
        stderr_tail: "fatal: not a git repository".to_string(),
        ..WorkspaceUpdateStatus::default()
    }));

    let status = refresh_workspace_update_versions(&root, shared, false).await;

    assert_eq!(status.installation_kind, "release_package");
    assert!(!status.source_update_available);
    assert!(status.stderr_tail.is_empty());
    assert!(status.stdout_tail.is_empty());
    assert_eq!(status.old_commit, None);
    assert_eq!(status.remote_commit, None);
    std::fs::remove_dir_all(root).expect("remove test workspace");
}

#[test]
fn workspace_update_source_checkout_detection_supports_git_files_and_directories() {
    let directory_root = temp_workspace_root();
    std::fs::create_dir(directory_root.join(".git")).expect("create git directory");
    assert!(workspace_source_update_available(&directory_root));
    assert_eq!(
        workspace_installation_kind(&directory_root),
        "source_checkout"
    );
    std::fs::remove_dir_all(directory_root).expect("remove git directory workspace");

    let worktree_root = temp_workspace_root();
    std::fs::write(
        worktree_root.join(".git"),
        "gitdir: ../repo/.git/worktrees/test\n",
    )
    .expect("write git file");
    assert!(workspace_source_update_available(&worktree_root));
    assert_eq!(
        workspace_installation_kind(&worktree_root),
        "source_checkout"
    );
    std::fs::remove_dir_all(worktree_root).expect("remove git file workspace");
}

#[test]
fn workspace_update_start_preserves_release_lookup_state() {
    let previous = WorkspaceUpdateStatus {
        latest_release_tag: Some("pi-aarch64-20260724-4".to_string()),
        latest_release_check_status: "available".to_string(),
        latest_release_checked_ts: Some(1_234),
        ..WorkspaceUpdateStatus::default()
    };

    let started = begin_workspace_update_status(&previous, WorkspaceUpdateMode::UiOnly);

    assert_eq!(started.status, "running");
    assert_eq!(started.step, "starting");
    assert_eq!(started.mode, "ui_only");
    assert_eq!(
        started.latest_release_tag.as_deref(),
        Some("pi-aarch64-20260724-4")
    );
    assert_eq!(started.latest_release_check_status, "available");
    assert_eq!(started.latest_release_checked_ts, Some(1_234));
}

#[test]
fn workspace_update_full_preserve_nginx_mode_is_explicit() {
    let previous = WorkspaceUpdateStatus::default();

    let started = begin_workspace_update_status(&previous, WorkspaceUpdateMode::FullPreserveNginx);

    assert_eq!(started.status, "running");
    assert_eq!(started.mode, "full_preserve_nginx");
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_update_cancel_terminates_the_dedicated_process_group() {
    let root = temp_workspace_root();
    let child_pid_path = root.join("workspace-update-child.pid");
    let command = format!(
        "trap '' TERM; sleep 120 & echo $! > {}; wait",
        child_pid_path.display()
    );
    let shared = Arc::new(Mutex::new(WorkspaceUpdateStatus::default()));
    let control = Arc::new(Mutex::new(WorkspaceUpdateControl::default()));
    let run_root = root.clone();
    let run_shared = shared.clone();
    let run_control = control.clone();
    let job = tokio::spawn(async move {
        run_workspace_update_command_streaming(
            "bash",
            &["-c", command.as_str()],
            &run_root,
            run_shared,
            run_control,
        )
        .await
    });

    for _ in 0..100 {
        if child_pid_path.is_file()
            && workspace_update_control_lock(control.as_ref())
                .active_child_pid
                .is_some()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(child_pid_path.is_file(), "child process did not start");
    let process_group_pid = workspace_update_control_lock(control.as_ref())
        .active_child_pid
        .expect("active process group");
    workspace_update_control_lock(control.as_ref()).cancel_requested = true;

    let result = tokio::time::timeout(std::time::Duration::from_secs(8), job)
        .await
        .expect("cancel timed out")
        .expect("join workspace update task");
    assert_eq!(result.unwrap_err(), WORKSPACE_UPDATE_CANCELED_ERROR);
    assert_eq!(
        workspace_update_status_lock(shared.as_ref()).status,
        "canceled"
    );

    let process_group = i32::try_from(process_group_pid).expect("process group fits i32");
    for _ in 0..100 {
        if unsafe { libc::kill(-process_group, 0) } != 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_ne!(
        unsafe { libc::kill(-process_group, 0) },
        0,
        "build process group survived cancellation"
    );

    std::fs::remove_dir_all(root).expect("remove temp workspace");
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_update_command_cleans_detached_processes_after_parent_exit() {
    let root = temp_workspace_root();
    let child_pid_path = root.join("workspace-update-detached-child.pid");
    let command = format!(
        "trap '' TERM; sleep 120 >/dev/null 2>&1 & echo $! > {}",
        child_pid_path.display()
    );
    let shared = Arc::new(Mutex::new(WorkspaceUpdateStatus::default()));
    let control = Arc::new(Mutex::new(WorkspaceUpdateControl::default()));

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        run_workspace_update_command_streaming(
            "bash",
            &["-c", command.as_str()],
            &root,
            shared,
            control.clone(),
        ),
    )
    .await
    .expect("detached process cleanup timed out")
    .expect("run workspace update command");
    assert_eq!(result.exit_code, Some(0));
    assert!(child_pid_path.is_file(), "detached child did not start");
    assert!(
        workspace_update_control_lock(control.as_ref())
            .active_child_pid
            .is_none(),
        "active process group was not cleared"
    );

    let child_pid = std::fs::read_to_string(&child_pid_path)
        .expect("read detached child pid")
        .trim()
        .parse::<i32>()
        .expect("parse detached child pid");
    for _ in 0..100 {
        if unsafe { libc::kill(child_pid, 0) } != 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_ne!(
        unsafe { libc::kill(child_pid, 0) },
        0,
        "detached build child survived command completion"
    );

    std::fs::remove_dir_all(root).expect("remove temp workspace");
}

#[test]
fn workspace_update_source_checkout_mode_preserves_installation_state() {
    let previous = WorkspaceUpdateStatus {
        installation_kind: "release_package".to_string(),
        source_update_available: false,
        ..WorkspaceUpdateStatus::default()
    };

    let started = begin_workspace_update_status(&previous, WorkspaceUpdateMode::SourceCheckout);

    assert_eq!(started.status, "running");
    assert_eq!(started.mode, "source_checkout");
    assert_eq!(started.installation_kind, "release_package");
    assert!(!started.source_update_available);
}

#[test]
fn workspace_update_release_lookup_errors_control_cached_tag_reuse() {
    assert!(LatestReleaseLookupError::RequestTimedOut.can_use_cached_tag());
    assert!(LatestReleaseLookupError::HttpStatus.can_use_cached_tag());
    assert!(!LatestReleaseLookupError::CompatibleReleaseNotFound.can_use_cached_tag());
    assert!(!LatestReleaseLookupError::UnsupportedPlatform.can_use_cached_tag());
    assert_eq!(
        LatestReleaseLookupError::CompatibleReleaseNotFound.as_str(),
        "compatible_release_not_found"
    );
}

#[test]
fn workspace_update_release_deploy_uses_stable_release_and_prebuilt_ui() {
    let script = include_str!("../../../../deploy-github-release.sh");
    assert!(script.contains("release.get(\"draft\") or release.get(\"prerelease\")"));
    assert!(script.contains("checksum_name = f\"{archive_name}.sha256\""));
    assert!(script.contains("release_checksum=verified"));
    assert!(script.contains("\"$ROOT_DIR/build-ui-nginx.sh\" --copy-if-configured"));
    assert!(script.contains("rollback_deployment"));
    assert!(script.contains("--package-mode"));
    assert!(script.contains("release_package_status=enabled"));
    assert!(script.contains("PACKAGE_MODE_ORIGINAL_MOVED"));
    assert!(script.contains("NEW_CONFIG_PATHS_FILE"));
    assert!(!script.contains("rm -rf data"));
    assert!(!script.contains("cp -a \"$PACKAGE_DIR/target/release/.\""));
    assert!(!script.contains("build-ui-nginx.sh --deploy-if-configured"));
    let source_checkout_script = include_str!("../../../../scripts/switch-to-source-checkout.sh");
    assert!(source_checkout_script.contains("git clone --quiet --single-branch"));
    assert!(source_checkout_script.contains("source_checkout_status=enabled"));
    assert!(source_checkout_script.contains("mv \"$ROOT_DIR\" \"$BACKUP_DIR\""));
}

#[test]
fn workspace_update_nginx_scripts_cover_upgrade_disable_and_release_packaging() {
    let deploy_script = include_str!("../../../../deploy-ui-nginx.sh");
    assert!(deploy_script.contains("--upgrade-nginx"));
    assert!(deploy_script.contains("brew upgrade nginx"));
    assert!(deploy_script.contains("apt-get install -y nginx"));
    assert!(deploy_script.contains("apk add --upgrade nginx"));

    let build_script = include_str!("../../../../build-all.sh");
    assert!(build_script.contains("preserve-nginx"));
    assert!(build_script.contains("Preserving nginx as requested"));
    assert!(build_script.contains("APP_PRESERVE_NGINX"));

    let disable_script = include_str!("../../../../scripts/disable-nginx-web.sh");
    assert!(disable_script.contains("brew services stop nginx"));
    assert!(disable_script.contains("systemctl disable --now nginx"));
    assert!(disable_script.contains("Agent Runtime UI"));
    assert!(disable_script.contains("*/\"$APP_DATA_NAMESPACE\"|*/nginx-ui"));
    assert!(disable_script.contains("Refusing to delete non-dedicated UI root"));

    let package_script = include_str!("../../../../package-release.sh");
    assert!(package_script.contains("copy_if_exists \"deploy-ui-nginx.sh\""));
    assert!(package_script.contains("copy_if_exists \"scripts\""));
}

#[test]
fn workspace_update_systemd_unit_name_accepts_only_machine_tokens() {
    assert!(is_safe_systemd_unit_name("agent-runtime.service"));
    assert!(is_safe_systemd_unit_name("agent-runtime-worker@1.service"));
    assert!(!is_safe_systemd_unit_name(""));
    assert!(!is_safe_systemd_unit_name("agent-runtime.service; reboot"));
    assert!(!is_safe_systemd_unit_name("agent-runtime service"));
}

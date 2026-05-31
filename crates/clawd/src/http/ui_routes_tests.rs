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
        "https://api.minimax.io/v1",
        "old-key",
        "minimax",
        "MiniMax-M2.7",
        "openai_compat",
        "https://api.minimax.io/v1",
        "new-key",
    ));
}

#[test]
fn llm_runtime_differs_when_only_base_url_changes() {
    assert!(llm_runtime_differs(
        "minimax",
        "MiniMax-M2.7",
        "openai_compat",
        "https://api.minimax.io/v1",
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
        "https://api.minimax.io/v1",
        "same-key",
        "minimax",
        "MiniMax-M2.7",
        "openai_compat",
        "https://api.minimax.io/v1",
        "same-key",
    ));
}

#[test]
fn llm_runtime_differs_when_only_minimax_provider_type_changes() {
    assert!(llm_runtime_differs(
        "minimax",
        "MiniMax-M2.7",
        "anthropic_claude",
        "https://api.minimax.io/v1",
        "same-key",
        "minimax",
        "MiniMax-M2.7",
        "openai_compat",
        "https://api.minimax.io/v1",
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
base_url = "https://api.minimax.io/v1"
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

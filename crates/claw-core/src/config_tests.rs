use super::{AppConfig, ToolsConfig};
use std::fs;

fn unique_temp_config_dir(name: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "rustclaw-claw-core-config-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos()
    ));
    dir
}

#[test]
fn app_config_load_allows_missing_telegram_split_config() {
    let dir = unique_temp_config_dir("missing-telegram");
    fs::create_dir_all(&dir).expect("create temp config dir");
    let config_path = dir.join("config.toml");
    fs::write(
        &config_path,
        r#"
[server]
listen = "127.0.0.1:0"
request_timeout_seconds = 30

[database]
sqlite_path = "data/test.db"
busy_timeout_ms = 2000

[worker]
"#,
    )
    .expect("write temp config");

    let cfg = AppConfig::load(config_path.to_str().expect("utf-8 temp path"))
        .expect("config without telegram split file should load");

    assert!(cfg.telegram.bot_token.is_empty());
    assert_eq!(cfg.telegram.agent_id, "main");
    assert!(cfg.telegram_runtime_bots().is_empty());

    fs::remove_dir_all(dir).expect("remove temp config dir");
}

#[test]
fn app_config_load_defaults_mcp_boundary_closed() {
    let dir = unique_temp_config_dir("mcp-default");
    fs::create_dir_all(&dir).expect("create temp config dir");
    let config_path = dir.join("config.toml");
    fs::write(
        &config_path,
        r#"
[server]
listen = "127.0.0.1:0"
request_timeout_seconds = 30

[database]
sqlite_path = "data/test.db"
busy_timeout_ms = 2000

[worker]
"#,
    )
    .expect("write temp config");

    let cfg = AppConfig::load(config_path.to_str().expect("utf-8 temp path"))
        .expect("config without mcp table should load");

    assert!(!cfg.mcp.enabled);
    assert!(cfg.mcp.servers.is_empty());
    assert!(cfg.mcp.enabled_server_names().is_empty());

    fs::remove_dir_all(dir).expect("remove temp config dir");
}

#[test]
fn app_config_loads_mcp_server_boundary_without_runtime_effect() {
    let dir = unique_temp_config_dir("mcp-configured");
    fs::create_dir_all(&dir).expect("create temp config dir");
    let config_path = dir.join("config.toml");
    fs::write(
        &config_path,
        r#"
[server]
listen = "127.0.0.1:0"
request_timeout_seconds = 30

[database]
sqlite_path = "data/test.db"
busy_timeout_ms = 2000

[worker]

[mcp]
enabled = true

[mcp.servers.repo]
enabled = true
transport = "stdio"
command = "mcp-repo"
args = ["--workspace", "."]
timeout_seconds = 45
capability_prefix = "repo"
allowed_tools = ["search", "read"]

[mcp.servers.disabled]
enabled = false
transport = "sse"
url = "http://127.0.0.1:9000/events"
"#,
    )
    .expect("write temp config");

    let cfg = AppConfig::load(config_path.to_str().expect("utf-8 temp path"))
        .expect("config with mcp table should load");

    assert!(cfg.mcp.enabled);
    assert_eq!(cfg.mcp.enabled_server_names(), vec!["repo".to_string()]);
    let repo = cfg.mcp.servers.get("repo").expect("repo server");
    assert_eq!(repo.transport.as_token(), "stdio");
    assert_eq!(repo.command.as_deref(), Some("mcp-repo"));
    assert_eq!(repo.timeout_seconds, 45);
    assert_eq!(repo.capability_prefix.as_deref(), Some("repo"));
    assert_eq!(
        repo.allowed_tools,
        vec!["search".to_string(), "read".to_string()]
    );
    let disabled = cfg.mcp.servers.get("disabled").expect("disabled server");
    assert_eq!(disabled.transport.as_token(), "sse");
    assert_eq!(
        disabled.url.as_deref(),
        Some("http://127.0.0.1:9000/events")
    );

    fs::remove_dir_all(dir).expect("remove temp config dir");
}

#[test]
fn tools_defaults_are_least_privilege_coding_defaults() {
    let tools = ToolsConfig::default();
    assert_eq!(tools.access_profile, "coding");
    assert_eq!(tools.sandbox_mode.as_token(), "workspace_write");
    assert_eq!(tools.approval_policy.as_token(), "on_risk");
    assert!(tools.allow.is_empty());
    assert!(tools.deny.is_empty());
    assert!(!tools.allow_sudo);
    assert!(!tools.allow_path_outside_workspace);
}

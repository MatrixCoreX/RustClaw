use super::{base_skill_names, core_skills_always_enabled, AppConfig, ToolsConfig};
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
    assert_eq!(cfg.mcp.planner_visible_tools, 32);
    assert_eq!(cfg.mcp.catalog_search_max_results, 20);
    assert!(cfg.mcp.servers.is_empty());
    assert!(cfg.mcp.enabled_server_names().is_empty());

    fs::remove_dir_all(dir).expect("remove temp config dir");
}

#[test]
fn app_config_loads_mcp_server_runtime_policy_boundary() {
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
planner_visible_tools = 16
catalog_search_max_results = 8

[mcp.servers.repo]
enabled = true
transport = "stdio"
command = "mcp-repo"
args = ["--workspace", "."]
timeout_seconds = 45
max_concurrency = 3
max_output_bytes = 8192
max_schema_bytes = 4096
max_tools = 12
health_check_seconds = 15
reconnect_base_seconds = 3
reconnect_max_seconds = 45
trusted = true
capability_prefix = "repo"
allowed_tools = ["search", "read"]
auth_token_env = "RUSTCLAW_TEST_MCP_TOKEN"
env_refs = { REPO_TOKEN = "RUSTCLAW_TEST_REPO_TOKEN" }

[mcp.servers.repo.tool_policies.search]
effect = "observe"
risk_level = "low"
idempotent = true
network_access = false

[mcp.servers.disabled]
enabled = false
transport = "sse"
url = "http://127.0.0.1:9000/events"

[mcp.servers.oauth]
enabled = false
transport = "streamable_http"
url = "https://mcp.example.invalid/mcp"
oauth_client_id_env = "RUSTCLAW_TEST_MCP_OAUTH_CLIENT_ID"
oauth_client_secret_env = "RUSTCLAW_TEST_MCP_OAUTH_CLIENT_SECRET"
oauth_scopes = ["read", "write"]
oauth_resource = "https://mcp.example.invalid/mcp"
"#,
    )
    .expect("write temp config");

    let cfg = AppConfig::load(config_path.to_str().expect("utf-8 temp path"))
        .expect("config with mcp table should load");

    assert!(cfg.mcp.enabled);
    assert_eq!(cfg.mcp.planner_visible_tools, 16);
    assert_eq!(cfg.mcp.catalog_search_max_results, 8);
    assert_eq!(cfg.mcp.enabled_server_names(), vec!["repo".to_string()]);
    let repo = cfg.mcp.servers.get("repo").expect("repo server");
    assert_eq!(repo.transport.as_token(), "stdio");
    assert_eq!(repo.command.as_deref(), Some("mcp-repo"));
    assert_eq!(repo.timeout_seconds, 45);
    assert_eq!(repo.max_concurrency, 3);
    assert_eq!(repo.max_output_bytes, 8192);
    assert_eq!(repo.max_schema_bytes, 4096);
    assert_eq!(repo.max_tools, 12);
    assert_eq!(repo.health_check_seconds, 15);
    assert_eq!(repo.reconnect_base_seconds, 3);
    assert_eq!(repo.reconnect_max_seconds, 45);
    assert!(repo.trusted);
    assert_eq!(repo.capability_prefix.as_deref(), Some("repo"));
    assert_eq!(
        repo.auth_token_env.as_deref(),
        Some("RUSTCLAW_TEST_MCP_TOKEN")
    );
    assert_eq!(
        repo.env_refs.get("REPO_TOKEN").map(String::as_str),
        Some("RUSTCLAW_TEST_REPO_TOKEN")
    );
    assert_eq!(
        repo.allowed_tools,
        vec!["search".to_string(), "read".to_string()]
    );
    let search = repo.tool_policies.get("search").expect("search policy");
    assert_eq!(search.effect.as_token(), "observe");
    assert_eq!(search.risk_level.as_token(), "low");
    assert!(search.idempotent);
    assert!(!search.network_access);
    let disabled = cfg.mcp.servers.get("disabled").expect("disabled server");
    assert_eq!(disabled.transport.as_token(), "sse");
    assert_eq!(
        disabled.url.as_deref(),
        Some("http://127.0.0.1:9000/events")
    );
    let oauth = cfg.mcp.servers.get("oauth").expect("oauth server");
    assert_eq!(oauth.auth_mode_token(), "oauth_client_credentials");
    assert_eq!(
        oauth.oauth_client_id_env.as_deref(),
        Some("RUSTCLAW_TEST_MCP_OAUTH_CLIENT_ID")
    );
    assert_eq!(
        oauth.oauth_client_secret_env.as_deref(),
        Some("RUSTCLAW_TEST_MCP_OAUTH_CLIENT_SECRET")
    );
    assert_eq!(oauth.oauth_scopes, vec!["read", "write"]);
    assert_eq!(
        oauth.oauth_resource.as_deref(),
        Some("https://mcp.example.invalid/mcp")
    );

    fs::remove_dir_all(dir).expect("remove temp config dir");
}

#[test]
fn tools_defaults_are_least_privilege_coding_defaults() {
    let tools = ToolsConfig::default();
    assert_eq!(tools.access_profile, "coding");
    assert_eq!(tools.sandbox_mode.as_token(), "workspace_write");
    assert_eq!(tools.sandbox_backend.as_token(), "auto");
    assert_eq!(tools.approval_policy.as_token(), "on_risk");
    assert!(tools.allow.is_empty());
    assert!(tools.deny.is_empty());
    assert!(!tools.allow_sudo);
    assert!(!tools.allow_path_outside_workspace);
}

#[test]
fn default_workflow_and_knowledge_skills_are_always_on_in_ui() {
    for skill in ["schedule", "extension_manager", "kb"] {
        assert!(base_skill_names().contains(&skill));
        assert!(core_skills_always_enabled().contains(&skill));
    }
}

use super::runtime::apply_llm_vendor_api_key_envs_with;
use super::{
    llm_vendor_api_key_env_names, AppConfig, LlmVendorConfig, MemoryConfig, SkillsConfig,
    ToolsConfig, WorkspaceInstructionsConfig,
};
use std::fs;

fn unique_temp_config_dir(name: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "agent-runtime-claw-core-config-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos()
    ));
    dir
}

#[test]
fn llm_vendor_api_key_env_names_are_shared_by_runtime_and_ui_status() {
    assert_eq!(
        llm_vendor_api_key_env_names("minimax"),
        &["MINIMAX_API_KEY"]
    );
    assert_eq!(
        llm_vendor_api_key_env_names("MIMO"),
        &["XIAOMI_API_KEY", "MIMO_API_KEY"]
    );
    assert!(llm_vendor_api_key_env_names("unknown").is_empty());
}

#[test]
fn llm_api_key_from_toml_is_not_runtime_authority() {
    let mut provider = Some(
        toml::from_str::<LlmVendorConfig>(
            r#"
base_url = "https://api.minimaxi.com/v1"
api_key = "legacy-config-secret"
model = "MiniMax-M3"
"#,
        )
        .expect("parse vendor config"),
    );
    apply_llm_vendor_api_key_envs_with(&mut provider, "minimax", |_| None);
    assert!(provider
        .as_ref()
        .expect("minimax config")
        .api_key
        .is_empty());

    apply_llm_vendor_api_key_envs_with(&mut provider, "minimax", |_| {
        Some("REPLACE_ME_MINIMAX_API_KEY".to_string())
    });
    assert!(provider
        .as_ref()
        .expect("minimax config")
        .api_key
        .is_empty());

    apply_llm_vendor_api_key_envs_with(&mut provider, "minimax", |name| {
        (name == "MINIMAX_API_KEY").then(|| " environment-secret ".to_string())
    });
    assert_eq!(
        provider.expect("minimax config").api_key,
        "environment-secret"
    );
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
    assert_eq!(cfg.telegram.update_mode, "polling");
    assert_eq!(cfg.telegram.webhook_listen, "127.0.0.1:8090");
    assert!(cfg.telegram.webhook_public_url.is_empty());
    assert_eq!(cfg.telegram.webhook_secret_env, "TELEGRAM_WEBHOOK_SECRET");
    assert!(cfg.telegram_runtime_bots().is_empty());

    fs::remove_dir_all(dir).expect("remove temp config dir");
}

#[test]
fn whatsapp_web_defaults_keep_proactive_delivery_off_and_local_limits_explicit() {
    let config = super::WhatsappWebConfig::default();
    assert!(!config.enabled);
    assert!(!config.allow_proactive_send);
    assert_eq!(config.max_outbound_image_bytes, 100 * 1024 * 1024);
    assert_eq!(config.max_outbound_video_bytes, 100 * 1024 * 1024);
    assert_eq!(config.max_outbound_audio_bytes, 100 * 1024 * 1024);
    assert_eq!(config.max_outbound_file_bytes, 2 * 1024 * 1024 * 1024);
}

#[test]
fn app_config_loads_whatsapp_cloud_and_web_from_separate_files() {
    let dir = unique_temp_config_dir("split-whatsapp");
    fs::create_dir_all(dir.join("channels")).expect("create temp channel config dir");
    let config_path = dir.join("config.toml");
    fs::write(
        &config_path,
        r#"
[server]
request_timeout_seconds = 30

[database]
sqlite_path = "data/test.db"
busy_timeout_ms = 2000

[worker]
"#,
    )
    .expect("write base config");
    fs::write(
        dir.join("channels/whatsapp-cloud.toml"),
        r#"
[whatsapp]
enabled = false
webhook_path = "/split-cloud"
image_inbox_dir = "image/split-cloud"

[whatsapp_cloud]
enabled = true
webhook_path = "/split-cloud-capability"
"#,
    )
    .expect("write WhatsApp Cloud config");
    fs::write(
        dir.join("channels/whatsapp-web.toml"),
        r#"
[whatsapp_web]
enabled = true
bridge_listen = "127.0.0.1:18092"
auth_dir = "data/split-wa-web-auth"
allow_proactive_send = false
"#,
    )
    .expect("write WhatsApp Web config");

    let cfg = AppConfig::load(config_path.to_str().expect("utf-8 temp path"))
        .expect("split WhatsApp configs should load together");

    assert!(!cfg.whatsapp.enabled);
    assert_eq!(cfg.whatsapp.webhook_path, "/split-cloud");
    assert_eq!(cfg.whatsapp.image_inbox_dir, "image/split-cloud");
    assert!(cfg.whatsapp_cloud.enabled);
    assert_eq!(cfg.whatsapp_cloud.webhook_path, "/split-cloud-capability");
    assert!(cfg.whatsapp_web.enabled);
    assert_eq!(cfg.whatsapp_web.bridge_listen, "127.0.0.1:18092");
    assert_eq!(cfg.whatsapp_web.auth_dir, "data/split-wa-web-auth");
    assert!(!cfg.whatsapp_web.allow_proactive_send);

    fs::remove_dir_all(dir).expect("remove temp config dir");
}

#[test]
fn agents_toml_is_the_canonical_agent_source() {
    let dir = unique_temp_config_dir("canonical-agents");
    fs::create_dir_all(dir.join("channels")).expect("create temp config dir");
    let config_path = dir.join("config.toml");
    fs::write(
        &config_path,
        r#"
[server]
request_timeout_seconds = 30

[database]
sqlite_path = "data/test.db"
busy_timeout_ms = 2000

[worker]
"#,
    )
    .expect("write base config");
    fs::write(
        dir.join("channels/telegram.toml"),
        r#"
[[agents]]
id = "legacy"
name = "Legacy"
"#,
    )
    .expect("write legacy channel agents");
    fs::write(
        dir.join("agents.toml"),
        r#"
schema_version = 1

[[agents]]
id = "main"
name = "Primary"
persona_profile = "teacher"
"#,
    )
    .expect("write canonical agents");

    let cfg = AppConfig::load(config_path.to_str().expect("utf-8 temp path"))
        .expect("canonical agents config should load");
    let agents = cfg.normalized_agents();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].id, "main");
    assert_eq!(agents[0].name, "Primary");
    assert_eq!(agents[0].persona_profile, "teacher");

    fs::remove_dir_all(dir).expect("remove temp config dir");
}

#[test]
fn legacy_persona_prompt_maps_to_custom_and_unknown_profile_falls_back() {
    let dir = unique_temp_config_dir("agent-persona-compat");
    fs::create_dir_all(&dir).expect("create temp config dir");
    let config_path = dir.join("config.toml");
    fs::write(
        &config_path,
        r#"
[server]
request_timeout_seconds = 30

[database]
sqlite_path = "data/test.db"
busy_timeout_ms = 2000

[worker]

[[agents]]
id = "legacy"
persona_prompt = "语气温和"

[[agents]]
id = "unknown"
persona_profile = "not-a-profile"
"#,
    )
    .expect("write compatibility config");

    let cfg = AppConfig::load(config_path.to_str().expect("utf-8 temp path"))
        .expect("legacy agent config should load");
    let agents = cfg.normalized_agents();
    let legacy = agents
        .iter()
        .find(|agent| agent.id == "legacy")
        .expect("legacy agent");
    assert_eq!(legacy.persona_profile, "custom");
    assert_eq!(legacy.persona_fragment, "语气温和");
    assert!(legacy.persona_prompt.is_empty());
    let unknown = agents
        .iter()
        .find(|agent| agent.id == "unknown")
        .expect("unknown agent");
    assert_eq!(unknown.persona_profile, "inherit");

    fs::remove_dir_all(dir).expect("remove temp config dir");
}

#[test]
fn removed_worker_timeout_and_model_budget_fields_are_ignored_without_runtime_authority() {
    let dir = unique_temp_config_dir("legacy-worker-model-budget");
    fs::create_dir_all(&dir).expect("create temp config dir");
    let config_path = dir.join("config.toml");
    fs::write(
        &config_path,
        r#"
[server]
request_timeout_seconds = 30

[database]
sqlite_path = "data/test.db"
busy_timeout_ms = 2000

[worker]
task_timeout_seconds = 1
llm_max_calls_per_task = 40
llm_total_timeout_seconds = 900
"#,
    )
    .expect("write temp config");

    let cfg = AppConfig::load(config_path.to_str().expect("utf-8 temp path"))
        .expect("removed worker timeout and model-budget fields remain readable");
    assert_eq!(cfg.worker.concurrency, 1);

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
auth_token_env = "APP_TEST_MCP_TOKEN"
env_refs = { REPO_TOKEN = "APP_TEST_REPO_TOKEN" }

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
oauth_client_id_env = "APP_TEST_MCP_OAUTH_CLIENT_ID"
oauth_client_secret_env = "APP_TEST_MCP_OAUTH_CLIENT_SECRET"
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
    assert_eq!(repo.auth_token_env.as_deref(), Some("APP_TEST_MCP_TOKEN"));
    assert_eq!(
        repo.env_refs.get("REPO_TOKEN").map(String::as_str),
        Some("APP_TEST_REPO_TOKEN")
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
        Some("APP_TEST_MCP_OAUTH_CLIENT_ID")
    );
    assert_eq!(
        oauth.oauth_client_secret_env.as_deref(),
        Some("APP_TEST_MCP_OAUTH_CLIENT_SECRET")
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
    assert_eq!(tools.admin_access_profile, "full");
    assert_eq!(tools.sandbox_mode.as_token(), "workspace_write");
    assert_eq!(tools.sandbox_backend.as_token(), "auto");
    assert_eq!(tools.approval_policy.as_token(), "on_risk");
    assert!(tools.allow.is_empty());
    assert!(tools.deny.is_empty());
    assert!(!tools.allow_sudo);
    assert!(!tools.allow_path_outside_workspace);
}

#[test]
fn workspace_instruction_config_is_disabled_by_default_and_validates_enabled_budgets() {
    let disabled = WorkspaceInstructionsConfig::default();
    assert!(disabled.validate().is_ok());

    let enabled = WorkspaceInstructionsConfig {
        enabled_for_coding: true,
        enabled_for_non_coding: false,
        user_instruction_paths: Vec::new(),
        filenames: vec!["AGENTS.md".to_string()],
        max_total_bytes: 32_768,
        max_file_bytes: 131_072,
        max_files: 16,
    };
    assert!(enabled.validate().is_ok());
    assert_eq!(
        WorkspaceInstructionsConfig {
            filenames: vec!["../AGENTS.md".to_string()],
            ..enabled.clone()
        }
        .validate(),
        Err("workspace_instruction_filename_invalid".to_string())
    );
    assert_eq!(
        WorkspaceInstructionsConfig {
            filenames: vec!["AGENTS.md".to_string(), "AGENTS.md".to_string()],
            ..enabled.clone()
        }
        .validate(),
        Err("workspace_instruction_filenames_duplicate".to_string())
    );
    assert_eq!(
        WorkspaceInstructionsConfig {
            max_total_bytes: 0,
            ..enabled
        }
        .validate(),
        Err("workspace_instruction_budget_invalid".to_string())
    );
}

#[test]
fn skill_config_defaults_do_not_duplicate_registry_membership() {
    let skills = SkillsConfig::default();
    assert!(skills.skills_list.is_empty());
    assert!(skills.uninstalled_skills.is_empty());
    assert_eq!(
        skills.registry_path.as_deref(),
        Some("configs/skills_registry.toml")
    );
}

#[test]
fn memory_defaults_are_projected_from_the_tracked_release_toml() {
    let defaults = MemoryConfig::default();
    let tracked: MemoryConfig = toml::from_str(include_str!("../../../configs/memory.toml"))
        .expect("tracked memory config");
    assert_eq!(defaults.config_path, "configs/memory.toml");
    assert_eq!(defaults.recall_limit, tracked.recall_limit);
    assert_eq!(defaults.item_max_chars, tracked.item_max_chars);
    assert_eq!(defaults.prompt_max_chars, tracked.prompt_max_chars);
    assert_eq!(defaults.retention_days, tracked.retention_days);
    assert_eq!(defaults.max_rows, tracked.max_rows);
    assert_eq!(defaults.embedding_model, tracked.embedding_model);
    assert_eq!(defaults.embedding_version, tracked.embedding_version);
    assert_eq!(defaults.embedding_metric, tracked.embedding_metric);
    assert_eq!(
        defaults.embedding_query_cache_max_bytes,
        tracked.embedding_query_cache_max_bytes
    );
    assert_eq!(
        defaults.embedding_circuit_failure_threshold,
        tracked.embedding_circuit_failure_threshold
    );
}

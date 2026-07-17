use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use claw_core::config::{AppConfig, McpConfig, McpServerConfig};
use claw_core::types::ApiResponse;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path as FsPath, PathBuf};
use toml_edit::{value, Array, DocumentMut, Item, Table, Value as TomlValue};

use crate::AppState;

#[derive(Debug, Default, Deserialize)]
pub(super) struct McpToolsQuery {
    server_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct UpdateMcpConfigRequest {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    servers: Vec<McpServerUpdate>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct McpServerUpdate {
    #[serde(default)]
    server_id: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    trusted: bool,
    #[serde(default)]
    transport: String,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env_refs: BTreeMap<String, String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    auth_token_env: Option<String>,
    #[serde(default)]
    oauth_client_id_env: Option<String>,
    #[serde(default)]
    oauth_client_secret_env: Option<String>,
    #[serde(default)]
    oauth_scopes: Vec<String>,
    #[serde(default)]
    oauth_resource: Option<String>,
    #[serde(default)]
    allowed_tools: Vec<String>,
}

#[derive(Debug, Serialize)]
struct McpConfigView {
    config_path: &'static str,
    enabled: bool,
    restart_required: bool,
    servers: Vec<McpServerConfigView>,
}

#[derive(Debug, Serialize)]
struct McpServerConfigView {
    server_id: String,
    enabled: bool,
    trusted: bool,
    transport: String,
    command: Option<String>,
    args: Vec<String>,
    env_refs: BTreeMap<String, String>,
    url: Option<String>,
    auth_token_env: Option<String>,
    oauth_client_id_env: Option<String>,
    oauth_client_secret_env: Option<String>,
    oauth_scopes: Vec<String>,
    oauth_resource: Option<String>,
    allowed_tools: Vec<String>,
    has_static_env: bool,
    has_advanced_policy: bool,
}

fn require_mcp_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ApiResponse<Value>>)> {
    let identity = crate::http::ui_routes::require_ui_identity(state, headers)?;
    if identity.role.eq_ignore_ascii_case("admin") {
        Ok(())
    } else {
        Err(super::api_err(StatusCode::FORBIDDEN, "mcp_admin_required"))
    }
}

fn normalized_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalized_list(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn normalize_update_request(
    mut request: UpdateMcpConfigRequest,
) -> Result<UpdateMcpConfigRequest, &'static str> {
    let mut server_ids = BTreeSet::new();
    for server in &mut request.servers {
        server.server_id = server.server_id.trim().to_string();
        if server.server_id.is_empty() {
            return Err("mcp_server_id_required");
        }
        if !server_ids.insert(server.server_id.clone()) {
            return Err("mcp_server_id_duplicate");
        }
        server.transport = server.transport.trim().to_ascii_lowercase();
        if !matches!(server.transport.as_str(), "stdio" | "streamable_http") {
            return Err("mcp_transport_unsupported");
        }
        server.command = normalized_optional(server.command.take());
        server.url = normalized_optional(server.url.take());
        server.auth_token_env = normalized_optional(server.auth_token_env.take());
        server.oauth_client_id_env = normalized_optional(server.oauth_client_id_env.take());
        server.oauth_client_secret_env = normalized_optional(server.oauth_client_secret_env.take());
        server.oauth_resource = normalized_optional(server.oauth_resource.take());
        server.args = normalized_list(std::mem::take(&mut server.args));
        server.oauth_scopes = normalized_list(std::mem::take(&mut server.oauth_scopes));
        server.allowed_tools = normalized_list(std::mem::take(&mut server.allowed_tools));
        let mut env_refs = BTreeMap::new();
        for (name, env_ref) in std::mem::take(&mut server.env_refs) {
            let name = name.trim().to_string();
            let env_ref = env_ref.trim().to_string();
            if name.is_empty() || env_ref.is_empty() {
                return Err("mcp_stdio_env_ref_invalid");
            }
            if env_refs.insert(name, env_ref).is_some() {
                return Err("mcp_stdio_env_ref_duplicate");
            }
        }
        server.env_refs = env_refs;
        if server.auth_token_env.is_some()
            && (server.oauth_client_id_env.is_some() || server.oauth_client_secret_env.is_some())
        {
            return Err("mcp_http_auth_conflict");
        }
        if server.transport == "stdio" {
            server.url = None;
            server.auth_token_env = None;
            server.oauth_client_id_env = None;
            server.oauth_client_secret_env = None;
            server.oauth_scopes.clear();
            server.oauth_resource = None;
        } else {
            server.command = None;
            server.args.clear();
            server.env_refs.clear();
        }
    }
    request
        .servers
        .sort_by(|left, right| left.server_id.cmp(&right.server_id));
    Ok(request)
}

fn config_view(config: &McpConfig, restart_required: bool) -> McpConfigView {
    let mut servers = config
        .servers
        .iter()
        .map(|(server_id, server)| server_config_view(server_id, server))
        .collect::<Vec<_>>();
    servers.sort_by(|left, right| left.server_id.cmp(&right.server_id));
    McpConfigView {
        config_path: "configs/config.toml",
        enabled: config.enabled,
        restart_required,
        servers,
    }
}

fn server_config_view(server_id: &str, server: &McpServerConfig) -> McpServerConfigView {
    McpServerConfigView {
        server_id: server_id.to_string(),
        enabled: server.enabled,
        trusted: server.trusted,
        transport: server.transport.as_token().to_string(),
        command: server.command.clone(),
        args: server.args.clone(),
        env_refs: server.env_refs.clone().into_iter().collect(),
        url: server.url.clone(),
        auth_token_env: server.auth_token_env.clone(),
        oauth_client_id_env: server.oauth_client_id_env.clone(),
        oauth_client_secret_env: server.oauth_client_secret_env.clone(),
        oauth_scopes: server.oauth_scopes.clone(),
        oauth_resource: server.oauth_resource.clone(),
        allowed_tools: server.allowed_tools.clone(),
        has_static_env: !server.env.is_empty(),
        has_advanced_policy: !server.tool_policies.is_empty() || server.capability_prefix.is_some(),
    }
}

fn string_array_item(values: &[String]) -> Item {
    let mut array = Array::new();
    for value in values {
        array.push(value.as_str());
    }
    Item::Value(TomlValue::Array(array))
}

fn set_optional_string(table: &mut Table, key: &str, field: &Option<String>) {
    if let Some(field) = field {
        table.insert(key, value(field));
    } else {
        table.remove(key);
    }
}

fn render_mcp_config_update(
    raw: &str,
    request: UpdateMcpConfigRequest,
) -> Result<(String, AppConfig), &'static str> {
    let request = normalize_update_request(request)?;
    let mut document = raw
        .parse::<DocumentMut>()
        .map_err(|_| "mcp_config_parse_failed")?;
    if !document.as_table().contains_key("mcp") {
        document["mcp"] = Item::Table(Table::new());
    }
    let mcp = document["mcp"]
        .as_table_mut()
        .ok_or("mcp_config_section_invalid")?;
    mcp.insert("enabled", value(request.enabled));
    let existing_servers = mcp
        .get("servers")
        .and_then(Item::as_table)
        .cloned()
        .unwrap_or_default();
    let mut next_servers = Table::new();
    for server in request.servers {
        let mut table = existing_servers
            .get(&server.server_id)
            .and_then(Item::as_table)
            .cloned()
            .unwrap_or_default();
        table.insert("enabled", value(server.enabled));
        table.insert("trusted", value(server.trusted));
        table.insert("transport", value(server.transport));
        set_optional_string(&mut table, "command", &server.command);
        table.insert("args", string_array_item(&server.args));
        set_optional_string(&mut table, "url", &server.url);
        set_optional_string(&mut table, "auth_token_env", &server.auth_token_env);
        set_optional_string(
            &mut table,
            "oauth_client_id_env",
            &server.oauth_client_id_env,
        );
        set_optional_string(
            &mut table,
            "oauth_client_secret_env",
            &server.oauth_client_secret_env,
        );
        table.insert("oauth_scopes", string_array_item(&server.oauth_scopes));
        set_optional_string(&mut table, "oauth_resource", &server.oauth_resource);
        table.insert("allowed_tools", string_array_item(&server.allowed_tools));
        if server.env_refs.is_empty() {
            table.remove("env_refs");
        } else {
            let mut env_refs = Table::new();
            for (name, env_ref) in server.env_refs {
                env_refs.insert(&name, value(env_ref));
            }
            table.insert("env_refs", Item::Table(env_refs));
        }
        next_servers.insert(&server.server_id, Item::Table(table));
    }
    mcp.insert("servers", Item::Table(next_servers));
    let updated = document.to_string();
    let parsed = toml::from_str::<AppConfig>(&updated).map_err(|_| "mcp_config_invalid")?;
    crate::mcp_runtime::McpRuntime::validate_configuration(&parsed.mcp)?;
    Ok((updated, parsed))
}

fn stage_config_file(path: &FsPath, raw: &str) -> std::io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("config.toml");
    let staged = path.with_file_name(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staged)?;
    file.write_all(raw.as_bytes())?;
    file.sync_all()?;
    Ok(staged)
}

fn write_mcp_config(
    workspace_root: &FsPath,
    active_raw: &str,
    mounted_raw: &str,
) -> std::io::Result<()> {
    let active = workspace_root.join("configs/config.toml");
    let mounted = workspace_root.join("docker/config/config.toml");
    let active_staged = stage_config_file(&active, active_raw)?;
    let mounted_staged = match stage_config_file(&mounted, mounted_raw) {
        Ok(path) => path,
        Err(error) => {
            let _ = std::fs::remove_file(active_staged);
            return Err(error);
        }
    };
    // Publish the deployment copy first so a failure cannot change the live config.
    if let Err(error) = std::fs::rename(&mounted_staged, &mounted) {
        let _ = std::fs::remove_file(active_staged);
        let _ = std::fs::remove_file(mounted_staged);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&active_staged, &active) {
        let _ = std::fs::remove_file(active_staged);
        return Err(error);
    }
    Ok(())
}

pub(super) async fn get_mcp_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_mcp_admin(&state, &headers) {
        return response;
    }
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let config = match AppConfig::load(&path.to_string_lossy()) {
        Ok(config) => config,
        Err(_) => {
            return super::api_err(StatusCode::INTERNAL_SERVER_ERROR, "mcp_config_read_failed")
        }
    };
    super::api_ok(json!(config_view(&config.mcp, false)))
}

pub(super) async fn update_mcp_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpdateMcpConfigRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_mcp_admin(&state, &headers) {
        return response;
    }
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(_) => {
            return super::api_err(StatusCode::INTERNAL_SERVER_ERROR, "mcp_config_read_failed")
        }
    };
    let mounted_path = state
        .skill_rt
        .workspace_root
        .join("docker/config/config.toml");
    let mounted_raw = match std::fs::read_to_string(&mounted_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => raw.clone(),
        Err(_) => {
            return super::api_err(StatusCode::INTERNAL_SERVER_ERROR, "mcp_config_read_failed")
        }
    };
    let (updated, parsed) = match render_mcp_config_update(&raw, request.clone()) {
        Ok(result) => result,
        Err(error_code) => return super::api_err(StatusCode::BAD_REQUEST, error_code),
    };
    let (mounted_updated, _) = match render_mcp_config_update(&mounted_raw, request) {
        Ok(result) => result,
        Err(error_code) => return super::api_err(StatusCode::BAD_REQUEST, error_code),
    };
    if write_mcp_config(&state.skill_rt.workspace_root, &updated, &mounted_updated).is_err() {
        return super::api_err(StatusCode::INTERNAL_SERVER_ERROR, "mcp_config_write_failed");
    }
    super::api_ok(json!(config_view(&parsed.mcp, true)))
}

pub(super) async fn list_mcp_servers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_mcp_admin(&state, &headers) {
        return response;
    }
    super::api_ok(json!({
        "servers": state.mcp_lifecycle_snapshots(),
    }))
}

pub(super) async fn list_mcp_tools(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<McpToolsQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_mcp_admin(&state, &headers) {
        return response;
    }
    let server_id = query
        .server_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let tools = state
        .mcp_tools()
        .into_iter()
        .filter(|tool| server_id.is_none_or(|server_id| tool.server_id == server_id))
        .collect::<Vec<_>>();
    super::api_ok(json!({ "tools": tools }))
}

pub(super) async fn test_mcp_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(server_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_mcp_admin(&state, &headers) {
        return response;
    }
    let server_id = server_id.trim();
    if server_id.is_empty() {
        return super::api_err(StatusCode::BAD_REQUEST, "mcp_server_id_required");
    }
    match state.probe_mcp_server(server_id).await {
        Ok(outcome) => super::api_ok(json!({ "probe": outcome })),
        Err(error_code) => {
            let status = if error_code == "mcp_server_not_configured" {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            super::api_err(status, error_code)
        }
    }
}

#[cfg(test)]
#[path = "mcp_admin_routes_tests.rs"]
mod tests;

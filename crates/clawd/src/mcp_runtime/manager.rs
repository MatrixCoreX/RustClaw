use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::time::Instant;

use claw_core::config::{McpConfig, McpServerConfig, McpTransportConfig};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use serde_json::{json, Map, Value};
use tokio::process::Command;
use tokio::sync::RwLock as AsyncRwLock;

use super::client::{McpClient, RmcpClient};
use super::types::{
    McpCallOutcome, McpLifecycleSnapshot, McpLifecycleState, McpProbeOutcome, McpRuntimeError,
    McpToolDescriptor, McpToolPolicy,
};

pub(crate) struct McpRuntime {
    config: Arc<McpConfig>,
    clients: AsyncRwLock<HashMap<String, Arc<RmcpClient>>>,
    catalog: RwLock<HashMap<String, McpToolDescriptor>>,
    lifecycle: RwLock<HashMap<String, McpLifecycleSnapshot>>,
}

impl McpRuntime {
    pub(crate) fn new(config: McpConfig) -> Self {
        let mut lifecycle = HashMap::new();
        for (server_id, server) in &config.servers {
            let enabled = config.enabled && server.enabled;
            lifecycle.insert(
                server_id.clone(),
                McpLifecycleSnapshot {
                    server_id: server_id.clone(),
                    state: if enabled {
                        McpLifecycleState::Stopped
                    } else {
                        McpLifecycleState::Disabled
                    },
                    transport: server.transport.as_token().to_string(),
                    tool_count: 0,
                    last_error_code: None,
                },
            );
        }
        Self {
            config: Arc::new(config),
            clients: AsyncRwLock::new(HashMap::new()),
            catalog: RwLock::new(HashMap::new()),
            lifecycle: RwLock::new(lifecycle),
        }
    }

    #[cfg(test)]
    pub(crate) fn disabled() -> Self {
        Self::new(McpConfig::default())
    }

    pub(crate) async fn start(&self) {
        if !self.config.enabled {
            return;
        }
        let duplicate_prefixes = self.duplicate_enabled_prefixes();
        for server_id in self.config.enabled_server_names() {
            self.set_lifecycle(&server_id, McpLifecycleState::Starting, 0, None);
            let Some(server) = self.config.servers.get(&server_id).cloned() else {
                self.set_lifecycle(
                    &server_id,
                    McpLifecycleState::Degraded,
                    0,
                    Some("mcp_server_config_missing"),
                );
                continue;
            };
            if duplicate_prefixes.contains(&server_namespace(&server_id, &server)) {
                self.set_lifecycle(
                    &server_id,
                    McpLifecycleState::Degraded,
                    0,
                    Some("mcp_capability_prefix_duplicate"),
                );
                continue;
            }
            match self.start_server(&server_id, &server).await {
                Ok(tool_count) => {
                    self.set_lifecycle(&server_id, McpLifecycleState::Ready, tool_count, None)
                }
                Err(error) => {
                    tracing::warn!(
                        server_id,
                        error_code = error.code(),
                        error_context = error.context().unwrap_or_default(),
                        "mcp_server_start_failed"
                    );
                    self.set_lifecycle(
                        &server_id,
                        McpLifecycleState::Degraded,
                        0,
                        Some(error.code()),
                    );
                }
            }
        }
    }

    pub(crate) async fn stop(&self) {
        let clients = {
            let mut guard = self.clients.write().await;
            std::mem::take(&mut *guard)
        };
        for (server_id, client) in clients {
            let error = client.shutdown().await.err();
            self.remove_server_tools(&server_id);
            self.set_lifecycle(
                &server_id,
                McpLifecycleState::Stopped,
                0,
                error.as_ref().map(McpRuntimeError::code),
            );
        }
    }

    pub(crate) fn lifecycle_snapshots(&self) -> Vec<McpLifecycleSnapshot> {
        let mut snapshots = self
            .lifecycle
            .read()
            .expect("mcp lifecycle lock poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.server_id.cmp(&right.server_id));
        snapshots
    }

    pub(crate) fn tools(&self) -> Vec<McpToolDescriptor> {
        let mut tools = self
            .catalog
            .read()
            .expect("mcp catalog lock poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| left.capability.cmp(&right.capability));
        tools
    }

    pub(crate) fn tool(&self, capability: &str) -> Option<McpToolDescriptor> {
        self.catalog
            .read()
            .expect("mcp catalog lock poisoned")
            .get(capability)
            .cloned()
    }

    pub(crate) async fn probe(&self, server_id: &str) -> Result<McpProbeOutcome, McpRuntimeError> {
        if !self.config.servers.contains_key(server_id) {
            return Err(McpRuntimeError::new("mcp_server_not_configured"));
        }
        let client = self
            .clients
            .read()
            .await
            .get(server_id)
            .cloned()
            .ok_or_else(|| McpRuntimeError::new("mcp_server_not_ready"))?;
        let started = Instant::now();
        if let Err(error) = client.ping().await {
            self.set_lifecycle(
                server_id,
                McpLifecycleState::Degraded,
                0,
                Some(error.code()),
            );
            return Err(error);
        }
        Ok(McpProbeOutcome {
            server_id: server_id.to_string(),
            status: "ok".to_string(),
            latency_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        })
    }

    pub(crate) async fn call(
        &self,
        capability: &str,
        args: Value,
    ) -> Result<McpCallOutcome, McpRuntimeError> {
        let descriptor = self
            .tool(capability)
            .ok_or_else(|| McpRuntimeError::new("mcp_capability_unavailable"))?;
        let args = match args {
            Value::Object(args) => args,
            Value::Null => Map::new(),
            _ => return Err(McpRuntimeError::new("mcp_arguments_not_object")),
        };
        let client = self
            .clients
            .read()
            .await
            .get(&descriptor.server_id)
            .cloned()
            .ok_or_else(|| McpRuntimeError::new("mcp_client_unavailable"))?;
        if client.is_closed() {
            self.set_lifecycle(
                &descriptor.server_id,
                McpLifecycleState::Degraded,
                0,
                Some("mcp_transport_closed"),
            );
            return Err(McpRuntimeError::new("mcp_transport_closed"));
        }
        let result = client.call_tool(&descriptor.tool_name, args).await?;
        let max_output_bytes = self
            .config
            .servers
            .get(&descriptor.server_id)
            .map(|server| server.max_output_bytes.max(128))
            .unwrap_or(128);
        project_call_result(descriptor, result, max_output_bytes)
    }

    async fn start_server(
        &self,
        server_id: &str,
        server: &McpServerConfig,
    ) -> Result<usize, McpRuntimeError> {
        validate_server_config(server_id, server)?;
        let timeout = Duration::from_secs(server.timeout_seconds.max(1));
        let service = match server.transport {
            McpTransportConfig::Stdio => connect_stdio(server, timeout).await?,
            McpTransportConfig::StreamableHttp => connect_http(server, timeout).await?,
            McpTransportConfig::Sse => {
                return Err(McpRuntimeError::new("mcp_transport_unsupported"));
            }
        };
        let client = Arc::new(RmcpClient::new(
            service,
            server.max_concurrency,
            timeout,
            server.max_tools,
        ));
        let tools = match client.list_tools().await {
            Ok(tools) => tools,
            Err(error) => {
                let _ = client.shutdown().await;
                return Err(error);
            }
        };
        let descriptors = match build_tool_descriptors(server_id, server, tools) {
            Ok(descriptors) => descriptors,
            Err(error) => {
                let _ = client.shutdown().await;
                return Err(error);
            }
        };
        let tool_count = descriptors.len();
        let duplicate = {
            let catalog = self.catalog.read().expect("mcp catalog lock poisoned");
            descriptors
                .iter()
                .find(|descriptor| catalog.contains_key(&descriptor.capability))
                .map(|descriptor| descriptor.capability.clone())
        };
        if let Some(capability) = duplicate {
            let _ = client.shutdown().await;
            return Err(McpRuntimeError::with_context(
                "mcp_capability_duplicate",
                capability,
            ));
        }
        {
            let mut catalog = self.catalog.write().expect("mcp catalog lock poisoned");
            for descriptor in descriptors {
                catalog.insert(descriptor.capability.clone(), descriptor);
            }
        }
        self.clients
            .write()
            .await
            .insert(server_id.to_string(), client);
        Ok(tool_count)
    }

    fn duplicate_enabled_prefixes(&self) -> HashSet<String> {
        let mut counts = HashMap::<String, usize>::new();
        for server_id in self.config.enabled_server_names() {
            if let Some(server) = self.config.servers.get(&server_id) {
                *counts
                    .entry(server_namespace(&server_id, server))
                    .or_default() += 1;
            }
        }
        counts
            .into_iter()
            .filter_map(|(prefix, count)| (count > 1).then_some(prefix))
            .collect()
    }

    fn remove_server_tools(&self, server_id: &str) {
        self.catalog
            .write()
            .expect("mcp catalog lock poisoned")
            .retain(|_, tool| tool.server_id != server_id);
    }

    fn set_lifecycle(
        &self,
        server_id: &str,
        state: McpLifecycleState,
        tool_count: usize,
        last_error_code: Option<&str>,
    ) {
        let transport = self
            .config
            .servers
            .get(server_id)
            .map(|server| server.transport.as_token())
            .unwrap_or("unknown");
        self.lifecycle
            .write()
            .expect("mcp lifecycle lock poisoned")
            .insert(
                server_id.to_string(),
                McpLifecycleSnapshot {
                    server_id: server_id.to_string(),
                    state,
                    transport: transport.to_string(),
                    tool_count,
                    last_error_code: last_error_code.map(str::to_string),
                },
            );
    }
}

async fn connect_stdio(
    server: &McpServerConfig,
    timeout: Duration,
) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ()>, McpRuntimeError> {
    let command = server
        .command
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| McpRuntimeError::new("mcp_stdio_command_missing"))?;
    let mut process = Command::new(command);
    process.args(&server.args);
    process.envs(&server.env);
    process.kill_on_drop(true);
    for (name, env_ref) in &server.env_refs {
        let value = std::env::var(env_ref).map_err(|_| {
            McpRuntimeError::with_context("mcp_secret_reference_unavailable", env_ref)
        })?;
        process.env(name, value);
    }
    let transport = TokioChildProcess::new(process)
        .map_err(|_| McpRuntimeError::new("mcp_stdio_spawn_failed"))?;
    tokio::time::timeout(timeout, ().serve(transport))
        .await
        .map_err(|_| McpRuntimeError::new("mcp_initialize_timeout"))?
        .map_err(|_| McpRuntimeError::new("mcp_initialize_failed"))
}

async fn connect_http(
    server: &McpServerConfig,
    timeout: Duration,
) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ()>, McpRuntimeError> {
    let url = server
        .url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| McpRuntimeError::new("mcp_http_url_missing"))?;
    let mut transport_config = StreamableHttpClientTransportConfig::with_uri(url.to_string())
        .reinit_on_expired_session(true);
    if let Some(env_ref) = server
        .auth_token_env
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let token = std::env::var(env_ref).map_err(|_| {
            McpRuntimeError::with_context("mcp_secret_reference_unavailable", env_ref)
        })?;
        transport_config = transport_config.auth_header(token);
    }
    let transport = StreamableHttpClientTransport::from_config(transport_config);
    tokio::time::timeout(timeout, ().serve(transport))
        .await
        .map_err(|_| McpRuntimeError::new("mcp_initialize_timeout"))?
        .map_err(|_| McpRuntimeError::new("mcp_initialize_failed"))
}

fn validate_server_config(
    server_id: &str,
    server: &McpServerConfig,
) -> Result<(), McpRuntimeError> {
    if !server.trusted {
        return Err(McpRuntimeError::new("mcp_server_untrusted"));
    }
    if !valid_machine_id(server_id) {
        return Err(McpRuntimeError::new("mcp_server_id_invalid"));
    }
    if !valid_machine_id(&server_namespace(server_id, server)) {
        return Err(McpRuntimeError::new("mcp_capability_prefix_invalid"));
    }
    if server.timeout_seconds == 0
        || server.max_concurrency == 0
        || server.max_output_bytes < 128
        || server.max_schema_bytes < 128
        || server.max_tools == 0
    {
        return Err(McpRuntimeError::new("mcp_server_limit_invalid"));
    }
    for tool in &server.allowed_tools {
        if !valid_machine_id(tool) {
            return Err(McpRuntimeError::new("mcp_allowed_tool_invalid"));
        }
    }
    match server.transport {
        McpTransportConfig::Stdio if server.command.as_deref().is_none_or(str::is_empty) => {
            Err(McpRuntimeError::new("mcp_stdio_command_missing"))
        }
        McpTransportConfig::StreamableHttp if server.url.as_deref().is_none_or(str::is_empty) => {
            Err(McpRuntimeError::new("mcp_http_url_missing"))
        }
        McpTransportConfig::Sse => Err(McpRuntimeError::new("mcp_transport_unsupported")),
        _ => Ok(()),
    }
}

fn build_tool_descriptors(
    server_id: &str,
    server: &McpServerConfig,
    tools: Vec<rmcp::model::Tool>,
) -> Result<Vec<McpToolDescriptor>, McpRuntimeError> {
    let allowed = server
        .allowed_tools
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut names = HashSet::new();
    let mut descriptors = Vec::new();
    for tool in tools {
        let tool_name = tool.name.as_ref();
        if !valid_machine_id(tool_name) {
            return Err(McpRuntimeError::with_context(
                "mcp_tool_name_invalid",
                tool_name,
            ));
        }
        if !names.insert(tool_name.to_string()) {
            return Err(McpRuntimeError::with_context(
                "mcp_tool_name_duplicate",
                tool_name,
            ));
        }
        if !allowed.contains(tool_name) {
            continue;
        }
        let input_schema = Value::Object(tool.input_schema.as_ref().clone());
        validate_input_schema(&input_schema, server.max_schema_bytes)?;
        let (required_args, optional_args) = schema_argument_names(&input_schema);
        let namespace = server_namespace(server_id, server);
        let policy = server
            .tool_policies
            .get(tool_name)
            .map(McpToolPolicy::from)
            .unwrap_or_default();
        descriptors.push(McpToolDescriptor {
            capability: format!("mcp.{namespace}.{tool_name}"),
            server_id: server_id.to_string(),
            tool_name: tool_name.to_string(),
            description: tool
                .description
                .map(|value| value.chars().take(4096).collect()),
            input_schema,
            required_args,
            optional_args,
            policy,
        });
    }
    Ok(descriptors)
}

fn validate_input_schema(schema: &Value, max_bytes: usize) -> Result<(), McpRuntimeError> {
    let bytes = serde_json::to_vec(schema)
        .map_err(|_| McpRuntimeError::new("mcp_tool_schema_serialize_failed"))?;
    if bytes.len() > max_bytes {
        return Err(McpRuntimeError::new("mcp_tool_schema_too_large"));
    }
    let Some(object) = schema.as_object() else {
        return Err(McpRuntimeError::new("mcp_tool_schema_invalid"));
    };
    if object.get("type").and_then(Value::as_str) != Some("object") {
        return Err(McpRuntimeError::new("mcp_tool_schema_invalid"));
    }
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| McpRuntimeError::new("mcp_tool_schema_invalid"))?;
    if let Some(required) = object.get("required") {
        let required = required
            .as_array()
            .ok_or_else(|| McpRuntimeError::new("mcp_tool_schema_invalid"))?;
        if required.iter().any(|field| {
            field
                .as_str()
                .is_none_or(|field| !properties.contains_key(field))
        }) {
            return Err(McpRuntimeError::new("mcp_tool_schema_invalid"));
        }
    }
    Ok(())
}

fn schema_argument_names(schema: &Value) -> (Vec<String>, Vec<String>) {
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let mut required_args = required.iter().cloned().collect::<Vec<_>>();
    let mut optional_args = schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|properties| properties.keys())
        .filter(|field| !required.contains(*field))
        .cloned()
        .collect::<Vec<_>>();
    required_args.sort();
    optional_args.sort();
    (required_args, optional_args)
}

fn project_call_result(
    descriptor: McpToolDescriptor,
    result: rmcp::model::CallToolResult,
    max_output_bytes: usize,
) -> Result<McpCallOutcome, McpRuntimeError> {
    let output_bytes = serde_json::to_vec(&result)
        .map_err(|_| McpRuntimeError::new("mcp_result_serialize_failed"))?
        .len();
    let is_error = result.is_error == Some(true);
    if output_bytes > max_output_bytes {
        return Ok(McpCallOutcome {
            capability: descriptor.capability,
            server_id: descriptor.server_id,
            tool_name: descriptor.tool_name,
            status: if is_error { "error" } else { "ok" }.to_string(),
            structured_content: None,
            content: json!({
                "truncated": true,
                "original_bytes": output_bytes,
                "max_output_bytes": max_output_bytes,
            }),
            protocol_meta: None,
            output_bytes,
            truncated: true,
            error_code: is_error.then(|| "mcp_tool_result_error".to_string()),
        });
    }
    Ok(McpCallOutcome {
        capability: descriptor.capability,
        server_id: descriptor.server_id,
        tool_name: descriptor.tool_name,
        status: if is_error { "error" } else { "ok" }.to_string(),
        structured_content: result.structured_content,
        content: serde_json::to_value(result.content)
            .map_err(|_| McpRuntimeError::new("mcp_result_serialize_failed"))?,
        protocol_meta: result
            .meta
            .and_then(|value| serde_json::to_value(value).ok()),
        output_bytes,
        truncated: false,
        error_code: is_error.then(|| "mcp_tool_result_error".to_string()),
    })
}

fn server_namespace(server_id: &str, server: &McpServerConfig) -> String {
    server
        .capability_prefix
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(server_id)
        .to_string()
}

fn valid_machine_id(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

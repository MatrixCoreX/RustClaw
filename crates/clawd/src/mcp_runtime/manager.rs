use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::time::Instant;

use claw_core::config::{McpConfig, McpServerConfig, McpTransportConfig};
use rmcp::transport::auth::OAuthState;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use rmcp::transport::{AuthClient, ClientCredentialsConfig, TokioChildProcess};
use rmcp::ServiceExt;
use serde_json::{json, Map, Value};
use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, RwLock as AsyncRwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::client::{McpClient, RmcpClient};
use super::types::{
    McpCallOutcome, McpLifecycleSnapshot, McpLifecycleState, McpProbeOutcome, McpRuntimeError,
    McpToolDescriptor, McpToolPolicy,
};

const MCP_CATALOG_SEARCH_CAPABILITY: &str = "mcp.catalog.search";
const MCP_CATALOG_SEARCH_OUTPUT_BYTES: usize = 64 * 1024;

pub(crate) struct McpRuntime {
    config: Arc<McpConfig>,
    clients: AsyncRwLock<HashMap<String, Arc<RmcpClient>>>,
    catalog: RwLock<HashMap<String, McpToolDescriptor>>,
    lifecycle: RwLock<HashMap<String, McpLifecycleSnapshot>>,
    reconnect_locks: HashMap<String, Arc<AsyncMutex<()>>>,
    reconnect_state: RwLock<HashMap<String, ReconnectState>>,
    health_stop: CancellationToken,
    health_task: AsyncMutex<Option<JoinHandle<()>>>,
}

#[derive(Debug, Clone, Copy, Default)]
struct ReconnectState {
    attempt: u32,
    next_retry_at: Option<Instant>,
    retry_blocked: bool,
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
                    auth_mode: server.auth_mode_token().to_string(),
                    tool_count: 0,
                    last_error_code: None,
                },
            );
        }
        let reconnect_locks = config
            .servers
            .keys()
            .map(|server_id| (server_id.clone(), Arc::new(AsyncMutex::new(()))))
            .collect();
        Self {
            config: Arc::new(config),
            clients: AsyncRwLock::new(HashMap::new()),
            catalog: RwLock::new(HashMap::new()),
            lifecycle: RwLock::new(lifecycle),
            reconnect_locks,
            reconnect_state: RwLock::new(HashMap::new()),
            health_stop: CancellationToken::new(),
            health_task: AsyncMutex::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn disabled() -> Self {
        Self::new(McpConfig::default())
    }

    #[cfg(test)]
    pub(crate) fn reconnect_retry_blocked(&self, server_id: &str) -> bool {
        self.reconnect_state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(server_id)
            .is_some_and(|state| state.retry_blocked)
    }

    pub(crate) async fn start(&self) {
        if !self.config.enabled {
            return;
        }
        if self.config.planner_visible_tools < 2 || self.config.catalog_search_max_results == 0 {
            for server_id in self.config.enabled_server_names() {
                self.set_lifecycle(
                    &server_id,
                    McpLifecycleState::Degraded,
                    0,
                    Some("mcp_runtime_limit_invalid"),
                );
            }
            return;
        }
        let duplicate_namespaces = self.duplicate_enabled_namespaces();
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
            if duplicate_namespaces.contains(&server_namespace(&server_id, &server)) {
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
                    self.clear_reconnect_state(&server_id);
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
                    self.schedule_reconnect(&server_id, &error);
                }
            }
        }
    }

    pub(crate) async fn spawn_health_monitor(self: &Arc<Self>) {
        if !self.config.enabled || self.config.enabled_server_names().is_empty() {
            return;
        }
        let mut task = self.health_task.lock().await;
        if task.is_some() {
            return;
        }
        let runtime = Arc::clone(self);
        let interval = self.health_interval();
        *task = Some(tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = runtime.health_stop.cancelled() => break,
                    _ = tokio::time::sleep(interval) => runtime.health_tick().await,
                }
            }
        }));
    }

    pub(crate) async fn stop(&self) {
        self.health_stop.cancel();
        if let Some(task) = self.health_task.lock().await.take() {
            let _ = task.await;
        }
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
        for server_id in self.config.enabled_server_names() {
            self.remove_server_tools(&server_id);
            self.set_lifecycle(&server_id, McpLifecycleState::Stopped, 0, None);
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

    pub(crate) fn planner_tools(&self) -> Vec<McpToolDescriptor> {
        let tools = self.tools();
        let visible_limit = self.config.planner_visible_tools.max(2);
        if tools.len() <= visible_limit {
            return tools;
        }
        let mut visible = tools
            .into_iter()
            .take(visible_limit.saturating_sub(1))
            .collect::<Vec<_>>();
        visible.push(catalog_search_descriptor());
        visible
    }

    pub(crate) fn tool(&self, capability: &str) -> Option<McpToolDescriptor> {
        if capability == MCP_CATALOG_SEARCH_CAPABILITY
            && self.tools().len() > self.config.planner_visible_tools.max(2)
        {
            return Some(catalog_search_descriptor());
        }
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
            self.schedule_reconnect(server_id, &error);
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
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<McpCallOutcome, McpRuntimeError> {
        let descriptor = self
            .tool(capability)
            .ok_or_else(|| McpRuntimeError::new("mcp_capability_unavailable"))?;
        let args = match args {
            Value::Object(args) => args,
            Value::Null => Map::new(),
            _ => return Err(McpRuntimeError::new("mcp_arguments_not_object")),
        };
        if descriptor.capability == MCP_CATALOG_SEARCH_CAPABILITY {
            if cancellation
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
            {
                return Err(McpRuntimeError::new("mcp_call_cancelled"));
            }
            return self.search_catalog(descriptor, args);
        }
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
            self.schedule_reconnect(
                &descriptor.server_id,
                &McpRuntimeError::new("mcp_transport_closed"),
            );
            return Err(McpRuntimeError::new("mcp_transport_closed"));
        }
        let result = client
            .call_tool(&descriptor.tool_name, args, cancellation)
            .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                if reconnect_after_call_error(error.code()) {
                    self.set_lifecycle(
                        &descriptor.server_id,
                        McpLifecycleState::Degraded,
                        0,
                        Some(error.code()),
                    );
                    self.schedule_reconnect(&descriptor.server_id, &error);
                }
                return Err(error);
            }
        };
        let max_output_bytes = self
            .config
            .servers
            .get(&descriptor.server_id)
            .map(|server| server.max_output_bytes.max(128))
            .unwrap_or(128);
        project_call_result(descriptor, result, max_output_bytes)
    }

    fn search_catalog(
        &self,
        descriptor: McpToolDescriptor,
        args: Map<String, Value>,
    ) -> Result<McpCallOutcome, McpRuntimeError> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| McpRuntimeError::new("mcp_catalog_query_required"))?;
        let server_filter = args
            .get("server_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(self.config.catalog_search_max_results)
            .clamp(1, self.config.catalog_search_max_results.max(1));
        let normalized_query = query.to_lowercase();
        let query_terms = normalized_query.split_whitespace().collect::<Vec<_>>();
        let mut matches = self
            .tools()
            .into_iter()
            .filter(|tool| server_filter.is_none_or(|server_id| tool.server_id == server_id))
            .filter_map(|tool| {
                let haystack = format!(
                    "{} {} {} {}",
                    tool.capability,
                    tool.server_id,
                    tool.tool_name,
                    tool.description.as_deref().unwrap_or_default()
                )
                .to_lowercase();
                let score = if tool.capability.eq_ignore_ascii_case(query)
                    || tool.tool_name.eq_ignore_ascii_case(query)
                {
                    0
                } else if haystack.contains(&normalized_query) {
                    1
                } else if !query_terms.is_empty()
                    && query_terms.iter().all(|term| haystack.contains(term))
                {
                    2
                } else {
                    return None;
                };
                Some((score, tool))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.capability.cmp(&right.1.capability))
        });
        let match_count = matches.len();
        let mut result_items = Vec::new();
        let mut truncated = match_count > limit;
        for (_, tool) in matches.into_iter().take(limit) {
            let item = json!({
                "capability": tool.capability,
                "server_id": tool.server_id,
                "tool_name": tool.tool_name,
                "description": tool.description,
                "input_schema": tool.input_schema,
                "required_args": tool.required_args,
                "optional_args": tool.optional_args,
                "policy": tool.policy,
            });
            let mut candidate = result_items.clone();
            candidate.push(item.clone());
            if serde_json::to_vec(&candidate)
                .map(|bytes| bytes.len() > MCP_CATALOG_SEARCH_OUTPUT_BYTES)
                .unwrap_or(true)
            {
                truncated = true;
                break;
            }
            result_items.push(item);
        }
        let structured_content = json!({
            "query": query,
            "server_id": server_filter,
            "match_count": match_count,
            "returned_count": result_items.len(),
            "catalog_total": self.tools().len(),
            "truncated": truncated,
            "tools": result_items,
        });
        let output_bytes = serde_json::to_vec(&structured_content)
            .map_err(|_| McpRuntimeError::new("mcp_result_serialize_failed"))?
            .len();
        Ok(McpCallOutcome {
            capability: descriptor.capability,
            server_id: descriptor.server_id,
            tool_name: descriptor.tool_name,
            status: "ok".to_string(),
            structured_content: Some(structured_content),
            content: json!([]),
            protocol_meta: None,
            output_bytes,
            truncated,
            error_code: None,
        })
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

    pub(crate) async fn health_tick(&self) {
        for server_id in self.config.enabled_server_names() {
            let client = self.clients.read().await.get(&server_id).cloned();
            if let Some(client) = client {
                if !client.is_closed() {
                    if client.ping().await.is_ok() {
                        continue;
                    }
                    let error = McpRuntimeError::new("mcp_health_ping_failed");
                    self.set_lifecycle(
                        &server_id,
                        McpLifecycleState::Degraded,
                        0,
                        Some(error.code()),
                    );
                    self.disconnect_server(&server_id).await;
                    self.clear_reconnect_state(&server_id);
                    self.reconnect_server(&server_id).await;
                    continue;
                }
                let error = McpRuntimeError::new("mcp_transport_closed");
                self.set_lifecycle(
                    &server_id,
                    McpLifecycleState::Degraded,
                    0,
                    Some(error.code()),
                );
                self.disconnect_server(&server_id).await;
                self.clear_reconnect_state(&server_id);
                self.reconnect_server(&server_id).await;
                continue;
            }
            if self.reconnect_due(&server_id) {
                self.reconnect_server(&server_id).await;
            }
        }
    }

    async fn reconnect_server(&self, server_id: &str) {
        let Some(lock) = self.reconnect_locks.get(server_id) else {
            return;
        };
        let _guard = lock.lock().await;
        if self
            .clients
            .read()
            .await
            .get(server_id)
            .is_some_and(|client| !client.is_closed())
        {
            return;
        }
        let Some(server) = self.config.servers.get(server_id).cloned() else {
            return;
        };
        self.set_lifecycle(server_id, McpLifecycleState::Starting, 0, None);
        match self.start_server(server_id, &server).await {
            Ok(tool_count) => {
                self.clear_reconnect_state(server_id);
                self.set_lifecycle(server_id, McpLifecycleState::Ready, tool_count, None);
            }
            Err(error) => {
                self.set_lifecycle(
                    server_id,
                    McpLifecycleState::Degraded,
                    0,
                    Some(error.code()),
                );
                self.schedule_reconnect(server_id, &error);
            }
        }
    }

    async fn disconnect_server(&self, server_id: &str) {
        let client = self.clients.write().await.remove(server_id);
        self.remove_server_tools(server_id);
        if let Some(client) = client {
            let _ = client.shutdown().await;
        }
    }

    fn health_interval(&self) -> Duration {
        Duration::from_secs(
            self.config
                .enabled_server_names()
                .into_iter()
                .filter_map(|server_id| self.config.servers.get(&server_id))
                .map(|server| server.health_check_seconds.max(1))
                .min()
                .unwrap_or(30),
        )
    }

    fn reconnect_due(&self, server_id: &str) -> bool {
        self.reconnect_state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(server_id)
            .is_some_and(|state| {
                !state.retry_blocked
                    && state
                        .next_retry_at
                        .is_some_and(|next_retry_at| Instant::now() >= next_retry_at)
            })
    }

    fn schedule_reconnect(&self, server_id: &str, error: &McpRuntimeError) {
        let Some(server) = self.config.servers.get(server_id) else {
            return;
        };
        let mut states = self
            .reconnect_state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = states.entry(server_id.to_string()).or_default();
        if !retryable_connection_error(error.code()) {
            state.retry_blocked = true;
            state.next_retry_at = None;
            return;
        }
        state.retry_blocked = false;
        state.attempt = state.attempt.saturating_add(1).min(31);
        let base = server.reconnect_base_seconds.max(1);
        let maximum = server.reconnect_max_seconds.max(base);
        let exponential = base.saturating_mul(1_u64 << state.attempt.saturating_sub(1));
        let bounded = exponential.min(maximum);
        let jitter_percent = 80 + stable_server_jitter(server_id, state.attempt) % 41;
        let delay_ms = bounded.saturating_mul(1000).saturating_mul(jitter_percent) / 100;
        state.next_retry_at = Some(Instant::now() + Duration::from_millis(delay_ms.max(1)));
    }

    fn clear_reconnect_state(&self, server_id: &str) {
        self.reconnect_state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(server_id);
    }

    fn duplicate_enabled_namespaces(&self) -> HashSet<String> {
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
        let auth_mode = self
            .config
            .servers
            .get(server_id)
            .map(McpServerConfig::auth_mode_token)
            .unwrap_or("none");
        self.lifecycle
            .write()
            .expect("mcp lifecycle lock poisoned")
            .insert(
                server_id.to_string(),
                McpLifecycleSnapshot {
                    server_id: server_id.to_string(),
                    state,
                    transport: transport.to_string(),
                    auth_mode: auth_mode.to_string(),
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
    let transport_config = StreamableHttpClientTransportConfig::with_uri(url.to_string())
        .reinit_on_expired_session(true);
    if server.uses_oauth_client_credentials() {
        return connect_http_oauth_client_credentials(server, url, timeout, transport_config).await;
    }
    let mut transport_config = transport_config;
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

async fn connect_http_oauth_client_credentials(
    server: &McpServerConfig,
    url: &str,
    timeout: Duration,
    transport_config: StreamableHttpClientTransportConfig,
) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ()>, McpRuntimeError> {
    let client_id_env = server
        .oauth_client_id_env
        .as_deref()
        .ok_or_else(|| McpRuntimeError::new("mcp_oauth_client_id_ref_missing"))?;
    let client_secret_env = server
        .oauth_client_secret_env
        .as_deref()
        .ok_or_else(|| McpRuntimeError::new("mcp_oauth_client_secret_ref_missing"))?;
    let client_id = std::env::var(client_id_env).map_err(|_| {
        McpRuntimeError::with_context("mcp_secret_reference_unavailable", client_id_env)
    })?;
    let client_secret = std::env::var(client_secret_env).map_err(|_| {
        McpRuntimeError::with_context("mcp_secret_reference_unavailable", client_secret_env)
    })?;
    let mut oauth_state = tokio::time::timeout(timeout, OAuthState::new(url, None))
        .await
        .map_err(|_| McpRuntimeError::new("mcp_oauth_initialize_timeout"))?
        .map_err(|_| McpRuntimeError::new("mcp_oauth_initialize_failed"))?;
    let oauth_config = ClientCredentialsConfig::ClientSecret {
        client_id,
        client_secret,
        scopes: server.oauth_scopes.clone(),
        resource: server
            .oauth_resource
            .clone()
            .or_else(|| Some(url.to_string())),
    };
    tokio::time::timeout(
        timeout,
        oauth_state.authenticate_client_credentials(oauth_config),
    )
    .await
    .map_err(|_| McpRuntimeError::new("mcp_oauth_exchange_timeout"))?
    .map_err(|_| McpRuntimeError::new("mcp_oauth_exchange_failed"))?;
    let auth_manager = oauth_state
        .into_authorization_manager()
        .ok_or_else(|| McpRuntimeError::new("mcp_oauth_state_invalid"))?;
    let auth_client = AuthClient::new(reqwest_mcp::Client::default(), auth_manager);
    let transport = StreamableHttpClientTransport::with_client(auth_client, transport_config);
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
        || server.health_check_seconds == 0
        || server.reconnect_base_seconds == 0
        || server.reconnect_max_seconds < server.reconnect_base_seconds
    {
        return Err(McpRuntimeError::new("mcp_server_limit_invalid"));
    }
    for tool in &server.allowed_tools {
        if !valid_machine_id(tool) {
            return Err(McpRuntimeError::new("mcp_allowed_tool_invalid"));
        }
    }
    if server.auth_token_env.is_some() && server.uses_oauth_client_credentials() {
        return Err(McpRuntimeError::new("mcp_http_auth_conflict"));
    }
    if server.oauth_client_id_env.is_some() != server.oauth_client_secret_env.is_some() {
        return Err(McpRuntimeError::new("mcp_oauth_secret_refs_incomplete"));
    }
    if server.uses_oauth_client_credentials()
        && (!valid_env_reference(server.oauth_client_id_env.as_deref().unwrap_or_default())
            || !valid_env_reference(
                server
                    .oauth_client_secret_env
                    .as_deref()
                    .unwrap_or_default(),
            ))
    {
        return Err(McpRuntimeError::new("mcp_oauth_secret_ref_invalid"));
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

fn valid_env_reference(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
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

fn catalog_search_descriptor() -> McpToolDescriptor {
    McpToolDescriptor {
        capability: MCP_CATALOG_SEARCH_CAPABILITY.to_string(),
        server_id: "runtime".to_string(),
        tool_name: "catalog_search".to_string(),
        description: None,
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "server_id": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1},
            },
            "required": ["query"],
        }),
        required_args: vec!["query".to_string()],
        optional_args: vec!["limit".to_string(), "server_id".to_string()],
        policy: McpToolPolicy {
            effect: "observe".to_string(),
            risk_level: "low".to_string(),
            idempotent: true,
            isolation_profile: None,
            network_access: false,
            filesystem_write: false,
            external_publish: false,
            credential_access: false,
            subprocess: false,
            package_install: false,
            privilege_escalation: false,
        },
    }
}

fn retryable_connection_error(error_code: &str) -> bool {
    matches!(
        error_code,
        "mcp_secret_reference_unavailable"
            | "mcp_stdio_spawn_failed"
            | "mcp_initialize_timeout"
            | "mcp_initialize_failed"
            | "mcp_list_tools_timeout"
            | "mcp_list_tools_failed"
            | "mcp_health_ping_failed"
            | "mcp_ping_timeout"
            | "mcp_ping_failed"
            | "mcp_transport_closed"
            | "mcp_client_unavailable"
            | "mcp_call_failed"
            | "mcp_call_unexpected_response"
    )
}

fn reconnect_after_call_error(error_code: &str) -> bool {
    matches!(
        error_code,
        "mcp_call_failed"
            | "mcp_call_unexpected_response"
            | "mcp_transport_closed"
            | "mcp_client_unavailable"
    )
}

fn stable_server_jitter(server_id: &str, attempt: u32) -> u64 {
    server_id.bytes().fold(u64::from(attempt), |state, byte| {
        state
            .wrapping_mul(1099511628211)
            .wrapping_add(u64::from(byte))
    })
}

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResult, PaginatedRequestParams, Tool};
use rmcp::service::RunningService;
use rmcp::{Peer, RoleClient};
use serde_json::{Map, Value};
use tokio::sync::{Mutex, Semaphore};

use super::types::McpRuntimeError;

pub(crate) type McpClientFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, McpRuntimeError>> + Send + 'a>>;

pub(crate) trait McpClient: Send + Sync {
    fn list_tools(&self) -> McpClientFuture<'_, Vec<Tool>>;
    fn call_tool(
        &self,
        tool_name: &str,
        args: Map<String, Value>,
    ) -> McpClientFuture<'_, CallToolResult>;
    fn shutdown(&self) -> McpClientFuture<'_, ()>;
    fn is_closed(&self) -> bool;
}

pub(crate) struct RmcpClient {
    peer: Peer<RoleClient>,
    service: Mutex<Option<RunningService<RoleClient, ()>>>,
    concurrency: Arc<Semaphore>,
    timeout: Duration,
    max_tools: usize,
}

impl RmcpClient {
    pub(crate) fn new(
        service: RunningService<RoleClient, ()>,
        max_concurrency: usize,
        timeout: Duration,
        max_tools: usize,
    ) -> Self {
        let peer = service.peer().clone();
        Self {
            peer,
            service: Mutex::new(Some(service)),
            concurrency: Arc::new(Semaphore::new(max_concurrency.max(1))),
            timeout,
            max_tools: max_tools.max(1),
        }
    }
}

impl McpClient for RmcpClient {
    fn list_tools(&self) -> McpClientFuture<'_, Vec<Tool>> {
        Box::pin(async move {
            tokio::time::timeout(self.timeout, async {
                let mut tools = Vec::new();
                let mut cursor = None;
                loop {
                    let page = self
                        .peer
                        .list_tools(Some(PaginatedRequestParams::default().with_cursor(cursor)))
                        .await
                        .map_err(|_| McpRuntimeError::new("mcp_list_tools_failed"))?;
                    tools.extend(page.tools);
                    if tools.len() > self.max_tools {
                        return Err(McpRuntimeError::new("mcp_tool_limit_exceeded"));
                    }
                    cursor = page.next_cursor;
                    if cursor.is_none() {
                        return Ok(tools);
                    }
                }
            })
            .await
            .map_err(|_| McpRuntimeError::new("mcp_list_tools_timeout"))?
        })
    }

    fn call_tool(
        &self,
        tool_name: &str,
        args: Map<String, Value>,
    ) -> McpClientFuture<'_, CallToolResult> {
        let tool_name = tool_name.to_string();
        Box::pin(async move {
            let permit = tokio::time::timeout(self.timeout, self.concurrency.acquire())
                .await
                .map_err(|_| McpRuntimeError::new("mcp_concurrency_timeout"))?
                .map_err(|_| McpRuntimeError::new("mcp_client_stopped"))?;
            let request = CallToolRequestParams::new(tool_name).with_arguments(args);
            let result = tokio::time::timeout(self.timeout, self.peer.call_tool(request))
                .await
                .map_err(|_| McpRuntimeError::new("mcp_call_timeout"))?
                .map_err(|_| McpRuntimeError::new("mcp_call_failed"));
            drop(permit);
            result
        })
    }

    fn shutdown(&self) -> McpClientFuture<'_, ()> {
        Box::pin(async move {
            self.concurrency.close();
            let mut service_guard = self.service.lock().await;
            let Some(service) = service_guard.as_mut() else {
                return Ok(());
            };
            service
                .close_with_timeout(self.timeout)
                .await
                .map_err(|_| McpRuntimeError::new("mcp_shutdown_failed"))?;
            *service_guard = None;
            Ok(())
        })
    }

    fn is_closed(&self) -> bool {
        self.peer.is_transport_closed()
    }
}

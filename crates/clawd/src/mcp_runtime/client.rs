use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, CancelledNotificationParam,
    ClientRequest, PaginatedRequestParams, PingRequest, ServerResult, Tool,
};
use rmcp::service::{PeerRequestOptions, RunningService};
use rmcp::{Peer, RoleClient};
use serde_json::{Map, Value};
use tokio::sync::{Mutex, Semaphore};

use super::types::McpRuntimeError;

pub(crate) type McpClientFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, McpRuntimeError>> + Send + 'a>>;

pub(crate) trait McpClient: Send + Sync {
    fn ping(&self) -> McpClientFuture<'_, ()>;
    fn list_tools(&self) -> McpClientFuture<'_, Vec<Tool>>;
    fn call_tool(
        &self,
        tool_name: &str,
        args: Map<String, Value>,
        cancellation: Option<tokio_util::sync::CancellationToken>,
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
    fn ping(&self) -> McpClientFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::timeout(
                self.timeout,
                self.peer
                    .send_request(ClientRequest::PingRequest(PingRequest::default())),
            )
            .await
            .map_err(|_| McpRuntimeError::new("mcp_ping_timeout"))?
            .map(|_| ())
            .map_err(|_| McpRuntimeError::new("mcp_ping_failed"))
        })
    }

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
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) -> McpClientFuture<'_, CallToolResult> {
        let tool_name = tool_name.to_string();
        Box::pin(async move {
            let acquire = tokio::time::timeout(self.timeout, self.concurrency.acquire());
            tokio::pin!(acquire);
            let permit = if let Some(token) = cancellation.as_ref() {
                tokio::select! {
                    biased;
                    _ = token.cancelled() => {
                        return Err(McpRuntimeError::new("mcp_call_cancelled"));
                    }
                    permit = &mut acquire => permit,
                }
            } else {
                acquire.await
            }
            .map_err(|_| McpRuntimeError::new("mcp_concurrency_timeout"))?
            .map_err(|_| McpRuntimeError::new("mcp_client_stopped"))?;
            let params = CallToolRequestParams::new(tool_name).with_arguments(args);
            let mut request = self
                .peer
                .send_cancellable_request(
                    ClientRequest::CallToolRequest(CallToolRequest::new(params)),
                    PeerRequestOptions::no_options(),
                )
                .await
                .map_err(|_| McpRuntimeError::new("mcp_call_failed"))?;
            let request_peer = request.peer.clone();
            let request_id = request.id.clone();
            let timeout = tokio::time::sleep(self.timeout);
            tokio::pin!(timeout);
            let response = if let Some(token) = cancellation.as_ref() {
                tokio::select! {
                    biased;
                    _ = token.cancelled() => {
                        let _ = request_peer
                            .notify_cancelled(CancelledNotificationParam::new(
                                Some(request_id),
                                Some("task_cancelled".to_string()),
                            ))
                            .await;
                        return Err(McpRuntimeError::new("mcp_call_cancelled"));
                    }
                    _ = &mut timeout => {
                        let _ = request_peer
                            .notify_cancelled(CancelledNotificationParam::new(
                                Some(request_id),
                                Some("request_timeout".to_string()),
                            ))
                            .await;
                        return Err(McpRuntimeError::new("mcp_call_timeout"));
                    }
                    response = &mut request.rx => response,
                }
            } else {
                tokio::select! {
                    _ = &mut timeout => {
                        let _ = request_peer
                            .notify_cancelled(CancelledNotificationParam::new(
                                Some(request_id),
                                Some("request_timeout".to_string()),
                            ))
                            .await;
                        return Err(McpRuntimeError::new("mcp_call_timeout"));
                    }
                    response = &mut request.rx => response,
                }
            };
            drop(permit);
            let response = response
                .map_err(|_| McpRuntimeError::new("mcp_call_failed"))?
                .map_err(|_| McpRuntimeError::new("mcp_call_failed"))?;
            match response {
                ServerResult::CallToolResult(result) => Ok(result),
                _ => Err(McpRuntimeError::new("mcp_call_unexpected_response")),
            }
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

use crate::{
    session::Session,
    transport::{bytes_body, WireResponse},
    Result,
};
use http::{HeaderMap, Method};
use serde_json::Value;
use std::future::Future;

/// Device admission remains enforced for gateway traffic. Direct owner traffic
/// is a separate connection; it never creates a synthetic device identity.
pub trait OwnerTransport: Sync {
    fn owner_request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> impl Future<Output = Result<WireResponse>> + Send;
    fn cancelled(&self) -> bool;
    fn expected_node(&self) -> Option<(&str, Option<&str>)> {
        None
    }
}
impl<T: OwnerTransport + Send> OwnerTransport for std::sync::Arc<T> {
    async fn owner_request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<WireResponse> {
        self.as_ref().owner_request(method, path, body).await
    }
    fn cancelled(&self) -> bool {
        self.as_ref().cancelled()
    }
    fn expected_node(&self) -> Option<(&str, Option<&str>)> {
        self.as_ref().expected_node()
    }
}

impl OwnerTransport for Session {
    async fn owner_request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<WireResponse> {
        let info = self.info().await;
        if info
            .identity
            .as_ref()
            .and_then(|v| v.get("role"))
            .and_then(Value::as_str)
            != Some("admin")
        {
            return Err("wallet_admin_required".into());
        }
        let headers = HeaderMap::from_iter([(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        )]);
        self.request(
            method,
            path,
            headers,
            body.map(|b| bytes_body(b.to_string())),
        )
        .await
    }
    fn cancelled(&self) -> bool {
        self.cancelled.is_cancelled()
    }
}

use super::{
    nodes::{canonical_origin, Node},
    owner_transport::OwnerTransport,
    protocol::{Service, API},
};
use crate::{transport::WireResponse, Result};
use futures_util::StreamExt;
use http::Method;
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::Zeroizing;

const UPSTREAM: &str = "/v1/nni/server";
pub struct DirectSession {
    pub id: Uuid,
    pub node: Node,
    pub cancelled: CancellationToken,
    pub ledger: Option<String>,
    client: reqwest::Client,
    // Opaque per-connection challenge binding, not an administrator credential.
    // The server still requires the account's exact K1 proof for every write.
    binding: Zeroizing<String>,
}
impl DirectSession {
    pub fn new(node: Node) -> Result<Self> {
        if canonical_origin(&node.origin)? != node.origin {
            return Err("wallet_node_invalid".into());
        }
        let mut random = Zeroizing::new([0u8; 32]);
        getrandom::getrandom(random.as_mut()).map_err(|_| "wallet_random_unavailable")?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            .user_agent(concat!("agent-desktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| "wallet_network_failed")?;
        Ok(Self {
            id: Uuid::new_v4(),
            node,
            ledger: None,
            cancelled: CancellationToken::new(),
            client,
            binding: Zeroizing::new(hex::encode(*random)),
        })
    }
    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        owner: bool,
    ) -> Result<WireResponse> {
        if self.cancelled.is_cancelled() {
            return Err("stale_connection".into());
        }
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.node.origin))
            .header(http::header::ACCEPT, "application/json");
        if owner {
            request = request.header("x-agent-owner-context", self.binding.as_str());
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = tokio::select! {
            _ = self.cancelled.cancelled() => return Err("stale_connection".into()),
            result = request.send() => result.map_err(|_| "wallet_node_connection_failed")?,
        };
        if response.status().is_redirection() {
            return Err("wallet_node_redirect_rejected".into());
        }
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let cancel = self.cancelled.clone();
        let stream = response.bytes_stream();
        Ok(WireResponse {
            status,
            headers,
            body: Box::pin(
                stream
                    .take_until(cancel.cancelled_owned())
                    .map(|r| r.map_err(|_| "wallet_network_failed".into())),
            ),
        })
    }
    pub async fn market(&self, path: &str) -> Result<Value> {
        let history = super::history::HistoryRequest::parse(path)?;
        let path = match &history {
            Some(request) => request.path(),
            None => market_path(path)?,
        };
        let response = self.send(Method::GET, &path, None, false).await?;
        let status = response.status;
        let mut stream = response.body;
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err("wallet_response_invalid".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if self.cancelled.is_cancelled() {
            return Err("stale_connection".into());
        }
        let mut body: Value =
            serde_json::from_slice(&bytes).map_err(|_| "wallet_response_invalid")?;
        if !(200..300).contains(&status) {
            return Err(super::client::backend_error(
                status,
                body.get("error").and_then(Value::as_str),
            )
            .into());
        }
        if body.get("ok") != Some(&Value::Bool(true)) {
            return Err("wallet_backend_rejected".into());
        }
        if let Some(history) = history {
            body["data"] = history.project(&body["data"])?;
        }
        let data = body
            .get_mut("data")
            .and_then(Value::as_object_mut)
            .ok_or("wallet_response_invalid")?;
        data.insert("node_url".into(), Value::String(self.node.origin.clone()));
        Ok(body)
    }
}

impl OwnerTransport for DirectSession {
    async fn owner_request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<WireResponse> {
        let suffix = path.strip_prefix(API).ok_or("wallet_intent_invalid")?;
        let path = if method == Method::GET {
            let service = match suffix {
                "/capabilities?service=assets" => Service::Assets,
                "/capabilities?service=bancor" => Service::Bancor,
                _ => return Err("wallet_intent_invalid".into()),
            };
            let mut url = reqwest::Url::parse(&format!(
                "{}{UPSTREAM}/assets/owner/capabilities",
                self.node.origin
            ))
            .map_err(|_| "wallet_node_invalid")?;
            url.query_pairs_mut()
                .append_pair("service", service.name())
                .append_pair("node_url", &self.node.origin);
            format!("{}?{}", url.path(), url.query().unwrap_or_default())
        } else if method == Method::POST
            && matches!(
                suffix,
                "/read/public"
                    | "/read/request"
                    | "/read/verify"
                    | "/operations/request"
                    | "/operations/verify"
            )
        {
            if body
                .as_ref()
                .and_then(|v| v.get("node_url"))
                .and_then(Value::as_str)
                != Some(self.node.origin.as_str())
            {
                return Err("wallet_node_changed".into());
            }
            format!("{UPSTREAM}/assets/owner{suffix}")
        } else {
            return Err("wallet_intent_invalid".into());
        };
        self.send(method, &path, body, true).await
    }
    fn cancelled(&self) -> bool {
        self.cancelled.is_cancelled()
    }
    fn expected_node(&self) -> Option<(&str, Option<&str>)> {
        Some((&self.node.origin, self.ledger.as_deref()))
    }
}

pub(crate) fn market_path(raw: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(&format!("http://localhost{raw}"))
        .map_err(|_| "wallet_intent_invalid")?;
    let route = match url.path() {
        "/v1/nni/bancor/market" | "/v1/nni/assets/market" => "bancor/market",
        "/v1/nni/bancor/trades" => "bancor/trades",
        "/v1/nni/bancor/candles" => "bancor/candles",
        _ => return Err("wallet_intent_invalid".into()),
    };
    if !raw.starts_with("/v1/nni/") || url.fragment().is_some() {
        return Err("wallet_intent_invalid".into());
    }
    let mut keys = std::collections::HashSet::new();
    for (key, value) in url.query_pairs() {
        if !keys.insert(key.to_string()) {
            return Err("wallet_intent_invalid".into());
        }
        let valid = match (route, key.as_ref()) {
            ("bancor/candles", "interval_seconds") => value.parse::<u32>().is_ok_and(|v| {
                matches!(v, 60 | 300 | 900 | 3600 | 14400 | 86400 | 604800 | 31536000)
            }),
            ("bancor/candles" | "bancor/trades", "limit") => {
                value.parse::<u32>().is_ok_and(|v| v > 0 && v <= 2000)
            }
            ("bancor/candles", "end_time_unix") => {
                value.parse::<u64>().is_ok_and(|v| v <= i64::MAX as u64)
            }
            _ => false,
        };
        if !valid {
            return Err("wallet_intent_invalid".into());
        }
    }
    if route == "bancor/candles" {
        url.query_pairs_mut()
            .append_pair("price_kind", "pool_marginal_usd_per_aic");
    }
    Ok(format!(
        "{UPSTREAM}/{route}{}",
        url.query().map(|q| format!("?{q}")).unwrap_or_default()
    ))
}

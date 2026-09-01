const NNI_REMOTE_API_PREFIX: &str = "/v1/nni/server";
const NNI_REMOTE_API_TIMEOUT_SECONDS: u64 = 45;
const NNI_REMOTE_READ_MAX_ATTEMPTS: usize = 2;
const NNI_REMOTE_READ_ATTEMPT_TIMEOUT_SECONDS: u64 = 12;
const NNI_REMOTE_SIGNED_READ_GRACE_SECONDS: u64 = 5;
const NNI_REMOTE_READ_RETRY_DELAY_MILLIS: u64 = 250;
const NNI_REMOTE_READ_RETRY_AFTER_MAX_SECONDS: u64 = 60;

fn nni_remote_api_endpoint(node_url: &str, route: &str) -> String {
    let node_url = node_url.trim_end_matches('/');
    let route = route.strip_prefix('/').unwrap_or(route);
    format!("{node_url}{NNI_REMOTE_API_PREFIX}/{route}")
}

fn nni_remote_api_timeout() -> Duration {
    Duration::from_secs(NNI_REMOTE_API_TIMEOUT_SECONDS)
}

fn nni_remote_http_status_retryable(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

fn nni_remote_read_attempt_retryable(attempt: &Value) -> bool {
    attempt
        .get("retryable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn nni_remote_api_retry_after_seconds(body: &ApiResponse<Value>) -> Option<u64> {
    body.data
        .as_ref()
        .and_then(|data| data.get("retry_after_seconds"))
        .and_then(Value::as_u64)
        .filter(|seconds| *seconds > 0)
}

fn nni_remote_read_retry_delay(error: &Value) -> Duration {
    error
        .get("retry_after_seconds")
        .and_then(Value::as_u64)
        .filter(|seconds| *seconds > 0)
        .map(|seconds| Duration::from_secs(seconds.min(NNI_REMOTE_READ_RETRY_AFTER_MAX_SECONDS)))
        .unwrap_or_else(|| Duration::from_millis(NNI_REMOTE_READ_RETRY_DELAY_MILLIS))
}

async fn nni_remote_read_with_retry<F, Fut>(mut operation: F) -> Result<Value, Value>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Value, Value>>,
{
    nni_remote_read_with_retry_budget(
        &mut operation,
        Duration::from_secs(NNI_REMOTE_READ_ATTEMPT_TIMEOUT_SECONDS),
    )
    .await
}

fn nni_remote_signed_read_attempt_timeout() -> Duration {
    Duration::from_secs(
        NNI_REMOTE_API_TIMEOUT_SECONDS
            .saturating_mul(2)
            .saturating_add(nni_signature_helper_timeout_seconds())
            .saturating_add(NNI_REMOTE_SIGNED_READ_GRACE_SECONDS),
    )
}

async fn nni_remote_signed_read_with_retry<F, Fut>(mut operation: F) -> Result<Value, Value>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Value, Value>>,
{
    nni_remote_read_with_retry_budget(
        &mut operation,
        nni_remote_signed_read_attempt_timeout(),
    )
    .await
}

async fn nni_remote_read_with_retry_budget<F, Fut>(
    operation: &mut F,
    attempt_timeout: Duration,
) -> Result<Value, Value>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Value, Value>>,
{
    let mut attempt_number = 1;
    loop {
        let attempt = match tokio::time::timeout(attempt_timeout, operation()).await {
            Ok(result) => result,
            Err(_) => Err(json!({
                "error_code": "nni_remote_read_attempt_timeout",
                "retryable": true,
            })),
        };
        match attempt {
            Ok(data) => return Ok(data),
            Err(error)
                if attempt_number < NNI_REMOTE_READ_MAX_ATTEMPTS
                    && nni_remote_read_attempt_retryable(&error) =>
            {
                attempt_number += 1;
                tokio::time::sleep(nni_remote_read_retry_delay(&error)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn nni_selected_remote_node(config: &NniConfigResponse) -> Option<&String> {
    config
        .selected_node_url
        .as_ref()
        .or_else(|| config.remote_nodes.first())
}

fn prioritize_nni_nodes<'a>(
    selected: Option<&'a String>,
    remote_nodes: &'a [String],
) -> Vec<&'a String> {
    let mut nodes = Vec::with_capacity(remote_nodes.len());
    if let Some(selected) = selected.or_else(|| remote_nodes.first()) {
        nodes.push(selected);
    }
    for candidate in remote_nodes {
        if nodes.iter().all(|existing| *existing != candidate) {
            nodes.push(candidate);
        }
    }
    nodes
}

fn nni_selected_remote_nodes(config: &NniConfigResponse) -> Vec<&String> {
    prioritize_nni_nodes(config.selected_node_url.as_ref(), &config.remote_nodes)
}

#[cfg(test)]
fn nni_asset_service_remote_node(config: &NniConfigResponse) -> Option<&String> {
    config
        .asset_service_node_url
        .as_ref()
        .or(config.selected_node_url.as_ref())
        .or_else(|| config.remote_nodes.first())
}

fn nni_bancor_service_remote_node(config: &NniConfigResponse) -> Option<&String> {
    config
        .bancor_service_node_url
        .as_ref()
        .or(config.selected_node_url.as_ref())
        .or_else(|| config.remote_nodes.first())
}

fn nni_asset_service_remote_nodes(config: &NniConfigResponse) -> Vec<&String> {
    prioritize_nni_nodes(
        config
            .asset_service_node_url
            .as_ref()
            .or(config.selected_node_url.as_ref()),
        &config.remote_nodes,
    )
}

fn nni_bancor_service_remote_nodes(config: &NniConfigResponse) -> Vec<&String> {
    prioritize_nni_nodes(
        config
            .bancor_service_node_url
            .as_ref()
            .or(config.selected_node_url.as_ref()),
        &config.remote_nodes,
    )
}

#[cfg(test)]
#[path = "nni_asset_service_node_tests.rs"]
mod nni_asset_service_node_tests;

#[cfg(test)]
#[path = "nni_remote_api_tests.rs"]
mod nni_remote_api_tests;

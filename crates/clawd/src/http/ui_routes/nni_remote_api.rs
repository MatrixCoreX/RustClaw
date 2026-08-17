const NNI_REMOTE_API_PREFIX: &str = "/v1/nni/server";
const NNI_REMOTE_API_TIMEOUT_SECONDS: u64 = 45;
const NNI_REMOTE_READ_MAX_ATTEMPTS: usize = 2;
const NNI_REMOTE_READ_ATTEMPT_TIMEOUT_SECONDS: u64 = 12;
const NNI_REMOTE_READ_RETRY_DELAY_MILLIS: u64 = 250;

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

async fn nni_remote_read_with_retry<F, Fut>(mut operation: F) -> Result<Value, Value>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Value, Value>>,
{
    let mut attempt_number = 1;
    loop {
        let attempt = match tokio::time::timeout(
            Duration::from_secs(NNI_REMOTE_READ_ATTEMPT_TIMEOUT_SECONDS),
            operation(),
        )
        .await
        {
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
                tokio::time::sleep(Duration::from_millis(
                    NNI_REMOTE_READ_RETRY_DELAY_MILLIS,
                ))
                .await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn nni_selected_remote_node(config: &NniConfigResponse) -> Option<&String> {
    config.selected_node_url.as_ref()
}

fn nni_selected_remote_nodes(
    config: &NniConfigResponse,
) -> std::option::Iter<'_, String> {
    config.selected_node_url.iter()
}

#[cfg(test)]
mod nni_remote_api_tests {
    use super::*;

    #[test]
    fn endpoint_uses_one_versioned_api_prefix() {
        assert_eq!(
            nni_remote_api_endpoint("https://node.example/", "/bancor/market"),
            "https://node.example/v1/nni/server/bancor/market"
        );
    }

    #[test]
    fn remote_timeout_allows_slow_nodes_without_becoming_unbounded() {
        assert_eq!(nni_remote_api_timeout(), Duration::from_secs(45));
        assert_eq!(NNI_REMOTE_READ_ATTEMPT_TIMEOUT_SECONDS, 12);
        assert!(
            NNI_REMOTE_READ_MAX_ATTEMPTS as u64 * NNI_REMOTE_READ_ATTEMPT_TIMEOUT_SECONDS
                + NNI_REMOTE_READ_RETRY_DELAY_MILLIS.div_ceil(1_000)
                < 30
        );
    }

    #[test]
    fn only_explicit_transient_remote_statuses_are_retryable() {
        assert!(nni_remote_http_status_retryable(502));
        assert!(nni_remote_http_status_retryable(429));
        assert!(!nni_remote_http_status_retryable(400));
        assert!(!nni_remote_http_status_retryable(401));
    }

    #[tokio::test]
    async fn remote_read_retries_one_explicitly_retryable_failure() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result = nni_remote_read_with_retry({
            let calls = Arc::clone(&calls);
            move || {
                let calls = Arc::clone(&calls);
                async move {
                    let call = calls.fetch_add(1, Ordering::SeqCst);
                    if call == 0 {
                        Err(json!({"error_code": "test_transient", "retryable": true}))
                    } else {
                        Ok(json!({"status": "ok"}))
                    }
                }
            }
        })
        .await
        .expect("second read attempt should succeed");

        assert_eq!(result.get("status").and_then(Value::as_str), Some("ok"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn remote_read_does_not_retry_unmarked_failures() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result = nni_remote_read_with_retry({
            let calls = Arc::clone(&calls);
            move || {
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err(json!({"error_code": "test_terminal"}))
                }
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

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
    assert_eq!(NNI_REMOTE_SIGNED_READ_GRACE_SECONDS, 5);
    assert!(
        NNI_REMOTE_READ_MAX_ATTEMPTS as u64 * NNI_REMOTE_READ_ATTEMPT_TIMEOUT_SECONDS
            + NNI_REMOTE_READ_RETRY_DELAY_MILLIS.div_ceil(1_000)
            < 30
    );
    assert!(
        nni_remote_signed_read_attempt_timeout()
            >= nni_remote_api_timeout()
                .saturating_mul(2)
                .saturating_add(Duration::from_secs(
                    nni_signature_helper_timeout_seconds(),
                ))
    );
}

#[test]
fn only_explicit_transient_remote_statuses_are_retryable() {
    assert!(nni_remote_http_status_retryable(502));
    assert!(nni_remote_http_status_retryable(429));
    assert!(!nni_remote_http_status_retryable(400));
    assert!(!nni_remote_http_status_retryable(401));
}

#[test]
fn remote_read_honors_bounded_structured_retry_after() {
    let body = ApiResponse {
        ok: false,
        data: Some(json!({"retry_after_seconds": 7})),
        error: Some("nni_rate_limit_reward_private".to_string()),
    };
    assert_eq!(nni_remote_api_retry_after_seconds(&body), Some(7));
    assert_eq!(
        nni_remote_read_retry_delay(&json!({"retry_after_seconds": 2})),
        Duration::from_secs(2)
    );
    assert_eq!(
        nni_remote_read_retry_delay(&json!({"retry_after_seconds": 90})),
        Duration::from_secs(NNI_REMOTE_READ_RETRY_AFTER_MAX_SECONDS)
    );
    assert_eq!(
        nni_remote_read_retry_delay(&json!({})),
        Duration::from_millis(NNI_REMOTE_READ_RETRY_DELAY_MILLIS)
    );
}

#[test]
fn selected_node_is_preferred_and_other_nodes_remain_failover_candidates() {
    let selected = "https://node-b.example.test".to_string();
    let remote_nodes = vec![
        "https://node-a.example.test".to_string(),
        "https://node-b.example.test".to_string(),
    ];
    assert_eq!(
        prioritize_nni_nodes(Some(&selected), &remote_nodes)
            .into_iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "https://node-b.example.test",
            "https://node-a.example.test",
        ]
    );
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

#[tokio::test]
async fn signed_read_budget_does_not_inherit_the_short_public_read_timeout() {
    assert!(
        nni_remote_signed_read_attempt_timeout()
            > Duration::from_secs(NNI_REMOTE_READ_ATTEMPT_TIMEOUT_SECONDS)
    );
    let result = nni_remote_read_with_retry_budget(
        &mut || async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(json!({"status": "ok"}))
        },
        Duration::from_millis(100),
    )
    .await
    .expect("budgeted signed read should complete before its own deadline");
    assert_eq!(result.get("status").and_then(Value::as_str), Some("ok"));
}

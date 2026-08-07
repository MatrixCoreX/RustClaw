use super::*;

#[test]
fn delivery_client_errors_are_machine_only() {
    assert_eq!(
        ChannelDeliveryClientError::HttpStatus(409).to_string(),
        "channel_task_delivery_http_status_409"
    );
    assert_eq!(
        ChannelDeliveryClientError::InvalidResponse.to_string(),
        "channel_task_delivery_response_invalid"
    );
}

#[test]
fn only_transient_http_statuses_are_retried() {
    for status in [408, 425, 429, 500, 502, 599] {
        assert!(retryable_status(status), "status={status}");
    }
    for status in [400, 401, 403, 404, 409, 422] {
        assert!(!retryable_status(status), "status={status}");
    }
}

#[test]
fn delivery_reconciliation_retries_ambiguous_and_retryable_failures() {
    let response = |status, accepted, retryable| ChannelTaskDeliveryResponse {
        schema_version: CHANNEL_TASK_DELIVERY_RESPONSE_SCHEMA_VERSION,
        status,
        accepted,
        delivered: false,
        receipt: None,
        error_code: None,
        message_key: None,
        retryable,
    };
    assert!(delivery_response_is_retryable(&response(
        ChannelTaskDeliveryStatus::InProgress,
        false,
        false,
    )));
    assert!(delivery_response_is_retryable(&response(
        ChannelTaskDeliveryStatus::QueryRequired,
        false,
        false,
    )));
    assert!(delivery_response_is_retryable(&response(
        ChannelTaskDeliveryStatus::Failed,
        false,
        true,
    )));
    assert!(!delivery_response_is_retryable(&response(
        ChannelTaskDeliveryStatus::Failed,
        false,
        false,
    )));
    assert!(delivery_response_is_settled(&response(
        ChannelTaskDeliveryStatus::NotRequired,
        true,
        false,
    )));
    assert_eq!(delivery_reconcile_delay(1).as_secs(), 1);
    assert_eq!(delivery_reconcile_delay(6).as_secs(), 30);
    assert_eq!(delivery_reconcile_delay(99).as_secs(), 30);
}

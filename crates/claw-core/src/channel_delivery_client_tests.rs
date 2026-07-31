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

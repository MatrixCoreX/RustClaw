use crate::channel_delivery::{
    ChannelDeliverySource, ChannelTaskDeliveryContent, ChannelTaskDeliveryRequest,
    ChannelTaskDeliveryResponse, ChannelTaskDeliveryStatus,
    CHANNEL_TASK_DELIVERY_RESPONSE_SCHEMA_VERSION,
};
use crate::types::ApiResponse;
use thiserror::Error;

const DELIVERY_REQUEST_ATTEMPTS: u8 = 3;

#[derive(Debug, Error)]
pub enum ChannelDeliveryClientError {
    #[error("channel_task_delivery_request_failed")]
    Request,
    #[error("channel_task_delivery_http_status_{0}")]
    HttpStatus(u16),
    #[error("channel_task_delivery_response_invalid")]
    InvalidResponse,
    #[error("channel_task_delivery_rejected")]
    Rejected,
}

pub async fn request_task_delivery(
    client: &reqwest::Client,
    base_url: &str,
    task_id: &str,
    user_key: &str,
    source: ChannelDeliverySource,
) -> Result<ChannelTaskDeliveryResponse, ChannelDeliveryClientError> {
    request_task_delivery_with_content(
        client,
        base_url,
        task_id,
        user_key,
        source,
        ChannelTaskDeliveryContent::Full,
    )
    .await
}

pub async fn request_task_delivery_with_content(
    client: &reqwest::Client,
    base_url: &str,
    task_id: &str,
    user_key: &str,
    source: ChannelDeliverySource,
    content: ChannelTaskDeliveryContent,
) -> Result<ChannelTaskDeliveryResponse, ChannelDeliveryClientError> {
    let request = ChannelTaskDeliveryRequest::daemon_with_content(source, content);
    request
        .validate()
        .map_err(|_| ChannelDeliveryClientError::Rejected)?;
    let url = format!(
        "{}/v1/tasks/{}/delivery",
        base_url.trim_end_matches('/'),
        task_id
    );
    let mut attempt = 0;
    let response = loop {
        attempt += 1;
        match client
            .post(&url)
            .header(crate::product_identity::AUTH_KEY_HEADER, user_key.trim())
            .json(&request)
            .send()
            .await
        {
            Ok(response)
                if retryable_status(response.status().as_u16())
                    && attempt < DELIVERY_REQUEST_ATTEMPTS => {}
            Ok(response) => break response,
            Err(_) if attempt < DELIVERY_REQUEST_ATTEMPTS => {}
            Err(_) => return Err(ChannelDeliveryClientError::Request),
        }
        tokio::time::sleep(std::time::Duration::from_millis(150 * u64::from(attempt))).await;
    };
    if !response.status().is_success() {
        return Err(ChannelDeliveryClientError::HttpStatus(
            response.status().as_u16(),
        ));
    }
    let response = response
        .json::<ApiResponse<ChannelTaskDeliveryResponse>>()
        .await
        .map_err(|_| ChannelDeliveryClientError::InvalidResponse)?;
    if !response.ok {
        return Err(ChannelDeliveryClientError::Rejected);
    }
    let response = response
        .data
        .ok_or(ChannelDeliveryClientError::InvalidResponse)?;
    if response.schema_version != CHANNEL_TASK_DELIVERY_RESPONSE_SCHEMA_VERSION {
        return Err(ChannelDeliveryClientError::InvalidResponse);
    }
    Ok(response)
}

/// Keep reconciling a terminal task delivery until the provider has accepted it or a
/// non-retryable response is returned. This is deliberately lease/status driven instead
/// of using a wall-clock deadline: long-running tasks and temporary daemon/core outages
/// must not silently lose their terminal result.
pub async fn request_task_delivery_until_settled(
    client: &reqwest::Client,
    base_url: &str,
    task_id: &str,
    user_key: &str,
    source: ChannelDeliverySource,
) -> Result<ChannelTaskDeliveryResponse, ChannelDeliveryClientError> {
    request_task_delivery_until_settled_with_content(
        client,
        base_url,
        task_id,
        user_key,
        source,
        ChannelTaskDeliveryContent::Full,
    )
    .await
}

pub async fn request_task_delivery_until_settled_with_content(
    client: &reqwest::Client,
    base_url: &str,
    task_id: &str,
    user_key: &str,
    source: ChannelDeliverySource,
    content: ChannelTaskDeliveryContent,
) -> Result<ChannelTaskDeliveryResponse, ChannelDeliveryClientError> {
    let mut retry_attempt = 0_u32;
    loop {
        match request_task_delivery_with_content(
            client, base_url, task_id, user_key, source, content,
        )
        .await
        {
            Ok(response) if delivery_response_is_settled(&response) => return Ok(response),
            Ok(response) if !delivery_response_is_retryable(&response) => return Ok(response),
            Ok(_) => {}
            Err(error) if !delivery_error_is_retryable(&error) => return Err(error),
            Err(_) => {}
        }
        retry_attempt = retry_attempt.saturating_add(1);
        tokio::time::sleep(delivery_reconcile_delay(retry_attempt)).await;
    }
}

fn delivery_response_is_settled(response: &ChannelTaskDeliveryResponse) -> bool {
    response.accepted
        || matches!(
            response.status,
            ChannelTaskDeliveryStatus::Accepted
                | ChannelTaskDeliveryStatus::Delivered
                | ChannelTaskDeliveryStatus::Read
                | ChannelTaskDeliveryStatus::NotRequired
        )
}

fn delivery_response_is_retryable(response: &ChannelTaskDeliveryResponse) -> bool {
    matches!(
        response.status,
        ChannelTaskDeliveryStatus::InProgress | ChannelTaskDeliveryStatus::QueryRequired
    ) || (response.status == ChannelTaskDeliveryStatus::Failed && response.retryable)
}

fn delivery_error_is_retryable(error: &ChannelDeliveryClientError) -> bool {
    matches!(error, ChannelDeliveryClientError::Request)
        || matches!(error, ChannelDeliveryClientError::HttpStatus(status) if retryable_status(*status) || *status == 409)
}

fn delivery_reconcile_delay(attempt: u32) -> std::time::Duration {
    let exponent = attempt.saturating_sub(1).min(5);
    std::time::Duration::from_secs((1_u64 << exponent).min(30))
}

fn retryable_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429) || status >= 500
}

#[cfg(test)]
#[path = "channel_delivery_client_tests.rs"]
mod tests;

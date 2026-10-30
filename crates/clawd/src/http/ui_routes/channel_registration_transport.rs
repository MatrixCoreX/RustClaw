const OFFICIAL_REGISTRATION_RESPONSE_MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
enum ChannelRegistrationTransportError {
    #[error("channel_bind_registration_request_timeout")]
    RequestTimeout,
    #[error("channel_bind_registration_transport_unavailable")]
    TransportUnavailable,
    #[error("channel_bind_registration_response_unavailable")]
    ResponseUnavailable,
    #[error("channel_bind_registration_response_too_large")]
    ResponseTooLarge,
    #[error("channel_bind_registration_response_invalid")]
    ResponseInvalid,
    #[error("channel_bind_registration_redirect_rejected")]
    RedirectRejected,
    #[error("channel_bind_registration_http_status_rejected")]
    HttpStatusRejected,
}

pub(crate) fn official_registration_http_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
}

async fn call_official_registration<T: DeserializeOwned>(
    state: &AppState,
    platform: AgentAppChannel,
    params: &[(&str, &str)],
) -> anyhow::Result<T> {
    use ChannelRegistrationTransportError as Failure;

    let url = format!(
        "{}/oauth/v1/app/registration",
        official_accounts_base_url(platform)
    );
    let mut response = state
        .core
        .registration_http_client
        .post(url)
        .form(params)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                Failure::RequestTimeout
            } else {
                Failure::TransportUnavailable
            }
        })?;
    let status = response.status();
    if status.is_redirection() {
        return Err(Failure::RedirectRejected.into());
    }
    let is_poll = params
        .iter()
        .any(|&(field_name, value)| field_name == "action" && value == "poll");
    if !status.is_success() && !(status == StatusCode::BAD_REQUEST && is_poll) {
        return Err(Failure::HttpStatusRejected.into());
    }
    if response
        .content_length()
        .is_some_and(|length| length > OFFICIAL_REGISTRATION_RESPONSE_MAX_BYTES as u64)
    {
        return Err(Failure::ResponseTooLarge.into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        if error.is_timeout() {
            Failure::RequestTimeout
        } else {
            Failure::ResponseUnavailable
        }
    })? {
        if chunk.len() > OFFICIAL_REGISTRATION_RESPONSE_MAX_BYTES - body.len() {
            return Err(Failure::ResponseTooLarge.into());
        }
        body.extend_from_slice(&chunk);
    }
    // OAuth polling errors may use HTTP 400, but failed-status credentials
    // cannot authorize application. Typed decoding below still rejects duplicates.
    if status == StatusCode::BAD_REQUEST {
        let value: Value = serde_json::from_slice(&body).map_err(|_| Failure::ResponseInvalid)?;
        let error_present = value
            .get("error")
            .and_then(Value::as_str)
            .is_some_and(|code| !code.trim().is_empty());
        let credentials_present = ["client_id", "client_secret"]
            .iter()
            .any(|field| value.get(field).is_some_and(|value| !value.is_null()));
        if !error_present || credentials_present {
            return Err(Failure::HttpStatusRejected.into());
        }
    }
    serde_json::from_slice::<T>(&body).map_err(|_| Failure::ResponseInvalid.into())
}

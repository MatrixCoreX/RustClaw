use std::{
    ffi::OsString,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{
    body::to_bytes,
    extract::Request,
    http::{header, StatusCode},
    response::IntoResponse,
    Router,
};
use serde_json::json;

use super::super::{
    call_official_registration, AgentAppChannel, AppState, OfficialRegistrationPollResponse,
};

struct ProtocolServer {
    url: String,
    requests: Arc<AtomicUsize>,
    handle: tokio::task::JoinHandle<()>,
}

impl ProtocolServer {
    async fn new(status: StatusCode, body: Vec<u8>, location: Option<String>) -> Self {
        let requests = Arc::new(AtomicUsize::new(0));
        let capture = requests.clone();
        let app = Router::new().fallback(move |request: Request| {
            let requests = capture.clone();
            let body = body.clone();
            let location = location.clone();
            async move {
                assert!(!request.headers().contains_key("x-agent-key"));
                assert!(!request.headers().contains_key(header::AUTHORIZATION));
                let _ = to_bytes(request.into_body(), 4096).await.unwrap();
                requests.fetch_add(1, Ordering::SeqCst);
                let mut response = (status, body).into_response();
                response
                    .headers_mut()
                    .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
                if let Some(location) = location {
                    response
                        .headers_mut()
                        .insert(header::LOCATION, location.parse().unwrap());
                }
                response
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            url,
            requests,
            handle,
        }
    }

    fn count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for ProtocolServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

struct ProtocolFixture {
    server: ProtocolServer,
    state: AppState,
    platform: AgentAppChannel,
    env_name: String,
    prior_env: Option<OsString>,
}

impl ProtocolFixture {
    async fn new(
        platform: AgentAppChannel,
        status: StatusCode,
        body: Vec<u8>,
        location: Option<String>,
    ) -> Self {
        let server = ProtocolServer::new(status, body, location).await;
        let mut state = AppState::test_default_with_fixture_provider();
        state.core.registration_http_client =
            super::super::official_registration_http_client_builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();
        let env_name = format!("APP_{}", platform.accounts_env());
        let prior_env = std::env::var_os(&env_name);
        std::env::set_var(&env_name, &server.url);
        Self {
            server,
            state,
            platform,
            env_name,
            prior_env,
        }
    }

    async fn poll(&self) -> anyhow::Result<OfficialRegistrationPollResponse> {
        // Exercise transport only: no session mutation, credentials or service start.
        call_official_registration(
            &self.state,
            self.platform,
            &[("action", "poll"), ("device_code", "fixture-device-code")],
        )
        .await
    }
}

impl Drop for ProtocolFixture {
    fn drop(&mut self) {
        if let Some(prior) = &self.prior_env {
            std::env::set_var(&self.env_name, prior);
        } else {
            std::env::remove_var(&self.env_name);
        }
    }
}

fn credentials() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "client_id": "fixture-app-id", "client_secret": "fixture-app-secret",
        "user_info": {"open_id": "fixture-open-id"}
    }))
    .unwrap()
}

#[tokio::test]
async fn channel_registration_protocol_direct_credentials_control() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = ProtocolFixture::new(platform, StatusCode::OK, credentials(), None).await;
        let result = fixture.poll().await.unwrap();
        assert_eq!(result.client_id.as_deref(), Some("fixture-app-id"));
        assert_eq!(result.client_secret.as_deref(), Some("fixture-app-secret"));
        assert_eq!(fixture.server.count(), 1);
    }
}

#[tokio::test]
async fn channel_registration_protocol_preserves_typed_oauth_400_control() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for code in [
            "authorization_pending",
            "slow_down",
            "expired_token",
            "access_denied",
        ] {
            let body = serde_json::to_vec(&json!({"error":code})).unwrap();
            let fixture = ProtocolFixture::new(platform, StatusCode::BAD_REQUEST, body, None).await;
            let result = fixture.poll().await.unwrap();
            assert_eq!(result.error.as_deref(), Some(code));
            assert!(result.client_id.is_none() && result.client_secret.is_none());
            assert_eq!(fixture.server.count(), 1);
        }
    }
}

#[tokio::test]
async fn channel_registration_protocol_failed_status_cannot_authorize_credentials() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    let mut accepted = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for status in [400, 401, 403, 429, 500, 503] {
            let fixture = ProtocolFixture::new(
                platform,
                StatusCode::from_u16(status).unwrap(),
                credentials(),
                None,
            )
            .await;
            if fixture.poll().await.is_ok() {
                accepted.push((platform.channel(), status));
            }
            assert_eq!(fixture.server.count(), 1);
        }
    }
    assert!(
        accepted.is_empty(),
        "failed-status credentials accepted: {accepted:?}"
    );
}

#[tokio::test]
async fn channel_registration_protocol_decode_failure_does_not_expose_body() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    const MARKER: &str = "fixture-private-registration-body";
    let mut exposed = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let body = format!("{{\"client_secret\":\"{MARKER}\",\"broken\":").into_bytes();
        let fixture = ProtocolFixture::new(platform, StatusCode::OK, body, None).await;
        let error = fixture.poll().await.unwrap_err();
        if format!("{error:#}").contains(MARKER) {
            exposed.push(platform.channel());
        }
        assert_eq!(fixture.server.count(), 1);
    }
    assert!(
        exposed.is_empty(),
        "response body escaped diagnostic boundary: {exposed:?}"
    );
}

#[tokio::test]
async fn channel_registration_protocol_rejects_oversized_success_body() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    let mut accepted = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let body = serde_json::to_vec(&json!({
            "client_id":"fixture-app-id", "client_secret":"fixture-app-secret",
            "ignored_padding":"x".repeat(1024 * 1024 + 1)
        }))
        .unwrap();
        let fixture = ProtocolFixture::new(platform, StatusCode::OK, body, None).await;
        if fixture.poll().await.is_ok() {
            accepted.push(platform.channel());
        }
        assert_eq!(fixture.server.count(), 1);
    }
    assert!(
        accepted.is_empty(),
        "oversized credentials accepted: {accepted:?}"
    );
}

#[tokio::test]
async fn channel_registration_protocol_never_forwards_registration_redirects() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    let mut forwarded = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for status in [301, 302, 303, 307, 308] {
            let target = ProtocolServer::new(StatusCode::OK, credentials(), None).await;
            let fixture = ProtocolFixture::new(
                platform,
                StatusCode::from_u16(status).unwrap(),
                b"{}".to_vec(),
                Some(format!("{}/redirect-target", target.url)),
            )
            .await;
            let result = fixture.poll().await;
            if target.count() != 0 || result.is_ok() {
                forwarded.push((platform.channel(), status, target.count()));
            }
            assert_eq!(fixture.server.count(), 1);
        }
    }
    assert!(
        forwarded.is_empty(),
        "registration redirect not rejected: {forwarded:?}"
    );
}

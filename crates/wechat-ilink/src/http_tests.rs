use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use claw_core::channel_provider_error::ChannelProviderFailureClass;
use serde_json::json;
use serde_json::Value;
use tokio::net::TcpListener;

use super::{
    decode_ilink_provider_failure, is_explicit_unaccepted_sendmessage, post_ilink_json, IlinkAuth,
};

#[derive(Default)]
struct RetryServerState {
    calls: AtomicUsize,
    bodies: Mutex<Vec<Value>>,
}

async fn transient_sendmessage(
    State(state): State<Arc<RetryServerState>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    state.bodies.lock().expect("body lock").push(body);
    let call = state.calls.fetch_add(1, Ordering::SeqCst);
    if call == 0 {
        Json(json!({"ret": -2}))
    } else {
        Json(json!({"ret": 0}))
    }
}

#[test]
fn http_200_session_timeout_is_a_typed_authentication_failure() {
    let error = decode_ilink_provider_failure(
        "sendmessage",
        &json!({"ret": -14, "errmsg": "must not escape"}),
    )
    .expect("provider failure");

    assert_eq!(
        error.failure_class,
        ChannelProviderFailureClass::Authentication
    );
    assert_eq!(error.provider_error_code.as_deref(), Some("-14"));
    assert!(!error.to_string().contains("must not escape"));
}

#[test]
fn zero_or_absent_provider_codes_are_success() {
    for value in [json!({}), json!({"ret": 0}), json!({"errcode": 0})] {
        assert!(decode_ilink_provider_failure("sendmessage", &value).is_none());
    }
}

#[test]
fn only_explicit_unaccepted_sendmessage_is_locally_retryable() {
    let rejected = decode_ilink_provider_failure("sendmessage", &json!({"ret": -2}))
        .expect("provider rejection")
        .to_string();
    let expired = decode_ilink_provider_failure("sendmessage", &json!({"ret": -14}))
        .expect("session failure")
        .to_string();
    let other_operation = decode_ilink_provider_failure("getconfig", &json!({"ret": -2}))
        .expect("provider rejection")
        .to_string();

    assert!(is_explicit_unaccepted_sendmessage(&rejected));
    assert!(!is_explicit_unaccepted_sendmessage(&expired));
    assert!(!is_explicit_unaccepted_sendmessage(&other_operation));
}

#[tokio::test]
async fn explicit_unaccepted_sendmessage_retries_the_same_part() {
    let state = Arc::new(RetryServerState::default());
    let app = Router::new()
        .route("/ilink/bot/sendmessage", post(transient_sendmessage))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test API");
    });
    let body = json!({
        "msg": {
            "client_id": "stable-part-id",
            "context_token": "context",
            "item_list": [{"type": 1, "text_item": {"text": "result"}}]
        }
    });

    let result = post_ilink_json(
        &reqwest::Client::new(),
        &format!("http://{address}"),
        "bot-token",
        IlinkAuth {
            sk_route_tag: "",
            wechat_uin_base64: "",
        },
        "ilink/bot/sendmessage",
        &body,
        5_000,
    )
    .await;

    assert_eq!(result.expect("retry succeeds"), json!({"ret": 0}));
    assert_eq!(state.calls.load(Ordering::SeqCst), 2);
    let bodies = state.bodies.lock().expect("body lock");
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["msg"]["client_id"], "stable-part-id");
    assert_eq!(bodies[1]["msg"]["client_id"], "stable-part-id");
}

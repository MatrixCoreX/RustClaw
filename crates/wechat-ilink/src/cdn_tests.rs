use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use super::{send_weixin_image_from_file, B64};
use crate::http::IlinkAuth;

#[derive(Clone, Default)]
struct TestState {
    getuploadurl_body: Arc<Mutex<Option<Value>>>,
    sendmessage_body: Arc<Mutex<Option<Value>>>,
    upload_queries: Arc<Mutex<Vec<String>>>,
}

async fn handle_getuploadurl(State(state): State<TestState>, body: Bytes) -> impl IntoResponse {
    let parsed: Value = serde_json::from_slice(&body).expect("valid getuploadurl body");
    *state
        .getuploadurl_body
        .lock()
        .expect("getuploadurl body lock") = Some(parsed);
    Json(json!({
        "upload_param": "upload-token",
        "thumb_upload_param": "thumb-upload-token"
    }))
}

async fn handle_upload(State(state): State<TestState>, uri: Uri) -> impl IntoResponse {
    state
        .upload_queries
        .lock()
        .expect("upload queries lock")
        .push(uri.query().unwrap_or_default().to_string());
    let (legacy_param, query_param) = if uri
        .query()
        .unwrap_or_default()
        .contains("thumb-upload-token")
    {
        ("legacy-download-token-thumb", "download-query-token-thumb")
    } else {
        ("legacy-download-token", "download-query-token")
    };
    (
        StatusCode::OK,
        [
            ("x-encrypted-param", legacy_param),
            ("x-encrypted-query-param", query_param),
        ],
        "",
    )
}

async fn handle_sendmessage(
    State(state): State<TestState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let parsed: Value = serde_json::from_slice(&body).expect("valid sendmessage body");
    *state
        .sendmessage_body
        .lock()
        .expect("sendmessage body lock") = Some(parsed);
    assert_eq!(
        headers
            .get("authorizationtype")
            .and_then(|v| v.to_str().ok()),
        Some("ilink_bot_token")
    );
    Json(json!({ "ok": true }))
}

async fn spawn_test_server() -> (SocketAddr, TestState) {
    let state = TestState::default();
    let app = Router::new()
        .route("/ilink/bot/getuploadurl", post(handle_getuploadurl))
        .route("/upload", post(handle_upload))
        .route("/ilink/bot/sendmessage", post(handle_sendmessage))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test app");
    });
    (addr, state)
}

#[tokio::test]
async fn send_weixin_image_matches_openclaw_weixin_message_shape() {
    let (addr, state) = spawn_test_server().await;
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join(format!("wechat-ilink-test-{}.png", std::process::id()));
    tokio::fs::write(&file_path, b"fake-png-content")
        .await
        .expect("write temp image");

    let client = Client::new();
    let ilink_base = format!("http://{addr}");
    let cdn_base = format!("http://{addr}");
    send_weixin_image_from_file(
        &client,
        &ilink_base,
        "bot-token",
        IlinkAuth {
            sk_route_tag: "",
            wechat_uin_base64: "",
        },
        &cdn_base,
        "wechat-user",
        Some("ctx-token"),
        PathBuf::from(&file_path).as_path(),
        "test-channel",
        30_000,
    )
    .await
    .expect("send image");

    let payload = state
        .sendmessage_body
        .lock()
        .expect("sendmessage body lock")
        .clone()
        .expect("captured sendmessage body");
    let media_aes_key = payload["msg"]["item_list"][0]["image_item"]["media"]["aes_key"]
        .as_str()
        .expect("image media aes_key");
    assert_eq!(
        payload["msg"]["item_list"][0]["image_item"]["media"]["encrypt_query_param"].as_str(),
        Some("legacy-download-token")
    );
    assert!(
        payload["msg"]["item_list"][0]["image_item"]["aeskey"].is_null(),
        "openclaw-weixin does not send image_item.aeskey: {payload}"
    );
    assert!(
        payload["msg"]["item_list"][0]["image_item"]["thumb_media"].is_null(),
        "openclaw-weixin does not send thumb_media: {payload}"
    );
    assert!(
        payload["msg"]["item_list"][0]["image_item"]["thumb_size"].is_null(),
        "openclaw-weixin does not send thumb_size: {payload}"
    );
    assert!(
        payload["msg"]["item_list"][0]["image_item"]["hd_size"].is_null(),
        "openclaw-weixin does not send hd_size: {payload}"
    );
    let getuploadurl_body = state
        .getuploadurl_body
        .lock()
        .expect("getuploadurl body lock")
        .clone()
        .expect("captured getuploadurl body");
    let decoded_media_aes_key = B64.decode(media_aes_key).expect("decode media aes_key");
    assert_eq!(
        std::str::from_utf8(&decoded_media_aes_key).ok(),
        getuploadurl_body["aeskey"].as_str(),
        "openclaw-weixin base64-encodes the hex aeskey string"
    );
    assert_eq!(getuploadurl_body["media_type"].as_i64(), Some(1));
    assert!(
        getuploadurl_body["thumb_rawsize"].is_null(),
        "openclaw-weixin does not send thumb_rawsize: {getuploadurl_body}"
    );
    assert!(
        getuploadurl_body["thumb_filesize"].is_null(),
        "openclaw-weixin does not send thumb_filesize: {getuploadurl_body}"
    );
    assert!(
        getuploadurl_body["thumb_rawfilemd5"].is_null(),
        "openclaw-weixin does not send thumb_rawfilemd5: {getuploadurl_body}"
    );
    assert_eq!(getuploadurl_body["no_need_thumb"].as_bool(), Some(true));
    let upload_queries = state
        .upload_queries
        .lock()
        .expect("upload queries lock")
        .clone();
    assert_eq!(upload_queries.len(), 1, "expected origin upload only");
    assert!(upload_queries.iter().any(|q| q.contains("upload-token")));
    assert!(!upload_queries
        .iter()
        .any(|q| q.contains("thumb-upload-token")));

    let _ = tokio::fs::remove_file(&file_path).await;
}

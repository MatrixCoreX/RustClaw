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

use super::{
    send_weixin_file_from_file, send_weixin_image_from_file, send_weixin_video_from_file, B64,
};
use crate::http::IlinkAuth;

#[derive(Clone, Default)]
struct TestState {
    getuploadurl_body: Arc<Mutex<Option<Value>>>,
    sendmessage_body: Arc<Mutex<Option<Value>>>,
    upload_queries: Arc<Mutex<Vec<String>>>,
    upload_full_url: Arc<Mutex<Option<String>>>,
}

async fn handle_getuploadurl(State(state): State<TestState>, body: Bytes) -> impl IntoResponse {
    let parsed: Value = serde_json::from_slice(&body).expect("valid getuploadurl body");
    *state
        .getuploadurl_body
        .lock()
        .expect("getuploadurl body lock") = Some(parsed);
    let upload_full_url = state
        .upload_full_url
        .lock()
        .expect("upload full URL lock")
        .clone();
    Json(json!({
        "upload_param": "upload-token",
        "thumb_upload_param": "thumb-upload-token",
        "upload_full_url": upload_full_url,
    }))
}

async fn handle_upload(State(state): State<TestState>, uri: Uri) -> impl IntoResponse {
    state
        .upload_queries
        .lock()
        .expect("upload queries lock")
        .push(uri.to_string());
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
        .route("/upload-full", post(handle_upload))
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

fn unique_temp_file(label: &str, extension: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "wechat-ilink-{label}-{}-{}.{}",
        std::process::id(),
        rand::random::<u64>(),
        extension
    ))
}

#[tokio::test]
async fn send_weixin_image_matches_openclaw_weixin_message_shape() {
    let (addr, state) = spawn_test_server().await;
    let file_path = unique_temp_file("image", "png");
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
        None,
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

#[tokio::test]
async fn video_and_file_use_distinct_official_upload_and_item_types() {
    let (addr, state) = spawn_test_server().await;
    let client = Client::new();
    let base = format!("http://{addr}");
    let video_path = unique_temp_file("video", "mp4");
    tokio::fs::write(&video_path, b"fake-video")
        .await
        .expect("write video");
    send_weixin_video_from_file(
        &client,
        &base,
        "bot-token",
        IlinkAuth {
            sk_route_tag: "",
            wechat_uin_base64: "",
        },
        &base,
        "wechat-user",
        Some("ctx-token"),
        Some("run-video"),
        &video_path,
        "test-channel",
        30_000,
    )
    .await
    .expect("send video");
    let video_upload = state
        .getuploadurl_body
        .lock()
        .expect("video upload body")
        .clone()
        .expect("captured video upload");
    let video_message = state
        .sendmessage_body
        .lock()
        .expect("video message body")
        .clone()
        .expect("captured video message");
    assert_eq!(video_upload["media_type"], 2);
    assert_eq!(video_message["msg"]["item_list"][0]["type"], 5);
    assert_eq!(video_message["msg"]["run_id"], "run-video");
    assert!(
        video_message["msg"]["item_list"][0]["video_item"]["video_size"]
            .as_i64()
            .is_some_and(|size| size > 0)
    );

    let file_path = unique_temp_file("file", "pdf");
    tokio::fs::write(&file_path, b"fake-document")
        .await
        .expect("write file");
    send_weixin_file_from_file(
        &client,
        &base,
        "bot-token",
        IlinkAuth {
            sk_route_tag: "",
            wechat_uin_base64: "",
        },
        &base,
        "wechat-user",
        Some("ctx-token"),
        Some("run-file"),
        &file_path,
        "report.pdf",
        "test-channel",
        30_000,
    )
    .await
    .expect("send file");
    let file_upload = state
        .getuploadurl_body
        .lock()
        .expect("file upload body")
        .clone()
        .expect("captured file upload");
    let file_message = state
        .sendmessage_body
        .lock()
        .expect("file message body")
        .clone()
        .expect("captured file message");
    assert_eq!(file_upload["media_type"], 3);
    assert_eq!(file_message["msg"]["item_list"][0]["type"], 4);
    assert_eq!(
        file_message["msg"]["item_list"][0]["file_item"]["file_name"],
        "report.pdf"
    );
    assert_eq!(
        file_message["msg"]["item_list"][0]["file_item"]["len"],
        "13"
    );

    let _ = tokio::fs::remove_file(video_path).await;
    let _ = tokio::fs::remove_file(file_path).await;
}

#[tokio::test]
async fn provider_upload_full_url_takes_precedence_over_legacy_url_construction() {
    let (addr, state) = spawn_test_server().await;
    *state.upload_full_url.lock().expect("set upload full URL") =
        Some(format!("http://{addr}/upload-full"));
    let path = unique_temp_file("full-url", "png");
    tokio::fs::write(&path, b"fake-png")
        .await
        .expect("write image");

    send_weixin_image_from_file(
        &Client::new(),
        &format!("http://{addr}"),
        "bot-token",
        IlinkAuth {
            sk_route_tag: "",
            wechat_uin_base64: "",
        },
        &format!("http://{addr}"),
        "wechat-user",
        Some("ctx-token"),
        None,
        &path,
        "test-channel",
        30_000,
    )
    .await
    .expect("send image through full URL");

    let uploads = state.upload_queries.lock().expect("upload paths").clone();
    assert_eq!(uploads, vec!["/upload-full".to_string()]);
    let _ = tokio::fs::remove_file(path).await;
}

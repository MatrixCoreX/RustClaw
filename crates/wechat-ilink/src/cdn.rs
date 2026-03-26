//! Weixin CDN download/upload (`cdn-url.ts`, `cdn-upload.ts`, `upload.ts`, `send.ts`).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use md5::{Digest, Md5};
use rand::Rng;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use tracing::{info, warn};

use crate::crypto::{aes_ecb_padded_size, decrypt_aes_128_ecb, encrypt_aes_128_ecb};
use crate::http::{post_ilink_json, BaseInfo, IlinkAuth};

const DEFAULT_API_TIMEOUT_MS: u64 = 15_000;
const CDN_UPLOAD_MAX_RETRIES: u32 = 3;

/// `getuploadurl` / `UploadMediaType` (proto), same as OpenClaw `api/types.ts`.
pub const UPLOAD_MEDIA_TYPE_IMAGE: i64 = 1;
pub const UPLOAD_MEDIA_TYPE_VIDEO: i64 = 2;
pub const UPLOAD_MEDIA_TYPE_FILE: i64 = 3;
#[allow(dead_code)]
pub const UPLOAD_MEDIA_TYPE_VOICE: i64 = 4;

pub fn build_cdn_download_url(encrypted_query_param: &str, cdn_base_url: &str) -> String {
    let base = cdn_base_url.trim_end_matches('/');
    format!(
        "{base}/download?encrypted_query_param={}",
        urlencoding::encode(encrypted_query_param)
    )
}

pub fn build_cdn_upload_url(cdn_base_url: &str, upload_param: &str, filekey: &str) -> String {
    let base = cdn_base_url.trim_end_matches('/');
    format!(
        "{base}/upload?encrypted_query_param={}&filekey={}",
        urlencoding::encode(upload_param),
        urlencoding::encode(filekey)
    )
}

pub async fn download_decrypted_media(
    client: &Client,
    encrypt_query_param: &str,
    key: &[u8; 16],
    cdn_base_url: &str,
    label: &str,
) -> Result<Vec<u8>, String> {
    let url = build_cdn_download_url(encrypt_query_param, cdn_base_url);
    let ct = fetch_cdn_bytes(client, &url, label).await?;
    decrypt_aes_128_ecb(&ct, key).map_err(|e| format!("{label}: {e}"))
}

pub async fn fetch_cdn_bytes(client: &Client, url: &str, label: &str) -> Result<Vec<u8>, String> {
    let res = client
        .get(url)
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| format!("{label}: cdn fetch {e}"))?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!(
            "{label}: cdn download status={} body={}",
            status, body
        ));
    }
    res.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("{label}: cdn read body {e}"))
}

pub async fn download_remote_media_to_temp(
    client: &Client,
    url: &str,
    dest_dir: &Path,
    prefix: &str,
) -> Result<PathBuf, String> {
    let res = client
        .get(url)
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| format!("remote media download failed: fetch {e}"))?;
    if !res.status().is_success() {
        return Err(format!(
            "remote media download failed: {} {} url={}",
            res.status(),
            res.status().canonical_reason().unwrap_or(""),
            url
        ));
    }
    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = res
        .bytes()
        .await
        .map_err(|e| format!("remote media download failed: read body {e}"))?;
    tokio::fs::create_dir_all(dest_dir)
        .await
        .map_err(|e| format!("remote media download failed: mkdir {e}"))?;
    let suffix = hex::encode(rand::random::<[u8; 4]>());
    let mut filename = format!("{}-{}-{}", prefix.trim_matches('-'), ts_ms(), suffix);
    if let Some(ext) = infer_extension_from_content_type_or_url(content_type.as_deref(), url) {
        filename.push('.');
        filename.push_str(&ext);
    }
    let path = dest_dir.join(filename);
    tokio::fs::write(&path, &body)
        .await
        .map_err(|e| format!("remote media download failed: write {e}"))?;
    Ok(path)
}

#[derive(Serialize)]
pub struct GetUploadUrlReq {
    pub filekey: String,
    pub media_type: i64,
    pub to_user_id: String,
    pub rawsize: i64,
    pub rawfilemd5: String,
    pub filesize: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_rawsize: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_rawfilemd5: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_filesize: Option<i64>,
    pub no_need_thumb: bool,
    pub aeskey: String,
    pub base_info: BaseInfo,
}

#[derive(Debug, Deserialize)]
pub struct GetUploadUrlResp {
    #[serde(default)]
    pub upload_param: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub thumb_upload_param: Option<String>,
}

pub async fn ilink_get_upload_url(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    body: &GetUploadUrlReq,
) -> Result<GetUploadUrlResp, String> {
    let v = post_ilink_json(
        client,
        ilink_base_url,
        token,
        auth,
        "ilink/bot/getuploadurl",
        body,
        DEFAULT_API_TIMEOUT_MS,
    )
    .await?;
    serde_json::from_value(v).map_err(|e| format!("getuploadurl decode: {e}"))
}

pub struct UploadedCdnBlob {
    #[allow(dead_code)]
    pub filekey: String,
    pub download_encrypted_query_param: String,
    pub aeskey_hex: String,
    pub plaintext_size: usize,
    pub ciphertext_size: usize,
}

pub async fn upload_plaintext_to_cdn(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    cdn_base_url: &str,
    to_user_id: &str,
    plaintext: &[u8],
    upload_media_type: i64,
    channel_version: &str,
) -> Result<UploadedCdnBlob, String> {
    let rawsize = plaintext.len() as i64;
    let rawfilemd5 = {
        let mut h = Md5::new();
        h.update(plaintext);
        format!("{:x}", h.finalize())
    };
    let filesize = aes_ecb_padded_size(plaintext.len()) as i64;
    let (filekey, aeskey_bytes, aeskey_hex) = {
        let mut rng = rand::thread_rng();
        let filekey: String = hex::encode(rng.gen::<[u8; 16]>());
        let aeskey_bytes: [u8; 16] = rng.gen();
        let aeskey_hex = hex::encode(aeskey_bytes);
        (filekey, aeskey_bytes, aeskey_hex)
    };
    let req = GetUploadUrlReq {
        filekey: filekey.clone(),
        media_type: upload_media_type,
        to_user_id: to_user_id.to_string(),
        rawsize,
        rawfilemd5: rawfilemd5.clone(),
        filesize,
        thumb_rawsize: None,
        thumb_rawfilemd5: None,
        thumb_filesize: None,
        no_need_thumb: true,
        aeskey: aeskey_hex.clone(),
        base_info: BaseInfo {
            channel_version: channel_version.to_string(),
        },
    };
    let up = ilink_get_upload_url(client, ilink_base_url, token, auth, &req).await?;
    let upload_param = up
        .upload_param
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "getuploadurl: missing upload_param".to_string())?;

    let ciphertext = encrypt_aes_128_ecb(plaintext, &aeskey_bytes)?;
    let cdn_url = build_cdn_upload_url(cdn_base_url.trim_end_matches('/'), &upload_param, &filekey);
    let download_encrypted_query_param =
        upload_cdn_ciphertext(client, &cdn_url, &ciphertext, true, "cdn upload")
            .await?
            .ok_or_else(|| "cdn upload: missing x-encrypted-param".to_string())?;

    Ok(UploadedCdnBlob {
        filekey,
        download_encrypted_query_param,
        aeskey_hex,
        plaintext_size: plaintext.len(),
        ciphertext_size: ciphertext.len(),
    })
}

async fn upload_cdn_ciphertext(
    client: &Client,
    cdn_url: &str,
    ciphertext: &[u8],
    require_download_param: bool,
    label: &str,
) -> Result<Option<String>, String> {
    let mut last_err = String::new();
    for attempt in 1..=CDN_UPLOAD_MAX_RETRIES {
        let res = match client
            .post(cdn_url)
            .header("Content-Type", "application/octet-stream")
            .timeout(Duration::from_secs(120))
            .body(ciphertext.to_vec())
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("{label} attempt {attempt}: {e}");
                warn!("wechat-ilink: {}", last_err);
                continue;
            }
        };
        let status = res.status();
        if status.is_client_error() {
            let msg = match res
                .headers()
                .get("x-error-message")
                .and_then(|v| v.to_str().ok())
            {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => res.text().await.unwrap_or_default(),
            };
            return Err(format!("{label} client error {status}: {msg}"));
        }
        if !status.is_success() {
            last_err = format!("{label} attempt {attempt} status={status}");
            warn!("wechat-ilink: {}", last_err);
            continue;
        }
        // `sendmessage` media payloads follow OpenClaw weixin and use the legacy
        // `x-encrypted-param` token. `x-encrypted-query-param` is still accepted as
        // a fallback because some environments return both headers.
        let download_param = res
            .headers()
            .get("x-encrypted-param")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                res.headers()
                    .get("x-encrypted-query-param")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
                    .filter(|s| !s.is_empty())
            });
        if require_download_param {
            let Some(param) = download_param.filter(|s| !s.is_empty()) else {
                last_err = format!(
                    "{label} attempt {attempt}: missing x-encrypted-query-param/x-encrypted-param"
                );
                warn!("wechat-ilink: {}", last_err);
                continue;
            };
            return Ok(Some(param));
        }
        return Ok(None);
    }
    Err(last_err)
}

pub fn media_aes_key_b64_from_hex(aeskey_hex: &str) -> Result<String, String> {
    let trimmed = aeskey_hex.trim();
    let raw = hex::decode(trimmed).map_err(|e| format!("aeskey hex: {e}"))?;
    if raw.len() != 16 {
        return Err(format!("aeskey hex len {}", raw.len()));
    }
    Ok(B64.encode(trimmed.as_bytes()))
}

fn infer_extension_from_content_type_or_url(
    content_type: Option<&str>,
    url: &str,
) -> Option<String> {
    let content_type = content_type
        .and_then(|v| v.split(';').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_ascii_lowercase);
    if let Some(ct) = content_type.as_deref() {
        let mapped = match ct {
            "image/jpeg" => Some("jpg"),
            "image/png" => Some("png"),
            "image/webp" => Some("webp"),
            "image/gif" => Some("gif"),
            "image/bmp" => Some("bmp"),
            "video/mp4" => Some("mp4"),
            "video/webm" => Some("webm"),
            "video/quicktime" => Some("mov"),
            "application/pdf" => Some("pdf"),
            "text/plain" => Some("txt"),
            "application/json" => Some("json"),
            _ => None,
        };
        if let Some(ext) = mapped {
            return Some(ext.to_string());
        }
    }
    let url_path = url.split(['?', '#']).next().unwrap_or(url);
    let ext = Path::new(url_path)
        .extension()
        .and_then(|v| v.to_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    Some(ext.to_ascii_lowercase())
}

fn ts_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

const MSG_TYPE_BOT: i64 = 2;
const MSG_STATE_FINISH: i64 = 2;
const ITEM_IMAGE: i64 = 2;
const ITEM_FILE: i64 = 4;
const ITEM_VIDEO: i64 = 5;

async fn post_sendmessage(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    msg_obj: serde_json::Value,
    channel_version: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    let body = json!({
        "msg": msg_obj,
        "base_info": { "channel_version": channel_version },
    });
    post_ilink_json(
        client,
        ilink_base_url,
        token,
        auth,
        "ilink/bot/sendmessage",
        &body,
        timeout_ms.max(15_000),
    )
    .await?;
    Ok(())
}

pub async fn send_weixin_image_from_file(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    cdn_base_url: &str,
    to_user_id: &str,
    context_token: Option<&str>,
    file_path: &Path,
    channel_version: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    let plaintext = tokio::fs::read(file_path)
        .await
        .map_err(|e| format!("read outbound image: {e}"))?;
    let uploaded = upload_plaintext_to_cdn(
        client,
        ilink_base_url,
        token,
        auth,
        cdn_base_url,
        to_user_id,
        &plaintext,
        UPLOAD_MEDIA_TYPE_IMAGE,
        channel_version,
    )
    .await?;
    info!(
        "wechat-ilink: outbound image uploaded path={} raw={} cipher={}",
        file_path.display(),
        uploaded.plaintext_size,
        uploaded.ciphertext_size
    );
    let aes_b64 = media_aes_key_b64_from_hex(&uploaded.aeskey_hex)?;
    let ts = ts_ms();
    let mut msg_obj = json!({
        "from_user_id": "",
        "to_user_id": to_user_id,
        "client_id": format!("rustclaw-img-{ts}"),
        "message_type": MSG_TYPE_BOT,
        "message_state": MSG_STATE_FINISH,
        "item_list": [{
            "type": ITEM_IMAGE,
            "image_item": {
                "media": {
                    "encrypt_query_param": uploaded.download_encrypted_query_param,
                    "aes_key": aes_b64,
                    "encrypt_type": 1_i64
                },
                "mid_size": uploaded.ciphertext_size as i64
            }
        }]
    });
    if let Some(ct) = context_token.map(str::trim).filter(|s| !s.is_empty()) {
        msg_obj["context_token"] = json!(ct);
    }
    post_sendmessage(
        client,
        ilink_base_url,
        token,
        auth,
        msg_obj,
        channel_version,
        timeout_ms,
    )
    .await
}

pub async fn send_weixin_video_from_file(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    cdn_base_url: &str,
    to_user_id: &str,
    context_token: Option<&str>,
    file_path: &Path,
    channel_version: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    let plaintext = tokio::fs::read(file_path)
        .await
        .map_err(|e| format!("read outbound video: {e}"))?;
    let uploaded = upload_plaintext_to_cdn(
        client,
        ilink_base_url,
        token,
        auth,
        cdn_base_url,
        to_user_id,
        &plaintext,
        UPLOAD_MEDIA_TYPE_VIDEO,
        channel_version,
    )
    .await?;
    let aes_b64 = media_aes_key_b64_from_hex(&uploaded.aeskey_hex)?;
    let ts = ts_ms();
    let mut msg_obj = json!({
        "from_user_id": "",
        "to_user_id": to_user_id,
        "client_id": format!("rustclaw-vid-{ts}"),
        "message_type": MSG_TYPE_BOT,
        "message_state": MSG_STATE_FINISH,
        "item_list": [{
            "type": ITEM_VIDEO,
            "video_item": {
                "media": {
                    "encrypt_query_param": uploaded.download_encrypted_query_param,
                    "aes_key": aes_b64,
                    "encrypt_type": 1_i64
                },
                "video_size": uploaded.ciphertext_size as i64
            }
        }]
    });
    if let Some(ct) = context_token.map(str::trim).filter(|s| !s.is_empty()) {
        msg_obj["context_token"] = json!(ct);
    }
    post_sendmessage(
        client,
        ilink_base_url,
        token,
        auth,
        msg_obj,
        channel_version,
        timeout_ms,
    )
    .await
}

pub async fn send_weixin_file_from_file(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    cdn_base_url: &str,
    to_user_id: &str,
    context_token: Option<&str>,
    file_path: &Path,
    attachment_display_name: &str,
    channel_version: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    let plaintext = tokio::fs::read(file_path)
        .await
        .map_err(|e| format!("read outbound file: {e}"))?;
    let uploaded = upload_plaintext_to_cdn(
        client,
        ilink_base_url,
        token,
        auth,
        cdn_base_url,
        to_user_id,
        &plaintext,
        UPLOAD_MEDIA_TYPE_FILE,
        channel_version,
    )
    .await?;
    let aes_b64 = media_aes_key_b64_from_hex(&uploaded.aeskey_hex)?;
    let ts = ts_ms();
    let mut msg_obj = json!({
        "from_user_id": "",
        "to_user_id": to_user_id,
        "client_id": format!("rustclaw-file-{ts}"),
        "message_type": MSG_TYPE_BOT,
        "message_state": MSG_STATE_FINISH,
        "item_list": [{
            "type": ITEM_FILE,
            "file_item": {
                "media": {
                    "encrypt_query_param": uploaded.download_encrypted_query_param,
                    "aes_key": aes_b64,
                    "encrypt_type": 1_i64
                },
                "file_name": attachment_display_name,
                "len": format!("{}", uploaded.plaintext_size)
            }
        }]
    });
    if let Some(ct) = context_token.map(str::trim).filter(|s| !s.is_empty()) {
        msg_obj["context_token"] = json!(ct);
    }
    post_sendmessage(
        client,
        ilink_base_url,
        token,
        auth,
        msg_obj,
        channel_version,
        timeout_ms,
    )
    .await
}

#[cfg(test)]
mod tests {
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
}

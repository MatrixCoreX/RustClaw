//! Weixin CDN download/upload (`cdn-url.ts`, `cdn-upload.ts`, `upload.ts`, `send.ts`).

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use md5::{Digest, Md5};
use rand::Rng;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use tracing::warn;

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

#[derive(Serialize)]
pub struct GetUploadUrlReq {
    pub filekey: String,
    pub media_type: i64,
    pub to_user_id: String,
    pub rawsize: i64,
    pub rawfilemd5: String,
    pub filesize: i64,
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
    #[allow(dead_code)]
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
        rawfilemd5,
        filesize,
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

    let mut last_err = String::new();
    for attempt in 1..=CDN_UPLOAD_MAX_RETRIES {
        let res = match client
            .post(&cdn_url)
            .header("Content-Type", "application/octet-stream")
            .timeout(Duration::from_secs(120))
            .body(ciphertext.clone())
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("cdn upload attempt {attempt}: {e}");
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
            return Err(format!("cdn upload client error {status}: {msg}"));
        }
        if !status.is_success() {
            last_err = format!("cdn upload attempt {attempt} status={status}");
            warn!("wechat-ilink: {}", last_err);
            continue;
        }
        let download_param = res
            .headers()
            .get("x-encrypted-param")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let Some(download_encrypted_query_param) = download_param.filter(|s| !s.is_empty()) else {
            last_err = format!("cdn upload attempt {attempt}: missing x-encrypted-param");
            warn!("wechat-ilink: {}", last_err);
            continue;
        };
        return Ok(UploadedCdnBlob {
            filekey,
            download_encrypted_query_param,
            aeskey_hex,
            plaintext_size: plaintext.len(),
            ciphertext_size: ciphertext.len(),
        });
    }
    Err(last_err)
}

pub fn media_aes_key_b64_from_hex(aeskey_hex: &str) -> Result<String, String> {
    let raw = hex::decode(aeskey_hex.trim()).map_err(|e| format!("aeskey hex: {e}"))?;
    if raw.len() != 16 {
        return Err(format!("aeskey hex len {}", raw.len()));
    }
    Ok(B64.encode(raw))
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

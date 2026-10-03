//! Weixin CDN download/upload (`cdn-url.ts`, `cdn-upload.ts`, `upload.ts`, `send.ts`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use futures_util::StreamExt;
use md5::{Digest, Md5};
use rand::Rng;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio_util::io::ReaderStream;
use tracing::{info, warn};

use crate::contract::{
    new_wechat_client_id, WechatCdnMedia, WechatMessageItem, WechatSendMessageRequest,
    UPLOAD_MEDIA_TYPE_FILE, UPLOAD_MEDIA_TYPE_IMAGE, UPLOAD_MEDIA_TYPE_VIDEO,
};
use crate::crypto::{
    aes_ecb_padded_size, decrypt_aes_128_ecb, decrypt_aes_128_ecb_file, encrypt_aes_128_ecb,
    encrypt_aes_128_ecb_file,
};
use crate::http::{post_ilink_json, BaseInfo, IlinkAuth};

const DEFAULT_API_TIMEOUT_MS: u64 = 15_000;
const CDN_UPLOAD_MAX_RETRIES: u32 = 3;
const CDN_UPLOAD_STALL_TIMEOUT: Duration = Duration::from_secs(180);
const CDN_UPLOAD_ACTIVITY_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const REMOTE_MEDIA_DOWNLOAD_SAFETY_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const PROVIDER_ERROR_BODY_MAX_BYTES: usize = 64 * 1024;

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
    max_plaintext_bytes: u64,
) -> Result<Vec<u8>, String> {
    let url = build_cdn_download_url(encrypt_query_param, cdn_base_url);
    let max_ciphertext_bytes = padded_size_u64(max_plaintext_bytes)?;
    let ct = fetch_cdn_bytes(client, &url, label, max_ciphertext_bytes).await?;
    let plaintext = decrypt_aes_128_ecb(&ct, key).map_err(|e| format!("{label}: {e}"))?;
    if plaintext.len() as u64 > max_plaintext_bytes {
        return Err(format!(
            "{label}: decrypted media exceeds limit:{}:{}",
            plaintext.len(),
            max_plaintext_bytes
        ));
    }
    Ok(plaintext)
}

pub async fn fetch_cdn_bytes(
    client: &Client,
    url: &str,
    label: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    let mut res = client
        .get(url)
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| format!("{label}: cdn fetch {e}"))?;
    if !res.status().is_success() {
        let status = res.status();
        let body = response_text_prefix(res, PROVIDER_ERROR_BODY_MAX_BYTES).await;
        return Err(
            claw_core::channel_provider_error::ChannelProviderError::from_http_response(
                "wechat_ilink",
                "download_media",
                status.as_u16(),
                &body,
            )
            .to_string(),
        );
    }
    if let Some(actual_bytes) = res.content_length() {
        if actual_bytes > max_bytes {
            return Err(format!(
                "{label}: cdn body exceeds limit:{actual_bytes}:{max_bytes}"
            ));
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| format!("{label}: cdn read body {e}"))?
    {
        let next = body.len() as u64 + chunk.len() as u64;
        if next > max_bytes {
            return Err(format!(
                "{label}: cdn body exceeds limit:{next}:{max_bytes}"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub async fn download_decrypted_media_to_file(
    client: &Client,
    encrypt_query_param: &str,
    key: &[u8; 16],
    cdn_base_url: &str,
    label: &str,
    destination: &Path,
    max_plaintext_bytes: u64,
) -> Result<u64, String> {
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| format!("{label}: destination parent missing"))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("{label}: destination mkdir {error}"))?;
    let encrypted_path = temporary_path(parent, "cdn-encrypted");
    let plaintext_path = temporary_path(parent, "cdn-plaintext");
    let result = async {
        let url = build_cdn_download_url(encrypt_query_param, cdn_base_url);
        let response = client
            .get(url)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|error| format!("{label}: cdn fetch {error}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response_text_prefix(response, PROVIDER_ERROR_BODY_MAX_BYTES).await;
            return Err(
                claw_core::channel_provider_error::ChannelProviderError::from_http_response(
                    "wechat_ilink",
                    "download_media",
                    status.as_u16(),
                    &body,
                )
                .to_string(),
            );
        }
        claw_core::channel_media_download::persist_bounded_response(
            response,
            &encrypted_path,
            padded_size_u64(max_plaintext_bytes)?,
        )
        .await
        .map_err(|error| format!("{label}: {error}"))?;
        let plaintext_size = decrypt_aes_128_ecb_file(&encrypted_path, &plaintext_path, key)
            .await
            .map_err(|error| format!("{label}: {error}"))?;
        if plaintext_size > max_plaintext_bytes {
            return Err(format!(
                "{label}: decrypted media exceeds limit:{plaintext_size}:{max_plaintext_bytes}"
            ));
        }
        replace_file(&plaintext_path, destination)
            .await
            .map_err(|error| format!("{label}: destination replace {error}"))?;
        Ok(plaintext_size)
    }
    .await;
    let _ = tokio::fs::remove_file(&encrypted_path).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&plaintext_path).await;
    }
    result
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
        let status = res.status();
        return Err(
            claw_core::channel_provider_error::ChannelProviderError::from_http_response(
                "wechat_ilink",
                "download_remote_media",
                status.as_u16(),
                "",
            )
            .to_string(),
        );
    }
    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
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
    claw_core::channel_media_download::persist_bounded_response(
        res,
        &path,
        REMOTE_MEDIA_DOWNLOAD_SAFETY_MAX_BYTES,
    )
    .await
    .map_err(|e| format!("remote media download failed: {e}"))?;
    Ok(path)
}

fn padded_size_u64(plaintext_size: u64) -> Result<u64, String> {
    let plaintext_size = usize::try_from(plaintext_size)
        .map_err(|_| "media size exceeds platform address space".to_string())?;
    Ok(aes_ecb_padded_size(plaintext_size) as u64)
}

fn temporary_path(parent: &Path, label: &str) -> PathBuf {
    parent.join(format!(
        ".{label}-{}-{}.part",
        std::process::id(),
        hex::encode(rand::random::<[u8; 8]>())
    ))
}

async fn response_text_prefix(mut response: reqwest::Response, max_bytes: usize) -> String {
    let mut body = Vec::with_capacity(max_bytes.min(4096));
    while body.len() < max_bytes {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) | Err(_) => break,
        };
        let remaining = max_bytes - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    String::from_utf8_lossy(&body).into_owned()
}

async fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    match tokio::fs::rename(source, destination).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            tokio::fs::remove_file(destination).await?;
            tokio::fs::rename(source, destination).await
        }
        Err(error) => Err(error),
    }
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
    #[serde(default)]
    pub upload_full_url: Option<String>,
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
    let upload_full_url = up
        .upload_full_url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let upload_param = up
        .upload_param
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if upload_full_url.is_none() && upload_param.is_none() {
        return Err("getuploadurl: missing upload_full_url/upload_param".to_string());
    }

    let ciphertext = encrypt_aes_128_ecb(plaintext, &aeskey_bytes)?;
    let cdn_url = upload_full_url.unwrap_or_else(|| {
        build_cdn_upload_url(
            cdn_base_url.trim_end_matches('/'),
            upload_param.as_deref().unwrap_or_default(),
            &filekey,
        )
    });
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

pub async fn upload_file_to_cdn(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    cdn_base_url: &str,
    to_user_id: &str,
    plaintext_path: &Path,
    upload_media_type: i64,
    channel_version: &str,
) -> Result<UploadedCdnBlob, String> {
    let (rawsize, rawfilemd5) = file_size_and_md5(plaintext_path).await?;
    let filesize = padded_size_u64(rawsize)?;
    let rawsize_i64 = i64::try_from(rawsize)
        .map_err(|_| "outbound media size exceeds provider contract".to_string())?;
    let filesize_i64 = i64::try_from(filesize)
        .map_err(|_| "encrypted media size exceeds provider contract".to_string())?;
    let plaintext_size = usize::try_from(rawsize)
        .map_err(|_| "outbound media exceeds platform address space".to_string())?;
    let ciphertext_size = usize::try_from(filesize)
        .map_err(|_| "encrypted media exceeds platform address space".to_string())?;
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
        rawsize: rawsize_i64,
        rawfilemd5,
        filesize: filesize_i64,
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
    let upload_full_url = up
        .upload_full_url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let upload_param = up
        .upload_param
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if upload_full_url.is_none() && upload_param.is_none() {
        return Err("getuploadurl: missing upload_full_url/upload_param".to_string());
    }
    let encrypted_path = temporary_path(&std::env::temp_dir(), "channel-cdn-upload");
    let result = async {
        let (actual_rawsize, actual_filesize) =
            encrypt_aes_128_ecb_file(plaintext_path, &encrypted_path, &aeskey_bytes).await?;
        if actual_rawsize != rawsize || actual_filesize != filesize {
            return Err("outbound media changed while preparing upload".to_string());
        }
        let cdn_url = upload_full_url.unwrap_or_else(|| {
            build_cdn_upload_url(
                cdn_base_url.trim_end_matches('/'),
                upload_param.as_deref().unwrap_or_default(),
                &filekey,
            )
        });
        let download_encrypted_query_param = upload_cdn_ciphertext_file(
            client,
            &cdn_url,
            &encrypted_path,
            filesize,
            true,
            "cdn upload",
        )
        .await?
        .ok_or_else(|| "cdn upload: missing x-encrypted-param".to_string())?;
        Ok(UploadedCdnBlob {
            filekey,
            download_encrypted_query_param,
            aeskey_hex,
            plaintext_size,
            ciphertext_size,
        })
    }
    .await;
    let _ = tokio::fs::remove_file(encrypted_path).await;
    result
}

async fn file_size_and_md5(path: &Path) -> Result<(u64, String), String> {
    let expected_size = tokio::fs::metadata(path)
        .await
        .map_err(|error| format!("outbound media metadata: {error}"))?
        .len();
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("outbound media open: {error}"))?;
    let mut hash = Md5::new();
    let mut actual_size = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("outbound media read: {error}"))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        actual_size = actual_size.saturating_add(read as u64);
    }
    if actual_size != expected_size {
        return Err("outbound media changed while hashing".to_string());
    }
    Ok((actual_size, format!("{:x}", hash.finalize())))
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
            let provider_body = match res
                .headers()
                .get("x-error-message")
                .and_then(|v| v.to_str().ok())
            {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => response_text_prefix(res, PROVIDER_ERROR_BODY_MAX_BYTES).await,
            };
            return Err(
                claw_core::channel_provider_error::ChannelProviderError::from_http_response(
                    "wechat_ilink",
                    "upload_media",
                    status.as_u16(),
                    &provider_body,
                )
                .to_string(),
            );
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

async fn upload_cdn_ciphertext_file(
    client: &Client,
    cdn_url: &str,
    ciphertext_path: &Path,
    ciphertext_size: u64,
    require_download_param: bool,
    label: &str,
) -> Result<Option<String>, String> {
    let mut last_err = String::new();
    for attempt in 1..=CDN_UPLOAD_MAX_RETRIES {
        let res = match upload_cdn_ciphertext_file_attempt(
            client,
            cdn_url,
            ciphertext_path,
            ciphertext_size,
            label,
            CDN_UPLOAD_STALL_TIMEOUT,
            CDN_UPLOAD_ACTIVITY_CHECK_INTERVAL,
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                last_err = error;
                warn!("wechat-ilink: {}", last_err);
                continue;
            }
        };
        let status = res.status();
        if status.is_client_error() {
            let provider_body = match res
                .headers()
                .get("x-error-message")
                .and_then(|value| value.to_str().ok())
            {
                Some(value) if !value.is_empty() => value.to_string(),
                _ => response_text_prefix(res, PROVIDER_ERROR_BODY_MAX_BYTES).await,
            };
            return Err(
                claw_core::channel_provider_error::ChannelProviderError::from_http_response(
                    "wechat_ilink",
                    "upload_media",
                    status.as_u16(),
                    &provider_body,
                )
                .to_string(),
            );
        }
        if !status.is_success() {
            last_err = format!("{label} attempt {attempt} status={status}");
            warn!("wechat-ilink: {}", last_err);
            continue;
        }
        let download_param = res
            .headers()
            .get("x-encrypted-param")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                res.headers()
                    .get("x-encrypted-query-param")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string)
                    .filter(|value| !value.is_empty())
            });
        if require_download_param && download_param.is_none() {
            last_err = format!("{label} attempt {attempt}: missing encrypted response parameter");
            warn!("wechat-ilink: {}", last_err);
            continue;
        }
        return Ok(download_param);
    }
    Err(last_err)
}

#[derive(Debug)]
struct UploadActivity {
    observed_bytes: u64,
    last_progress_at: Instant,
}

impl UploadActivity {
    fn new(now: Instant) -> Self {
        Self {
            observed_bytes: 0,
            last_progress_at: now,
        }
    }

    fn stalled(&mut self, uploaded_bytes: u64, now: Instant, timeout: Duration) -> bool {
        if uploaded_bytes != self.observed_bytes {
            self.observed_bytes = uploaded_bytes;
            self.last_progress_at = now;
            return false;
        }
        now.duration_since(self.last_progress_at) >= timeout
    }
}

async fn upload_cdn_ciphertext_file_attempt(
    client: &Client,
    cdn_url: &str,
    ciphertext_path: &Path,
    ciphertext_size: u64,
    label: &str,
    stall_timeout: Duration,
    activity_check_interval: Duration,
) -> Result<reqwest::Response, String> {
    let file = tokio::fs::File::open(ciphertext_path)
        .await
        .map_err(|error| format!("{label}: open encrypted body {error}"))?;
    let uploaded_bytes = Arc::new(AtomicU64::new(0));
    let progress = Arc::clone(&uploaded_bytes);
    let stream = ReaderStream::new(file).map(move |chunk| {
        if let Ok(bytes) = &chunk {
            progress.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
        chunk
    });
    let body = reqwest::Body::wrap_stream(stream);
    let request = client
        .post(cdn_url)
        .header("Content-Type", "application/octet-stream")
        .header(reqwest::header::CONTENT_LENGTH, ciphertext_size)
        // No wall-clock request deadline: a large upload may legitimately take a
        // long time. The activity monitor below stops only a stalled transfer.
        .body(body)
        .send();
    tokio::pin!(request);
    let started = Instant::now();
    let mut activity = UploadActivity::new(started);
    let mut monitor = tokio::time::interval_at(
        tokio::time::Instant::now() + activity_check_interval,
        activity_check_interval,
    );
    monitor.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            result = &mut request => {
                return result.map_err(|error| {
                    let kind = if error.is_timeout() {
                        claw_core::channel_provider_error::ChannelProviderTransportKind::Timeout
                    } else if error.is_connect() {
                        claw_core::channel_provider_error::ChannelProviderTransportKind::Connect
                    } else if error.is_body() {
                        claw_core::channel_provider_error::ChannelProviderTransportKind::Body
                    } else {
                        claw_core::channel_provider_error::ChannelProviderTransportKind::Request
                    };
                    claw_core::channel_provider_error::ChannelProviderError::from_transport(
                        "wechat_ilink",
                        "upload_media",
                        kind,
                        &error.to_string(),
                    )
                    .to_string()
                });
            }
            _ = monitor.tick() => {
                let transferred = uploaded_bytes.load(Ordering::Relaxed);
                if activity.stalled(transferred, Instant::now(), stall_timeout) {
                    return Err(
                        claw_core::channel_provider_error::ChannelProviderError::from_transport(
                            "wechat_ilink",
                            "upload_media",
                            claw_core::channel_provider_error::ChannelProviderTransportKind::Timeout,
                            &format!("{label}:upload_stalled:{transferred}:{ciphertext_size}"),
                        )
                        .to_string(),
                    );
                }
            }
        }
    }
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

async fn post_sendmessage(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    body: &WechatSendMessageRequest,
    timeout_ms: u64,
) -> Result<(), String> {
    post_ilink_json(
        client,
        ilink_base_url,
        token,
        auth,
        "ilink/bot/sendmessage",
        body,
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
    run_id: Option<&str>,
    file_path: &Path,
    channel_version: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    send_weixin_image_from_file_with_client_id(
        client,
        ilink_base_url,
        token,
        auth,
        cdn_base_url,
        to_user_id,
        context_token,
        run_id,
        None,
        file_path,
        channel_version,
        timeout_ms,
    )
    .await
}

pub async fn send_weixin_image_from_file_with_client_id(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    cdn_base_url: &str,
    to_user_id: &str,
    context_token: Option<&str>,
    run_id: Option<&str>,
    client_id: Option<&str>,
    file_path: &Path,
    channel_version: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    claw_core::channel_media_limits::validate_local_media_file(
        file_path,
        "wechat_ilink",
        "image",
        claw_core::channel_media_limits::wechat_image_max_bytes(),
    )?;
    let uploaded = upload_file_to_cdn(
        client,
        ilink_base_url,
        token,
        auth,
        cdn_base_url,
        to_user_id,
        file_path,
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
    let media = WechatCdnMedia::encrypted(uploaded.download_encrypted_query_param, aes_b64)?;
    let item = WechatMessageItem::image(media, uploaded.ciphertext_size)?;
    let body = WechatSendMessageRequest::finish(
        to_user_id,
        context_token.unwrap_or_default(),
        client_id
            .map(str::to_string)
            .unwrap_or_else(|| new_wechat_client_id("image")),
        run_id.map(str::to_string),
        item,
        channel_version,
    )?;
    post_sendmessage(client, ilink_base_url, token, auth, &body, timeout_ms).await
}

pub async fn send_weixin_video_from_file(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    cdn_base_url: &str,
    to_user_id: &str,
    context_token: Option<&str>,
    run_id: Option<&str>,
    file_path: &Path,
    channel_version: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    send_weixin_video_from_file_with_client_id(
        client,
        ilink_base_url,
        token,
        auth,
        cdn_base_url,
        to_user_id,
        context_token,
        run_id,
        None,
        file_path,
        channel_version,
        timeout_ms,
    )
    .await
}

pub async fn send_weixin_video_from_file_with_client_id(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    cdn_base_url: &str,
    to_user_id: &str,
    context_token: Option<&str>,
    run_id: Option<&str>,
    client_id: Option<&str>,
    file_path: &Path,
    channel_version: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    claw_core::channel_media_limits::validate_local_media_file(
        file_path,
        "wechat_ilink",
        "video",
        claw_core::channel_media_limits::wechat_video_max_bytes(),
    )?;
    let uploaded = upload_file_to_cdn(
        client,
        ilink_base_url,
        token,
        auth,
        cdn_base_url,
        to_user_id,
        file_path,
        UPLOAD_MEDIA_TYPE_VIDEO,
        channel_version,
    )
    .await?;
    let aes_b64 = media_aes_key_b64_from_hex(&uploaded.aeskey_hex)?;
    let media = WechatCdnMedia::encrypted(uploaded.download_encrypted_query_param, aes_b64)?;
    let item = WechatMessageItem::video(media, uploaded.ciphertext_size)?;
    let body = WechatSendMessageRequest::finish(
        to_user_id,
        context_token.unwrap_or_default(),
        client_id
            .map(str::to_string)
            .unwrap_or_else(|| new_wechat_client_id("video")),
        run_id.map(str::to_string),
        item,
        channel_version,
    )?;
    post_sendmessage(client, ilink_base_url, token, auth, &body, timeout_ms).await
}

pub async fn send_weixin_file_from_file(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    cdn_base_url: &str,
    to_user_id: &str,
    context_token: Option<&str>,
    run_id: Option<&str>,
    file_path: &Path,
    attachment_display_name: &str,
    channel_version: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    send_weixin_file_from_file_with_client_id(
        client,
        ilink_base_url,
        token,
        auth,
        cdn_base_url,
        to_user_id,
        context_token,
        run_id,
        None,
        file_path,
        attachment_display_name,
        channel_version,
        timeout_ms,
    )
    .await
}

pub async fn send_weixin_file_from_file_with_client_id(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    cdn_base_url: &str,
    to_user_id: &str,
    context_token: Option<&str>,
    run_id: Option<&str>,
    client_id: Option<&str>,
    file_path: &Path,
    attachment_display_name: &str,
    channel_version: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    claw_core::channel_media_limits::validate_local_media_file(
        file_path,
        "wechat_ilink",
        "file",
        claw_core::channel_media_limits::wechat_file_max_bytes(),
    )?;
    let uploaded = upload_file_to_cdn(
        client,
        ilink_base_url,
        token,
        auth,
        cdn_base_url,
        to_user_id,
        file_path,
        UPLOAD_MEDIA_TYPE_FILE,
        channel_version,
    )
    .await?;
    let aes_b64 = media_aes_key_b64_from_hex(&uploaded.aeskey_hex)?;
    let media = WechatCdnMedia::encrypted(uploaded.download_encrypted_query_param, aes_b64)?;
    let item = WechatMessageItem::file(media, attachment_display_name, uploaded.plaintext_size)?;
    let body = WechatSendMessageRequest::finish(
        to_user_id,
        context_token.unwrap_or_default(),
        client_id
            .map(str::to_string)
            .unwrap_or_else(|| new_wechat_client_id("file")),
        run_id.map(str::to_string),
        item,
        channel_version,
    )?;
    post_sendmessage(client, ilink_base_url, token, auth, &body, timeout_ms).await
}

#[cfg(test)]
#[path = "cdn_tests.rs"]
mod tests;

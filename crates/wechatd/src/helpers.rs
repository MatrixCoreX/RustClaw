use super::*;

pub(super) fn current_ts_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) fn current_ts_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .unwrap_or(0)
}

pub(super) fn workspace_root_from_config_path(config_path: &str) -> PathBuf {
    Path::new(config_path)
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(super) fn wechat_runtime_status_file_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join("run")
        .join("wechatd-status")
        .join("primary.json")
}

pub(super) fn wechat_session_file_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join("data")
        .join("wechatd")
        .join("session.json")
}

pub(super) fn wechat_sync_buf_file_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join("data")
        .join("wechatd")
        .join("get_updates_buf.txt")
}

pub(super) fn qr_svg_data_url(qr_content: &str) -> Result<String, String> {
    let qr = QrCode::encode_text(qr_content, QrCodeEcc::Medium)
        .map_err(|e| format!("encode QR svg failed: {e:?}"))?;
    let border = 4;
    let size = qr.size();
    let canvas = size + border * 2;
    let mut path = String::new();
    for y in 0..size {
        for x in 0..size {
            if qr.get_module(x, y) {
                let px = x + border;
                let py = y + border;
                path.push_str(&format!("M{px},{py}h1v1h-1z"));
            }
        }
    }
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {canvas} {canvas}\" shape-rendering=\"crispEdges\"><rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/><path d=\"{path}\" fill=\"#111111\"/></svg>"
    );
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        BASE64_STANDARD.encode(svg)
    ))
}

pub(super) fn qr_render_content(response: &QRCodeResponse) -> &str {
    let content = response.qrcode_img_content.trim();
    if content.is_empty() {
        response.qrcode.trim()
    } else {
        content
    }
}

pub(super) fn stable_i64_from_string(input: &str) -> i64 {
    let mut h: i64 = 0;
    for b in input.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as i64);
    }
    h
}

pub(super) fn is_media_item(item: &MessageItem) -> bool {
    matches!(item.r#type, Some(2 | 3 | 4 | 5))
}

pub(super) fn body_from_message_item(item: &MessageItem) -> String {
    if item.r#type == Some(1) {
        let text = item
            .text_item
            .as_ref()
            .and_then(|v| v.text.as_deref())
            .map(str::trim)
            .unwrap_or("");
        if text.is_empty() {
            return String::new();
        }
        let Some(ref_msg) = item.ref_msg.as_ref() else {
            return text.to_string();
        };
        if ref_msg
            .message_item
            .as_deref()
            .map(is_media_item)
            .unwrap_or(false)
        {
            return text.to_string();
        }
        let mut quoted_parts = Vec::new();
        if let Some(title) = ref_msg
            .title
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            quoted_parts.push(title.to_string());
        }
        if let Some(ref_item) = ref_msg.message_item.as_deref() {
            let ref_body = body_from_message_item(ref_item);
            if !ref_body.trim().is_empty() {
                quoted_parts.push(ref_body);
            }
        }
        if quoted_parts.is_empty() {
            text.to_string()
        } else {
            format!("[引用: {}]\n{}", quoted_parts.join(" | "), text)
        }
    } else if item.r#type == Some(3) {
        item.voice_item
            .as_ref()
            .and_then(|v| v.text.as_deref())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .unwrap_or_default()
    } else {
        String::new()
    }
}

pub(super) fn body_from_item_list(items: &[MessageItem]) -> String {
    for item in items {
        let body = body_from_message_item(item);
        if !body.trim().is_empty() {
            return body;
        }
    }
    String::new()
}

pub(super) fn first_item_or_ref_item(
    msg: &WeixinMessage,
    mut matches: impl FnMut(&MessageItem) -> bool,
) -> Option<MessageItem> {
    let items = msg.item_list.as_ref()?;
    for item in items {
        if matches(item) {
            return Some(item.clone());
        }
    }
    for item in items {
        let Some(ref_item) = item
            .ref_msg
            .as_ref()
            .and_then(|v| v.message_item.as_deref())
        else {
            continue;
        };
        if matches(ref_item) {
            return Some(ref_item.clone());
        }
    }
    None
}

pub(super) fn extract_text_message(msg: &WeixinMessage) -> Option<String> {
    let body = body_from_item_list(msg.item_list.as_ref()?);
    (!body.trim().is_empty()).then_some(body)
}

/// True when the message carries image / video / file / raw voice items (no usable text).
pub(super) fn has_non_text_media_items(msg: &WeixinMessage) -> bool {
    first_item_or_ref_item(msg, |it| {
        let t = it.r#type.unwrap_or(0);
        if t == 2 || t == 4 || t == 5 {
            return true;
        }
        if t == 3 {
            let voice_text = it
                .voice_item
                .as_ref()
                .and_then(|v| v.text.as_deref())
                .map(str::trim)
                .unwrap_or("");
            return voice_text.is_empty();
        }
        false
    })
    .is_some()
}

pub(super) fn safe_inbox_user_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) fn build_wechat_inbox_rel_path(
    root_dir: &str,
    user_id: &str,
    file_name: &str,
) -> String {
    let base = root_dir.trim().trim_end_matches('/');
    let seg = safe_inbox_user_segment(user_id);
    let safe_name = sanitize_inbox_filename(file_name);
    if base.is_empty() {
        format!("{seg}/{safe_name}")
    } else {
        format!("{base}/{seg}/{safe_name}")
    }
}

pub(super) fn wechat_media_agent_context(
    media_kind: &str,
    rel_path: &str,
    file_name: Option<&str>,
) -> String {
    let mut event = json!({
        "event_type": "channel_media_saved",
        "channel": "wechat",
        "media_kind": media_kind,
        "workspace_relative_path": rel_path,
        "locator": {
            "kind": "workspace_relative_path",
            "path": rel_path
        }
    });
    if let Some(name) = file_name.map(str::trim).filter(|v| !v.is_empty()) {
        event["file_name"] = json!(name);
    }
    event.to_string()
}

pub(super) fn inbound_image_decrypt_params(msg: &WeixinMessage) -> Option<(String, [u8; 16])> {
    let it = first_item_or_ref_item(msg, |it| {
        it.r#type == Some(2)
            && it
                .image_item
                .as_ref()
                .and_then(|img| img.media.as_ref())
                .and_then(|media| media.encrypt_query_param.as_deref())
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
    })?;
    let img = it.image_item.as_ref()?;
    let media = img.media.as_ref()?;
    let ep = media.encrypt_query_param.as_deref()?.trim();
    let key = parse_aes_key_hex_or_base64_media(
        img.aeskey.as_deref(),
        media.aes_key.as_deref(),
        "inbound-image",
    )
    .ok()?;
    Some((ep.to_string(), key))
}

pub(super) fn inbound_voice_decrypt_params(msg: &WeixinMessage) -> Option<(String, [u8; 16])> {
    let it = first_item_or_ref_item(msg, |it| {
        if it.r#type != Some(3) {
            return false;
        }
        let Some(vo) = it.voice_item.as_ref() else {
            return false;
        };
        if !vo.text.as_deref().map(str::trim).unwrap_or("").is_empty() {
            return false;
        }
        vo.media
            .as_ref()
            .and_then(|media| media.encrypt_query_param.as_deref())
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    })?;
    let vo = it.voice_item.as_ref()?;
    let media = vo.media.as_ref()?;
    let ep = media.encrypt_query_param.as_deref()?.trim();
    let ak = media.aes_key.as_deref()?;
    let key = parse_aes_key_base64(ak, "inbound-voice").ok()?;
    Some((ep.to_string(), key))
}

pub(super) fn inbound_video_decrypt_params(msg: &WeixinMessage) -> Option<(String, [u8; 16])> {
    let it = first_item_or_ref_item(msg, |it| {
        it.r#type == Some(5)
            && it
                .video_item
                .as_ref()
                .and_then(|v| v.media.as_ref())
                .and_then(|media| media.encrypt_query_param.as_deref())
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
    })?;
    let v = it.video_item.as_ref()?;
    let media = v.media.as_ref()?;
    let ep = media.encrypt_query_param.as_deref()?.trim();
    let key = parse_aes_key_hex_or_base64_media(
        v.aeskey.as_deref(),
        media.aes_key.as_deref(),
        "inbound-video",
    )
    .ok()?;
    Some((ep.to_string(), key))
}

pub(super) fn inbound_file_decrypt_params(
    msg: &WeixinMessage,
) -> Option<(String, [u8; 16], String)> {
    let it = first_item_or_ref_item(msg, |it| {
        it.r#type == Some(4)
            && it
                .file_item
                .as_ref()
                .and_then(|f| f.media.as_ref())
                .and_then(|media| media.encrypt_query_param.as_deref())
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
    })?;
    let f = it.file_item.as_ref()?;
    let media = f.media.as_ref()?;
    let ep = media.encrypt_query_param.as_deref()?.trim();
    let ak = media.aes_key.as_deref()?;
    let key = parse_aes_key_base64(ak, "inbound-file").ok()?;
    let raw = f.file_name.as_deref().unwrap_or("attachment.bin").trim();
    let safe = sanitize_inbox_filename(raw);
    Some((ep.to_string(), key, safe))
}

pub(super) fn sanitize_inbox_filename(name: &str) -> String {
    let name = name.trim();
    let base = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let mut s: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        s = "attachment.bin".to_string();
    }
    if s.len() > 120 {
        s.truncate(120);
    }
    s
}

pub(super) fn inbox_rel_suits_doc_parse(rel: &str) -> bool {
    Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "pdf" | "docx" | "md" | "txt" | "html" | "htm"
            )
        })
        .unwrap_or(false)
}

pub(super) fn active_login_is_fresh(login: &ActiveLogin) -> bool {
    current_ts_ms().saturating_sub(login.started_at_ms) < ACTIVE_LOGIN_TTL_MS
}

pub(super) fn runtime_status_is_connected(status: &str) -> bool {
    matches!(status, "connected" | "polling" | "message_received")
}

pub(super) async fn write_json_file<T: Serialize>(path: &Path, value: &T) {
    let Some(parent) = path.parent() else {
        return;
    };
    if tokio::fs::create_dir_all(parent).await.is_err() {
        return;
    }
    let Ok(raw) = serde_json::to_vec_pretty(value) else {
        return;
    };
    let _ = tokio::fs::write(path, raw).await;
}

pub(super) async fn write_text_file(path: &Path, content: &str) {
    let Some(parent) = path.parent() else {
        return;
    };
    if tokio::fs::create_dir_all(parent).await.is_err() {
        return;
    }
    let _ = tokio::fs::write(path, content).await;
}

pub(super) fn load_text_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub(super) fn load_session_file(path: &Path) -> Option<PersistedSession> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub(super) async fn update_status(state: &State, mut mutate: impl FnMut(&mut WechatRuntimeStatus)) {
    let snapshot = {
        let mut guard = state.status.write().await;
        mutate(&mut guard);
        guard.clone()
    };
    write_json_file(&state.status_path, &snapshot).await;
}

pub(super) async fn healthz(AxumState(state): AxumState<State>) -> Json<WechatRuntimeStatus> {
    Json(state.status.read().await.clone())
}

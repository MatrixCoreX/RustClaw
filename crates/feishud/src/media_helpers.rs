use serde_json::Value;

use super::FeishuSection;

pub(super) fn safe_feishu_storage_segment(raw: &str, fallback: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return fallback.to_string();
    }
    let mut out = String::new();
    for c in t.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

pub(super) fn build_feishu_inbox_rel_path(
    root_dir: &str,
    chat_id: &str,
    file_name: &str,
) -> String {
    let seg = safe_feishu_storage_segment(chat_id, "unknown");
    format!("{}/{}/{}", root_dir.trim_end_matches('/'), seg, file_name)
}

/// 飞书消息资源下载：`type=image` 或 `type=file`（音频/视频/普通文件均用 file）。
pub(super) fn feishu_resource_key_and_query_type(
    message_type: &str,
    content: &Value,
) -> Option<(String, &'static str)> {
    match message_type {
        "image" | "sticker" => {
            let key = content.get("image_key").and_then(|v| v.as_str())?;
            Some((key.to_string(), "image"))
        }
        "file" | "audio" | "media" => {
            let key = content.get("file_key").and_then(|v| v.as_str())?;
            Some((key.to_string(), "file"))
        }
        _ => None,
    }
}

pub(super) fn feishu_inbox_root_for_message_type<'a>(
    message_type: &str,
    section: &'a FeishuSection,
) -> &'a str {
    match message_type {
        "image" | "sticker" => section.image_inbox_dir.as_str(),
        "media" => section.video_inbox_dir.as_str(),
        "audio" => section.audio_inbox_dir.as_str(),
        "file" => section.file_inbox_dir.as_str(),
        _ => section.file_inbox_dir.as_str(),
    }
}

pub(super) fn feishu_saved_file_name(message_type: &str, content: &Value, ts: u64) -> String {
    if let Some(name) = content.get("file_name").and_then(|v| v.as_str()) {
        let n = name.trim();
        if !n.is_empty() && !n.contains('/') && !n.contains('\\') {
            let safe = safe_feishu_storage_segment(n, "file");
            return format!("{}_{}", ts, safe);
        }
    }
    let ext = match message_type {
        "image" | "sticker" => "jpg",
        "media" => "mp4",
        "audio" => "m4a",
        "file" => "bin",
        _ => "bin",
    };
    format!("{}.{}", ts, ext)
}

pub(super) fn feishu_media_kind_label_zh(message_type: &str) -> &'static str {
    match message_type {
        "image" => "图片",
        "sticker" => "表情",
        "media" => "视频",
        "audio" => "语音",
        "file" => "文件",
        _ => "媒体",
    }
}

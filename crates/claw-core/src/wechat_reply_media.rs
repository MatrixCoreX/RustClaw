//! Extract local media paths from clawd/skill reply text (OpenClaw `send-media.ts` routing by type).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WechatOutboundKind {
    Image,
    Video,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WechatOutboundSource {
    LocalPath(PathBuf),
    RemoteUrl(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WechatOutboundMedia {
    pub kind: WechatOutboundKind,
    pub source: WechatOutboundSource,
}

/// Ordered media attachments to send after optional caption text (line order preserved).
pub fn extract_wechat_outbound_media(
    answer: &str,
    workspace_root: &Path,
) -> Vec<WechatOutboundMedia> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in answer.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("IMAGE_FILE:") {
            push_media_reference(
                &mut out,
                &mut seen,
                workspace_root,
                normalize_token(rest),
                Some(WechatOutboundKind::Image),
            );
            continue;
        }
        if let Some(rest) = t.strip_prefix("VIDEO_FILE:") {
            push_media_reference(
                &mut out,
                &mut seen,
                workspace_root,
                normalize_token(rest),
                Some(WechatOutboundKind::Video),
            );
            continue;
        }
        if let Some(rest) = t.strip_prefix("FILE:") {
            push_media_reference(
                &mut out,
                &mut seen,
                workspace_root,
                normalize_token(rest),
                None,
            );
            continue;
        }
        if let Some(rest) = t.strip_prefix("FILE_FILE:") {
            push_media_reference(
                &mut out,
                &mut seen,
                workspace_root,
                normalize_token(rest),
                Some(WechatOutboundKind::File),
            );
            continue;
        }
        if let Some(rest) = t.strip_prefix("IMAGE_URL:") {
            push_media_reference(
                &mut out,
                &mut seen,
                workspace_root,
                normalize_token(rest),
                Some(WechatOutboundKind::Image),
            );
            continue;
        }
        if let Some(rest) = t.strip_prefix("VIDEO_URL:") {
            push_media_reference(
                &mut out,
                &mut seen,
                workspace_root,
                normalize_token(rest),
                Some(WechatOutboundKind::Video),
            );
            continue;
        }
        if let Some(rest) = t.strip_prefix("FILE_URL:") {
            push_media_reference(
                &mut out,
                &mut seen,
                workspace_root,
                normalize_token(rest),
                Some(WechatOutboundKind::File),
            );
            continue;
        }
        if let Some(rest) = t.strip_prefix("MEDIA_URL:") {
            push_media_reference(
                &mut out,
                &mut seen,
                workspace_root,
                normalize_token(rest),
                None,
            );
            continue;
        }
        for prefix in [
            "图片已保存：",
            "图片生成成功并已保存：",
            "图片编辑成功并已保存：",
        ] {
            if let Some(rest) = t.strip_prefix(prefix) {
                push_media_reference(
                    &mut out,
                    &mut seen,
                    workspace_root,
                    normalize_token(rest),
                    Some(WechatOutboundKind::Image),
                );
                break;
            }
        }
        for prefix in ["视频已保存：", "视频生成成功并已保存："] {
            if let Some(rest) = t.strip_prefix(prefix) {
                push_media_reference(
                    &mut out,
                    &mut seen,
                    workspace_root,
                    normalize_token(rest),
                    Some(WechatOutboundKind::Video),
                );
                break;
            }
        }
        for prefix in ["Saved path:", "保存路径：", "文件路径：", "文件路径:"] {
            if let Some(rest) = t.strip_prefix(prefix) {
                push_media_reference(
                    &mut out,
                    &mut seen,
                    workspace_root,
                    normalize_token(rest),
                    None,
                );
                break;
            }
        }
        if let Some(p) = parse_written_bytes_line(t) {
            push_media_reference(&mut out, &mut seen, workspace_root, p, None);
        }
    }
    out
}

/// Back-compat: image-only paths (subset of [`extract_wechat_outbound_media`]).
pub fn extract_image_paths_from_reply(answer: &str, workspace_root: &Path) -> Vec<PathBuf> {
    extract_wechat_outbound_media(answer, workspace_root)
        .into_iter()
        .filter_map(|media| match media {
            WechatOutboundMedia {
                kind: WechatOutboundKind::Image,
                source: WechatOutboundSource::LocalPath(path),
            } => Some(path),
            _ => None,
        })
        .collect()
}

pub fn strip_wechat_delivery_lines(answer: &str) -> String {
    answer
        .lines()
        .filter(|line| {
            let t = line.trim();
            if t.starts_with("IMAGE_FILE:")
                || t.starts_with("VIDEO_FILE:")
                || t.starts_with("FILE:")
                || t.starts_with("FILE_FILE:")
                || t.starts_with("IMAGE_URL:")
                || t.starts_with("VIDEO_URL:")
                || t.starts_with("FILE_URL:")
                || t.starts_with("MEDIA_URL:")
            {
                return false;
            }
            if t.starts_with("图片已保存：")
                || t.starts_with("图片生成成功并已保存：")
                || t.starts_with("图片编辑成功并已保存：")
            {
                return false;
            }
            if t.starts_with("视频已保存：") || t.starts_with("视频生成成功并已保存：") {
                return false;
            }
            if t.starts_with("Saved path:")
                || t.starts_with("保存路径：")
                || t.starts_with("文件路径：")
                || t.starts_with("文件路径:")
            {
                return false;
            }
            if parse_written_bytes_line(t).is_some() {
                return false;
            }
            true
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_token(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('`').unwrap_or(s);
    let s = s.strip_suffix('`').unwrap_or(s);
    s.trim().trim_matches('"').trim_matches('\'').to_string()
}

fn push_media_reference(
    out: &mut Vec<WechatOutboundMedia>,
    seen: &mut HashSet<String>,
    workspace_root: &Path,
    token: String,
    forced_kind: Option<WechatOutboundKind>,
) {
    let Some(media) = parse_media_reference(workspace_root, token, forced_kind) else {
        return;
    };
    let key = media_dedupe_key(&media);
    if seen.insert(key) {
        out.push(media);
    }
}

fn parse_media_reference(
    workspace_root: &Path,
    token: String,
    forced_kind: Option<WechatOutboundKind>,
) -> Option<WechatOutboundMedia> {
    if token.is_empty() {
        return None;
    }
    if is_remote_url(&token) {
        return Some(WechatOutboundMedia {
            kind: forced_kind.unwrap_or_else(|| classify_remote_kind(&token)),
            source: WechatOutboundSource::RemoteUrl(token),
        });
    }
    let local_token = normalize_local_token(&token);
    let path = resolve_workspace_path(workspace_root, local_token)?;
    if !path.is_file() {
        return None;
    }
    Some(WechatOutboundMedia {
        kind: forced_kind.unwrap_or_else(|| classify_path_kind(&path)),
        source: WechatOutboundSource::LocalPath(path),
    })
}

fn media_dedupe_key(media: &WechatOutboundMedia) -> String {
    match media {
        WechatOutboundMedia {
            kind,
            source: WechatOutboundSource::LocalPath(path),
        } => format!("{kind:?}:local:{}", path.display()),
        WechatOutboundMedia {
            kind,
            source: WechatOutboundSource::RemoteUrl(url),
        } => format!("{kind:?}:remote:{url}"),
    }
}

fn normalize_local_token(token: &str) -> String {
    token
        .strip_prefix("file://")
        .unwrap_or(token)
        .to_string()
}

fn resolve_workspace_path(workspace_root: &Path, token: String) -> Option<PathBuf> {
    if token.is_empty() {
        return None;
    }
    let p = Path::new(&token);
    let candidate = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(&token)
    };
    candidate.canonicalize().ok().or_else(|| {
        if candidate.is_file() {
            Some(candidate)
        } else {
            None
        }
    })
}

fn is_remote_url(token: &str) -> bool {
    token.starts_with("http://") || token.starts_with("https://")
}

fn is_probably_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp"
            )
        })
        .unwrap_or(false)
}

fn is_probably_video(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "mp4" | "mov" | "webm" | "mkv" | "m4v"
            )
        })
        .unwrap_or(false)
}

fn classify_path_kind(p: &Path) -> WechatOutboundKind {
    if is_probably_image(p) {
        WechatOutboundKind::Image
    } else if is_probably_video(p) {
        WechatOutboundKind::Video
    } else {
        WechatOutboundKind::File
    }
}

fn classify_remote_kind(url: &str) -> WechatOutboundKind {
    let normalized = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    if normalized.ends_with(".jpg")
        || normalized.ends_with(".jpeg")
        || normalized.ends_with(".png")
        || normalized.ends_with(".webp")
        || normalized.ends_with(".gif")
        || normalized.ends_with(".bmp")
    {
        WechatOutboundKind::Image
    } else if normalized.ends_with(".mp4")
        || normalized.ends_with(".mov")
        || normalized.ends_with(".webm")
        || normalized.ends_with(".mkv")
        || normalized.ends_with(".m4v")
    {
        WechatOutboundKind::Video
    } else {
        WechatOutboundKind::File
    }
}

fn parse_written_bytes_line(line: &str) -> Option<String> {
    let t = line.trim();
    let rest = t.strip_prefix("written ")?;
    let (_bytes, path_part) = rest.split_once(" bytes to ")?;
    let p = normalize_token(path_part);
    if p.is_empty() {
        None
    } else {
        Some(p)
    }
}

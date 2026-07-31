use super::*;

#[test]
fn audio_tokens_are_media_and_missing_paths_remain_visible_for_delivery_errors() {
    let root = std::env::temp_dir();
    let missing = root.join("definitely-missing-channel-audio.mp3");
    let answer = format!(
        "VOICE_FILE:{}\nMUSIC_FILE:{}",
        missing.display(),
        missing
            .with_file_name("definitely-missing-channel-music.mp3")
            .display()
    );
    let media = extract_wechat_outbound_media(&answer, &root);
    assert_eq!(media.len(), 2);
    assert!(media
        .iter()
        .all(|item| item.kind == WechatOutboundKind::Audio));
    assert!(strip_wechat_delivery_lines(&answer).is_empty());
}

use std::fs;

fn temp_media_path(name: &str) -> PathBuf {
    let unique = format!(
        "{}_{}_{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos()
    );
    std::env::temp_dir().join(unique)
}

#[test]
fn extracts_structured_extra_output_path_without_reading_visible_text() {
    let image = temp_media_path("wechat_structured.png");
    fs::write(&image, b"not really an image").expect("write temp media");
    let old_visible = temp_media_path("wechat_visible.png");
    fs::write(&old_visible, b"not really an image").expect("write temp visible media");
    let answer = serde_json::json!({
        "text": format!("图片已保存：{}", old_visible.display()),
        "extra": {
            "media_type": "image",
            "output_path": image.to_string_lossy(),
        }
    })
    .to_string();

    let media = extract_wechat_outbound_media(&answer, Path::new("/"));

    assert_eq!(
        media,
        vec![WechatOutboundMedia {
            kind: WechatOutboundKind::Image,
            source: WechatOutboundSource::LocalPath(
                image.canonicalize().expect("canonicalize temp image")
            ),
        }]
    );
    fs::remove_file(image).ok();
    fs::remove_file(old_visible).ok();
}

#[test]
fn visible_language_media_prefixes_are_not_protocol() {
    let image = temp_media_path("wechat_visible_only.png");
    fs::write(&image, b"not really an image").expect("write temp media");
    let answer = format!("图片已保存：{}", image.display());

    let media = extract_wechat_outbound_media(&answer, Path::new("/"));

    assert!(media.is_empty());
    assert_eq!(strip_wechat_delivery_lines(&answer), answer);
    fs::remove_file(image).ok();
}

#[test]
fn structured_single_line_delivery_is_removed_from_caption() {
    let image = temp_media_path("wechat_structured_line.png");
    fs::write(&image, b"not really an image").expect("write temp media");
    let line = serde_json::json!({
        "media_delivery": {
            "type": "image_file",
            "path": image.to_string_lossy(),
        }
    })
    .to_string();
    let answer = format!("caption\n{line}");

    let media = extract_wechat_outbound_media(&answer, Path::new("/"));

    assert_eq!(media.len(), 1);
    assert_eq!(strip_wechat_delivery_lines(&answer), "caption");
    fs::remove_file(image).ok();
}

#[test]
fn browser_task_artifact_manifest_does_not_trigger_channel_media_delivery() {
    let answer = serde_json::json!({
        "text": "ready",
        "artifacts": [{
            "schema_version": 1,
            "id": "artifact-1",
            "filename": "report.pdf",
            "kind": "pdf",
            "mime_type": "application/pdf",
            "size_bytes": 42,
            "sha256": "a".repeat(64),
            "download_url": "/v1/tasks/task-1/artifacts/artifact-1/content"
        }]
    })
    .to_string();

    assert!(extract_wechat_outbound_media(&answer, Path::new("/")).is_empty());
    assert_eq!(strip_wechat_delivery_lines(&answer), answer);
}

#[test]
fn browser_manifest_preserves_existing_native_channel_media_delivery() {
    let image = temp_media_path("wechat_native_with_browser_manifest.png");
    fs::write(&image, b"not really an image").expect("write temp media");
    let answer = serde_json::json!({
        "extra": {"media_type": "image", "output_path": image.to_string_lossy()},
        "artifacts": [{
            "schema_version": 1,
            "id": "artifact-1",
            "filename": "image.png",
            "kind": "image",
            "mime_type": "image/png",
            "size_bytes": 20,
            "sha256": "b".repeat(64),
            "download_url": "/v1/tasks/task-1/artifacts/artifact-1/content",
            "preview_url": "/v1/tasks/task-1/artifacts/artifact-1/content?disposition=inline"
        }]
    })
    .to_string();

    let media = extract_wechat_outbound_media(&answer, Path::new("/"));
    assert_eq!(media.len(), 1);
    assert_eq!(
        media[0].source,
        WechatOutboundSource::LocalPath(image.canonicalize().expect("canonicalize temp image"))
    );
    fs::remove_file(image).ok();
}

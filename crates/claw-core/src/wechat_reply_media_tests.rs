use super::*;

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

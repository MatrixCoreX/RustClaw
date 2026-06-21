use std::path::Path;

const MEDIA_ARTIFACT_EXTENSIONS: &[&str] = &[
    "aac", "aif", "aiff", "avi", "avif", "bmp", "flac", "gif", "jpeg", "jpg", "m4a", "m4v", "mkv",
    "mov", "mp3", "mp4", "mpeg", "mpg", "ogg", "opus", "png", "tif", "tiff", "wav", "webm", "webp",
];

pub(crate) fn is_media_artifact_path(path: &str) -> bool {
    let path = path.trim();
    if path.is_empty() {
        return false;
    }
    let path = path
        .split_once(['?', '#'])
        .map(|(prefix, _)| prefix)
        .unwrap_or(path);
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|ext| {
            MEDIA_ARTIFACT_EXTENSIONS
                .iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::is_media_artifact_path;

    #[test]
    fn detects_common_binary_media_extensions() {
        assert!(is_media_artifact_path("document/out.PNG"));
        assert!(is_media_artifact_path("/tmp/voice.mp3"));
        assert!(is_media_artifact_path("https://example.test/video.mp4?x=1"));
        assert!(!is_media_artifact_path("document/out.txt"));
        assert!(!is_media_artifact_path("document/out.svg"));
    }
}

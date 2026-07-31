use super::*;

#[test]
fn rejects_empty_and_oversized_files_before_upload() {
    let dir = std::env::temp_dir().join(format!("channel-media-limit-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let empty = dir.join("empty.bin");
    std::fs::write(&empty, []).expect("write empty file");
    assert!(validate_local_media_file(&empty, "test", "文件", 10)
        .unwrap_err()
        .contains("空文件"));

    let large = dir.join("large.bin");
    let file = std::fs::File::create(&large).expect("create sparse file");
    file.set_len(11).expect("set sparse length");
    let error = validate_local_media_file(&large, "test", "视频", 10).unwrap_err();
    assert!(error.contains("过大"));
    assert!(error.contains("UI"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn typed_preflight_exposes_machine_failures_without_localized_prose() {
    let dir = std::env::temp_dir().join(format!(
        "channel-media-preflight-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let missing = preflight_local_media_file(&dir.join("missing.bin"), 10).unwrap_err();
    assert_eq!(missing.failure, LocalMediaPreflightFailure::Unreadable);
    assert_eq!(missing.error_code(), "channel_media_unreadable");
    assert_eq!(missing.message_key(), "channel.media.preflight.unreadable");

    let not_file = preflight_local_media_file(&dir, 10).unwrap_err();
    assert_eq!(not_file.failure, LocalMediaPreflightFailure::NotRegularFile);

    let large = dir.join("large.bin");
    let file = std::fs::File::create(&large).expect("create sparse file");
    file.set_len(11).expect("set sparse length");
    let too_large = preflight_local_media_file(&large, 10).unwrap_err();
    assert_eq!(too_large.failure, LocalMediaPreflightFailure::TooLarge);
    assert_eq!(too_large.actual_bytes, Some(11));
    assert_eq!(too_large.max_bytes, 10);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn whatsapp_video_probe_requires_h264_and_aac_or_no_audio() {
    let compatible = MediaProbe {
        streams: vec![
            MediaProbeStream {
                codec_type: "video".to_string(),
                codec_name: "h264".to_string(),
            },
            MediaProbeStream {
                codec_type: "audio".to_string(),
                codec_name: "aac".to_string(),
            },
        ],
    };
    assert!(video_probe_is_compatible(&compatible));

    let incompatible = MediaProbe {
        streams: vec![
            MediaProbeStream {
                codec_type: "video".to_string(),
                codec_name: "vp9".to_string(),
            },
            MediaProbeStream {
                codec_type: "audio".to_string(),
                codec_name: "opus".to_string(),
            },
        ],
    };
    assert!(!video_probe_is_compatible(&incompatible));
}

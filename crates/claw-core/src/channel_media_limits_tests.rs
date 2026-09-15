use super::*;

#[test]
fn rejects_empty_and_oversized_files_before_upload() {
    let dir = std::env::temp_dir().join(format!("channel-media-limit-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let empty = dir.join("empty.bin");
    std::fs::write(&empty, []).expect("write empty file");
    assert_eq!(
        validate_local_media_file(&empty, "test", "file", 10).unwrap_err(),
        "channel_media_preflight_failed:channel_media_empty:0:10"
    );

    let large = dir.join("large.bin");
    let file = std::fs::File::create(&large).expect("create sparse file");
    file.set_len(11).expect("set sparse length");
    assert_eq!(
        validate_local_media_file(&large, "test", "video", 10).unwrap_err(),
        "channel_media_preflight_failed:channel_media_too_large:11:10"
    );

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
    assert_eq!(too_large.max_bytes, Some(10));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unlimited_wechat_media_keep_file_validation_without_a_byte_ceiling() {
    let dir = std::env::temp_dir().join(format!(
        "wechat-media-unlimited-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp directory");
    let path = dir.join("video.mp4");
    let file = std::fs::File::create(&path).expect("sparse test file");
    for limit in [wechat_image_max_bytes(), wechat_video_max_bytes(), wechat_file_max_bytes()] {
        assert_eq!(limit, None);
        for size in [127_140_539, 4 * 1024 * MIB] {
            file.set_len(size).expect("sparse file length");
            assert_eq!(preflight_local_media_file(&path, limit).unwrap(), size);
            assert_eq!(
                validate_local_media_file(&path, "wechat_ilink", "video", limit).unwrap(),
                size
            );
        }
        file.set_len(0).expect("empty file");
        assert_eq!(
            preflight_local_media_file(&path, limit)
                .unwrap_err()
                .failure,
            LocalMediaPreflightFailure::Empty
        );
        assert_eq!(
            validate_local_media_file(&path, "wechat_ilink", "file", limit).unwrap_err(),
            "channel_media_preflight_failed:channel_media_empty:0:none"
        );
        assert_eq!(
            preflight_local_media_file(&dir, limit).unwrap_err().failure,
            LocalMediaPreflightFailure::NotRegularFile
        );
        assert_eq!(
            preflight_local_media_file(&dir.join("missing"), limit)
                .unwrap_err()
                .failure,
            LocalMediaPreflightFailure::Unreadable
        );
    }
    file.set_len(127_140_539).unwrap();
    for limit in [
        telegram_file_max_bytes(),
        lark_image_max_bytes(),
        feishu_file_max_bytes(),
    ] {
        assert_eq!(
            preflight_local_media_file(&path, limit)
                .unwrap_err()
                .failure,
            LocalMediaPreflightFailure::TooLarge
        );
    }
    drop(file);
    std::fs::remove_dir_all(dir).expect("remove temporary test files");
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

use super::*;

#[test]
fn advertised_attachment_constraints_match_materialization_enforcement() {
    let value = serde_json::to_value(ui_attachment_constraints()).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["channel"], "ui_base64");
    assert_eq!(value["max_attachments"], MAX_UI_ATTACHMENTS);
    assert_eq!(value["max_attachment_bytes"], MAX_UI_ATTACHMENT_BYTES);
    assert_eq!(
        value["max_total_attachment_bytes"],
        MAX_UI_TOTAL_ATTACHMENT_BYTES
    );
    assert_eq!(
        value["error_codes"],
        json!([
            "ui_attachments_too_many",
            "ui_attachment_too_large",
            "ui_attachments_total_too_large"
        ])
    );
}

#[test]
fn data_url_decodes_mime_and_bytes() {
    let (bytes, mime) =
        decode_data_url("data:text/plain;base64,aGVsbG8=").expect("decode data url");
    assert_eq!(bytes, b"hello");
    assert_eq!(mime.as_deref(), Some("text/plain"));
}

#[test]
fn safe_upload_filename_adds_extension() {
    assert_eq!(
        safe_upload_filename("../voice", "audio", "audio/webm"),
        "voice.webm"
    );
}

#[test]
fn office_mime_types_keep_machine_readable_extensions_without_a_name() {
    for (mime_type, expected) in [
        (
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "attachment.docx",
        ),
        (
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "attachment.xlsx",
        ),
        (
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "attachment.pptx",
        ),
    ] {
        assert_eq!(
            default_name_for_attachment("file", mime_type),
            expected,
            "{mime_type}"
        );
    }
}

#[test]
fn channel_ingress_attachment_validation_accepts_one_stable_regular_file() {
    let root = std::env::temp_dir().join(format!("channel-attachment-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("data/channel")).expect("create fixture");
    std::fs::write(root.join("data/channel/message.bin"), b"payload").expect("write fixture");
    let attachment = ChannelIngressAttachment {
        kind: "file".to_string(),
        path: "data/channel/message.bin".to_string(),
        mime_type: Some("application/octet-stream".to_string()),
        size: Some(7),
    };
    assert_eq!(
        validate_channel_ingress_attachments(&root, &[attachment]),
        Ok(())
    );
    std::fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn channel_ingress_attachment_validation_rejects_escape_symlink_and_size_mismatch() {
    let root = std::env::temp_dir().join(format!("channel-attachment-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("data/channel")).expect("create fixture");
    std::fs::write(root.join("data/channel/message.bin"), b"payload").expect("write fixture");
    let attachment = |path: &str, size| ChannelIngressAttachment {
        kind: "file".to_string(),
        path: path.to_string(),
        mime_type: None,
        size,
    };
    assert_eq!(
        validate_channel_ingress_attachments(&root, &[attachment("../outside", None)]),
        Err("channel_attachment_path_invalid")
    );
    assert_eq!(
        validate_channel_ingress_attachments(
            &root,
            &[attachment("data/channel/message.bin", Some(8))]
        ),
        Err("channel_attachment_size_mismatch")
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            root.join("data/channel/message.bin"),
            root.join("data/channel/link.bin"),
        )
        .expect("create symlink");
        assert_eq!(
            validate_channel_ingress_attachments(
                &root,
                &[attachment("data/channel/link.bin", None)]
            ),
            Err("channel_attachment_type_invalid")
        );
    }
    std::fs::remove_dir_all(root).expect("remove fixture");
}

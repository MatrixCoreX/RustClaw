use super::*;

#[test]
fn local_io_failure_is_typed_and_contains_no_exception_path_or_secret() {
    for (kind, code) in [
        (
            io::ErrorKind::QuotaExceeded,
            "channel.media.local_quota_exceeded",
        ),
        (
            io::ErrorKind::StorageFull,
            "channel.media.local_storage_full",
        ),
        (
            io::ErrorKind::PermissionDenied,
            "channel.media.local_permission_denied",
        ),
        (io::ErrorKind::NotFound, "channel.media.local_unavailable"),
        (
            io::ErrorKind::UnexpectedEof,
            "channel.media.local_input_changed",
        ),
    ] {
        let error = ChannelMediaIoError::from_io(
            MediaIoOperation::Read,
            &io::Error::new(kind, "private-path secret-token"),
        );
        let frame = error.to_string();
        assert!(!frame.contains("private-path") && !frame.contains("secret-token"));
        assert_eq!(error.error_code(), code);
        assert_eq!(ChannelMediaIoError::decode(&frame), Some(error));
    }
}

#[test]
fn malformed_or_unversioned_local_io_frames_are_rejected() {
    let frame = ChannelMediaIoError::input_changed().to_string();
    assert!(ChannelMediaIoError::decode("Disk quota exceeded (os error 122)").is_none());
    assert!(ChannelMediaIoError::decode(
        &frame.replace("\"schema_version\":1", "\"schema_version\":2")
    )
    .is_none());
    assert!(
        ChannelMediaIoError::decode(&frame.replace("\"validate\"", "\"unsupported\"")).is_none()
    );
    assert!(
        ChannelMediaIoError::decode(&frame.replace("\"changed\"", "\"secret-path\"")).is_none()
    );
    let wrapped = io::Error::other(ChannelMediaIoError::input_changed());
    assert_eq!(
        ChannelMediaIoError::find_in_chain(&wrapped)
            .unwrap()
            .error_code(),
        "channel.media.local_input_changed"
    );
}

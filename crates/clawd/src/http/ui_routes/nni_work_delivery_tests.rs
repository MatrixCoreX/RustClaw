use super::*;

fn delivery() -> NniArtifactDelivery {
    NniArtifactDelivery { schema_version: 1, job_id: uuid::Uuid::new_v4().to_string(),
        origin: "https://media.example.org".to_string(), upload_token: "a".repeat(64),
        required_kinds: vec!["video".to_string(), "audio".to_string()], max_file_bytes: 1024 }
}

#[test]
fn artifact_delivery_rejects_credentials_paths_and_unbounded_requests() {
    let mut grant = delivery();
    assert!(nni_validate_artifact_delivery(&grant));
    for origin in ["http://media.example.org", "https://media.example.org/path", "https://a:b@media.example.org", "https://media.example.org/?token=a"] {
        grant.origin = origin.to_string();
        assert!(!nni_validate_artifact_delivery(&grant));
    }
    grant = delivery(); grant.required_kinds.push("video".to_string());
    assert!(!nni_validate_artifact_delivery(&grant));
    grant = delivery(); grant.max_file_bytes = 512 * 1024 * 1024;
    assert!(nni_validate_artifact_delivery(&grant));
    grant.max_file_bytes = 513 * 1024 * 1024;
    assert!(!nni_validate_artifact_delivery(&grant));
    grant = delivery(); grant.job_id = "../../secret".to_string();
    assert!(!nni_validate_artifact_delivery(&grant));
}

#[test]
fn artifact_filename_header_encodes_unicode_and_header_delimiters() {
    assert_eq!(percent_encode_filename("video.mp4"), "video.mp4");
    assert_eq!(percent_encode_filename("a b\r\n.mp4"), "a%20b%0D%0A.mp4");
    assert!(percent_encode_filename("音频.m4a").is_ascii());
}

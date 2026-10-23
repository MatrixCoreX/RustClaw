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

#[test]
fn original_media_delivery_accepts_video_or_image_archives_only() {
    assert!(nni_delivery_kind_matches("media", "video", "video/mp4"));
    assert!(nni_delivery_kind_matches("media", "file", "application/zip"));
    assert!(!nni_delivery_kind_matches("media", "file", "application/json"));
    assert!(!nni_delivery_kind_matches("media", "audio", "audio/mp4"));
    let mut grant = delivery();
    grant.required_kinds = vec!["media".to_string(), "audio".to_string()];
    assert!(nni_validate_artifact_delivery(&grant));
}

#[test]
fn host_artifact_kinds_project_only_supported_mime_pairs() {
    assert!(nni_delivery_kind_matches("media", "archive", "application/zip"));
    assert_eq!(nni_artifact_upload_kind("archive", "application/zip"), Some("file"));
    assert_eq!(nni_artifact_upload_kind("file", "text/plain"), Some("text"));
    assert_eq!(nni_artifact_upload_kind("file", "text/plain; charset=utf-8"), Some("text"));
    for mime in ["image/jpeg", "image/png", "image/webp", "image/gif", "image/avif"] {
        assert_eq!(nni_artifact_upload_kind("image", mime), Some("image"));
        assert!(nni_delivery_kind_matches("media", "image", mime));
    }
    for (kind, mime) in [("archive", "application/gzip"), ("file", "application/json"),
        ("file", "text/html"), ("image", "image/svg+xml"), ("image", "text/html"),
        ("video", "text/plain")] {
        assert_eq!(nni_artifact_upload_kind(kind, mime), None);
        assert!(!nni_delivery_kind_matches("media", kind, mime));
    }
}

#[tokio::test]
async fn materialized_single_image_archive_and_articles_use_receiver_wire_kinds() {
    let workspace = std::env::temp_dir().join(format!("agent-runtime-work-artifacts-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    let zip = workspace.join("image_bundle.zip");
    let article = workspace.join("post_article.txt");
    let fallback_article = workspace.join("fallback_article.txt");
    let image = workspace.join("original.webp");
    std::fs::write(&zip, b"PK\x03\x04image-archive").unwrap();
    std::fs::write(&article, b"Original platform article").unwrap();
    std::fs::write(&fallback_article, b"Article with inferred UTF-8 MIME").unwrap();
    std::fs::write(&image, b"RIFF-original-raster-image").unwrap();
    let local_task_id = "task-original-image-article";
    let raw = json!({"artifacts": [
        {"path": zip, "mime_type": "application/zip"},
        {"path": article, "mime_type": "text/plain"},
        {"path": fallback_article},
        {"path": image, "mime_type": "image/webp"}
    ]});
    let materialized = crate::task_artifacts::materialize_task_result_artifacts(
        &workspace, local_task_id, &raw.to_string(),
    ).unwrap();
    let result: Value = serde_json::from_str(&materialized).unwrap();
    let manifests = crate::task_artifacts::manifests_from_result(Some(&result));
    assert_eq!(manifests.len(), 4);
    assert_eq!((&*manifests[0].kind, &*manifests[0].mime_type), ("archive", "application/zip"));
    assert_eq!((&*manifests[1].kind, &*manifests[1].mime_type), ("file", "text/plain"));
    assert_eq!((&*manifests[2].kind, &*manifests[2].mime_type), ("file", "text/plain; charset=utf-8"));
    assert_eq!((&*manifests[3].kind, &*manifests[3].mime_type), ("image", "image/webp"));
    assert_eq!(manifests.iter().filter(|m| nni_delivery_kind_matches("media", &m.kind, &m.mime_type)).count(), 2);
    assert_eq!(manifests.iter().filter(|m| nni_artifact_upload_kind(&m.kind, &m.mime_type) == Some("text")).count(), 2);

    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = axum::Router::new().fallback(move |headers: HeaderMap| {
        let sender = sender.clone();
        async move {
            let sha256 = headers.get("x-artifact-sha256").unwrap().to_str().unwrap().to_string();
            sender.send(headers).unwrap();
            Json(json!({"ok": true, "data": {"status": "already_uploaded", "sha256": sha256}}))
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut grant = delivery();
    grant.origin = format!("http://{address}"); // Loopback exercises the HTTP adapter, not grant validation.
    let assignment = NniWorkAssignment {
        schema_version: 1, heartbeat_task_id: "heartbeat-fixture".to_string(), task_id: "work-fixture".to_string(),
        task_type: "capability_v1".to_string(), target_device_pubkey: "a".repeat(128),
        target_platform: "linux".to_string(), target_arch: "x86_64".to_string(), attempt: 1,
        lease_token: "b".repeat(64), lease_expires_at_unix: 1, payload_canonical: "{}".to_string(),
        payload_sha256: "c".repeat(64), issued_at_unix: 0,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for (manifest, wire_kind) in manifests.iter().zip(["file", "text", "text", "image"]) {
        let original = manifest.clone();
        let path = crate::task_artifacts::validated_delivery_artifact_path(&workspace, local_task_id, manifest).unwrap();
        nni_upload_artifact_chunks(&client, &assignment, &grant, manifest, &path).await.unwrap();
        let headers = tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await.unwrap().unwrap();
        assert_eq!(headers.get("x-artifact-kind").unwrap(), wire_kind);
        assert_eq!(headers.get("content-type").unwrap(), manifest.mime_type.as_str());
        assert_eq!(headers.get("x-artifact-sha256").unwrap(), manifest.sha256.as_str());
        assert_eq!(*manifest, original);
    }
    server.abort();
    std::fs::remove_dir_all(workspace).unwrap();
}

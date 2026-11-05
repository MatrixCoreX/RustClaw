//! File confinement, type and size rejection contracts.
use super::*;

#[tokio::test]
async fn aipp_asset_handlers_keep_path_type_and_size_rejections() {
    let preview = SurfaceFixture::new(Surface::Preview);
    write_record(
        preview.storage_root(),
        1,
        json!({"image_screenshot_path":"../outside.png"}),
    );
    error_body(
        request(
            Surface::Preview,
            &preview.state,
            Some("aipp-view-admin"),
            "",
        )
        .await,
        StatusCode::NOT_FOUND,
        "aipp_preview_path_invalid",
        false,
    )
    .await;
    write_record(
        preview.storage_root(),
        1,
        json!({"image_screenshot_path":"images/preview.png"}),
    );
    fs::File::create(preview.storage_root().join("exports/images/preview.png"))
        .unwrap()
        .set_len(AIPP_PREVIEW_MAX_BYTES + 1)
        .unwrap();
    error_body(
        request(
            Surface::Preview,
            &preview.state,
            Some("aipp-view-admin"),
            "",
        )
        .await,
        StatusCode::NOT_FOUND,
        "aipp_preview_path_invalid",
        false,
    )
    .await;
    let bundle = SurfaceFixture::new(Surface::Bundle);
    error_body(
        request(
            Surface::Bundle,
            &bundle.state,
            Some("aipp-view-admin"),
            "aipp/../outside.txt",
        )
        .await,
        StatusCode::NOT_FOUND,
        "aipp_bundle_path_invalid",
        false,
    )
    .await;
    fs::write(bundle.bundle_root().join("unknown.bin"), b"fixture").unwrap();
    error_body(
        request(
            Surface::Bundle,
            &bundle.state,
            Some("aipp-view-admin"),
            "aipp/unknown.bin",
        )
        .await,
        StatusCode::NOT_FOUND,
        "aipp_bundle_type_unsupported",
        false,
    )
    .await;
    fs::File::create(bundle.bundle_root().join("index.html"))
        .unwrap()
        .set_len(AIPP_BUNDLE_ASSET_MAX_BYTES + 1)
        .unwrap();
    error_body(
        request(Surface::Bundle, &bundle.state, Some("aipp-view-admin"), "").await,
        StatusCode::NOT_FOUND,
        "aipp_bundle_path_invalid",
        false,
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn aipp_asset_handlers_reject_symlink_escapes_after_preparation_offload() {
    for surface in [Surface::Preview, Surface::Bundle] {
        let fixture = SurfaceFixture::new(surface);
        let outside = fixture.workspace.0.join("outside.png");
        fs::write(&outside, b"private fixture").unwrap();
        let target = if matches!(surface, Surface::Bundle) {
            fixture.bundle_root().join("index.html")
        } else {
            fixture.storage_root().join("exports/images/preview.png")
        };
        fs::remove_file(&target).unwrap();
        std::os::unix::fs::symlink(outside, target).unwrap();
        error_body(
            request(surface, &fixture.state, Some("aipp-view-admin"), "").await,
            StatusCode::NOT_FOUND,
            if matches!(surface, Surface::Bundle) {
                "aipp_bundle_path_invalid"
            } else {
                "aipp_preview_path_invalid"
            },
            false,
        )
        .await;
    }
}

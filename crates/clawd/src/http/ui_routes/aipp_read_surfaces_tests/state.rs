//! Fresh state and streaming-response contracts.
use super::*;

#[tokio::test]
async fn aipp_catalog_reads_fresh_manifest_and_install_state_without_changing_skill_enable() {
    let fixture = SurfaceFixture::new(Surface::Catalog);
    for installed in [false, true] {
        aipp_admission_service(&fixture.state)
            .unwrap()
            .set_aipp_installed("fixture_view", installed)
            .unwrap();
        let mut manifest = skill_sdk::PackageManifest::load(&fixture.manifest_path()).unwrap();
        manifest
            .aipp
            .as_mut()
            .unwrap()
            .titles
            .insert("en".into(), format!("Fixture {installed}"));
        fs::write(fixture.manifest_path(), manifest.to_toml_string().unwrap()).unwrap();
        let response = request(
            Surface::Catalog,
            &fixture.state,
            Some("aipp-view-admin"),
            "",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["data"]["apps"][0]["installed"], installed);
        assert_eq!(
            body["data"]["apps"][0]["titles"]["en"],
            format!("Fixture {installed}")
        );
        assert!(fixture.state.get_skills_list().contains("fixture_view"));
    }
}

#[tokio::test]
async fn aipp_assets_stream_fresh_bytes_with_existing_security_headers_and_install_semantics() {
    for surface in [Surface::Preview, Surface::Bundle] {
        let fixture = SurfaceFixture::new(surface);
        let expected = match surface {
            Surface::Preview => PREVIEW,
            _ => BUNDLE,
        };
        let response = request(surface, &fixture.state, Some("aipp-view-admin"), "").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[axum::http::header::CACHE_CONTROL],
            "private, max-age=300"
        );
        assert_eq!(
            response.headers()[axum::http::header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_LENGTH]
                .to_str()
                .unwrap()
                .parse::<usize>()
                .unwrap(),
            expected.len()
        );
        if matches!(surface, Surface::Bundle) {
            assert_eq!(
                response.headers()[axum::http::header::CONTENT_SECURITY_POLICY],
                AIPP_BUNDLE_CSP
            );
            assert_eq!(
                response.headers()[axum::http::header::REFERRER_POLICY],
                "no-referrer"
            );
            assert!(response.headers().contains_key("permissions-policy"));
        } else {
            assert_eq!(
                response.headers()[axum::http::header::CONTENT_TYPE],
                "image/png"
            );
        }
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap()
                .as_ref(),
            expected
        );
        aipp_admission_service(&fixture.state)
            .unwrap()
            .set_aipp_installed("fixture_view", false)
            .unwrap();
        error_body(
            request(surface, &fixture.state, Some("aipp-view-admin"), "").await,
            StatusCode::NOT_FOUND,
            if matches!(surface, Surface::Bundle) {
                "aipp_not_installed"
            } else {
                "aipp_not_available"
            },
            false,
        )
        .await;
        aipp_admission_service(&fixture.state)
            .unwrap()
            .set_aipp_installed("fixture_view", true)
            .unwrap();
        let file = if matches!(surface, Surface::Bundle) {
            fixture.bundle_root().join("index.html")
        } else {
            fixture.storage_root().join("exports/images/preview.png")
        };
        fs::write(file, b"updated fixture").unwrap();
        let response = request(surface, &fixture.state, Some("aipp-view-admin"), "").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap()
                .as_ref(),
            b"updated fixture"
        );
    }
}

#[tokio::test]
async fn aipp_read_surfaces_require_auth_before_invalid_manifest_and_fail_closed_after_it_changes()
{
    for surface in SURFACES {
        let fixture = SurfaceFixture::new(surface);
        fs::write(fixture.manifest_path(), "invalid manifest").unwrap();
        error_body(
            request(surface, &fixture.state, Some("aipp-view-user"), "").await,
            StatusCode::FORBIDDEN,
            "admin_required",
            true,
        )
        .await;
        let response = request(surface, &fixture.state, Some("aipp-view-admin"), "").await;
        if matches!(surface, Surface::Catalog) {
            assert_eq!(response.status(), StatusCode::OK);
            let body: Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 65536)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(body["data"]["apps"], json!([]));
        } else {
            error_body(response, StatusCode::NOT_FOUND, "aipp_not_available", false).await;
        }
    }
}

#[tokio::test]
async fn aipp_bundle_revalidates_artifact_digests_without_caching_success_or_failure() {
    let fixture = SurfaceFixture::new(Surface::Bundle);
    let install = fixture.admitted_bundle();
    assert_eq!(
        request(Surface::Bundle, &fixture.state, Some("aipp-view-admin"), "")
            .await
            .status(),
        StatusCode::OK
    );
    fs::write(install.join("aipp/index.html"), b"tampered package").unwrap();
    error_body(
        request(Surface::Bundle, &fixture.state, Some("aipp-view-admin"), "").await,
        StatusCode::NOT_FOUND,
        "aipp_install_receipt_invalid",
        false,
    )
    .await;
    fs::write(install.join("aipp/index.html"), BUNDLE).unwrap();
    let response = request(Surface::Bundle, &fixture.state, Some("aipp-view-admin"), "").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap()
            .as_ref(),
        BUNDLE
    );
}

#[tokio::test]
async fn aipp_bundle_rechecks_registry_generation_binding_on_every_request() {
    let fixture = SurfaceFixture::new(Surface::Bundle);
    fixture.admitted_bundle();
    let original = fixture.state.get_skill_views_snapshot();
    let mut bindings = original.binding.admission_bindings.as_ref().clone();
    bindings.get_mut("fixture_view").unwrap().manifest_digest = "0".repeat(64);
    let mut changed = crate::SkillViewsSnapshot {
        registry: original.registry.clone(),
        skills_list: original.skills_list.clone(),
        binding: Default::default(),
    };
    changed.binding.admission_bindings = Arc::new(bindings);
    *fixture.state.core.skill_views_snapshot.write().unwrap() = Arc::new(changed);
    error_body(
        request(Surface::Bundle, &fixture.state, Some("aipp-view-admin"), "").await,
        StatusCode::NOT_FOUND,
        "aipp_not_available",
        false,
    )
    .await;
    *fixture.state.core.skill_views_snapshot.write().unwrap() = original;
    assert_eq!(
        request(Surface::Bundle, &fixture.state, Some("aipp-view-admin"), "")
            .await
            .status(),
        StatusCode::OK
    );
}

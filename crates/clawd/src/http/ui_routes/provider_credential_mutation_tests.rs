use super::*;

fn prepared(
    fixture: &Fixture,
    values: &[(&str, &str)],
) -> ([ModelSettingsFile; 5], Vec<(String, String)>) {
    let mut files = load_model_settings_files(&fixture.state, true).unwrap();
    let requested = values
        .iter()
        .map(|(vendor, key)| (vendor.to_string(), key.to_string()))
        .collect();
    let updates = prepare_text_provider_keys(&fixture.state, &mut files[0], &requested).unwrap();
    (files, updates)
}

#[tokio::test]
async fn provider_credentials_failed_persistence_restores_the_complete_vendor_batch_and_configuration(
) {
    let fixture = fixture(&["custom", "mimo"]);
    let before = fixture.files();
    let credentials = credential_bytes(&fixture);
    let mirror = fixture.root.join("invalid-mirror.toml");
    fs::create_dir(&mirror).unwrap();
    let (files, updates) = prepared(&fixture, &[("custom", NEW_KEY), ("mimo", NEW_KEY)]);
    let result = publish_model_provider_keys(&fixture.state, files, &updates, Some(mirror.clone()));
    assert!(result.is_err());
    assert_eq!(fixture.files(), before);
    assert_eq!(credential_bytes(&fixture), credentials);
    assert!(mirror.is_dir());
}

#[tokio::test]
async fn provider_credentials_selected_main_and_mirror_publish_only_empty_text_key_references() {
    let mut fixture = fixture(&["custom", "mimo", "google"]);
    let defaults = fixture.files();
    let directory = fixture.root.join("selected");
    fs::create_dir(&directory).unwrap();
    for name in [
        "config.toml",
        "image.toml",
        "audio.toml",
        "video.toml",
        "music.toml",
    ] {
        fs::copy(
            fixture.root.join("configs").join(name),
            directory.join(name),
        )
        .unwrap();
    }
    fixture.state.reload_ctx.config_path_for_reload = "selected/config.toml".into();
    let mirror = fixture.root.join("persisted.toml");
    fs::write(&mirror, b"# old fixture mirror\n").unwrap();
    let (files, updates) = prepared(&fixture, &[("custom", NEW_KEY), ("mimo", NEW_KEY)]);
    publish_model_provider_keys(&fixture.state, files, &updates, Some(mirror.clone())).unwrap();
    let raw = fs::read_to_string(directory.join("config.toml")).unwrap();
    assert_eq!(raw.as_bytes(), fs::read(mirror).unwrap().as_slice());
    assert!(!raw.contains(NEW_KEY));
    assert_eq!(fixture.files(), defaults);
    for vendor in ["custom", "mimo"] {
        assert_eq!(effective(&fixture, vendor).as_deref(), Some(NEW_KEY));
    }
    assert_eq!(effective(&fixture, "google").as_deref(), Some(OLD_KEY));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(model_environment::path(&fixture.root))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn provider_credentials_corrupt_managed_file_fails_closed_without_fallback_or_secret_leakage()
{
    let fixture = fixture(&["custom"]);
    let path = model_environment::path(&fixture.root);
    fs::write(
        &path,
        format!("CUSTOM_API_KEY={OLD_KEY}\nCUSTOM_API_KEY={NEW_KEY}\n"),
    )
    .unwrap();
    let before = fixture.files();
    let credentials = fs::read(&path).unwrap();
    for (method, payload) in [
        (Method::GET, None),
        (Method::POST, Some(json!({"llm":{"custom":NEW_KEY}}))),
    ] {
        let response = fixture.api(ADMIN, method, ENDPOINT, payload).await;
        assert_eq!(response.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.1["error"], "provider_keys_read_failed");
        assert_private(&fixture, &response.1);
        assert_eq!(fixture.files(), before);
        assert_eq!(fs::read(&path).unwrap(), credentials);
    }
}

#[tokio::test]
async fn provider_credentials_readback_matches_runtime_broker_without_a_managed_override() {
    let fixture = fixture(&["custom"]);
    fs::remove_file(model_environment::path(&fixture.root)).unwrap();
    let private = claw_core::git_remote_config::git_credential_store_path(&fixture.root);
    claw_core::secrets::set_file_secret(
        &private,
        "text_custom_api_key",
        "fixture-private-fallback",
    )
    .unwrap();
    let expected = effective(&fixture, "custom").unwrap();
    let before = fixture.files();
    let private_before = fs::read(&private).unwrap();
    let response = fixture.api(ADMIN, Method::GET, ENDPOINT, None).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["data"]["llm"]["custom"], mask_secret(&expected));
    assert!(!response.1.to_string().contains("fixture-private-fallback"));
    assert_eq!(fixture.files(), before);
    assert_eq!(fs::read(private).unwrap(), private_before);
    assert!(!model_environment::path(&fixture.root).exists());
}

#[tokio::test]
async fn provider_credentials_unknown_display_mask_is_rejected_without_rotating_a_key() {
    let fixture = fixture(&["custom"]);
    let before = fixture.files();
    let credentials = credential_bytes(&fixture);
    for mask in ["****", "abcd****efgh"] {
        let response = fixture
            .api(
                ADMIN,
                Method::POST,
                ENDPOINT,
                Some(json!({"llm":{"custom":mask}})),
            )
            .await;
        assert_eq!(response.0, StatusCode::BAD_REQUEST);
        assert_eq!(response.1["error"], "provider_keys_mask_invalid");
        assert_eq!(fixture.files(), before);
        assert_eq!(credential_bytes(&fixture), credentials);
    }
}

#[tokio::test]
async fn provider_credentials_invalid_supported_vendor_value_cannot_partially_rotate_another_vendor(
) {
    let fixture = fixture(&["custom", "mimo"]);
    let before = fixture.files();
    let credentials = credential_bytes(&fixture);
    let response = fixture
        .api(
            ADMIN,
            Method::POST,
            ENDPOINT,
            Some(json!({"llm":{"custom":NEW_KEY,"mimo":"fixture\ninvalid"}})),
        )
        .await;
    assert_eq!(response.0, StatusCode::BAD_REQUEST);
    assert_eq!(fixture.files(), before);
    assert_eq!(credential_bytes(&fixture), credentials);
}

#[tokio::test]
async fn provider_credentials_legacy_and_modern_vendor_updates_share_the_same_publication_boundary()
{
    let fixture = fixture(&["custom", "mimo"]);
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        ADMIN.parse().unwrap(),
    );
    let legacy = UpdateLlmConfigRequest {
        selected_vendor: "custom".into(),
        selected_model: "fixture-after-legacy".into(),
        vendor_base_url: Some("https://fixture.invalid/v1".into()),
        vendor_api_key: Some("fixture-legacy-replacement".into()),
        vendor_api_format: Some("openai_compat".into()),
    };
    let (modern, legacy) = tokio::join!(
        fixture.api(
            ADMIN,
            Method::POST,
            ENDPOINT,
            Some(json!({"llm":{"mimo":NEW_KEY}}))
        ),
        update_llm_config(State(fixture.state.clone()), headers, Json(legacy))
    );
    assert_eq!(modern.0, StatusCode::OK);
    assert_eq!(legacy.0, StatusCode::OK);
    assert_eq!(
        effective(&fixture, "custom").as_deref(),
        Some("fixture-legacy-replacement")
    );
    assert_eq!(effective(&fixture, "mimo").as_deref(), Some(NEW_KEY));
    let parsed: toml::Value =
        toml::from_str(&fs::read_to_string(fixture.root.join("configs/config.toml")).unwrap())
            .unwrap();
    assert_eq!(
        parsed["llm"]["selected_model"].as_str(),
        Some("fixture-after-legacy")
    );
    for vendor in ["custom", "mimo"] {
        assert_eq!(parsed["llm"][vendor]["api_key"].as_str(), Some(""));
    }
    assert_private(&fixture, &modern.1);
}

#[tokio::test]
async fn provider_credentials_provider_neutral_punctuation_is_literal_and_does_not_require_a_key_prefix(
) {
    let fixture = fixture(&["custom"]);
    let key = "fixture'\"\\= $VALUE # ; *** suffix";
    let response = fixture
        .api(
            ADMIN,
            Method::POST,
            ENDPOINT,
            Some(json!({"llm":{"custom":key}})),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(effective(&fixture, "custom").as_deref(), Some(key));
    assert!(!response.1.to_string().contains(key));
}

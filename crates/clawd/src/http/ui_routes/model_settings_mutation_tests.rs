use super::*;

#[cfg(unix)]
#[tokio::test]
async fn model_settings_new_optional_files_are_private_and_existing_metadata_is_preserved() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let fixture = Fixture::new();
    let image = fixture.root.join("configs/image.toml");
    let audio = fixture.root.join("configs/audio.toml");
    fs::remove_file(&image).unwrap();
    fs::set_permissions(&audio, fs::Permissions::from_mode(0o660)).unwrap();
    let before = fs::metadata(&audio).unwrap();
    let mut payload = model_payload();
    payload["image_vision"]["api_key"] = json!(NEW_SECRET);
    let response = fixture
        .api(ADMIN, Method::POST, MODELS, Some(payload))
        .await;
    assert_eq!(response.0, StatusCode::OK);
    fixture.assert_private(&response.1);
    assert_eq!(fs::metadata(&image).unwrap().mode() & 0o777, 0o600);
    assert!(fs::read_to_string(&image).unwrap().contains(NEW_SECRET));
    let after = fs::metadata(&audio).unwrap();
    assert_eq!(
        (after.uid(), after.gid(), after.mode() & 0o777),
        (before.uid(), before.gid(), before.mode() & 0o777)
    );
}

#[tokio::test]
async fn model_settings_save_preserves_unrelated_fields_and_secret_free_response_contracts() {
    let fixture = Fixture::new();
    let response = fixture
        .api(ADMIN, Method::POST, MODELS, Some(model_payload()))
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["ok"], true);
    assert_eq!(response.1["data"]["restart_required"], true);
    for section in [
        "llm",
        "image_vision",
        "audio_transcribe",
        "video_generation",
        "music_generation",
    ] {
        assert_eq!(response.1["data"][section]["model"], "fixture-next");
    }
    fixture.assert_private(&response.1);
    for (name, raw) in fixture.files() {
        let raw = String::from_utf8(raw.unwrap()).unwrap();
        let parsed: toml::Value = toml::from_str(&raw).unwrap();
        assert!(parsed.get("sentinel").is_some(), "{name}");
        assert!(raw.contains(SECRET), "existing key lost: {name}");
    }
    for key in [USER, ADMIN] {
        let read = fixture.api(key, Method::GET, MODELS, None).await;
        assert_eq!(read, response);
    }
}

#[tokio::test]
async fn model_settings_structural_failure_in_later_module_cannot_partially_save() {
    for keys in [false, true] {
        let fixture = Fixture::new();
        fixture.write(
            "audio.toml",
            "[audio_transcribe]\nproviders='fixture-invalid-shape'\n",
        );
        let before = fixture.files();
        let response = fixture
            .api(
                ADMIN,
                Method::POST,
                if keys { KEYS } else { MODELS },
                Some(if keys { key_payload() } else { model_payload() }),
            )
            .await;
        assert_error(
            &response,
            StatusCode::INTERNAL_SERVER_ERROR,
            if keys {
                "provider_keys_write_failed"
            } else {
                "model_config_write_failed"
            },
        );
        assert_eq!(fixture.files(), before);
        fixture.assert_private(&response.1);
    }
}

#[tokio::test]
async fn model_settings_masked_key_round_trip_is_a_byte_preserving_noop() {
    let fixture = Fixture::new();
    let before = fixture.files();
    let original = fixture.api(ADMIN, Method::GET, KEYS, None).await;
    assert_eq!(original.0, StatusCode::OK);
    fixture.assert_private(&original.1);
    let response = fixture
        .api(ADMIN, Method::POST, KEYS, Some(original.1["data"].clone()))
        .await;
    assert_eq!(response, original);
    assert_eq!(fixture.files(), before);
    let response = fixture
        .api(ADMIN, Method::POST, KEYS, Some(key_payload()))
        .await;
    assert_eq!(response.0, StatusCode::OK);
    fixture.assert_private(&response.1);
    for (name, raw) in fixture.files() {
        let raw = String::from_utf8(raw.unwrap()).unwrap();
        if name == "config.toml" {
            assert!(
                !raw.contains(NEW_SECRET),
                "text keys must remain write-only broker secrets"
            );
            let parsed: toml::Value = toml::from_str(&raw).unwrap();
            assert_eq!(parsed["llm"]["custom"]["api_key"].as_str(), Some(""));
            assert_eq!(
                effective_model_provider_key(&fixture.state, "custom").unwrap(),
                NEW_SECRET
            );
        } else {
            assert!(raw.contains(NEW_SECRET));
        }
        assert!(!raw.contains(SECRET));
        assert!(toml::from_str::<toml::Value>(&raw)
            .unwrap()
            .get("sentinel")
            .is_some());
    }
}

#[tokio::test]
async fn model_settings_model_form_cannot_replace_a_key_with_its_display_mask() {
    let fixture = Fixture::new();
    let payload = json!({"image_vision":{"vendor":"fixture","model":"fixture-next","api_key":mask_secret(SECRET)}});
    let response = fixture
        .api(ADMIN, Method::POST, MODELS, Some(payload))
        .await;
    assert_eq!(response.0, StatusCode::OK);
    let raw = fs::read_to_string(fixture.root.join("configs/image.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&raw).unwrap();
    assert_eq!(
        parsed["image_vision"]["providers"]["fixture"]["api_key"].as_str(),
        Some(SECRET)
    );
}

#[tokio::test]
async fn model_settings_concurrent_forms_preserve_each_others_keys_and_model_selection() {
    let fixture = Fixture::new();
    let (keys, models) = tokio::join!(
        fixture.api(ADMIN, Method::POST, KEYS, Some(key_payload())),
        fixture.api(ADMIN, Method::POST, MODELS, Some(model_payload()))
    );
    assert_eq!(keys.0, StatusCode::OK);
    assert_eq!(models.0, StatusCode::OK);
    let response = fixture.api(ADMIN, Method::GET, MODELS, None).await;
    assert_eq!(response.1["data"]["image_vision"]["model"], "fixture-next");
    let raw = fs::read_to_string(fixture.root.join("configs/image.toml")).unwrap();
    assert!(raw.contains(NEW_SECRET));
    fixture.assert_private(&response.1);
}

fn edit(fixture: &Fixture, name: &'static str) -> ModelSettingsEdit {
    let original = fs::read(fixture.root.join("configs").join(name)).ok();
    let mut updated = original.clone().unwrap_or_default();
    updated.extend_from_slice(b"\n# fixture update\n");
    ModelSettingsEdit {
        name,
        original,
        updated,
    }
}

#[test]
fn model_settings_out_of_band_edit_aborts_before_any_publication() {
    let fixture = Fixture::new();
    let edits = [edit(&fixture, "config.toml"), edit(&fixture, "image.toml")];
    fixture.write("image.toml", "# fixture external edit\n");
    let before = fixture.files();
    let mut publications = 0;
    let error = commit_model_settings_edits(&fixture.root, &edits, |_, _| {
        publications += 1;
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "model_settings_changed_during_save");
    assert_eq!(publications, 0);
    assert_eq!(fixture.files(), before);
}

#[test]
fn model_settings_late_write_failure_restores_all_existing_bytes() {
    let fixture = Fixture::new();
    let edits = [edit(&fixture, "config.toml"), edit(&fixture, "image.toml")];
    let before = fixture.files();
    let mut calls = 0;
    let error = commit_model_settings_edits(&fixture.root, &edits, |path, bytes| {
        calls += 1;
        if calls == 2 {
            return Err(std::io::Error::other("fixture publication failure"));
        }
        crate::secure_workspace_fs::atomic_write_workspace_file(&fixture.root, path, bytes)
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "model_settings_write_failed");
    assert_eq!(fixture.files(), before);
}

#[test]
fn model_settings_post_rename_failure_restores_bytes_and_removes_new_optional_file() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.root.join("configs/image.toml")).unwrap();
    let edits = [
        edit(&fixture, "config.toml"),
        edit(&fixture, "image.toml"),
        edit(&fixture, "audio.toml"),
    ];
    let before = fixture.files();
    let mut calls = 0;
    let error = commit_model_settings_edits(&fixture.root, &edits, |path, bytes| {
        calls += 1;
        crate::secure_workspace_fs::atomic_write_workspace_file(&fixture.root, path, bytes)?;
        if calls == 3 {
            Err(std::io::Error::other("fixture sync failure after rename"))
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "model_settings_write_failed");
    assert_eq!(fixture.files(), before);
}

#[test]
fn model_settings_failed_restore_is_not_reported_as_a_complete_rollback() {
    let fixture = Fixture::new();
    let edits = [edit(&fixture, "config.toml"), edit(&fixture, "image.toml")];
    let original = fixture.files();
    let mut calls = 0;
    let error = commit_model_settings_edits(&fixture.root, &edits, |path, bytes| {
        calls += 1;
        if calls == 3 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "fixture lost restore target",
            ));
        }
        crate::secure_workspace_fs::atomic_write_workspace_file(&fixture.root, path, bytes)?;
        if calls == 2 {
            Err(std::io::Error::other("fixture sync failure after rename"))
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "model_settings_rollback_failed");
    assert_eq!(fixture.files()["config.toml"], original["config.toml"]);
    assert_ne!(fixture.files()["image.toml"], original["image.toml"]);
}

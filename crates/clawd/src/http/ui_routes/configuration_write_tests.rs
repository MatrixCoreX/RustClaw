use super::model_settings_integrity_tests::support::{Fixture, ADMIN, SECRET};
use super::*;
use axum::http::Method;
use std::future::Future;
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};

#[path = "configuration_write_fixture_tests.rs"]
mod fixture;
use fixture::*;

#[tokio::test]
async fn configuration_write_parent_components_follow_the_selected_read_and_save_targets() {
    let (mut fixture, main) = selected_fixture();
    fixture.state.reload_ctx.config_path_for_reload = "selected/../selected/runtime.toml".into();
    let defaults = fixture.files();
    let read = fixture
        .api(ADMIN, Method::GET, "/v1/admin/model-config", None)
        .await;
    assert_eq!(read.0, StatusCode::OK);
    assert_eq!(read.1["data"]["llm"]["model"], "selected-model");
    let saved = fixture
        .api(
            ADMIN,
            Method::POST,
            "/v1/admin/model-config",
            Some(json!({
                "llm":{"vendor":"custom","model":"parent-components-saved"},
                "image_vision":{"vendor":"fixture","model":"parent-components-image"}
            })),
        )
        .await;
    assert_eq!(saved.0, StatusCode::OK);
    assert_eq!(saved.1["data"]["llm"]["model"], "parent-components-saved");
    assert_eq!(
        saved.1["data"]["image_vision"]["model"],
        "parent-components-image"
    );
    assert!(fs::read_to_string(main)
        .unwrap()
        .contains("parent-components-saved"));
    assert_eq!(fixture.files(), defaults);
}

#[tokio::test(flavor = "current_thread")]
async fn configuration_write_both_model_apis_recheck_revoked_authority_after_waiting() {
    for legacy in [false, true] {
        for disabled in [false, true] {
            let fixture = custom_fixture();
            let before = fixture.files();
            let paths = model_settings_lock_paths(&fixture.state);
            let (held_tx, held_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let mut release = ReleaseGate(Some(release_tx));
            let owner = std::thread::spawn(move || {
                crate::configuration_files::with_locked_files(&paths, || {
                    held_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                })
                .unwrap()
            });
            tokio::task::spawn_blocking(move || held_rx.recv_timeout(Duration::from_secs(10)))
                .await
                .unwrap()
                .unwrap();
            let state = fixture.state.clone();
            let request = tokio::spawn(async move {
                if legacy {
                    update_llm_config(
                        State(state),
                        headers(),
                        Json(legacy_request(Some(REPLACEMENT))),
                    )
                    .await
                    .0
                } else {
                    model_settings_request(
                        state,
                        headers(),
                        ModelSettingsOperation::SaveModels,
                        |_| Ok(json!({"unexpected":true})),
                    )
                    .await
                    .0
                }
            });
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !crate::configuration_files::has_contender(&active_runtime_config_path(
                &fixture.state,
            )) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "fixture request did not reach shared configuration lock"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            fixture.sql(if disabled {
                "UPDATE auth_keys SET enabled=0 WHERE user_key='fixture-settings-admin'"
            } else {
                "UPDATE auth_keys SET role='user' WHERE user_key='fixture-settings-admin'"
            });
            release.release();
            let status = request.await.unwrap();
            owner.join().unwrap();
            assert_eq!(
                status,
                if disabled {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::FORBIDDEN
                }
            );
            assert_eq!(fixture.files(), before);
            assert!(!claw_core::secrets::model_environment::path(&fixture.root).exists());
        }
    }
}

#[tokio::test]
async fn configuration_write_selected_absolute_files_outside_workspace_remain_supported() {
    let (outside, main) = selected_fixture();
    let mut fixture = custom_fixture();
    fixture.state.reload_ctx.config_path_for_reload = main.to_string_lossy().into_owned();
    let defaults = fixture.files();
    let response = fixture
        .api(
            ADMIN,
            Method::POST,
            "/v1/admin/model-config",
            Some(json!({
                "llm":{"vendor":"custom","model":"outside-after-save"},
                "image_vision":{"vendor":"fixture","model":"outside-image-after-save"}
            })),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["data"]["llm"]["model"], "outside-after-save");
    assert_eq!(fixture.files(), defaults);
    assert!(fs::read_to_string(main)
        .unwrap()
        .contains("outside-after-save"));
    assert!(fs::read_to_string(outside.root.join("selected/image.toml"))
        .unwrap()
        .contains("outside-image-after-save"));
}

#[test]
fn configuration_write_modern_main_persistence_saves_or_restores_both_destinations() {
    for failed in [false, true] {
        let (fixture, main) = selected_fixture();
        let mirror_fixture = Fixture::new();
        let mirror = mirror_fixture.root.join("mirror.toml");
        if failed {
            fs::create_dir(&mirror).unwrap();
        } else {
            fs::write(&mirror, b"# old mirror\n").unwrap();
        }
        let before = fs::read(&main).unwrap();
        let defaults = fixture.files();
        let mut files = load_model_settings_files(&fixture.state, true).unwrap();
        files[0].changed = true;
        files[0].value["llm"]["selected_model"] =
            toml::Value::String("mirrored-selected-model".into());
        let result =
            save_model_settings_files_with_persistence(&fixture.state, files, Some(mirror.clone()));
        if failed {
            assert!(result.is_err());
            assert_eq!(fs::read(&main).unwrap(), before);
            assert!(mirror.is_dir());
        } else {
            result.unwrap();
            assert_eq!(fs::read(&main).unwrap(), fs::read(mirror).unwrap());
            assert_ne!(fs::read(&main).unwrap(), before);
        }
        assert_eq!(fixture.files(), defaults);
    }
}

#[tokio::test]
async fn configuration_write_invalid_legacy_toml_never_publishes_or_rotates_credentials() {
    let fixture = custom_fixture();
    let before = fixture.files();
    let mut request = legacy_request(Some(REPLACEMENT));
    request.selected_model = "fixture\0not-toml".into();
    let (status, body) =
        update_llm_config(State(fixture.state.clone()), headers(), Json(request)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body.0.error.as_deref(), Some("llm_config_invalid"));
    assert_eq!(fixture.files(), before);
    assert!(!claw_core::secrets::model_environment::path(&fixture.root).exists());
}

#[tokio::test]
async fn configuration_write_legacy_failures_do_not_expose_configuration_contents_or_paths() {
    let fixture = custom_fixture();
    fixture.write("config.toml", &format!("api_key = {SECRET}\n"));
    let (status, Json(body)) = update_llm_config(
        State(fixture.state.clone()),
        headers(),
        Json(legacy_request(None)),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body.error.as_deref(), Some("llm_config_read_failed"));
    fixture.assert_private(&serde_json::to_value(body).unwrap());
}

#[test]
fn configuration_write_persistence_failure_does_not_modify_the_active_config() {
    let fixture = Fixture::new();
    let active = fixture.root.join("configs/config.toml");
    let mirror = fixture.root.join("mounted/config.toml");
    fs::create_dir_all(&mirror).unwrap();
    let before = fs::read(&active).unwrap();
    assert!(
        write_runtime_config_to_paths(&active, Some(&mirror), "# failed publication\n").is_err()
    );
    assert_eq!(
        fs::read(&active).unwrap(),
        before,
        "failed persistence publication must retain active bytes"
    );
    assert!(mirror.is_dir());
}

#[test]
fn configuration_write_channel_mirror_failure_does_not_modify_the_active_config() {
    let fixture = Fixture::new();
    let active = fixture.root.join("configs/channel-probe.toml");
    fs::write(&active, b"# owned channel baseline\n").unwrap();
    let mirror = fixture.root.join("docker/config/channel-probe.toml");
    fs::create_dir_all(&mirror).unwrap();
    assert!(write_workspace_and_mounted_file(
        &fixture.root,
        "configs/channel-probe.toml",
        "# failed mirror\n"
    )
    .is_err());
    assert_eq!(
        fs::read(&active).unwrap(),
        b"# owned channel baseline\n",
        "failed channel mirror publication must retain active bytes"
    );
    assert!(mirror.is_dir());
}

#[tokio::test]
async fn configuration_write_model_reads_follow_the_selected_main_and_split_files() {
    let (fixture, _) = selected_fixture();
    let defaults = fixture.files();
    let response = fixture
        .api(ADMIN, Method::GET, "/v1/admin/model-config", None)
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(
        response.1["data"]["llm"]["model"], "selected-model",
        "model reads must use the active main configuration"
    );
    assert_eq!(
        response.1["data"]["image_vision"]["model"],
        "selected-image"
    );
    assert_eq!(fixture.files(), defaults);
}

#[tokio::test]
async fn configuration_write_model_saves_do_not_rewrite_the_workspace_default() {
    let (fixture, main) = selected_fixture();
    let defaults = fixture.files();
    let response = fixture
        .api(
            ADMIN,
            Method::POST,
            "/v1/admin/model-config",
            Some(json!({
                "llm": {"vendor":"custom", "model":"selected-after-save"},
                "image_vision": {"vendor":"fixture", "model":"selected-image-after-save"}
            })),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(
        fixture.files(),
        defaults,
        "saving a selected config must not rewrite default templates"
    );
    let main: toml::Value = toml::from_str(&fs::read_to_string(main).unwrap()).unwrap();
    assert_eq!(
        main["llm"]["selected_model"].as_str(),
        Some("selected-after-save")
    );
    let image: toml::Value =
        toml::from_str(&fs::read_to_string(fixture.root.join("selected/image.toml")).unwrap())
            .unwrap();
    assert_eq!(
        image["image_vision"]["default_model"].as_str(),
        Some("selected-image-after-save")
    );
}

#[tokio::test]
async fn configuration_write_provider_keys_follow_the_selected_files() {
    let (fixture, main) = selected_fixture();
    claw_core::secrets::model_environment::save(&fixture.root, "custom", SELECTED_SECRET).unwrap();
    let mut selected: toml::Value = toml::from_str(&fs::read_to_string(&main).unwrap()).unwrap();
    selected["llm"].as_table_mut().unwrap().insert(
        "mimo".into(),
        toml::Value::Table(toml::toml! { api_key = "" }),
    );
    fs::write(&main, toml::to_string(&selected).unwrap()).unwrap();
    claw_core::secrets::model_environment::save(&fixture.root, "mimo", SELECTED_SECRET).unwrap();
    let defaults = fixture.files();
    let response = fixture
        .api(ADMIN, Method::GET, "/v1/admin/provider-keys", None)
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(
        response.1["data"]["llm"]["custom"],
        json!(mask_secret(SELECTED_SECRET)),
        "key reads must report the selected provider's effective broker credential"
    );
    assert_eq!(
        response.1["data"]["llm"]["mimo"],
        json!(mask_secret(SELECTED_SECRET))
    );
    let default_config: toml::Value =
        toml::from_str(&fs::read_to_string(fixture.root.join("configs/config.toml")).unwrap())
            .unwrap();
    assert!(
        !read_llm_provider_keys(&fixture.state, &default_config)
            .unwrap()
            .contains_key("mimo"),
        "the provider inventory must still come from the selected configuration"
    );
    let response = fixture
        .api(
            ADMIN,
            Method::POST,
            "/v1/admin/provider-keys",
            Some(json!({
                "llm":{"custom":REPLACEMENT}, "image":{"image_vision":{"fixture":REPLACEMENT}}
            })),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(fixture.files(), defaults);
    let main: toml::Value = toml::from_str(&fs::read_to_string(main).unwrap()).unwrap();
    assert_eq!(main["llm"]["custom"]["api_key"].as_str(), Some(""));
    assert_eq!(
        effective_model_provider_key(&fixture.state, "custom").unwrap(),
        REPLACEMENT
    );
}

#[test]
fn configuration_write_rollback_does_not_overwrite_an_independent_commit() {
    let fixture = Fixture::new();
    let edits = [edit(&fixture, "config.toml"), edit(&fixture, "image.toml")];
    let independent = b"# independent configuration committed after preparation\n";
    let main = fixture.root.join("configs/config.toml");
    let mut calls = 0;
    let result = commit_model_settings_edits(&fixture.root, &edits, |path, bytes| {
        calls += 1;
        if calls == 2 {
            crate::secure_workspace_fs::atomic_write_workspace_file(
                &fixture.root,
                &main,
                independent,
            )?;
            return Err(std::io::Error::other("owned late publication failure"));
        }
        crate::secure_workspace_fs::atomic_write_workspace_file(&fixture.root, path, bytes)
    });
    assert!(result.is_err());
    assert_eq!(
        fs::read(&main).unwrap(),
        independent,
        "recovery must preserve a later independent commit"
    );
}

#[tokio::test]
async fn configuration_write_legacy_blank_key_does_not_erase_existing_credentials() {
    let fixture = custom_fixture();
    let (status, Json(body)) = update_llm_config(
        State(fixture.state.clone()),
        headers(),
        Json(legacy_request(None)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.ok);
    let parsed: toml::Value =
        toml::from_str(&fs::read_to_string(fixture.root.join("configs/config.toml")).unwrap())
            .unwrap();
    let literal = parsed["llm"]["custom"]["api_key"].as_str().unwrap_or("");
    let environment = claw_core::secrets::model_environment::lookup_at(
        &claw_core::secrets::model_environment::path(&fixture.root),
        "custom",
    )
    .unwrap();
    assert!(
        literal == SECRET
            || environment
                .as_ref()
                .is_some_and(|secret| secret.expose() == SECRET),
        "a blank credential update must preserve the previous effective secret"
    );
}

#[test]
fn configuration_write_failed_legacy_publication_does_not_rotate_the_secret() {
    let fixture = custom_fixture();
    let original = fixture.files();
    let path = claw_core::secrets::model_environment::path(&fixture.root);
    let before = fs::read(&path).ok();
    let (status, Json(body)) = update_llm_config_with_writer(
        fixture.state.clone(),
        headers(),
        legacy_request(Some(REPLACEMENT)),
        |_, _| Err(std::io::Error::other("owned TOML publication failure")),
    );
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body.ok);
    assert_eq!(fixture.files(), original);
    assert_eq!(
        fs::read(path).ok(),
        before,
        "failed configuration publication must not rotate the saved credential"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn configuration_write_legacy_and_modern_requests_share_the_write_boundary() {
    let fixture = custom_fixture();
    let state = fixture.state.clone();
    let (ready, waiting) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let mut release = ReleaseGate(Some(release));
    let modern = tokio::spawn(model_settings_request(
        state,
        headers(),
        ModelSettingsOperation::SaveModels,
        move |state| {
            let mut files = load_model_settings_files(state, true)?;
            files[0]
                .value
                .get_mut("sentinel")
                .unwrap()
                .as_table_mut()
                .unwrap()
                .insert(
                    "modern".to_string(),
                    toml::Value::String("retained".to_string()),
                );
            files[0].changed = true;
            ready.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(30))?;
            save_model_settings_files(state, files)?;
            Ok(json!({"saved":true}))
        },
    ));
    tokio::task::spawn_blocking(move || waiting.recv_timeout(Duration::from_secs(30)))
        .await
        .unwrap()
        .unwrap();
    let mut legacy = std::pin::pin!(update_llm_config(
        State(fixture.state.clone()),
        headers(),
        Json(legacy_request(None))
    ));
    let waker = Waker::from(Arc::new(NoopWake));
    let first = legacy.as_mut().poll(&mut Context::from_waker(&waker));
    let pending = first.is_pending();
    if pending {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !crate::configuration_files::has_contender(&active_runtime_config_path(
            &fixture.state,
        )) {
            assert!(std::time::Instant::now() < deadline, "legacy request must actually contend for the modern request's lock, not only yield to a worker");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    release.release();
    let modern = modern.await.unwrap();
    let legacy = match first {
        Poll::Ready(result) => result,
        Poll::Pending => legacy.await,
    };
    assert!(
        pending,
        "overlapping legacy saves must wait for the active configuration transaction"
    );
    assert_eq!(modern.0, StatusCode::OK);
    assert_eq!(legacy.0, StatusCode::OK);
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(fixture.root.join("configs/config.toml")).unwrap())
            .unwrap();
    assert_eq!(config["sentinel"]["modern"].as_str(), Some("retained"));
    assert_eq!(
        config["llm"]["selected_model"].as_str(),
        Some("legacy-selected")
    );
}

#[cfg(unix)]
#[test]
fn configuration_write_success_preserves_both_destinations_existing_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let active = fixture.root.join("configs/config.toml");
    let mirror = fixture.root.join("persisted.toml");
    fs::write(&mirror, b"# original\n").unwrap();
    fs::set_permissions(&active, fs::Permissions::from_mode(0o640)).unwrap();
    fs::set_permissions(&mirror, fs::Permissions::from_mode(0o600)).unwrap();
    write_runtime_config_to_paths(&active, Some(&mirror), "# success\n").unwrap();
    assert_eq!(fs::read(&active).unwrap(), b"# success\n");
    assert_eq!(fs::read(&mirror).unwrap(), b"# success\n");
    assert_eq!(
        fs::metadata(active).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        fs::metadata(mirror).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn configuration_write_explicit_persistence_path_may_be_outside_the_workspace() {
    let fixture = Fixture::new();
    let other = Fixture::new();
    let active = fixture.root.join("configs/config.toml");
    let mirror = other.root.join("explicit-persisted.toml");
    write_runtime_config_to_paths(&active, Some(&mirror), "# explicit destination\n").unwrap();
    assert_eq!(fs::read(active).unwrap(), b"# explicit destination\n");
    assert_eq!(fs::read(mirror).unwrap(), b"# explicit destination\n");
}

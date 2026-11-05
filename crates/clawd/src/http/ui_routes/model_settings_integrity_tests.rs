use super::*;
use axum::http::Method;
#[path = "model_settings_integrity_fixture_tests.rs"]
pub(super) mod support;
use support::*;
#[path = "model_settings_mutation_tests.rs"]
mod mutations;
#[path = "model_settings_offload_tests.rs"]
mod offload;

#[tokio::test]
async fn model_settings_malformed_files_are_not_empty_successes_or_partial_saves() {
    for name in FILES {
        let fixture = Fixture::new();
        fixture.write(name, &format!("api_key = {SECRET}\n"));
        let before = fixture.files();
        for (path, body) in [(MODELS, model_payload()), (KEYS, key_payload())] {
            for key in [USER, ADMIN] {
                let response = fixture.api(key, Method::GET, path, None).await;
                assert_error(
                    &response,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "model_settings_parse_failed",
                );
                fixture.assert_private(&response.1);
            }
            let response = fixture.api(ADMIN, Method::POST, path, Some(body)).await;
            assert_error(
                &response,
                StatusCode::INTERNAL_SERVER_ERROR,
                "model_settings_parse_failed",
            );
            fixture.assert_private(&response.1);
            assert_eq!(fixture.files(), before);
        }
    }
}

#[tokio::test]
async fn model_settings_unreadable_and_non_utf8_files_fail_without_exposure_or_writes() {
    for directory in [true, false] {
        let fixture = Fixture::new();
        let path = fixture.root.join("configs/music.toml");
        fs::remove_file(&path).unwrap();
        if directory {
            fs::create_dir(&path).unwrap();
        } else {
            fs::write(&path, [0xff, 0xfe]).unwrap();
        }
        let before = fixture.files();
        for (endpoint, body) in [(MODELS, model_payload()), (KEYS, key_payload())] {
            for method in [Method::GET, Method::POST] {
                let body = (method == Method::POST).then(|| body.clone());
                let response = fixture.api(ADMIN, method, endpoint, body).await;
                assert_error(
                    &response,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    if directory {
                        "model_settings_read_failed"
                    } else {
                        "model_settings_parse_failed"
                    },
                );
                fixture.assert_private(&response.1);
            }
        }
        assert_eq!(fixture.files(), before);
    }
}

#[tokio::test]
async fn model_settings_missing_optional_files_keep_first_use_defaults_and_recover_freshly() {
    let fixture = Fixture::new();
    for name in FILES {
        fs::remove_file(fixture.root.join("configs").join(name)).unwrap();
    }
    for key in [USER, ADMIN] {
        for path in [MODELS, KEYS] {
            let response = fixture.api(key, Method::GET, path, None).await;
            assert_eq!(response.0, StatusCode::OK);
            assert_eq!(response.1["ok"], true);
        }
    }
    let empty = fixture.files();
    for (path, payload) in [(MODELS, model_payload()), (KEYS, key_payload())] {
        let response = fixture.api(ADMIN, Method::POST, path, Some(payload)).await;
        assert_error(
            &response,
            StatusCode::INTERNAL_SERVER_ERROR,
            "model_settings_read_failed",
        );
        assert_eq!(fixture.files(), empty);
    }
    fixture.write(
        "config.toml",
        "[llm]\nselected_vendor='fixture'\nselected_model='fresh'\n",
    );
    let response = fixture.api(USER, Method::GET, MODELS, None).await;
    assert_eq!(response.1["data"]["llm"]["model"], "fresh");
    fixture.write("audio.toml", "invalid");
    assert_eq!(
        fixture.api(USER, Method::GET, MODELS, None).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    fixture.write(
        "audio.toml",
        "[audio_transcribe]\ndefault_vendor='local'\ndefault_model='fixture-local'\n",
    );
    let response = fixture.api(USER, Method::GET, MODELS, None).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(
        response.1["data"]["audio_transcribe"]["model"],
        "fixture-local"
    );
}

#[tokio::test]
async fn model_settings_auth_and_write_role_checks_precede_all_configuration_io() {
    let fixture = Fixture::new();
    fixture.write("config.toml", &format!("api_key = {SECRET}\n"));
    let before = fixture.files();
    for (path, body) in [(MODELS, model_payload()), (KEYS, key_payload())] {
        for (key, code) in [
            ("", "auth_key_required"),
            ("fixture-settings-unknown", "auth_key_invalid"),
        ] {
            for method in [Method::GET, Method::POST] {
                let body = (method == Method::POST).then(|| body.clone());
                let response = fixture.api(key, method, path, body).await;
                assert_error(&response, StatusCode::UNAUTHORIZED, code);
            }
        }
        assert_error(
            &fixture.api(USER, Method::POST, path, Some(body)).await,
            StatusCode::FORBIDDEN,
            "admin_required",
        );
    }
    assert_eq!(fixture.files(), before);
}

#[tokio::test]
async fn model_settings_recheck_revocation_roles_and_database_failure_without_cached_grants() {
    let fixture = Fixture::new();
    for path in [MODELS, KEYS] {
        assert_eq!(
            fixture.api(ADMIN, Method::GET, path, None).await.0,
            StatusCode::OK
        );
    }
    fixture.sql("UPDATE auth_keys SET role='user' WHERE user_key='fixture-settings-admin'");
    assert_error(
        &fixture
            .api(ADMIN, Method::POST, MODELS, Some(model_payload()))
            .await,
        StatusCode::FORBIDDEN,
        "admin_required",
    );
    fixture.sql("UPDATE auth_keys SET enabled=0 WHERE user_key='fixture-settings-admin'");
    for path in [MODELS, KEYS] {
        assert_error(
            &fixture.api(ADMIN, Method::GET, path, None).await,
            StatusCode::UNAUTHORIZED,
            "auth_key_invalid",
        );
    }
    fixture.sql("DROP TABLE auth_keys");
    for (path, body) in [(MODELS, model_payload()), (KEYS, key_payload())] {
        for method in [Method::GET, Method::POST] {
            let body = (method == Method::POST).then(|| body.clone());
            let response = fixture.api(USER, method, path, body).await;
            assert_error(
                &response,
                StatusCode::INTERNAL_SERVER_ERROR,
                "auth_lookup_failed",
            );
            fixture.assert_private(&response.1);
            assert!(!response.1.to_string().contains("auth_keys"));
        }
    }
}

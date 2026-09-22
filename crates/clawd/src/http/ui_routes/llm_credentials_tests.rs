use super::*;
use axum::http::HeaderValue;
use claw_core::secrets::{EnvFileSecretsBroker, SecretsBroker};

const ADMIN_KEY: &str = "llm-credential-test-admin";

fn fixture(vendor: &str) -> (PathBuf, AppState, HeaderMap) {
    let root = std::env::temp_dir().join(format!("llm-credentials-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("configs")).unwrap();
    let config = toml::toml! {
        [llm]
        selected_vendor = ""
        selected_model = ""
    };
    let mut config = toml::Value::Table(config);
    config["llm"].as_table_mut().unwrap().insert(
        vendor.to_string(),
        toml::Value::Table(toml::toml! {
            api_key = ""
            base_url = "https://provider.example.test/v1"
            model = "fixture-model"
            models = ["fixture-model"]
        }),
    );
    std::fs::write(
        root.join("configs/config.toml"),
        toml::to_string(&config).unwrap(),
    )
    .unwrap();
    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.seed_test_auth_identity(ADMIN_KEY, "admin");
    let mut headers = HeaderMap::new();
    headers.insert("x-agent-key", HeaderValue::from_static(ADMIN_KEY));
    (root, state, headers)
}

fn request(vendor: &str, key: Option<&str>) -> UpdateLlmConfigRequest {
    UpdateLlmConfigRequest {
        selected_vendor: vendor.to_string(),
        selected_model: "fixture-model".to_string(),
        vendor_base_url: Some("https://provider.example.test/v1".to_string()),
        vendor_api_format: Some("openai_compat".to_string()),
        vendor_api_key: key.map(str::to_string),
    }
}

#[tokio::test]
async fn llm_credentials_all_vendors_save_privately_and_preserve_blank_keys() {
    for vendor in llm_vendor_names() {
        let (root, state, headers) = fixture(vendor);
        let path = claw_core::secrets::model_environment::path(&root);
        let name = claw_core::secrets::text_secret_name_for_vendor(vendor);
        for key in [
            Some("fixture-first-key"),
            None,
            Some("  "),
            Some("fixture-rotated-key"),
        ] {
            let (status, Json(body)) = update_llm_config(
                State(state.clone()),
                headers.clone(),
                Json(request(vendor, key)),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "vendor={vendor}");
            assert!(body.ok);
            let expected = if key == Some("fixture-rotated-key") {
                "fixture-rotated-key"
            } else {
                "fixture-first-key"
            };
            let raw = std::fs::read_to_string(root.join("configs/config.toml")).unwrap();
            let parsed: toml::Value = toml::from_str(&raw).unwrap();
            assert_eq!(parsed["llm"][vendor]["api_key"].as_str(), Some(""));
            assert!(!raw.contains("fixture-first-key") && !raw.contains("fixture-rotated-key"));
            assert!(!serde_json::to_string(&body).unwrap().contains(expected));
            assert_eq!(
                claw_core::secrets::model_environment::lookup_at(&path, vendor)
                    .unwrap()
                    .unwrap()
                    .expose(),
                expected
            );
            let broker = EnvFileSecretsBroker::new(
                claw_core::git_remote_config::git_credential_store_path(&root),
            );
            assert_eq!(broker.lookup(&name).unwrap().unwrap().expose(), expected);
            let (status, Json(loaded)) =
                get_llm_config(State(state.clone()), headers.clone()).await;
            assert_eq!(status, StatusCode::OK);
            let loaded = loaded.data.unwrap();
            let info = loaded["vendors"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["name"] == vendor)
                .unwrap();
            assert_eq!(info["api_key_configured"], true);
            assert_eq!(info["api_key_source"], "environment_file");
            assert!(!loaded.to_string().contains(expected));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn llm_credentials_non_admin_cannot_save_or_test_keys() {
    for vendor in ["minimax", "custom"] {
        let (root, state, mut headers) = fixture(vendor);
        state.seed_test_auth_identity("llm-credential-test-user", "user");
        headers.insert(
            "x-agent-key",
            HeaderValue::from_static("llm-credential-test-user"),
        );
        let before = std::fs::read(root.join("configs/config.toml")).unwrap();
        let responses = [
            update_llm_config(
                State(state.clone()),
                headers.clone(),
                Json(request(vendor, Some("fixture-key"))),
            )
            .await,
            test_llm_config(
                State(state.clone()),
                headers,
                Json(request(vendor, Some("fixture-key"))),
            )
            .await,
        ];
        for (status, Json(body)) in responses {
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(
                body.error.as_deref(),
                Some("vendor_api_key_private_store_not_allowed")
            );
        }
        assert_eq!(
            std::fs::read(root.join("configs/config.toml")).unwrap(),
            before
        );
        assert!(!claw_core::git_remote_config::git_credential_store_path(&root).exists());
        assert!(!claw_core::secrets::model_environment::path(&root).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn llm_credentials_connection_test_does_not_replace_draft_with_saved_key() {
    let (root, _, _) = fixture("minimax");
    let path = claw_core::git_remote_config::git_credential_store_path(&root);
    claw_core::secrets::set_file_secret(&path, "text_minimax_api_key", "saved-key").unwrap();
    let runtime = build_llm_test_runtime(
        "minimax",
        "fixture-model",
        "https://provider.example.test/v1",
        "draft-key",
        None,
        false,
    )
    .unwrap();
    assert_eq!(
        runtime.api_key_using(&EnvFileSecretsBroker::new(path)),
        "draft-key"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn llm_credentials_draft_test_reaches_provider_without_persisting_key() {
    let (root, state, headers) = fixture("minimax");
    let before = std::fs::read(root.join("configs/config.toml")).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(|headers: HeaderMap| async move {
        assert_eq!(headers.get("authorization").unwrap(), "Bearer fixture-draft-key");
        Json(json!({"choices": [{"message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]}))
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut req = request("minimax", Some("fixture-draft-key"));
    req.vendor_base_url = Some(format!("http://{address}/v1"));
    let (status, Json(body)) = test_llm_config(State(state), headers, Json(req)).await;
    server.abort();
    assert_eq!(status, StatusCode::OK, "response={body:?}");
    assert_eq!(body.data.unwrap()["response_text"], "ok");
    assert_eq!(
        std::fs::read(root.join("configs/config.toml")).unwrap(),
        before
    );
    assert!(!claw_core::git_remote_config::git_credential_store_path(&root).exists());
    assert!(!claw_core::secrets::model_environment::path(&root).exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn llm_credentials_hosted_relay_still_rejects_manual_keys() {
    let (root, state, headers) = fixture("custom");
    let path = root.join("configs/config.toml");
    let mut config: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["llm"].as_table_mut().unwrap().insert(
        "hosted_relay".into(),
        toml::Value::Table(toml::toml! {
            enabled = true
            vendor = "custom"
            model = "fixture-model"
            base_url = "https://provider.example.test/v1"
            daily_request_limit = 1000
        }),
    );
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    for (status, Json(body)) in [
        update_llm_config(
            State(state.clone()),
            headers.clone(),
            Json(request("custom", Some("fixture-key"))),
        )
        .await,
        test_llm_config(
            State(state),
            headers,
            Json(request("custom", Some("fixture-key"))),
        )
        .await,
    ] {
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body.error.as_deref(),
            Some("hosted_relay_manual_key_not_allowed")
        );
    }
    assert!(!claw_core::git_remote_config::git_credential_store_path(&root).exists());
    assert!(!claw_core::secrets::model_environment::path(&root).exists());
    std::fs::remove_dir_all(root).unwrap();
}

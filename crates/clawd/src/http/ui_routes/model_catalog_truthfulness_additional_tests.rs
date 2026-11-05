use super::*;

#[tokio::test]
async fn model_catalog_truthfulness_task_model_selection_uses_the_same_selected_path() {
    let mut fixture = Fixture::new();
    let main = selected_main("fixture-selected-runtime")
        .replace("https://fixture.invalid/v1", "http://localhost:12345/v1");
    fixture.write("selected/runtime.toml", &format!(
        "[server]\nrequest_timeout_seconds=30\n[database]\nsqlite_path='data/fixture.db'\nbusy_timeout_ms=2000\n[worker]\n{main}"));
    fixture.state.reload_ctx.config_path_for_reload = "selected/runtime.toml".into();
    let mut payload = json!({"text":"fixture", "model_selection": {
        "provider":"custom", "model":"fixture-selected-runtime"
    }});
    crate::task_model_selection::validate_and_stamp_task_model_selection(
        &fixture.state,
        &mut payload,
    )
    .unwrap();
    assert_eq!(payload["_agent_model_selection"]["provider"], "custom");
    assert_eq!(
        payload["_agent_model_selection"]["model"],
        "fixture-selected-runtime"
    );
    assert_eq!(
        payload["_agent_model_selection"]["authority"],
        "server_validated_model_catalog"
    );
}

#[tokio::test]
async fn model_catalog_truthfulness_selected_model_does_not_require_a_discovery_cache() {
    let fixture = Fixture::new();
    let mut config: toml::Value = selected_main("fixture-default-model").parse().unwrap();
    config["llm"]["selected_model"] = toml::Value::String("fixture-uncached-model".into());
    fixture.write("configs/config.toml", &toml::to_string(&config).unwrap());
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    let entries = response.1["data"]["entries"].as_array().unwrap();
    let selected = entries
        .iter()
        .find(|entry| entry["active_text_provider"] == true)
        .unwrap();
    assert_eq!(selected["model"], "fixture-uncached-model");
    assert_eq!(selected["supports_text"], true);
    fixture.assert_private(&response.1);
}

#[tokio::test]
async fn model_catalog_truthfulness_media_cannot_replace_the_text_transport() {
    let fixture = Fixture::new();
    fixture.write(
        "configs/config.toml",
        &selected_main("fixture-shared-model"),
    );
    fixture.write(
        "configs/image.toml",
        r#"
[image_vision]
default_vendor = "custom"
default_model = "fixture-shared-model"
[image_vision.providers.custom]
base_url = "http://localhost:8123/v1"
api_key = "fixture-independent-image-secret"
model = "fixture-shared-model"
"#,
    );
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    let entries = response.1["data"]["entries"].as_array().unwrap();
    let selected = entries
        .iter()
        .find(|entry| entry["active_text_provider"] == true)
        .unwrap();
    assert_eq!(selected["base_url_kind"], "custom_or_unknown");
    assert_ne!(selected["credential_state"], "not_required_local");
    assert_ne!(selected["credential_state"], "configured_inline");
    assert_eq!(selected["supports_image_understanding"], true);
    assert!(!response
        .1
        .to_string()
        .contains("fixture-independent-image-secret"));
    fixture.assert_private(&response.1);
}

#[tokio::test]
async fn model_catalog_truthfulness_teaching_rejects_unknown_credential_states() {
    let fixture = Fixture::new();
    let mut catalog =
        claw_core::model_catalog::build_model_catalog_from_workspace(&fixture.root).unwrap();
    catalog.selected_provider = catalog.entries[0].provider.clone();
    catalog.selected_model = catalog.entries[0].model.clone();
    for (state, ready) in [
        ("configured_inline", true),
        ("configured_env", true),
        ("device_enrollment", true),
        ("not_required_local", true),
        ("missing", false),
        ("unavailable", false),
        ("future_unrecognized", false),
        ("null", false),
        ("", false),
    ] {
        catalog.entries[0].credential_state = state.into();
        catalog.entries[0].supports_text = true;
        let trace = build_model_readiness_trace_for_debug(&catalog);
        assert_eq!(trace["ready"], ready, "{state}");
    }
}

#[tokio::test]
async fn model_catalog_truthfulness_reads_wait_for_complete_shared_publication() {
    let fixture = Fixture::new();
    let main = fixture.root.join("configs/config.toml");
    let image = fixture.root.join("configs/image.toml");
    let main_for_lock = main.clone();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        crate::configuration_files::with_locked_files(&[main_for_lock], || {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(20)).unwrap();
        })
        .unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(20)).unwrap();
    let state = fixture.state.clone();
    let response =
        tokio::spawn(async move { get_model_catalog(State(state), headers(ADMIN)).await });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !crate::configuration_files::has_contender(&main) {
        assert!(
            !response.is_finished(),
            "read bypassed the shared configuration lock"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "read did not reach the shared lock"
        );
        tokio::task::yield_now().await;
    }
    fs::write(&main, selected_main("fixture-published-text")).unwrap();
    fs::write(
        &image,
        "[image_vision]\ndefault_vendor='custom'\ndefault_model='fixture-published-image'\n",
    )
    .unwrap();
    release_tx.send(()).unwrap();
    writer.join().unwrap();
    let (status, Json(body)) = response.await.unwrap();
    assert_eq!(status, StatusCode::OK);
    let body = serde_json::to_value(body).unwrap();
    assert_eq!(body["data"]["selected_model"], "fixture-published-text");
    assert!(body["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["model"] == "fixture-published-image"));
    fixture.assert_private(&body);
}

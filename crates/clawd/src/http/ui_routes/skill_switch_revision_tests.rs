use super::*;

impl Fixture {
    async fn config_request(&self, method: Method, payload: Value) -> (StatusCode, Value) {
        let response = axum::Router::new()
            .nest("/v1", build_ui_router())
            .with_state(self.state.clone())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/v1/skills/config")
                    .header("content-type", "application/json")
                    .header(claw_core::product_identity::AUTH_KEY_HEADER, STORE_TEST_KEY)
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn observed_revision(&self) -> Value {
        let response = self.config_request(Method::GET, Value::Null).await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
        response.1["data"]["revision"].clone()
    }
}

#[tokio::test]
async fn switch_revision_read_is_an_opaque_stable_token() {
    let fixture = Fixture::new().await;
    let revision = fixture.observed_revision().await;
    let token = revision
        .as_str()
        .expect("skill switch reads need a revision");
    assert_eq!(token.len(), 64);
    assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(fixture.observed_revision().await, revision);
}

#[tokio::test]
async fn switch_revision_missing_observation_rejects_without_mutation() {
    let fixture = Fixture::new().await;
    let before = fixture.service().snapshot().unwrap();
    let config = fixture.config();
    let response = fixture
        .config_request(
            Method::POST,
            serde_json::json!({"skill_switches":{ALPHA:false}}),
        )
        .await;
    assert_eq!(response.0, StatusCode::BAD_REQUEST, "{}", response.1);
    assert_eq!(response.1["error"], "skill_switch_revision_required");
    assert_eq!(
        fixture.service().snapshot().unwrap().generation,
        before.generation
    );
    assert_eq!(
        fixture.service().snapshot().unwrap().state(ALPHA),
        Some(AdmissionState::Enabled)
    );
    assert_eq!(fixture.config(), config);
}

#[tokio::test]
async fn switch_revision_stale_page_cannot_overwrite_a_newer_state() {
    let fixture = Fixture::new().await;
    let revision = fixture.observed_revision().await;
    let first = fixture
        .save(STORE_TEST_KEY, serde_json::json!({BETA:false}))
        .await;
    assert_eq!(first.0, StatusCode::OK, "{}", first.1);
    let before = fixture.service().snapshot().unwrap();
    let config = fixture.config();
    let response = fixture
        .config_request(
            Method::POST,
            serde_json::json!({
                "skill_switches": {ALPHA:false, BETA:true}, "expected_revision": revision,
            }),
        )
        .await;
    assert_eq!(response.0, StatusCode::CONFLICT, "{}", response.1);
    assert_eq!(response.1["error"], "skill_switch_revision_changed");
    let after = fixture.service().snapshot().unwrap();
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.state(ALPHA), Some(AdmissionState::Enabled));
    assert_eq!(after.state(BETA), Some(AdmissionState::InstalledDisabled));
    assert_eq!(fixture.config(), config);
}

#[tokio::test]
async fn switch_revision_unadmitted_names_do_not_become_startup_facts() {
    let fixture = Fixture::new().await;
    let revision = fixture.observed_revision().await;
    let config = fixture.config();
    let before = fixture.service().snapshot().unwrap();
    let response = fixture
        .config_request(
            Method::POST,
            serde_json::json!({
                "skill_switches": {"switch_unadmitted_fixture":true}, "expected_revision":revision,
            }),
        )
        .await;
    assert_eq!(response.0, StatusCode::CONFLICT, "{}", response.1);
    assert_eq!(response.1["error"], "skill_admission_missing");
    assert_eq!(fixture.config(), config);
    assert_eq!(
        fixture.service().snapshot().unwrap().generation,
        before.generation
    );
}

#[tokio::test]
async fn switch_revision_wrong_token_fails_closed() {
    let fixture = Fixture::new().await;
    let config = fixture.config();
    let before = fixture.service().snapshot().unwrap();
    let response = fixture
        .config_request(
            Method::POST,
            serde_json::json!({
                "skill_switches": {ALPHA:false}, "expected_revision":"0".repeat(64),
            }),
        )
        .await;
    assert_eq!(response.0, StatusCode::CONFLICT, "{}", response.1);
    assert_eq!(response.1["error"], "skill_switch_revision_changed");
    assert_eq!(fixture.config(), config);
    assert_eq!(
        fixture.service().snapshot().unwrap().generation,
        before.generation
    );
}

#[tokio::test]
async fn switch_revision_does_not_bind_unrelated_configuration_formatting() {
    let fixture = Fixture::new().await;
    let revision = fixture.observed_revision().await;
    let original = fixture.config();
    let mut reformatted = original.clone();
    reformatted.extend_from_slice(b"\n# unrelated configuration formatting\n");
    std::fs::write(fixture.root.join("configs/config.toml"), &reformatted).unwrap();
    let response = fixture
        .config_request(
            Method::POST,
            serde_json::json!({
                "skill_switches": {ALPHA:false}, "expected_revision":revision,
            }),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.1);
    assert_eq!(fixture.config(), reformatted);
    assert_eq!(
        fixture.service().snapshot().unwrap().state(ALPHA),
        Some(AdmissionState::InstalledDisabled)
    );
}

#[tokio::test]
async fn switch_revision_does_not_hash_unrelated_private_credentials() {
    let fixture = Fixture::new().await;
    let revision = fixture.observed_revision().await;
    let mut config: toml::Value =
        toml::from_str(std::str::from_utf8(&fixture.config()).unwrap()).unwrap();
    config["llm"]["openai"]["api_key"] = toml::Value::String("fixture_key_changed".to_string());
    let updated = toml::to_string(&config).unwrap();
    std::fs::write(fixture.root.join("configs/config.toml"), &updated).unwrap();
    assert_eq!(fixture.observed_revision().await, revision);
    let response = fixture
        .config_request(
            Method::POST,
            serde_json::json!({
                "skill_switches": {ALPHA:false}, "expected_revision":revision,
            }),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.1);
    assert_eq!(fixture.config(), updated.as_bytes());
}

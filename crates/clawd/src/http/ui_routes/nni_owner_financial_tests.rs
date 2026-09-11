use super::*;

#[test]
fn selected_node_requires_unambiguous_origin_and_never_accepts_remote_http() {
    assert_eq!(
        canonical_origin("https://example.test/").unwrap(),
        ("https://example.test".into(), false)
    );
    assert_eq!(
        canonical_origin("http://127.0.0.1:9999").unwrap(),
        ("http://127.0.0.1:9999".into(), true)
    );
    for raw in [
        "http://192.168.1.2",
        "https://[::1]",
        "https://[::ffff:127.0.0.1]",
        "http://localhost:9999",
        "https://user:password@example.test",
        "https://example.test/path",
        "https://example.test?node=elsewhere",
    ] {
        assert!(canonical_origin(raw).is_err(), "{raw}");
    }
}

#[test]
fn gateway_rejects_ambiguous_top_level_routing_fields() {
    let raw = r#"{"schema_version":1,"protocol":"asset_owner_v1","ledger_id":"ledger","node_url":"https://node.example.test","service":"assets","service":"bancor","account":"owner","operation_id":"6425d24d-26fb-4839-9e1c-779dc68c81c8","intent":{"kind":"balances"}}"#;
    assert!(serde_json::from_str::<RequestEnvelope>(raw).is_err());
    assert!(serde_json::from_str::<RequestEnvelope>(
        &raw.replace("\"service\":\"assets\",", "\"admin\":true,")
    )
    .is_err());
}

#[tokio::test]
async fn upstream_rejection_is_bounded_and_never_follows_redirect_or_forwards_secrets() {
    let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = server.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/redirect",
            get(|| async {
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [(header::LOCATION, "http://127.0.0.1:1")],
                    "",
                )
            }),
        )
        .route(
            "/rate",
            get(|| async {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [(header::RETRY_AFTER, "7")],
                    Json(json!({"ok":false,"error":"nni_rate_limit_asset_transfer"})),
                )
            }),
        )
        .route("/large", get(|| async { "x".repeat(MAX_RESPONSE + 1) }))
        .route(
            "/error",
            get(|| async {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({"ok":false,"error":"internal secret"})),
                )
            }),
        );
    let task = tokio::spawn(async move {
        axum::serve(server, app).await.unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    assert!(forward(
        client.get(format!("http://{address}/redirect")),
        "capabilities",
        "assets",
        "https://edge.example.test",
        None
    )
    .await
    .is_err());
    assert!(forward(
        client.get(format!("http://{address}/large")),
        "capabilities",
        "assets",
        "https://edge.example.test",
        None
    )
    .await
    .is_err());
    let rate = forward(
        client.get(format!("http://{address}/rate")),
        "capabilities",
        "assets",
        "https://edge.example.test",
        None,
    )
    .await
    .unwrap();
    assert_eq!(rate.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(rate.headers()[header::RETRY_AFTER], "7");
    assert_eq!(rate.headers()[header::CACHE_CONTROL], "no-store");
    let error = forward(
        client.get(format!("http://{address}/error")),
        "capabilities",
        "assets",
        "https://edge.example.test",
        None,
    )
    .await
    .unwrap();
    let bytes = to_bytes(error.into_body(), 4096).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("secret"));
    task.abort();
    let _ = task.await;
}

#[test]
fn success_projection_rejects_extra_duplicate_and_cross_node_fields_but_preserves_signed_bytes() {
    let origin = "https://edge.example.test";
    let project =
        |bytes: &[u8], suffix: &str| response::project(bytes, suffix, "assets", origin, None);
    let raw = "{ \"memo\":\"\\u4e2d\\u6587\", \"amount_units\":\"1\" }";
    let body = json!({"ok":true,"data":{"signing_payload":raw}}).to_string();
    let projected: Value =
        serde_json::from_slice(&project(body.as_bytes(), "read/request").unwrap()).unwrap();
    assert_eq!(
        projected["data"]["signing_payload"]
            .as_str()
            .unwrap()
            .as_bytes(),
        raw.as_bytes()
    );
    assert!(project(
        br#"{"ok":true,"data":{"signing_payload":"{}","secret":"hidden"}}"#,
        "read/request"
    )
    .is_err());
    assert!(project(
        br#"{"ok":true,"data":{"signing_payload":"{}","signing_payload":"[]"}}"#,
        "read/request"
    )
    .is_err());
    assert!(project(
        br#"{"ok":true,"ok":true,"data":{"signing_payload":"{}"}}"#,
        "read/request"
    )
    .is_err());
    let mut cap = json!({"ok":true,"data":{"schema_version":1,"protocol":"asset_owner_v1",
        "ledger_id":"ledger","node_url":origin,"service":"assets","actions":["balances","transfer"]}});
    assert!(project(cap.to_string().as_bytes(), "capabilities").is_ok());
    cap["data"]["node_url"] = json!("https://elsewhere.example.test");
    assert!(project(cap.to_string().as_bytes(), "capabilities").is_err());
    cap["data"]["node_url"] = json!(origin);
    cap["data"]["actions"] = json!(["balances", "bancor_trade"]);
    assert!(project(cap.to_string().as_bytes(), "capabilities").is_err());
}

#[tokio::test]
async fn gateway_auth_and_body_source_cannot_be_bypassed_by_client_metadata() {
    use tower::ServiceExt;
    let root = std::env::temp_dir().join(format!("owner-gateway-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("data/nni")).unwrap();
    let config = json!({"schema_version":2,"remote_nodes":["https://edge.example.test"],
        "selected_node_url":"https://edge.example.test","joined":false});
    std::fs::write(
        root.join("data/nni/runtime-config.json"),
        config.to_string(),
    )
    .unwrap();
    let mut state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.skill_rt.workspace_root = root.clone();
    state.seed_test_auth_identity("test-owner-admin", "admin");
    state.seed_test_auth_identity("test-owner-member", "user");
    let app = Router::new().nest("/v1", routes()).with_state(state);
    let path = "/v1/nni/assets/owner/read/request";
    let body = json!({"schema_version":1,"protocol":"asset_owner_v1","ledger_id":"ledger",
        "node_url":"https://other.example.test","service":"assets",
        "account":"73MTSWz2Nks4Eaf8G8F7Nr6jbHorZSM774HFmtrdEuahUsc7To",
        "operation_id":uuid::Uuid::new_v4(),"intent":{"kind":"balances"}});
    for (key, assertion, status) in [
        ("", None, StatusCode::UNAUTHORIZED),
        ("test-owner-member", None, StatusCode::FORBIDDEN),
        ("test-owner-admin", Some("forged"), StatusCode::FORBIDDEN),
        ("test-owner-admin", None, StatusCode::CONFLICT),
    ] {
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri(path)
            .header(claw_core::product_identity::AUTH_KEY_HEADER, key)
            .header("content-type", "application/json");
        if let Some(value) = assertion {
            req = req.header(owner_gateway_context::HEADER, value);
        }
        let response = app
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{key}");
    }
    assert_eq!(
        std::fs::read_to_string(root.join("data/nni/runtime-config.json")).unwrap(),
        config.to_string()
    );
    std::fs::remove_dir_all(root).unwrap();
}

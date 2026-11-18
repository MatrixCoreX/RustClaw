use super::*;

#[tokio::test]
async fn large_unicode_export_uses_one_real_http_preview_and_one_confirmation() {
    let env = Env::new();
    let expected = (0..48)
        .map(|i| format!("{i:02}{}", "文".repeat(17_998)))
        .collect::<Vec<_>>();
    let raw = serde_json::to_vec(&export(expected.clone())).unwrap();
    assert!(raw.len() > 2 * 1024 * 1024);
    let id = env.ready(&raw).await;
    let (status, preview) = env
        .call(
            Method::POST,
            &format!("/memory/import/uploads/{id}/preview"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["data"]["accepted_items"], 48);
    {
        let db = env.fixture.state().core.db.get().unwrap();
        let rows: i64 = db
            .query_row("SELECT COUNT(*) FROM memory_facts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }
    let confirmation = request_confirmation(&preview);
    let (status, reply) = env
        .call(
            Method::POST,
            "/memory/import/confirm",
            Some(confirmation.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(reply["data"]["imported_items"], 48);
    let (status, _) = env
        .call(Method::POST, "/memory/import/confirm", Some(confirmation))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (_, state) = env
        .call(Method::GET, &format!("/memory/import/uploads/{id}"), None)
        .await;
    assert_eq!(state["data"]["state"], "confirmed");
    let db = env.fixture.state().core.db.get().unwrap();
    let mut statement = db
        .prepare("SELECT fact_text FROM memory_facts ORDER BY fact_text")
        .unwrap();
    let actual = statement
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(actual, expected);
    let sessions: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_import_sessions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(sessions, 1);
}

#[tokio::test]
async fn legacy_single_request_limit_is_not_disabled_for_unrelated_routes() {
    let env = Env::new();
    let export = export(
        (0..48)
            .map(|i| format!("{i:02}{}", "文".repeat(17_998)))
            .collect(),
    );
    let raw = serde_json::to_vec(&json!({"export":export})).unwrap();
    assert!(raw.len() > 2 * 1024 * 1024);
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    let (status, _, _) = env
        .raw(
            Method::POST,
            "/memory/import/preview",
            Some(&env.fixture.key),
            Body::from(raw),
            headers,
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

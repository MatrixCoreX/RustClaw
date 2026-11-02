use super::*;

fn request(value: Value) -> AccountControlRequest {
    serde_json::from_value(value).unwrap()
}

#[test]
fn aipp_account_control_uses_manifest_refs_and_requires_platform_confirmation() {
    let controls = skill_sdk::AippAccountControls {
        schema_version: 1,
        status_capability: "fixture.read_account".into(),
        logout_capability: "fixture.forget_account".into(),
    };
    assert_eq!(
        selected_capability(&controls, &request(json!({"operation":"status"}))),
        Some("fixture.read_account")
    );
    for value in [
        json!({"operation":"logout","platform":"weibo"}),
        json!({"operation":"logout","confirm":true}),
        json!({"operation":"logout","platform":"../outside","confirm":true}),
        json!({"operation":"arbitrary_capability","confirm":true}),
    ] {
        assert!(selected_capability(&controls, &request(value)).is_none());
    }
    assert_eq!(
        selected_capability(
            &controls,
            &request(json!({"operation":"logout","platform":"weibo","confirm":true}))
        ),
        Some("fixture.forget_account")
    );
    assert!(serde_json::from_value::<AccountControlRequest>(
        json!({"operation":"status","capability":"foreign.mutate"})
    )
    .is_err());
}

#[tokio::test]
async fn aipp_account_control_rejects_non_admin_before_reading_profiles() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.seed_test_auth_identity("ordinary-account-fixture", "user");
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-agent-key",
        axum::http::HeaderValue::from_static("ordinary-account-fixture"),
    );
    let response = control_aipp_account(
        State(state),
        headers,
        AxumPath("fixture".into()),
        Json(request(
            json!({"operation":"logout","platform":"weibo","confirm":true}),
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn aipp_account_control_cannot_call_an_uninstalled_app() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.seed_test_auth_identity("admin-account-fixture", "admin");
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-agent-key",
        axum::http::HeaderValue::from_static("admin-account-fixture"),
    );
    let response = control_aipp_account(
        State(state),
        headers,
        AxumPath("uninstalled_fixture".into()),
        Json(request(json!({"operation":"status"}))),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn aipp_account_use_has_a_public_allowlist_and_no_prose_inference() {
    let row = json!({"schema_version":1,"platform":"weibo","mode":"authenticated_browser",
        "cookie":"private-fixture","profile_path":"/private/fixture",
        "account":{"status":"verified","display_name":"Self account","checked_at":123,"token":"private-fixture"}});
    let rows = public_account_use(Some(
        &json!([row.clone(), {"platform":"weibo","mode":"authenticated_browser"}]),
    ));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["account"]["display_name"], "Self account");
    assert!(!serde_json::to_string(&rows)
        .unwrap()
        .contains("private-fixture"));
    let result = json!({"text":"logged in as an author", "task_journal":{"trace":{"capability_results":[{"data":{"extra":{"account_use":[row]}}}]}}});
    assert_eq!(task_account_use(&result).len(), 1);
    assert!(task_account_use(&json!({"text":"logged in as an author"})).is_empty());
}

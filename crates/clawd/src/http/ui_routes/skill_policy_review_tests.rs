use super::*;
async fn installed_unapproved() -> (AppState, PathBuf, axum::Router, Value) {
    let (state, workspace) = isolated_skill_store_state();
    let name = "policy_probe";
    let bundle = workspace.join("third_party").join(name);
    let interface = write_external_skill_fixture(&bundle, name);
    // Even an import-time enable flag must not grant execution authority.
    let (status, response) = super::super::finalize_imported_bundle(
        &state,
        &bundle,
        "third_party/policy_probe",
        "local-test",
        true,
        false,
        &interface,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let data = response.0.data.unwrap();
    assert_eq!(data["installed"], true);
    assert_eq!(data["enabled"], false);
    assert_eq!(data["admission_state"], "awaiting_policy_approval");
    assert!(data["policy_grant_digest"].is_null());
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state.clone());
    (state, workspace, router, data)
}
fn decision(review: &Value, choice: &str) -> Value {
    serde_json::json!({"decision":choice,"enable":true,"registry_generation":review["registry_generation"],"manifest_digest":review["manifest_digest"],"capability_request_digest":review["capability_request_digest"]})
}
#[tokio::test]
async fn permission_review_approval_revocation_and_stale_guard() {
    let (state, workspace, router, data) = installed_unapproved().await;
    let review = &data["policy_review"];
    assert_eq!(review["admission_state"], "awaiting_policy_approval");
    assert!(!admission_service(&state)
        .unwrap()
        .catalog_snapshot()
        .unwrap()
        .enabled
        .contains("policy_probe"));
    let (status, list) = call_skill_store_api(
        router.clone(),
        Method::GET,
        "/v1/skills/policy-reviews",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["data"]["total"], 1);
    let (status, approved) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/policy_probe/policy",
        Some(decision(review, "approve")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(approved["data"]["admission_state"], "enabled");
    assert!(admission_service(&state)
        .unwrap()
        .catalog_snapshot()
        .unwrap()
        .enabled
        .contains("policy_probe"));
    let (status, stale) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/policy_probe/policy",
        Some(decision(review, "approve")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(stale["error"], "skill_policy_review_stale");
    let (status, revoked) = call_skill_store_api(
        router,
        Method::POST,
        "/v1/skills/policy_probe/policy",
        Some(decision(&approved["data"], "revoke")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        revoked["data"]["admission_state"],
        "awaiting_policy_approval"
    );
    assert!(!admission_service(&state)
        .unwrap()
        .catalog_snapshot()
        .unwrap()
        .enabled
        .contains("policy_probe"));
    std::fs::remove_dir_all(workspace).unwrap();
}
#[tokio::test]
async fn tampered_manifest_review_cannot_publish_a_grant() {
    let (state, workspace, router, data) = installed_unapproved().await;
    let mut request = decision(&data["policy_review"], "approve");
    request["manifest_digest"] = serde_json::json!("tampered");
    let (status, _) = call_skill_store_api(
        router,
        Method::POST,
        "/v1/skills/policy_probe/policy",
        Some(request),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!admission_service(&state)
        .unwrap()
        .catalog_snapshot()
        .unwrap()
        .enabled
        .contains("policy_probe"));
    std::fs::remove_dir_all(workspace).unwrap();
}

#[tokio::test]
async fn unapproved_install_can_be_discarded_without_creating_a_grant_or_deleting_files() {
    let (state, workspace, router, data) = installed_unapproved().await;
    let (status, removed) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/policy_probe/policy",
        Some(decision(&data["policy_review"], "discard")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(removed["data"]["admission_state"], "tombstoned");
    let snapshot = admission_service(&state)
        .unwrap()
        .catalog_snapshot()
        .unwrap();
    assert!(snapshot.tombstoned.contains("policy_probe"));
    assert!(!snapshot.enabled.contains("policy_probe"));
    let (_, list) =
        call_skill_store_api(router, Method::GET, "/v1/skills/policy-reviews", None).await;
    assert_eq!(list["data"]["total"], 0);
    assert!(installed_package_pointer(&workspace, "policy_probe").exists());
    std::fs::remove_dir_all(workspace).unwrap();
}

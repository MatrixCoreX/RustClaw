use super::*;
use axum::body::{to_bytes, Body};
use axum::http::{Method, Request};
use tower::ServiceExt;
#[test]
fn large_first_category_cannot_starve_later_categories() {
    let root = std::env::temp_dir().join(format!("storage-fairness-{}", uuid::Uuid::new_v4()));
    for name in ["large", "logs", "programs"] {
        fs::create_dir_all(root.join(name)).unwrap();
    }
    for index in 0..64 {
        fs::write(root.join("large").join(index.to_string()), [1; 32]).unwrap();
    }
    fs::write(root.join("logs/current"), [2; 64]).unwrap();
    fs::write(root.join("programs/runner"), [3; 128]).unwrap();
    let root = root.canonicalize().unwrap();
    let roots = [
        ("large", root.join("large")),
        ("logs", root.join("logs")),
        ("programs", root.join("programs")),
    ];
    let mut budget = 12;
    let counts = scan_storage_categories(&roots, &root, &mut budget);
    assert!(counts[0].partial);
    assert_eq!(counts[1].bytes, 64);
    assert_eq!(counts[2].bytes, 128);
    assert!(!counts[1].partial && !counts[2].partial);
    assert!(budget < 12);
    let mut exhausted = 0;
    let counts = scan_storage_categories(&roots, &root, &mut exhausted);
    assert!(counts.iter().all(|count| count.partial && count.bytes == 0));
    assert_eq!(exhausted, 0);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn inventory_is_bounded_and_never_changes_files() {
    let root = std::env::temp_dir().join(format!("storage-inventory-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    fs::write(root.join("evidence.bin"), [7; 4096]).unwrap();
    let mut budget = 100;
    let count = scan_storage_category("evidence", &root, &root, &mut budget);
    assert_eq!(count.bytes, 4096);
    assert_eq!(count.files, 1);
    assert!(!count.partial);
    assert!(count.protected);
    let mut budget = 0;
    assert!(scan_storage_category("evidence", &root, &root, &mut budget).partial);
    assert_eq!(fs::read(root.join("evidence.bin")).unwrap(), vec![7; 4096]);
    fs::remove_dir_all(root).unwrap();
}
#[cfg(unix)]
#[test]
fn inventory_does_not_follow_symlinks_or_escape_root() {
    let dir = std::env::temp_dir().join(format!("storage-symlink-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(dir.join("inside")).unwrap();
    fs::write(dir.join("outside.bin"), [0; 4096]).unwrap();
    let inside = dir.join("inside").canonicalize().unwrap();
    std::os::unix::fs::symlink(dir.join("outside.bin"), inside.join("link")).unwrap();
    let mut budget = 100;
    let summary = scan_storage_category("outputs", &inside, &inside, &mut budget);
    assert_eq!(summary.bytes, 0);
    assert!(summary.partial);
    let mut budget = 100;
    assert!(scan_storage_category("outputs", &dir, &inside, &mut budget).partial);
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn storage_and_policy_endpoints_require_admin_and_do_not_mutate_on_denial() {
    for role in [None, Some("user")] {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        if let Some(role) = role {
            state.seed_test_auth_identity("storage-test-user", role);
        }
        let router = Router::new()
            .nest("/v1", build_ui_router())
            .with_state(state.clone());
        for (method, path, body) in [
            (Method::GET, "/v1/system/storage-summary", ""),
            (Method::GET, "/v1/system/checkpoint-status", ""),
            (Method::POST, "/v1/system/storage-scan", ""),
            (Method::POST, "/v1/system/storage-compact", ""),
            (Method::GET, "/v1/skills/policy-reviews", ""),
            (
                Method::POST,
                "/v1/skills/probe/policy",
                r#"{"decision":"approve","registry_generation":1,"manifest_digest":"digest","capability_request_digest":"digest"}"#,
            ),
        ] {
            let mut req = Request::builder()
                .uri(path)
                .method(method)
                .header("content-type", "application/json");
            if role.is_some() {
                req = req.header("x-agent-key", "storage-test-user");
            }
            let response = router
                .clone()
                .oneshot(req.body(Body::from(body)).unwrap())
                .await
                .unwrap();
            assert!(matches!(
                response.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ));
            assert!(to_bytes(response.into_body(), 4096).await.is_ok());
        }
        let db = state.core.db.get().unwrap();
        let tables:i64=db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('runtime_storage_inventory_samples','runtime_result_object_maintenance')",[],|r|r.get(0)).unwrap();
        assert_eq!(tables, 0);
    }
}
#[test]
fn runtime_revision_does_not_invent_source_or_build_evidence() {
    let a = "a".repeat(40);
    let b = "b".repeat(40);
    assert_eq!(
        RuntimeVersionSummary::from_revisions(&a, Some(a.clone())).source_match,
        "matched"
    );
    assert_eq!(
        RuntimeVersionSummary::from_revisions(&a, Some(b)).source_match,
        "different"
    );
    assert_eq!(
        RuntimeVersionSummary::from_revisions("", Some(a.clone())).source_match,
        "unavailable"
    );
    assert_eq!(
        RuntimeVersionSummary::from_revisions(&a, None).source_match,
        "unavailable"
    );
    assert_eq!(
        RuntimeVersionSummary::from_revisions(&a, Some("invalid".into())).source_revision,
        None
    );
}

#[tokio::test]
async fn inventory_get_is_read_only_and_recorded_trend_is_bounded() {
    let root = std::env::temp_dir().join(format!("storage-admin-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("logs")).unwrap();
    fs::write(root.join("logs/private-log-name"), "private-log-body").unwrap();
    let mut state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.skill_rt.workspace_root = root.clone();
    state.seed_test_auth_identity("inventory-admin", "admin");
    let router = Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state.clone());
    let read = || {
        Request::builder()
            .uri("/v1/system/storage-summary")
            .header("x-agent-key", "inventory-admin")
            .body(Body::empty())
            .unwrap()
    };
    let response = router.clone().oneshot(read()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let raw = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(!text.contains("private-log-name"));
    assert!(!text.contains("private-log-body"));
    assert!(!text.contains(root.to_str().unwrap()));
    let db = state.core.db.get().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='runtime_storage_inventory_samples'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    drop(db);
    for _ in 0..27 {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/system/storage-scan")
                    .header("x-agent-key", "inventory-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let db = state.core.db.get().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM runtime_storage_inventory_samples",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 24);
    drop(db);
    assert_eq!(
        fs::read_to_string(root.join("logs/private-log-name")).unwrap(),
        "private-log-body"
    );
    fs::remove_dir_all(root).unwrap();
}

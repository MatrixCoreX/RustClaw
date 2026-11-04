use super::*;
use std::time::{Duration, Instant};

async fn observe_auth_pool_wait(key: &'static str, expected: StatusCode, error: Option<&str>) {
    let (state, _root) = fixture();
    let held = state.core.db.get().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    let started = Instant::now();
    let ((status, Json(response)), timer_delay) = tokio::join!(read_items(&state, key), async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        started.elapsed()
    });
    release.join().unwrap();
    assert_eq!(status, expected);
    assert_eq!(response.ok, expected == StatusCode::OK);
    assert_eq!(response.error.as_deref(), error);
    if expected == StatusCode::OK {
        assert_eq!(response.data.unwrap()["total_item_count"], 0);
    }
    eprintln!(
        "aipp_items_auth_pool_hold_ms=500 timer_delay_ms={}",
        timer_delay.as_millis()
    );
    assert!(timer_delay < Duration::from_millis(400), "{timer_delay:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn aipp_items_admin_auth_pool_wait_does_not_block_the_async_executor() {
    observe_auth_pool_wait("aipp-view-admin", StatusCode::OK, None).await;
}

#[tokio::test(flavor = "current_thread")]
async fn aipp_items_invalid_auth_pool_wait_does_not_block_the_async_executor() {
    observe_auth_pool_wait(
        "aipp-view-unknown",
        StatusCode::UNAUTHORIZED,
        Some("auth_key_invalid"),
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn aipp_items_non_admin_auth_pool_wait_does_not_block_the_async_executor() {
    observe_auth_pool_wait(
        "aipp-view-user",
        StatusCode::FORBIDDEN,
        Some("admin_required"),
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn aipp_items_inline_read_control_demonstrates_executor_stall() {
    let (state, _root) = fixture();
    let held = state.core.db.get().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        axum::http::HeaderValue::from_static("aipp-view-admin"),
    );
    let started = Instant::now();
    let ((status, _), timer_delay) = tokio::join!(
        async { read_aipp_items(&state, &headers, "fixture_view", &AippMediaQuery::default()) },
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            started.elapsed()
        },
    );
    release.join().unwrap();
    assert_eq!(status, StatusCode::OK);
    eprintln!(
        "aipp_items_inline_control_hold_ms=500 timer_delay_ms={}",
        timer_delay.as_millis()
    );
    assert!(timer_delay >= Duration::from_millis(500), "{timer_delay:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn aipp_items_missing_key_is_rejected_without_waiting_for_the_database() {
    let (state, _root) = fixture();
    let _held = state.core.db.get().unwrap();
    let (status, Json(response)) = tokio::time::timeout(
        Duration::from_secs(2),
        get_aipp_items(
            State(state.clone()),
            HeaderMap::new(),
            AxumPath("fixture_view".into()),
            Query(AippMediaQuery::default()),
        ),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("auth_key_required"));
}

#[tokio::test]
async fn aipp_items_rechecks_role_and_revocation_on_every_request() {
    let (state, _root) = fixture();
    assert_eq!(
        read_items(&state, "aipp-view-admin").await.0,
        StatusCode::OK
    );
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE auth_keys SET role='user' WHERE user_key='aipp-view-admin'",
            [],
        )
        .unwrap();
    let (status, Json(response)) = read_items(&state, "aipp-view-admin").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("admin_required"));
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE auth_keys SET role='admin', enabled=0 WHERE user_key='aipp-view-admin'",
            [],
        )
        .unwrap();
    let (status, Json(response)) = read_items(&state, "aipp-view-admin").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("auth_key_invalid"));
}

#[tokio::test]
async fn aipp_items_auth_database_failure_never_returns_items() {
    let (state, _root) = fixture();
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("DROP TABLE auth_keys", [])
        .unwrap();
    let (status, Json(response)) = read_items(&state, "aipp-view-admin").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!response.ok);
    assert!(response.data.is_none());
    assert!(response.error.is_some());
}

#[tokio::test]
async fn aipp_items_task_filter_errors_keep_their_bad_request_status() {
    let (state, _root) = fixture();
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        axum::http::HeaderValue::from_static("aipp-view-admin"),
    );
    for query in [
        AippMediaQuery {
            channel: Some("unsupported".into()),
            ..Default::default()
        },
        AippMediaQuery {
            status: Some("unsupported".into()),
            ..Default::default()
        },
        AippMediaQuery {
            sort_order: Some("unsupported".into()),
            ..Default::default()
        },
    ] {
        let (status, Json(response)) = get_aipp_items(
            State(state.clone()),
            headers.clone(),
            AxumPath("fixture_view".into()),
            Query(query),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(!response.ok);
        let code = response.error.as_deref().unwrap();
        assert_eq!(
            response.data,
            Some(json!({
                "error_code": code,
                "message_key": format!("clawd.ui.aipp.{code}"),
            }))
        );
        assert!(matches!(
            response.error.as_deref(),
            Some("aipp_task_activity_filter_invalid" | "aipp_media_sort_order_invalid")
        ));
    }
}

#[tokio::test]
async fn aipp_items_media_contract_reads_fresh_records_and_keeps_error_status() {
    let (state, root) = fixture();
    let path = root.0.join("sources/fixture_view/skill.toml");
    let mut manifest = skill_sdk::PackageManifest::load(&path).unwrap();
    let aipp = manifest.aipp.as_mut().unwrap();
    aipp.renderer = "collection_feed_v1".into();
    aipp.data_contract = "media_collection_v1".into();
    aipp.task_channel_scope = None;
    fs::write(&path, manifest.to_toml_string().unwrap()).unwrap();
    let storage = Workspace(
        state
            .core
            .skill_storage
            .directory_path("fixture_view")
            .unwrap(),
    );
    fs::create_dir_all(storage.0.join("records")).unwrap();
    for sequence in 1..=2 {
        write_record(
            &storage.0,
            sequence,
            json!({
                "global_sequence": sequence, "kind": "image", "platform": "fixture",
                "title": format!("Fixture {sequence}"),
            }),
        );
        let (status, Json(response)) = read_items(&state, "aipp-view-admin").await;
        assert_eq!(status, StatusCode::OK);
        assert!(response.ok);
        let data = response.data.unwrap();
        assert_eq!(data["items"].as_array().unwrap().len(), sequence as usize);
        assert_eq!(data["items"][0]["global_sequence"], sequence);
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        axum::http::HeaderValue::from_static("aipp-view-admin"),
    );
    let (status, Json(response)) = get_aipp_items(
        State(state),
        headers,
        AxumPath("fixture_view".into()),
        Query(AippMediaQuery {
            sort_order: Some("unsupported".into()),
            ..Default::default()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!response.ok);
    assert_eq!(
        response.error.as_deref(),
        Some("aipp_media_sort_order_invalid")
    );
}

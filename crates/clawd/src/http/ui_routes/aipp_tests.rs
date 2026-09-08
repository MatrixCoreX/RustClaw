use super::*;

fn fixture_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("aipp-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("records")).expect("records directory");
    fs::create_dir_all(root.join("exports/video_covers")).expect("preview directory");
    fs::create_dir_all(root.join("exports/images")).expect("image preview directory");
    root
}

fn write_record(root: &Path, sequence: u64, value: Value) {
    fs::write(
        root.join("records").join(format!("{sequence:012}.json")),
        serde_json::to_vec(&value).expect("record JSON"),
    )
    .expect("write record");
}

fn task_activity_db() -> rusqlite::Connection {
    let db = rusqlite::Connection::open_in_memory().expect("activity database");
    db.execute_batch(
        r#"
        CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            channel TEXT NOT NULL,
            status TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            result_json TEXT,
            error_text TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE task_event_stream (
            task_id TEXT NOT NULL,
            event_json TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL
        );
        CREATE TABLE task_event_archive (
            task_id TEXT NOT NULL,
            event_json TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL
        );
        "#,
    )
    .expect("activity schema");
    db
}

fn insert_task_activity(
    db: &rusqlite::Connection,
    task_id: &str,
    channel: &str,
    status: &str,
    skill: &str,
    action_ref: &str,
    archived: bool,
) {
    let payload = json!({
        "text": format!("process https://media.example.test/{task_id}?v=1"),
        "context_token": "must-not-leak",
    });
    let result = json!({
        "text": format!("processed {task_id}"),
        "task_journal": { "secret": "must-not-leak" },
        "artifacts": [{
            "id": "artifact-1",
            "filename": "result.txt",
            "kind": "file",
            "mime_type": "text/plain",
            "size_bytes": 12,
            "download_url": format!("/v1/tasks/{task_id}/artifacts/artifact-1/content"),
            "local_path": "/private/result.txt",
        }],
    });
    db.execute(
        "INSERT INTO tasks(task_id, channel, status, payload_json, result_json, error_text, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, '100', '101')",
        rusqlite::params![task_id, channel, status, payload.to_string(), result.to_string()],
    )
    .expect("task row");
    let event = json!({
        "event_kind": "tool_finished",
        "payload": {
            "skill": skill,
            "requested_action_ref": action_ref,
        }
    });
    let table = if archived {
        "task_event_archive"
    } else {
        "task_event_stream"
    };
    db.execute(
        &format!("INSERT INTO {table}(task_id, event_json, created_at_ms) VALUES (?1, ?2, 1000)"),
        rusqlite::params![task_id, event.to_string()],
    )
    .expect("activity event");
}

#[test]
fn task_activity_page_projects_current_and_archived_skill_tasks_without_secrets() {
    let db = task_activity_db();
    insert_task_activity(
        &db,
        "task-current",
        "wechat",
        "succeeded",
        "media_download",
        "media_download.download",
        false,
    );
    insert_task_activity(
        &db,
        "task-archived",
        "ui",
        "succeeded",
        "media_download",
        "media_download.transcribe",
        true,
    );
    insert_task_activity(
        &db,
        "task-other",
        "telegram",
        "succeeded",
        "another_skill",
        "another_skill.run",
        false,
    );

    let page =
        read_aipp_task_activity_page(&db, "media_download", "all", &AippMediaQuery::default())
            .expect("activity page");
    let items = page["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(page["page_item_count"], 2);
    assert_eq!(items[0]["task_id"], "task-archived");
    assert_eq!(items[0]["channel"], "ui");
    assert_eq!(items[1]["channel"], "wechat");
    assert_eq!(
        items[1]["source_urls"][0],
        "https://media.example.test/task-current?v=1"
    );
    assert_eq!(items[1]["actions"][0], "media_download.download");
    assert_eq!(
        items[1]["artifacts"][0]["download_url"],
        "/v1/tasks/task-current/artifacts/artifact-1/content"
    );
    let external_only = read_aipp_task_activity_page(
        &db,
        "media_download",
        "communication",
        &AippMediaQuery::default(),
    )
    .expect("communication-only activity page");
    assert_eq!(external_only["page_item_count"], 1);
    assert_eq!(external_only["items"][0]["channel"], "wechat");
    let encoded = serde_json::to_string(&page).expect("page JSON");
    assert!(!encoded.contains("context_token"));
    assert!(!encoded.contains("must-not-leak"));
    assert!(!encoded.contains("local_path"));
    assert!(!encoded.contains("/private/result.txt"));
}

#[test]
fn task_activity_page_filters_channels_search_and_uses_stable_cursors() {
    let db = task_activity_db();
    for (task_id, channel) in [
        ("task-one", "wechat"),
        ("task-two", "ui"),
        ("task-three", "wechat"),
    ] {
        insert_task_activity(
            &db,
            task_id,
            channel,
            "succeeded",
            "media_download",
            "media_download.download",
            false,
        );
    }
    let first = read_aipp_task_activity_page(
        &db,
        "media_download",
        "communication",
        &AippMediaQuery {
            limit: Some(1),
            channel: Some("wechat".to_string()),
            query: Some("PROCESSED".to_string()),
            ..AippMediaQuery::default()
        },
    )
    .expect("first activity page");
    assert_eq!(first["page_item_count"], 1);
    assert_eq!(first["items"][0]["task_id"], "task-three");
    let cursor = first["next_cursor_sequence"].as_u64().expect("cursor");
    let second = read_aipp_task_activity_page(
        &db,
        "media_download",
        "communication",
        &AippMediaQuery {
            limit: Some(1),
            channel: Some("wechat".to_string()),
            cursor_sequence: Some(cursor),
            ..AippMediaQuery::default()
        },
    )
    .expect("second activity page");
    assert_eq!(second["items"][0]["task_id"], "task-one");

    let invalid = read_aipp_task_activity_page(
        &db,
        "media_download",
        "communication",
        &AippMediaQuery {
            channel: Some("unknown".to_string()),
            ..AippMediaQuery::default()
        },
    )
    .expect_err("invalid channel");
    assert_eq!(invalid, "aipp_task_activity_filter_invalid");
}

#[test]
fn media_page_is_newest_first_filtered_and_field_bounded() {
    let root = fixture_root();
    write_record(
        &root,
        1,
        json!({
            "global_sequence": 1,
            "sequence": 1,
            "kind": "image",
            "platform": "xiaohongshu",
            "title": "First useful note",
            "platform_text": "visible copy",
            "recognized_text": "alpha",
            "image_url": "https://images.example.test/one.webp",
            "image_screenshot_path": "images/one.png",
            "source_page_url": "https://source.example.test/one",
            "engagement": {
                "schema_version": 1,
                "platform": "xiaohongshu",
                "captured_at": "2026-09-07T00:00:00Z",
                "metrics": {
                    "likes": { "display": "1.2万" },
                    "comments": { "display": "318", "value": 318 },
                    "unknown": { "display": "must-not-leak" }
                }
            },
            "secret": "must-not-leak",
        }),
    );
    write_record(
        &root,
        2,
        json!({
            "global_sequence": 2,
            "sequence": 1,
            "kind": "video",
            "platform": "douyin",
            "title": "Second useful clip",
            "recognized_text": "beta",
            "cover_screenshot_path": "video_covers/two.png",
            "video_page_url": "http://insecure.example.test/two",
        }),
    );

    let page = read_aipp_media_page(
        &root,
        &AippMediaQuery {
            limit: Some(10),
            query: Some("USEFUL".to_string()),
            ..AippMediaQuery::default()
        },
    )
    .expect("media page");
    let items = page["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["global_sequence"], 2);
    assert_eq!(items[1]["global_sequence"], 1);
    assert!(items[0].get("secret").is_none());
    assert!(items[0].get("recognized_text").is_none());
    assert!(items[1].get("recognized_text").is_none());
    assert!(items[0]["source_url"].is_null());
    assert_eq!(
        items[1]["engagement"]["metrics"]["likes"]["display"],
        "1.2万"
    );
    assert_eq!(items[1]["engagement"]["metrics"]["comments"]["value"], 318);
    assert!(items[1]["engagement"]["metrics"].get("unknown").is_none());
    assert_eq!(items[1]["preview_available"], true);
    assert_eq!(page["matching_total"], 2);

    let legacy_ocr_search = read_aipp_media_page(
        &root,
        &AippMediaQuery {
            query: Some("alpha".to_string()),
            ..AippMediaQuery::default()
        },
    )
    .expect("legacy OCR search");
    assert_eq!(legacy_ocr_search["matching_total"], 0);
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn media_page_bounds_skill_owned_copy_and_ignores_oversized_records() {
    let root = fixture_root();
    write_record(
        &root,
        1,
        json!({
            "global_sequence": 1,
            "kind": "image",
            "platform": "test",
            "title": "a".repeat(700),
            "platform_text": "b".repeat(40_000),
            "recognized_text": "b".repeat(40_000),
        }),
    );
    fs::write(
        root.join("records/000000000002.json"),
        vec![b'x'; AIPP_MEDIA_RECORD_MAX_BYTES as usize + 1],
    )
    .expect("oversized record");

    let page = read_aipp_media_page(&root, &AippMediaQuery::default()).expect("media page");
    let items = page["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"].as_str().map(str::len), Some(512));
    assert_eq!(
        items[0]["platform_text"].as_str().map(str::len),
        Some(32_768)
    );
    assert!(items[0].get("recognized_text").is_none());
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn media_page_keeps_bounded_state_when_no_records_exist() {
    let root = fixture_root();
    fs::remove_dir_all(root.join("records")).expect("remove records directory");
    fs::write(
        root.join("state.json"),
        serde_json::to_vec(&json!({
            "updated_at": "2026-09-07T12:00:00Z",
            "platforms": {
                "douyin": { "state": "enabled", "enabled": true, "paused": false }
            }
        }))
        .expect("state JSON"),
    )
    .expect("write state");

    let page = read_aipp_media_page(&root, &AippMediaQuery::default()).expect("media page");
    assert_eq!(page["items"].as_array().map(Vec::len), Some(0));
    assert_eq!(page["platform_states"]["douyin"]["enabled"], true);
    assert_eq!(page["updated_at"], "2026-09-07T12:00:00Z");
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn media_page_uses_stable_cursor_pagination() {
    let root = fixture_root();
    for sequence in 1..=3 {
        write_record(
            &root,
            sequence,
            json!({
                "global_sequence": sequence,
                "sequence": sequence,
                "kind": "video",
                "platform": "douyin",
                "title": format!("item {sequence}"),
            }),
        );
    }
    let first = read_aipp_media_page(
        &root,
        &AippMediaQuery {
            limit: Some(2),
            ..AippMediaQuery::default()
        },
    )
    .expect("first page");
    assert_eq!(first["items"][0]["global_sequence"], 3);
    assert_eq!(first["items"][1]["global_sequence"], 2);
    assert_eq!(first["next_before_sequence"], 2);

    let second = read_aipp_media_page(
        &root,
        &AippMediaQuery {
            limit: Some(2),
            before_sequence: Some(2),
            ..AippMediaQuery::default()
        },
    )
    .expect("second page");
    assert_eq!(second["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(second["items"][0]["global_sequence"], 1);
    assert_eq!(second["matching_total"], 3);
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn media_page_supports_oldest_first_collection_time_pagination() {
    let root = fixture_root();
    for sequence in 1..=3 {
        write_record(
            &root,
            sequence,
            json!({
                "global_sequence": sequence,
                "sequence": sequence,
                "kind": "video",
                "platform": "douyin",
                "title": format!("item {sequence}"),
                "discovered_at": format!("2026-09-07T00:00:0{sequence}Z"),
            }),
        );
    }
    let first = read_aipp_media_page(
        &root,
        &AippMediaQuery {
            limit: Some(2),
            sort_order: Some("oldest".to_string()),
            ..AippMediaQuery::default()
        },
    )
    .expect("oldest first page");
    assert_eq!(first["sort_order"], "oldest");
    assert_eq!(first["items"][0]["global_sequence"], 1);
    assert_eq!(first["items"][1]["global_sequence"], 2);
    assert_eq!(first["next_cursor_sequence"], 2);

    let second = read_aipp_media_page(
        &root,
        &AippMediaQuery {
            limit: Some(2),
            cursor_sequence: Some(2),
            sort_order: Some("oldest".to_string()),
            ..AippMediaQuery::default()
        },
    )
    .expect("oldest second page");
    assert_eq!(second["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(second["items"][0]["global_sequence"], 3);
    assert_eq!(second["next_cursor_sequence"], Value::Null);
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn media_page_rejects_unknown_sort_order() {
    let root = fixture_root();
    let error = read_aipp_media_page(
        &root,
        &AippMediaQuery {
            sort_order: Some("random".to_string()),
            ..AippMediaQuery::default()
        },
    )
    .expect_err("unknown sort order must fail");
    assert_eq!(error, "aipp_media_sort_order_invalid");
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn preview_resolution_stays_inside_skill_exports() {
    let root = fixture_root();
    fs::write(root.join("exports/video_covers/safe.png"), b"png").expect("preview");
    write_record(
        &root,
        1,
        json!({
            "global_sequence": 1,
            "kind": "video",
            "cover_screenshot_path": "video_covers/safe.png",
        }),
    );
    let (path, media_type) = resolve_aipp_preview(&root, 1).expect("safe preview");
    assert!(path.ends_with("video_covers/safe.png"));
    assert_eq!(media_type, "image/png");

    fs::write(root.join("exports/images/note.webp"), b"webp").expect("image preview");
    write_record(
        &root,
        2,
        json!({
            "global_sequence": 2,
            "kind": "image",
            "image_screenshot_path": "images/note.webp",
        }),
    );
    let (path, media_type) = resolve_aipp_preview(&root, 2).expect("image preview");
    assert!(path.ends_with("images/note.webp"));
    assert_eq!(media_type, "image/webp");

    write_record(
        &root,
        2,
        json!({
            "global_sequence": 2,
            "kind": "video",
            "cover_screenshot_path": "../state.json",
        }),
    );
    assert_eq!(
        resolve_aipp_preview(&root, 2).expect_err("traversal rejected"),
        "aipp_preview_path_invalid"
    );
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn sandbox_bundle_assets_are_confined_to_the_declared_package_root() {
    let root = std::env::temp_dir().join(format!("aipp-bundle-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("aipp")).expect("bundle root");
    fs::write(root.join("aipp/index.html"), b"<!doctype html>").expect("entrypoint");
    fs::write(root.join("outside.txt"), b"private").expect("outside fixture");
    let raw = include_str!("../../../../../optional_skills/media_discovery/skill.toml")
        .replace("renderer = \"collection_feed_v1\"", "renderer = \"sandbox_bundle_v1\"")
        .replace(
            "data_contract = \"media_collection_v1\"",
            "data_contract = \"capability_bridge_v1\"",
        )
        .replace(
            "icon = \"gallery_vertical_end\"",
            "asset_root = \"aipp\"\nentrypoint = \"aipp/index.html\"\nbridge_capabilities = [\"media_discovery.status\"]\nicon = \"gallery_vertical_end\"",
        );
    let manifest = skill_sdk::PackageManifest::from_toml_str(&raw).expect("sandbox manifest");
    let active = ActiveAippPackage {
        aipp: manifest.aipp.clone().expect("Ai APP declaration"),
        manifest,
        package_root: root.clone(),
    };

    let (path, content_type) =
        resolve_aipp_bundle_asset(&active, "aipp/index.html").expect("safe asset");
    assert!(path.ends_with("aipp/index.html"));
    assert_eq!(content_type, "text/html; charset=utf-8");
    assert_eq!(
        resolve_aipp_bundle_asset(&active, "aipp/../outside.txt")
            .expect_err("traversal must be rejected"),
        "aipp_bundle_path_invalid"
    );
    assert_eq!(
        resolve_aipp_bundle_asset(&active, "outside.txt")
            .expect_err("paths outside asset_root must be rejected"),
        "aipp_bundle_path_invalid"
    );
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn sandbox_bundle_headers_confine_code_and_browser_features() {
    let mut html = axum::response::Response::new(axum::body::Body::empty());
    apply_aipp_bundle_headers(&mut html, "text/html; charset=utf-8");
    assert_eq!(
        html.headers()
            .get(axum::http::header::CONTENT_SECURITY_POLICY)
            .and_then(|value| value.to_str().ok()),
        Some(AIPP_BUNDLE_CSP)
    );
    assert_eq!(
        html.headers()
            .get(axum::http::header::REFERRER_POLICY)
            .and_then(|value| value.to_str().ok()),
        Some("no-referrer")
    );
    assert!(html
        .headers()
        .get(axum::http::HeaderName::from_static("permissions-policy"))
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("microphone=()")));

    let mut script = axum::response::Response::new(axum::body::Body::empty());
    apply_aipp_bundle_headers(&mut script, "text/javascript; charset=utf-8");
    assert!(script
        .headers()
        .get(axum::http::header::CONTENT_SECURITY_POLICY)
        .is_none());
    assert_eq!(
        script
            .headers()
            .get(axum::http::header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
}

use super::*;

fn fixture_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("aipp-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("records")).expect("records directory");
    fs::create_dir_all(root.join("exports/video_covers")).expect("preview directory");
    root
}

fn write_record(root: &Path, sequence: u64, value: Value) {
    fs::write(
        root.join("records").join(format!("{sequence:012}.json")),
        serde_json::to_vec(&value).expect("record JSON"),
    )
    .expect("write record");
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
    assert!(items[0]["source_url"].is_null());
    assert_eq!(
        items[1]["engagement"]["metrics"]["likes"]["display"],
        "1.2万"
    );
    assert_eq!(items[1]["engagement"]["metrics"]["comments"]["value"], 318);
    assert!(items[1]["engagement"]["metrics"].get("unknown").is_none());
    assert_eq!(page["matching_total"], 2);
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
        items[0]["recognized_text"].as_str().map(str::len),
        Some(32_768)
    );
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

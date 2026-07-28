use super::*;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "execution_failed");
    assert_eq!(extra["error_code"], "execution_failed");
    assert_eq!(extra["message_key"], "skill.rss_fetch.execution_failed");
    assert_eq!(extra["retryable"], false);
}

fn successful_test_output() -> Result<SkillOutput, SkillFailure> {
    Ok(SkillOutput {
        text: "ok".to_string(),
        extra: Some(json!({"schema_version": 1})),
    })
}

#[test]
fn persistence_conflict_cannot_return_success_at_response_boundary() {
    let response = response_from_execution(
        "persist-conflict".to_string(),
        successful_test_output(),
        Some(PersistenceError {
            error_kind: "config_persist_failed",
            failure_phase: "config_persistence",
            cause_code: "config_write_conflict".to_string(),
            side_effect_applied: false,
        }),
    );

    assert_eq!(response.status, "error");
    assert!(response.text.is_empty());
    assert_eq!(
        response.error_text.as_deref(),
        Some("config_persist_failed")
    );
    let extra = response.extra.expect("structured persistence failure");
    assert_eq!(extra["error_code"], "config_persist_failed");
    assert_eq!(extra["cause_code"], "config_write_conflict");
    assert_eq!(extra["side_effect_applied"], false);
}

#[test]
fn lock_creation_failure_cannot_return_success_at_response_boundary() {
    let root = persistence_test_path("missing-parent");
    let path = root.join("missing").join("rss.toml");
    let config = make_cfg_with_sources("general", vec!["https://example.com/feed".into()]);
    let cause = save_config_if_unchanged_at(&path, &config, &config)
        .expect_err("missing parent must prevent lock creation");
    let response = response_from_execution(
        "lock-failure".to_string(),
        successful_test_output(),
        Some(PersistenceError {
            error_kind: "config_persist_failed",
            failure_phase: "config_persistence",
            cause_code: cause,
            side_effect_applied: false,
        }),
    );

    assert_eq!(response.status, "error");
    let extra = response.extra.expect("structured persistence failure");
    assert_eq!(extra["cause_code"], "config_lock_open_failed");
}

#[test]
fn atomic_replace_failure_cannot_return_success_at_response_boundary() {
    let path = persistence_test_path("replace-target-directory");
    std::fs::create_dir_all(&path).expect("create directory at config target");
    let config = make_cfg_with_sources("general", vec!["https://example.com/feed".into()]);
    let cause = save_config_atomically_at(&path, &config)
        .expect_err("a config file cannot atomically replace a directory");
    let response = response_from_execution(
        "replace-failure".to_string(),
        successful_test_output(),
        Some(PersistenceError {
            error_kind: "config_persist_failed",
            failure_phase: "config_persistence",
            cause_code: cause,
            side_effect_applied: false,
        }),
    );

    assert_eq!(response.status, "error");
    let extra = response.extra.expect("structured persistence failure");
    assert_eq!(extra["cause_code"], "config_atomic_replace_failed");
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn request_without_skill_storage_descriptor_fails_structurally() {
    let request = Req {
        request_id: "missing-storage".to_string(),
        args: json!({"action": "source_health"}),
        context: None,
    };
    let failure = runtime_from_request(&request).expect_err("storage descriptor is mandatory");

    assert_eq!(failure.error_text, "skill_storage_required");
    assert_eq!(failure.extra["error_code"], "skill_storage_required");
    assert_eq!(failure.extra["failure_phase"], "storage_contract");
    assert_eq!(failure.extra["side_effect_applied"], false);
}

#[test]
fn operator_config_strips_machine_owned_fields() {
    let mut config = make_cfg_with_sources("general", vec!["https://example.com/feed".to_string()]);
    config
        .rss
        .categories
        .get_mut("general")
        .expect("general category")
        .source_entries = Some(vec![SourceStateEntry {
        url: "https://example.com/feed".to_string(),
        failure_count: 2,
        last_error: "timeout".to_string(),
        last_failed_at: "1".to_string(),
    }]);
    config.rss.deprecated = Some(DeprecatedSection {
        sources: vec![DeprecatedEntry {
            url: "https://old.example.com/feed".to_string(),
            category: "general".to_string(),
            reason: "consecutive_fetch_failures".to_string(),
            failure_count: 3,
            last_error: "timeout".to_string(),
            deprecated_at: "2".to_string(),
        }],
    });
    config
        .rss
        .pending_categories
        .insert("robotics".to_string(), PendingCategoryEntry::default());

    let operator = operator_config(&config);

    assert!(operator.rss.deprecated.is_none());
    assert!(operator.rss.pending_categories.is_empty());
    assert!(operator.rss.categories["general"].source_entries.is_none());
    assert_eq!(
        operator.rss.categories["general"].sources.as_deref(),
        Some(&["https://example.com/feed".to_string()][..])
    );
}

#[test]
fn applying_machine_state_reconciles_pending_categories_already_promoted() {
    let mut config = make_cfg_with_sources("general", vec!["https://example.com/feed".into()]);
    let mut state = RssMachineState::default();
    state
        .pending_categories
        .insert("general".to_string(), PendingCategoryEntry::default());
    state
        .pending_categories
        .insert("robotics".to_string(), PendingCategoryEntry::default());

    apply_machine_state(&mut config, &state);

    assert!(!config.rss.pending_categories.contains_key("general"));
    assert!(config.rss.pending_categories.contains_key("robotics"));
    assert_ne!(machine_state_from_config(&config), state);
}

#[test]
fn unknown_category_returns_bounded_machine_replan_contract_without_state_change() {
    let mut cfg = make_cfg_with_sources("general", vec!["https://example.com/feed".to_string()]);
    let before = config_snapshot(&cfg).expect("config snapshot");
    let failure = execute(
        &mut cfg,
        serde_json::json!({
            "action": "latest",
            "category": "not_configured",
            "limit": 3
        }),
    )
    .expect_err("unknown category must fail before network dispatch");

    assert_eq!(failure.extra["error_kind"], "category_not_configured");
    assert_eq!(failure.extra["error_code"], "category_not_configured");
    assert_eq!(failure.extra["retryable"], true);
    assert_eq!(failure.extra["failure_phase"], "pre_dispatch");
    assert_eq!(failure.extra["side_effect_applied"], false);
    assert_eq!(failure.extra["recovery_action"], "replan_arguments");
    assert_eq!(failure.extra["default_category"], "general");
    assert_eq!(failure.extra["available_categories"], json!(["general"]));
    assert!(!config_changed(Some(&before), &cfg));
}

fn make_cfg_with_sources(category: &str, sources: Vec<String>) -> RootConfig {
    let mut categories = HashMap::new();
    let cat = RssCategoryConfig {
        sources: Some(sources),
        ..RssCategoryConfig::default()
    };
    categories.insert(category.to_string(), cat);
    RootConfig {
        rss: RssConfig {
            default_category: Some("general".to_string()),
            default_limit: Some(10),
            timeout_seconds: Some(20),
            deprecate_after_failures: Some(3),
            categories,
            discovery: None,
            deprecated: None,
            pending_categories: BTreeMap::new(),
        },
    }
}

fn make_cfg_legacy_only(
    category: &str,
    primary: Vec<String>,
    secondary: Vec<String>,
    fallback: Vec<String>,
) -> RootConfig {
    let mut categories = HashMap::new();
    let cat = RssCategoryConfig {
        sources: None,
        source_entries: None,
        candidate_entries: None,
        primary,
        secondary,
        fallback,
        output_language: None,
        bilingual_summary: None,
        topic: None,
    };
    categories.insert(category.to_string(), cat);
    RootConfig {
        rss: RssConfig {
            default_category: Some("general".to_string()),
            default_limit: Some(10),
            timeout_seconds: Some(20),
            deprecate_after_failures: None,
            categories,
            discovery: None,
            deprecated: None,
            pending_categories: BTreeMap::new(),
        },
    }
}

#[test]
fn all_sources_for_category_uses_sources_when_present() {
    let cfg = make_cfg_with_sources(
        "crypto",
        vec![
            "https://a.com/feed".to_string(),
            "https://b.com/rss".to_string(),
        ],
    );
    let urls = all_sources_for_category(&cfg, "crypto");
    assert_eq!(urls.len(), 2);
    assert_eq!(urls[0], "https://a.com/feed");
    assert_eq!(urls[1], "https://b.com/rss");
}

#[test]
fn all_sources_for_category_merges_legacy_primary_secondary_fallback_when_no_sources() {
    let cfg = make_cfg_legacy_only(
        "x",
        vec!["https://p1.com".to_string()],
        vec!["https://s1.com".to_string()],
        vec!["https://f1.com".to_string()],
    );
    let urls = all_sources_for_category(&cfg, "x");
    assert_eq!(urls.len(), 3);
    assert_eq!(urls[0], "https://p1.com");
    assert_eq!(urls[1], "https://s1.com");
    assert_eq!(urls[2], "https://f1.com");
}

#[test]
fn all_sources_for_category_empty_for_unknown_category() {
    let cfg = make_cfg_with_sources("known", vec!["https://a.com".to_string()]);
    let urls = all_sources_for_category(&cfg, "unknown");
    assert!(urls.is_empty());
}

#[test]
fn sort_feed_items_by_date_orders_by_date_desc() {
    let mut items = vec![
        FeedItem {
            title: "a".into(),
            link: "l1".into(),
            date: "2020-01-01".into(),
            source: String::new(),
            layer: String::new(),
        },
        FeedItem {
            title: "b".into(),
            link: "l2".into(),
            date: "2022-06-15".into(),
            source: String::new(),
            layer: String::new(),
        },
        FeedItem {
            title: "c".into(),
            link: "l3".into(),
            date: "2021-03-10".into(),
            source: String::new(),
            layer: String::new(),
        },
    ];
    sort_feed_items_by_date(&mut items);
    assert_eq!(items[0].date, "2022-06-15");
    assert_eq!(items[1].date, "2021-03-10");
    assert_eq!(items[2].date, "2020-01-01");
}

#[test]
fn sort_feed_items_by_date_empty_dates_at_end() {
    let mut items = vec![
        FeedItem {
            title: "a".into(),
            link: "l1".into(),
            date: "2022-01-01".into(),
            source: String::new(),
            layer: String::new(),
        },
        FeedItem {
            title: "b".into(),
            link: "l2".into(),
            date: "".into(),
            source: String::new(),
            layer: String::new(),
        },
        FeedItem {
            title: "c".into(),
            link: "l3".into(),
            date: "2021-01-01".into(),
            source: String::new(),
            layer: String::new(),
        },
    ];
    sort_feed_items_by_date(&mut items);
    assert_eq!(items[0].date, "2022-01-01");
    assert_eq!(items[1].date, "2021-01-01");
    assert_eq!(items[2].date, "");
}

#[test]
fn all_sources_fail_returns_err() {
    let mut cfg = make_cfg_with_sources(
        "fail_cat",
        vec!["https://nonexistent.invalid.example/feed".to_string()],
    );
    let args = serde_json::json!({
        "action": "latest",
        "category": "fail_cat",
        "limit": 5,
        "timeout_seconds": 2
    });
    let args = args.as_object().unwrap().clone();
    let result = execute(&mut cfg, serde_json::Value::Object(args));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("no feed items") || err.contains("all") || err.contains("failed"));
}

#[test]
fn removed_legacy_actions_are_rejected_by_machine_code() {
    for action in [
        "fetch_crypto_news",
        "fetch_tech_news",
        "fetch_news",
        "fetch_feed",
    ] {
        let mut cfg = make_cfg_with_sources("general", vec![]);
        let failure = execute(
            &mut cfg,
            serde_json::json!({
                "action": action,
                "url": "https://example.com/feed.xml"
            }),
        )
        .expect_err("removed legacy action must not be normalized");
        assert_eq!(failure.error_text, "rss_fetch.unsupported_action");
        assert_eq!(failure.extra["error_kind"], "invalid_input");
    }
}

#[test]
fn fetch_without_url_returns_err() {
    let mut cfg = make_cfg_with_sources("g", vec!["https://example.com/feed".to_string()]);
    let args = serde_json::json!({ "action": "fetch" });
    let result = execute(&mut cfg, args);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("fetch requires url") || err.contains("feed_urls"),
        "unexpected error: {err}"
    );
}

#[test]
fn fetch_with_empty_feed_urls_returns_err() {
    let mut cfg = make_cfg_with_sources("g", vec![]);
    let args = serde_json::json!({ "action": "fetch", "feed_urls": [] });
    let result = execute(&mut cfg, args);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("feed_urls") || err.contains("fetch requires"),
        "unexpected: {err}"
    );
}

#[test]
fn fetch_rejects_non_http_url() {
    let mut cfg = make_cfg_with_sources("g", vec![]);
    let args = serde_json::json!({ "action": "fetch", "url": "ftp://example.com/x" });
    let result = execute(&mut cfg, args);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("http"));
}

/// Omitted `action` defaults to category `latest`, not direct `fetch`.
#[test]
fn omitted_action_uses_latest_category_mode() {
    let mut cfg = make_cfg_with_sources(
        "omitted_cat",
        vec!["https://nonexistent.invalid.example/feed".to_string()],
    );
    let args = serde_json::json!({
        "category": "omitted_cat",
        "limit": 5,
        "timeout_seconds": 1
    });
    let result = execute(
        &mut cfg,
        serde_json::Value::Object(args.as_object().unwrap().clone()),
    );
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("no feed items") || err.contains("all") || err.contains("failed"),
        "expected layered-news failure, got: {err}"
    );
}

#[test]
fn news_action_same_error_shape_as_latest_for_failed_sources() {
    let mut cfg_latest = make_cfg_with_sources(
        "cat",
        vec!["https://nonexistent.invalid.example/feed".to_string()],
    );
    let latest_args = serde_json::json!({"action": "latest", "category": "cat", "timeout_seconds": 1, "limit": 3});
    let r_latest = execute(
        &mut cfg_latest,
        serde_json::Value::Object(latest_args.as_object().unwrap().clone()),
    );
    let mut cfg_news = make_cfg_with_sources(
        "cat",
        vec!["https://nonexistent.invalid.example/feed".to_string()],
    );
    let news_args =
        serde_json::json!({"action": "news", "category": "cat", "timeout_seconds": 1, "limit": 3});
    let r_news = execute(
        &mut cfg_news,
        serde_json::Value::Object(news_args.as_object().unwrap().clone()),
    );
    assert_eq!(r_latest.is_err(), r_news.is_err());
}

#[test]
fn generated_skill_prompt_body_rss_fetch_md_exists() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let p = repo.join("prompts/layers/generated/skills/rss_fetch.md");
    assert!(
        p.is_file(),
        "generated skill prompt body should exist: {}",
        p.display()
    );
}

/// Needs network: direct `fetch` with a public RSS URL.
#[test]
#[ignore]
fn fetch_with_url_succeeds_direct_feed() {
    let mut cfg = make_cfg_with_sources("unused", vec![]);
    let args = serde_json::json!({
        "action": "fetch",
        "url": "https://www.coindesk.com/arc/outboundfeeds/rss/",
        "limit": 2,
        "timeout_seconds": 15
    });
    let result = execute(
        &mut cfg,
        serde_json::Value::Object(args.as_object().unwrap().clone()),
    );
    assert!(result.is_ok(), "{result:?}");
    let text = result.unwrap().text;
    assert!(!text.trim().is_empty());
}

#[test]
fn deprecated_urls_excluded_from_all_sources() {
    let mut cfg = make_cfg_with_sources(
        "c",
        vec!["https://a.com".to_string(), "https://b.com".to_string()],
    );
    cfg.rss.deprecated = Some(DeprecatedSection {
        sources: vec![DeprecatedEntry {
            url: "https://b.com".to_string(),
            category: "c".to_string(),
            reason: "consecutive_fetch_failures".to_string(),
            failure_count: 3,
            last_error: String::new(),
            deprecated_at: "0".to_string(),
        }],
    });
    let urls = all_sources_for_category(&cfg, "c");
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0], "https://a.com");
}

#[test]
fn single_failure_does_not_deprecate() {
    let mut cfg = make_cfg_with_sources(
        "one",
        vec!["https://nonexistent.invalid.example/feed".to_string()],
    );
    cfg.rss.deprecate_after_failures = Some(3);
    let args = serde_json::json!({"action": "latest", "category": "one", "limit": 5, "timeout_seconds": 1});
    let _ = execute(
        &mut cfg,
        serde_json::Value::Object(args.as_object().unwrap().clone()),
    );
    assert!(cfg
        .rss
        .deprecated
        .as_ref()
        .map(|d| d.sources.is_empty())
        .unwrap_or(true));
}

#[test]
fn old_config_without_deprecated_reads_ok() {
    let cfg = make_cfg_legacy_only("x", vec!["https://p.com".to_string()], vec![], vec![]);
    let urls = all_sources_for_category(&cfg, "x");
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0], "https://p.com");
    assert!(cfg.rss.deprecated.is_none());
}

#[test]
fn already_deprecated_not_duplicated() {
    let mut cfg = make_cfg_with_sources("c", vec!["https://bad.com".to_string()]);
    cfg.rss.deprecate_after_failures = Some(1);
    cfg.rss.deprecated = Some(DeprecatedSection {
        sources: vec![DeprecatedEntry {
            url: "https://bad.com".to_string(),
            category: "c".to_string(),
            reason: "consecutive_fetch_failures".to_string(),
            failure_count: 1,
            last_error: String::new(),
            deprecated_at: "0".to_string(),
        }],
    });
    let urls = all_sources_for_category(&cfg, "c");
    assert!(urls.is_empty());
    let dep_count = cfg
        .rss
        .deprecated
        .as_ref()
        .map(|d| d.sources.len())
        .unwrap_or(0);
    assert_eq!(dep_count, 1);
}

#[test]
fn success_resets_failure_count_in_state() {
    let mut cfg = make_cfg_with_sources("c", vec!["https://a.com".to_string()]);
    cfg.rss.categories.get_mut("c").unwrap().source_entries = Some(vec![SourceStateEntry {
        url: "https://a.com".to_string(),
        failure_count: 1,
        last_error: "timeout".to_string(),
        last_failed_at: "1".to_string(),
    }]);
    let mut state_updates = HashMap::new();
    state_updates.insert(
        "https://a.com".to_string(),
        SourceStateEntry {
            url: "https://a.com".to_string(),
            failure_count: 0,
            last_error: String::new(),
            last_failed_at: String::new(),
        },
    );
    apply_deprecation_and_state(&mut cfg, "c", &state_updates, &[]);
    let entries = cfg
        .rss
        .categories
        .get("c")
        .and_then(|c| c.source_entries.as_ref())
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].failure_count, 0);
}

/// 连续失败达到阈值后，该 source 会从 category 移入 deprecated（需短超时以快速跑完）。
#[test]
fn consecutive_failures_reach_threshold_then_moved_to_deprecated() {
    let mut cfg = make_cfg_with_sources(
        "fail3",
        vec!["https://nonexistent.invalid.example/feed".to_string()],
    );
    cfg.rss.deprecate_after_failures = Some(3);
    for _ in 0..3 {
        let args = serde_json::json!({
            "action": "latest",
            "category": "fail3",
            "limit": 5,
            "timeout_seconds": 1
        });
        let _ = execute(
            &mut cfg,
            serde_json::Value::Object(args.as_object().unwrap().clone()),
        );
    }
    assert!(
        cfg.rss
            .deprecated
            .as_ref()
            .map(|d| d.sources.len())
            .unwrap_or(0)
            >= 1,
        "expected url to be in deprecated after 3 consecutive failures"
    );
    let urls = all_sources_for_category(&cfg, "fail3");
    assert!(
        urls.is_empty(),
        "deprecated source should no longer be in active sources"
    );
}

/// 需要网络：至少一个源可访问时，应返回 ok 且 text 含 sources_ok>=1。
#[test]
#[ignore]
fn partial_fail_returns_ok_when_at_least_one_source_succeeds() {
    let mut cfg = make_cfg_with_sources(
        "mixed",
        vec![
            "https://nonexistent.invalid.example/feed".to_string(),
            "https://www.coindesk.com/arc/outboundfeeds/rss/".to_string(),
        ],
    );
    let args = serde_json::json!({
        "action": "latest",
        "category": "mixed",
        "limit": 3,
        "timeout_seconds": 10
    });
    let args = args.as_object().unwrap().clone();
    let result = execute(&mut cfg, serde_json::Value::Object(args));
    assert!(
        result.is_ok(),
        "expected ok when at least one source succeeds: {:?}",
        result
    );
    let text = result.unwrap().text;
    assert!(text.contains("sources_ok="));
    assert!(text.contains("items="));
}

/// 需要网络：多源合并后按 limit 截断，且去重。
#[test]
#[ignore]
fn limit_applied_after_merge_and_dedupe() {
    let mut cfg = make_cfg_with_sources(
        "multi",
        vec![
            "https://www.coindesk.com/arc/outboundfeeds/rss/".to_string(),
            "https://cointelegraph.com/rss".to_string(),
        ],
    );
    let args = serde_json::json!({
        "action": "latest",
        "category": "multi",
        "limit": 2,
        "timeout_seconds": 15
    });
    let args = args.as_object().unwrap().clone();
    let result = execute(&mut cfg, serde_json::Value::Object(args));
    assert!(result.is_ok(), "{:?}", result);
    let text = result.unwrap().text;
    assert!(text.starts_with("sources_ok="));
    assert!(text.contains("items=2") || text.contains("items=1"));
}

#[test]
fn feed_item_extra_exposes_structured_news_fields() {
    let item = FeedItem {
        title: "Example market update".to_string(),
        link: "https://example.com/news/1".to_string(),
        date: "2026-06-02T10:00:00Z".to_string(),
        source: "https://example.com/feed.xml".to_string(),
        layer: "feed".to_string(),
    };

    let extra = feed_item_extra(&item, "macro_market");

    assert_eq!(
        extra.get("title").and_then(Value::as_str),
        Some("Example market update")
    );
    assert_eq!(
        extra.get("link").and_then(Value::as_str),
        Some("https://example.com/news/1")
    );
    assert_eq!(
        extra.get("source_host").and_then(Value::as_str),
        Some("example.com")
    );
    assert_eq!(extra.get("layer").and_then(Value::as_str), Some("feed"));
    assert_eq!(
        extra.get("topic").and_then(Value::as_str),
        Some("macro_market")
    );
}

#[test]
fn feed_item_topic_uses_machine_token_not_title_keywords() {
    let item = FeedItem {
        title: "SEC hack funding launch macro jobs".to_string(),
        link: "https://example.com/news/1".to_string(),
        date: "2026-06-02T10:00:00Z".to_string(),
        source: "https://example.com/feed.xml".to_string(),
        layer: "feed".to_string(),
    };

    let extra = feed_item_extra(&item, "tech_ecosystem");

    assert_eq!(
        extra.get("topic").and_then(Value::as_str),
        Some("tech_ecosystem")
    );
}

#[test]
fn news_topic_token_prefers_machine_config_and_rejects_sentence_values() {
    let mut cfg = make_cfg_with_sources("tech", vec!["https://example.com/feed.xml".to_string()]);
    cfg.rss.categories.get_mut("tech").expect("category").topic =
        Some("Tech_Ecosystem".to_string());
    let args = serde_json::json!({"topic": "please classify this title"});
    let args = args.as_object().unwrap().clone();

    assert_eq!(
        news_topic_token(Some(&cfg), &args, Some("tech")),
        "tech_ecosystem"
    );
}

fn persistence_test_path(label: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "rustclaw-rss-persistence-{}-{}-{nonce}",
        std::process::id(),
        label
    ));
    std::fs::create_dir_all(&directory).expect("create persistence test directory");
    directory.join("rss.toml")
}

#[test]
fn config_loading_reports_missing_and_malformed_files() {
    let missing = persistence_test_path("missing-config");
    assert_eq!(
        load_root_config_at(&missing).expect_err("missing config must not use defaults"),
        "config_not_found"
    );

    let malformed = persistence_test_path("malformed-config");
    std::fs::write(&malformed, "[rss\ndefault_limit = 10").expect("write malformed config fixture");
    assert_eq!(
        load_root_config_at(&malformed).expect_err("malformed config must not use defaults"),
        "config_parse_failed"
    );

    std::fs::remove_dir_all(missing.parent().unwrap()).expect("remove missing-config directory");
    std::fs::remove_dir_all(malformed.parent().unwrap())
        .expect("remove malformed-config directory");
}

#[test]
fn config_persistence_replaces_atomically_and_remains_parseable() {
    let path = persistence_test_path("atomic");
    let mut original = RootConfig::default();
    original.rss.default_category = Some("general".to_string());
    save_config_atomically_at(&path, &original).expect("write original config");
    let expected = original.clone();

    let mut updated = original.clone();
    updated.rss.default_limit = Some(12);
    save_config_if_unchanged_at(&path, &updated, &expected).expect("atomic update");

    let raw = std::fs::read_to_string(&path).expect("read persisted config");
    let parsed: RootConfig = toml::from_str(&raw).expect("parse persisted config");
    assert_eq!(parsed.rss.default_limit, Some(12));
    let leaked_temporary = std::fs::read_dir(path.parent().unwrap())
        .expect("list persistence directory")
        .flatten()
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".rss.toml.tmp.")
        });
    assert!(!leaked_temporary);

    std::fs::remove_dir_all(path.parent().unwrap()).expect("remove test directory");
}

#[test]
fn config_persistence_rejects_stale_snapshot_without_overwrite() {
    let path = persistence_test_path("conflict");
    let mut original = RootConfig::default();
    original.rss.default_limit = Some(10);
    save_config_atomically_at(&path, &original).expect("write original config");
    let stale_config = original.clone();

    let mut external = original.clone();
    external.rss.default_limit = Some(20);
    save_config_atomically_at(&path, &external).expect("simulate concurrent update");

    let mut stale_update = original;
    stale_update.rss.default_limit = Some(30);
    assert_eq!(
        save_config_if_unchanged_at(&path, &stale_update, &stale_config).unwrap_err(),
        "config_write_conflict"
    );
    let persisted: RootConfig =
        toml::from_str(&std::fs::read_to_string(&path).expect("read config"))
            .expect("parse config");
    assert_eq!(persisted.rss.default_limit, Some(20));

    std::fs::remove_dir_all(path.parent().unwrap()).expect("remove test directory");
}

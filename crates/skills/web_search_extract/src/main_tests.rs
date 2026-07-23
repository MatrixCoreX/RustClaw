use super::*;

fn input_for_test() -> SearchInput {
    SearchInput {
        request_id: "test".to_string(),
        action: "search".to_string(),
        query: "rust async tutorial".to_string(),
        top_k: 3,
        cursor: 0,
        lang: None,
        time_range: None,
        domains_allow: Vec::new(),
        domains_deny: Vec::new(),
        backend: Some("duckduckgo_html".to_string()),
        include_snippet: true,
    }
}

#[test]
fn response_extra_exposes_empty_candidates_for_search_evidence() {
    let payload = json!({
        "status": "ok",
        "backend": "duckduckgo_html",
        "items": [],
        "extract_urls": [],
        "summary": "No results found",
        "citations": []
    });

    let extra = build_response_extra(&input_for_test(), &payload);

    assert_eq!(extra.get("action").and_then(Value::as_str), Some("search"));
    assert_eq!(
        extra
            .get("candidates")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        extra
            .pointer("/field_value/result_count")
            .and_then(Value::as_u64),
        Some(0)
    );
}

#[test]
fn response_extra_exposes_structured_candidates_for_search_evidence() {
    let payload = json!({
        "status": "ok",
        "backend": "duckduckgo_html",
        "items": [
            {
                "title": "Rust Async",
                "url": "https://example.com/rust-async",
                "rank": 1,
                "source": "example.com",
                "snippet": "Tutorial"
            }
        ],
        "extract_urls": ["https://example.com/rust-async"],
        "summary": "result_count=1 backend=duckduckgo_html",
        "citations": ["https://example.com/rust-async"]
    });

    let extra = build_response_extra(&input_for_test(), &payload);

    assert_eq!(
        extra
            .pointer("/field_value/result_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        extra.pointer("/items/0/url").and_then(Value::as_str),
        Some("https://example.com/rust-async")
    );
    assert_eq!(
        extra
            .pointer("/candidates/0/source")
            .and_then(Value::as_str),
        Some("example.com")
    );
    assert_eq!(
        extra.pointer("/extract_urls/0").and_then(Value::as_str),
        Some("https://example.com/rust-async")
    );
}

#[test]
fn parse_bing_html_results_extracts_title_url_and_snippet() {
    let html = r#"
    <ol id="b_results">
      <li class="b_algo">
        <h2><a href="https://rust-lang.github.io/async-book/">Asynchronous Programming in Rust</a></h2>
        <div class="b_caption"><p>The Async Book explains futures, async, and await in Rust.</p></div>
      </li>
      <li class="b_algo">
        <h2><a href="https://tokio.rs/tokio/tutorial">Tokio Tutorial</a></h2>
        <div class="b_caption"><p>Learn to build asynchronous applications with Tokio.</p></div>
      </li>
    </ol>
    "#;

    let items = parse_bing_html_results(html, 3);

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].title, "Asynchronous Programming in Rust");
    assert_eq!(items[0].url, "https://rust-lang.github.io/async-book/");
    assert_eq!(
        items[0].snippet.as_deref(),
        Some("The Async Book explains futures, async, and await in Rust.")
    );
    assert_eq!(items[1].source, "bing");
}

#[test]
fn duckduckgo_parser_accepts_multi_class_result_body_and_redirects() {
    let input = SearchInput {
        query: "RustClaw GitHub".to_string(),
        ..input_for_test()
    };
    let html = r#"
    <div class="result results_links results_links_deep web-result ">
      <div class="links_main links_deep result__body">
        <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fgithub.com%2FAdaimade%2FRustClaw&amp;rut=abc">RustClaw - GitHub</a>
        <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fgithub.com%2FAdaimade%2FRustClaw&amp;rut=abc"><b>RustClaw</b> repo.</a>
      </div>
    </div>
    "#;

    let mut items = parse_duckduckgo_html_results(html, &input);
    normalize_and_filter(&mut items, &input);

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "RustClaw - GitHub");
    assert_eq!(items[0].url, "https://github.com/Adaimade/RustClaw");
    assert_eq!(items[0].source, "github.com");
    assert_eq!(items[0].snippet.as_deref(), Some("RustClaw repo."));
}

#[test]
fn missing_backend_defaults_to_duckduckgo_html() {
    let backend = resolve_backend(None).expect("default backend");

    assert!(matches!(backend, Backend::DuckDuckGoHtml));
}

#[test]
fn parse_input_projects_site_operator_to_domain_filter() {
    let input = parse_input(&json!({
        "request_id": "site",
        "args": {
            "action": "search_extract",
            "query": "site:docs.rs tokio task"
        }
    }))
    .expect("parse site query");

    assert_eq!(input.domains_allow, vec!["docs.rs"]);
}

#[test]
fn query_without_site_operators_preserves_plain_terms() {
    assert_eq!(
        query_without_site_operators("site:docs.rs tokio task"),
        "tokio task"
    );
    assert_eq!(
        query_without_site_operators("RustClaw site:github.com"),
        "RustClaw"
    );
}

#[test]
fn domain_filter_allows_matching_fallback_source_only() {
    let input = SearchInput {
        query: "site:docs.rs tokio task".to_string(),
        domains_allow: vec!["docs.rs".to_string()],
        ..input_for_test()
    };

    assert!(domain_allowed_by_filter(&input, "docs.rs"));
    assert!(!domain_allowed_by_filter(&input, "github.com"));
}

#[test]
fn docs_rs_fallback_requires_explicit_domain_scope() {
    let generic = input_for_test();
    let docs_scoped = SearchInput {
        domains_allow: vec!["docs.rs".to_string()],
        ..input_for_test()
    };

    assert!(!domain_explicitly_allowed(&generic, "docs.rs"));
    assert!(domain_explicitly_allowed(&docs_scoped, "docs.rs"));
}

#[test]
fn parse_docs_rs_results_extracts_release_rows() {
    let html = r#"
    <ul>
      <li><a href="/tokio/latest/tokio/task/" class="release"><div class="pure-g"><div class="name">tokio::task</div><div class="description">Task tools for Tokio</div></div></a></li>
      <li><a href="/tokio-tasks/latest/tokio_tasks/" class="release"><div class="pure-g"><div class="name">tokio-tasks-0.5.4</div><div class="description">Task management for tokio</div></div></a></li>
    </ul>
    "#;

    let items = parse_docs_rs_results(html, 3);

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].title, "tokio::task");
    assert_eq!(items[0].url, "https://docs.rs/tokio/latest/tokio/task/");
    assert_eq!(items[0].source, "docs.rs");
    assert_eq!(items[0].snippet.as_deref(), Some("Task tools for Tokio"));
}

#[test]
fn strict_input_rejects_wrong_types_and_out_of_range_pages() {
    let wrong_query = parse_input(&json!({
        "request_id": "bad-query",
        "args": {"action": "search", "query": 42}
    }))
    .expect_err("query type must be checked");
    assert_eq!(wrong_query.code, "INVALID_INPUT");

    let excessive_limit = parse_input(&json!({
        "request_id": "bad-limit",
        "args": {"action": "search", "query": "rust", "top_k": 21}
    }))
    .expect_err("limit must not be silently clamped");
    assert_eq!(excessive_limit.code, "INVALID_INPUT");

    let excessive_cursor = parse_input(&json!({
        "request_id": "bad-cursor",
        "args": {"action": "search", "query": "rust", "cursor": 101}
    }))
    .expect_err("cursor must remain bounded");
    assert_eq!(excessive_cursor.code, "INVALID_INPUT");
}

#[test]
fn error_response_uses_outer_skill_error_contract() {
    let response = error_response(
        "request-7",
        &SearchError::new("INVALID_ACTION", "unsupported action"),
    );

    assert_eq!(
        response.get("request_id").and_then(Value::as_str),
        Some("request-7")
    );
    assert_eq!(
        response.get("status").and_then(Value::as_str),
        Some("error")
    );
    assert_eq!(
        response
            .pointer("/extra/error_code")
            .and_then(Value::as_str),
        Some("INVALID_ACTION")
    );
    assert_eq!(
        response
            .pointer("/extra/retryable")
            .and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn candidate_pages_are_bounded_ranked_and_snapshot_identified() {
    let input = SearchInput {
        top_k: 1,
        cursor: 1,
        ..input_for_test()
    };
    let items = vec![
        SearchItem {
            title: "One".to_string(),
            url: "https://one.example/".to_string(),
            snippet: None,
            source: "one.example".to_string(),
            rank: 1,
        },
        SearchItem {
            title: "Two".to_string(),
            url: "https://two.example/".to_string(),
            snippet: Some("second".to_string()),
            source: "two.example".to_string(),
            rank: 2,
        },
        SearchItem {
            title: "Three".to_string(),
            url: "https://three.example/".to_string(),
            snippet: None,
            source: "three.example".to_string(),
            rank: 3,
        },
    ];

    let payload = build_search_payload(&input, "fixture", items);

    assert_eq!(
        payload.pointer("/items/0/title").and_then(Value::as_str),
        Some("Two")
    );
    assert_eq!(
        payload.pointer("/items/0/rank").and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        payload.pointer("/page/next_cursor").and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        payload
            .pointer("/page/previous_cursor")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        payload.pointer("/page/stability").and_then(Value::as_str),
        Some("backend_best_effort")
    );
    assert!(payload
        .get("snapshot_id")
        .and_then(Value::as_str)
        .is_some_and(|value| value.starts_with("sha256:")));
}

#[test]
fn candidate_urls_reject_non_http_credentials_and_private_targets() {
    for blocked in [
        "ftp://example.com/file",
        "https://user:secret@example.com/",
        "http://127.0.0.1/private",
        "http://169.254.169.254/latest/meta-data/",
        "http://service.local/",
        "http://service.internal/",
    ] {
        assert_eq!(normalize_url(blocked), None, "{blocked} must be rejected");
    }
    assert_eq!(
        normalize_url("https://EXAMPLE.com:443/path?utm_source=test&x=1#fragment").as_deref(),
        Some("https://example.com/path?x=1")
    );
}

#[test]
fn candidate_metadata_is_bounded_before_model_exposure() {
    let input = input_for_test();
    let mut items = vec![SearchItem {
        title: "t".repeat(MAX_TITLE_CHARS + 20),
        url: "https://example.com/item".to_string(),
        snippet: Some("s".repeat(MAX_SNIPPET_CHARS + 20)),
        source: String::new(),
        rank: 1,
    }];

    normalize_and_filter(&mut items, &input);

    assert_eq!(items[0].title.chars().count(), MAX_TITLE_CHARS);
    assert_eq!(
        items[0]
            .snippet
            .as_deref()
            .expect("snippet")
            .chars()
            .count(),
        MAX_SNIPPET_CHARS
    );
}

#[test]
fn backend_body_limit_fails_without_returning_partial_content() {
    let mut exact = std::io::Cursor::new(vec![b'x'; MAX_BACKEND_RESPONSE_BYTES]);
    assert_eq!(
        read_bounded_backend_body(&mut exact, Some(MAX_BACKEND_RESPONSE_BYTES))
            .expect("exact body limit")
            .len(),
        MAX_BACKEND_RESPONSE_BYTES
    );

    let mut oversized = std::io::Cursor::new(vec![b'x'; MAX_BACKEND_RESPONSE_BYTES + 1]);
    let error = read_bounded_backend_body(&mut oversized, None)
        .expect_err("oversized body must fail loudly");
    assert!(error.to_string().contains("byte limit"));
}

#[test]
fn response_extra_marks_search_metadata_as_untrusted() {
    let payload = build_search_payload(
        &input_for_test(),
        "fixture",
        vec![SearchItem {
            title: "Candidate".to_string(),
            url: "https://example.com/".to_string(),
            snippet: Some("metadata".to_string()),
            source: "example.com".to_string(),
            rank: 1,
        }],
    );
    let extra = build_response_extra(&input_for_test(), &payload);

    assert_eq!(
        extra
            .pointer("/trust/classification")
            .and_then(Value::as_str),
        Some("untrusted_search_metadata")
    );
    assert_eq!(
        extra
            .pointer("/trust/instructions_executable")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        extra.pointer("/source_refs/0/kind").and_then(Value::as_str),
        Some("search_candidate")
    );
}

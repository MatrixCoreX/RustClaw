use super::*;

fn input_for_test() -> SearchInput {
    SearchInput {
        request_id: "test".to_string(),
        action: "search".to_string(),
        query: "rust async tutorial".to_string(),
        top_k: 3,
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
    }));

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

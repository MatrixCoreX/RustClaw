use super::*;
use serde_json::json;

#[test]
fn test_args_non_object_returns_error() {
    let req = Request {
        request_id: "test-1".to_string(),
        args: json!("not an object"),
        _context: None,
        _user_id: 1,
        _chat_id: 1,
    };

    let resp = handle(req);
    assert_eq!(resp.status, "error");
    assert!(resp.error_text.is_some());
    assert!(resp.error_text.unwrap().contains("args must be object"));
}

#[test]
fn test_parse_open_extract_args_valid() {
    let obj = json!({
        "action": "open_extract",
        "url": "https://example.com",
        "max_pages": 5,
        "wait_until": "load"
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_open_extract_args(&obj);
    assert!(args.is_ok());
    let args = args.unwrap();
    assert_eq!(args.action, "open_extract");
    assert_eq!(args.url, Some("https://example.com".to_string()));
    assert_eq!(args.max_pages, Some(5));
    assert_eq!(args.wait_until, Some("load".to_string()));
}

#[test]
fn test_parse_open_extract_args_missing_url() {
    let obj = json!({
        "action": "open_extract"
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_open_extract_args(&obj);
    assert!(args.is_err());
    assert!(args
        .unwrap_err()
        .contains("at least one of url or urls is required"));
}

#[test]
fn test_parse_search_page_args_valid() {
    let obj = json!({
        "action": "search_page",
        "query": "test query",
        "engine": "google",
        "top_k": 10
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_search_page_args(&obj);
    assert!(args.is_ok());
    let args = args.unwrap();
    assert_eq!(args.action, "search_page");
    assert_eq!(args.query, "test query");
    assert_eq!(args.engine, Some("google".to_string()));
    assert_eq!(args.top_k, Some(10));
}

#[test]
fn test_parse_search_page_args_missing_query() {
    let obj = json!({
        "action": "search_page"
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_search_page_args(&obj);
    assert!(args.is_err());
    assert!(args.unwrap_err().contains("query is required"));
}

#[test]
fn test_parse_search_extract_args_valid() {
    let obj = json!({
        "action": "search_extract",
        "query": "test query",
        "engine": "google",
        "top_k": 10,
        "extract_top_n": 3,
        "wait_until": "networkidle"
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_search_extract_args(&obj);
    assert!(args.is_ok());
    let args = args.unwrap();
    assert_eq!(args.action, "search_extract");
    assert_eq!(args.query, "test query");
    assert_eq!(args.engine, Some("google".to_string()));
    assert_eq!(args.top_k, Some(10));
    assert_eq!(args.extract_top_n, Some(3));
    assert_eq!(args.wait_until, Some("networkidle".to_string()));
}

#[test]
fn test_parse_open_extract_args_max_pages_zero() {
    let obj = json!({
        "action": "open_extract",
        "url": "https://example.com",
        "max_pages": 0
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_open_extract_args(&obj);
    assert!(args.is_err());
    assert!(args
        .unwrap_err()
        .contains("max_pages must be between 1 and 10"));
}

#[test]
fn test_parse_open_extract_args_max_pages_too_large() {
    let obj = json!({
        "action": "open_extract",
        "url": "https://example.com",
        "max_pages": 11
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_open_extract_args(&obj);
    assert!(args.is_err());
    assert!(args
        .unwrap_err()
        .contains("max_pages must be between 1 and 10"));
}

#[test]
fn test_parse_open_extract_args_max_pages_valid_range() {
    for val in [1, 5, 10] {
        let obj = json!({
            "action": "open_extract",
            "url": "https://example.com",
            "max_pages": val
        })
        .as_object()
        .unwrap()
        .clone();

        let args = parse_open_extract_args(&obj);
        assert!(args.is_ok(), "max_pages={} should be valid", val);
        assert_eq!(args.unwrap().max_pages, Some(val as u32));
    }
}

#[test]
fn test_parse_search_page_args_top_k_zero() {
    let obj = json!({
        "action": "search_page",
        "query": "test",
        "top_k": 0
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_search_page_args(&obj);
    assert!(args.is_err());
    assert!(args.unwrap_err().contains("top_k must be between 1 and 20"));
}

#[test]
fn test_parse_search_page_args_top_k_too_large() {
    let obj = json!({
        "action": "search_page",
        "query": "test",
        "top_k": 21
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_search_page_args(&obj);
    assert!(args.is_err());
    assert!(args.unwrap_err().contains("top_k must be between 1 and 20"));
}

#[test]
fn test_parse_search_page_args_top_k_valid_range() {
    for val in [1, 10, 20] {
        let obj = json!({
            "action": "search_page",
            "query": "test",
            "top_k": val
        })
        .as_object()
        .unwrap()
        .clone();

        let args = parse_search_page_args(&obj);
        assert!(args.is_ok(), "top_k={} should be valid", val);
        assert_eq!(args.unwrap().top_k, Some(val as u32));
    }
}

#[test]
fn test_parse_search_extract_args_top_k_zero() {
    let obj = json!({
        "action": "search_extract",
        "query": "test",
        "top_k": 0
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_search_extract_args(&obj);
    assert!(args.is_err());
    assert!(args.unwrap_err().contains("top_k must be between 1 and 20"));
}

#[test]
fn test_parse_search_extract_args_top_k_too_large() {
    let obj = json!({
        "action": "search_extract",
        "query": "test",
        "top_k": 21
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_search_extract_args(&obj);
    assert!(args.is_err());
    assert!(args.unwrap_err().contains("top_k must be between 1 and 20"));
}

#[test]
fn test_parse_search_extract_args_extract_top_n_zero() {
    let obj = json!({
        "action": "search_extract",
        "query": "test",
        "extract_top_n": 0
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_search_extract_args(&obj);
    assert!(args.is_err());
    assert!(args
        .unwrap_err()
        .contains("extract_top_n must be between 1 and 10"));
}

#[test]
fn test_parse_search_extract_args_extract_top_n_too_large() {
    let obj = json!({
        "action": "search_extract",
        "query": "test",
        "extract_top_n": 11
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_search_extract_args(&obj);
    assert!(args.is_err());
    assert!(args
        .unwrap_err()
        .contains("extract_top_n must be between 1 and 10"));
}

#[test]
fn test_parse_search_extract_args_extract_top_n_valid_range() {
    for val in [1, 5, 10] {
        let obj = json!({
            "action": "search_extract",
            "query": "test",
            "extract_top_n": val
        })
        .as_object()
        .unwrap()
        .clone();

        let args = parse_search_extract_args(&obj);
        assert!(args.is_ok(), "extract_top_n={} should be valid", val);
        assert_eq!(args.unwrap().extract_top_n, Some(val as u32));
    }
}

#[test]
fn test_parse_open_extract_args_new_options() {
    let obj = json!({
        "action": "open_extract",
        "url": "https://example.com",
        "content_mode": "raw",
        "max_text_chars": 4096,
        "min_content_chars": 120,
        "fail_fast": true,
        "wait_map_path": "configs/browser_web_wait_map.json"
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_open_extract_args(&obj).unwrap();
    assert_eq!(args.content_mode, Some("raw".to_string()));
    assert_eq!(args.max_text_chars, Some(4096));
    assert_eq!(args.min_content_chars, Some(120));
    assert_eq!(args.fail_fast, Some(true));
    assert_eq!(
        args.wait_map_path,
        Some("configs/browser_web_wait_map.json".to_string())
    );
}

#[test]
fn test_parse_open_extract_args_invalid_content_mode() {
    let obj = json!({
        "action": "open_extract",
        "url": "https://example.com",
        "content_mode": "debug"
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_open_extract_args(&obj);
    assert!(args.is_err());
    assert!(args.unwrap_err().contains("content_mode must be one of"));
}

#[test]
fn test_parse_search_page_args_region_lang() {
    let obj = json!({
        "action": "search_page",
        "query": "test",
        "region": "us",
        "lang": "en"
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_search_page_args(&obj).unwrap();
    assert_eq!(args.region, Some("us".to_string()));
    assert_eq!(args.lang, Some("en".to_string()));
}

#[test]
fn test_parse_search_extract_args_summarize_and_mode() {
    let obj = json!({
        "action": "search_extract",
        "query": "test",
        "summarize": false,
        "content_mode": "raw",
        "max_text_chars": 1600,
        "min_content_chars": 90,
        "fail_fast": true
    })
    .as_object()
    .unwrap()
    .clone();

    let args = parse_search_extract_args(&obj).unwrap();
    assert_eq!(args.summarize, Some(false));
    assert_eq!(args.content_mode, Some("raw".to_string()));
    assert_eq!(args.max_text_chars, Some(1600));
    assert_eq!(args.min_content_chars, Some(90));
    assert_eq!(args.fail_fast, Some(true));
}

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use reqwest::blocking::{Client, Response};
use reqwest::header::CONTENT_LENGTH;
use reqwest::redirect::Policy;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::env;
use std::io::{self, BufRead, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const SKILL_NAME: &str = "web_search_extract";
const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 20;
const MAX_CURSOR: usize = 100;
const MAX_CANDIDATE_WINDOW: usize = 101;
const MAX_BACKEND_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_QUERY_CHARS: usize = 2_000;
const MAX_OPTION_CHARS: usize = 128;
const MAX_DOMAIN_FILTERS: usize = 32;
const MAX_TITLE_CHARS: usize = 300;
const MAX_SNIPPET_CHARS: usize = 1_000;
const MAX_URL_BYTES: usize = 4_096;

#[derive(Clone, Debug)]
struct SearchInput {
    request_id: String,
    action: String,
    query: String,
    top_k: usize,
    cursor: usize,
    lang: Option<String>,
    time_range: Option<String>,
    domains_allow: Vec<String>,
    domains_deny: Vec<String>,
    backend: Option<String>,
    include_snippet: bool,
}

#[derive(Debug)]
struct SearchError {
    code: &'static str,
    detail: String,
    retryable: bool,
}

impl SearchError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
            retryable: false,
        }
    }

    fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }
}

#[derive(Clone, Debug, Serialize)]
struct SearchItem {
    title: String,
    url: String,
    snippet: Option<String>,
    source: String,
    rank: usize,
}

#[derive(Clone, Debug)]
enum Backend {
    SerpApi,
    DuckDuckGoHtml,
    BingHtml,
}

impl Backend {
    fn from_name(v: &str) -> Option<Self> {
        match v.to_ascii_lowercase().as_str() {
            "serpapi" => Some(Self::SerpApi),
            "duckduckgo_html" | "duckduckgo" | "ddg" => Some(Self::DuckDuckGoHtml),
            "bing_html" | "bing" => Some(Self::BingHtml),
            _ => None,
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            Self::SerpApi => "serpapi",
            Self::DuckDuckGoHtml => "duckduckgo_html",
            Self::BingHtml => "bing_html",
        }
    }
}

fn main() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let out = match serde_json::from_str::<Value>(&line) {
            Ok(request) => {
                let request_id = request
                    .get("request_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                match parse_input(&request) {
                    Ok(input) => match handle(&input) {
                        Ok(text_payload) => json!({
                            "request_id": input.request_id,
                            "status": "ok",
                            "text": serde_json::to_string(&text_payload)?,
                            "error_text": Value::Null,
                            "extra": build_response_extra(&input, &text_payload)
                        }),
                        Err(error) => error_response(&input.request_id, &error),
                    },
                    Err(error) => error_response(request_id, &error),
                }
            }
            Err(error) => error_response(
                "unknown",
                &SearchError::new("INVALID_INPUT", error.to_string()),
            ),
        };
        writeln!(stdout, "{}", serde_json::to_string(&out)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn error_response(request_id: &str, error: &SearchError) -> Value {
    json!({
        "request_id": request_id,
        "status": "error",
        "text": "",
        "error_text": error.detail,
        "extra": {
            "schema_version": 1,
            "source_skill": SKILL_NAME,
            "status": "error",
            "error_kind": error.code,
            "error_code": error.code,
            "message_key": format!("skill.{}.{}", SKILL_NAME, error.code.to_ascii_lowercase()),
            "retryable": error.retryable,
            "items": [],
            "candidates": [],
            "extract_urls": [],
            "citations": []
        }
    })
}

fn build_response_extra(input: &SearchInput, text_payload: &Value) -> Value {
    let items = text_payload
        .get("items")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let extract_urls = text_payload
        .get("extract_urls")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let citations = text_payload
        .get("citations")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let status = text_payload
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let result_count = items.as_array().map(Vec::len).unwrap_or(0);
    let page = text_payload
        .get("page")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let snapshot_id = text_payload
        .get("snapshot_id")
        .cloned()
        .unwrap_or(Value::Null);
    let source_refs = items
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            json!({
                "url": item.get("url").cloned().unwrap_or(Value::Null),
                "title": item.get("title").cloned().unwrap_or(Value::Null),
                "rank": item.get("rank").cloned().unwrap_or(Value::Null),
                "source": item.get("source").cloned().unwrap_or(Value::Null),
                "kind": "search_candidate"
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": 1,
        "action": input.action,
        "query": input.query,
        "top_k": input.top_k,
        "cursor": input.cursor,
        "backend": text_payload.get("backend").cloned().unwrap_or(Value::Null),
        "backend_connected": status == "ok",
        "status": status,
        "error_code": text_payload.get("error_code").cloned().unwrap_or(Value::Null),
        "field_value": {
            "status": status,
            "result_count": result_count,
            "summary": text_payload.get("summary").cloned().unwrap_or(Value::Null),
        },
        "items": items.clone(),
        "candidates": items,
        "extract_urls": extract_urls,
        "citations": citations,
        "source_refs": source_refs,
        "page": page,
        "snapshot_id": snapshot_id,
        "truncated": text_payload
            .pointer("/page/has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "trust": {
            "classification": "untrusted_search_metadata",
            "instructions_executable": false
        },
        "provenance": {
            "source": "web_search_backend",
            "backend": text_payload.get("backend").cloned().unwrap_or(Value::Null),
            "observed_at": unix_ts()
        }
    })
}

fn parse_input(req: &Value) -> std::result::Result<SearchInput, SearchError> {
    let args = req.get("args").unwrap_or(req);
    let args = args
        .as_object()
        .ok_or_else(|| SearchError::new("INVALID_INPUT", "args must be object"))?;
    let request_id = req
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let action = args
        .get("action")
        .or_else(|| req.get("action"))
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| SearchError::new("INVALID_INPUT", "action must be string"))
        })
        .transpose()?
        .unwrap_or("search")
        .to_string();
    let query = args
        .get("query")
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| SearchError::new("INVALID_INPUT", "query must be string"))
        })
        .transpose()?
        .unwrap_or_default()
        .trim()
        .to_string();
    if query.is_empty() {
        return Err(SearchError::new("INVALID_INPUT", "query is required"));
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(SearchError::new(
            "INVALID_INPUT",
            "query exceeds supported length",
        ));
    }
    if !matches!(action.as_str(), "search" | "search_extract") {
        return Err(SearchError::new("INVALID_ACTION", "unsupported action"));
    }
    let top_k = bounded_usize(
        args.get("top_k").or_else(|| args.get("topK")),
        DEFAULT_LIMIT,
        1,
        MAX_LIMIT,
        "top_k",
    )?;
    let cursor = bounded_usize(args.get("cursor"), 0, 0, MAX_CURSOR, "cursor")?;
    let lang = optional_string(args.get("lang"), "lang")?;
    let time_range = optional_string(args.get("time_range"), "time_range")?;
    let mut domains_allow = get_string_array(args.get("domains_allow"), "domains_allow")?;
    if domains_allow.is_empty() {
        domains_allow = site_domains_from_query(&query);
    }
    let domains_deny = get_string_array(args.get("domains_deny"), "domains_deny")?;
    let backend = args
        .get("backend")
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| SearchError::new("INVALID_INPUT", "backend must be string"))
        })
        .transpose()?
        .or_else(|| env::var("WEB_SEARCH_BACKEND").ok());
    if backend
        .as_deref()
        .is_some_and(|value| value.chars().count() > MAX_OPTION_CHARS)
    {
        return Err(SearchError::new(
            "INVALID_INPUT",
            "backend exceeds supported length",
        ));
    }
    let include_snippet = args
        .get("include_snippet")
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| SearchError::new("INVALID_INPUT", "include_snippet must be boolean"))
        })
        .transpose()?
        .unwrap_or(true);

    Ok(SearchInput {
        request_id,
        action,
        query,
        top_k,
        cursor,
        lang,
        time_range,
        domains_allow,
        domains_deny,
        backend,
        include_snippet,
    })
}

fn bounded_usize(
    value: Option<&Value>,
    default: usize,
    minimum: usize,
    maximum: usize,
    field: &str,
) -> std::result::Result<usize, SearchError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let value = value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| SearchError::new("INVALID_INPUT", format!("{field} must be integer")))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(SearchError::new(
            "INVALID_INPUT",
            format!("{field} is outside the supported range"),
        ));
    }
    Ok(value)
}

fn optional_string(
    value: Option<&Value>,
    field: &str,
) -> std::result::Result<Option<String>, SearchError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| value.chars().count() <= MAX_OPTION_CHARS)
        .map(str::to_string)
        .map(Some)
        .ok_or_else(|| SearchError::new("INVALID_INPUT", format!("{field} must be string")))
}

fn get_string_array(
    value: Option<&Value>,
    field: &str,
) -> std::result::Result<Vec<String>, SearchError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| SearchError::new("INVALID_INPUT", format!("{field} must be array")))?;
    if values.len() > MAX_DOMAIN_FILTERS {
        return Err(SearchError::new(
            "INVALID_INPUT",
            format!("{field} has too many entries"),
        ));
    }
    values
        .iter()
        .map(|value| {
            let value = value.as_str().ok_or_else(|| {
                SearchError::new("INVALID_INPUT", format!("{field} items must be strings"))
            })?;
            normalize_domain(value)
        })
        .collect()
}

fn normalize_domain(value: &str) -> std::result::Result<String, SearchError> {
    let domain = value
        .trim()
        .trim_start_matches('.')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if domain.is_empty()
        || domain.len() > 253
        || !domain
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(SearchError::new(
            "INVALID_INPUT",
            "domain filter is invalid",
        ));
    }
    Ok(domain)
}

fn handle(input: &SearchInput) -> std::result::Result<Value, SearchError> {
    let backend = resolve_backend(input.backend.as_deref()).map_err(search_failure)?;
    let search_result = search_selected_backend(input, &backend);
    let (mut backend_label, mut items) = match search_result {
        Ok(result) => result,
        Err(_) if input.backend.is_none() => {
            search_fallback_sources(input).map_err(search_failure)?
        }
        Err(error) => return Err(search_failure(error)),
    };

    normalize_and_filter(&mut items, input);
    if items.is_empty() {
        if let Ok((fallback_backend, mut fallback_items)) = search_fallback_sources(input) {
            normalize_and_filter(&mut fallback_items, input);
            if !fallback_items.is_empty() {
                backend_label = fallback_backend;
                items = fallback_items;
            }
        }
    }
    Ok(build_search_payload(input, &backend_label, items))
}

fn search_selected_backend(
    input: &SearchInput,
    backend: &Backend,
) -> Result<(String, Vec<SearchItem>)> {
    let exact_backend = input.backend.is_some();
    match backend {
        Backend::SerpApi => match search_serpapi(input) {
            Ok(items) => Ok((Backend::SerpApi.as_str().to_string(), items)),
            Err(error) if exact_backend => Err(error),
            Err(_) => match search_bing_html(input) {
                Ok(items) => Ok((Backend::BingHtml.as_str().to_string(), items)),
                Err(_) => search_duckduckgo_html(input)
                    .map(|items| (Backend::DuckDuckGoHtml.as_str().to_string(), items)),
            },
        },
        Backend::DuckDuckGoHtml => match search_duckduckgo_html(input) {
            Ok(items) => Ok((Backend::DuckDuckGoHtml.as_str().to_string(), items)),
            Err(error) if exact_backend => Err(error),
            Err(_) => {
                search_bing_html(input).map(|items| (Backend::BingHtml.as_str().to_string(), items))
            }
        },
        Backend::BingHtml => {
            search_bing_html(input).map(|items| (Backend::BingHtml.as_str().to_string(), items))
        }
    }
}

fn build_search_payload(
    input: &SearchInput,
    backend_label: &str,
    mut items: Vec<SearchItem>,
) -> Value {
    items.truncate(candidate_window(input));
    let snapshot_id = search_snapshot_id(input, backend_label, &items);
    let observed_count = items.len();
    let page_start = input.cursor.min(observed_count);
    let page_end = page_start.saturating_add(input.top_k).min(observed_count);
    let has_more = page_end < observed_count;
    let mut items = items[page_start..page_end].to_vec();
    if !input.include_snippet {
        items.iter_mut().for_each(|item| item.snippet = None);
    }

    for (idx, it) in items.iter_mut().enumerate() {
        it.rank = input.cursor + idx + 1;
    }

    let extract_urls = items.iter().map(|x| x.url.clone()).collect::<Vec<_>>();
    let citations = extract_urls.clone();

    json!({
        "status":"ok",
        "error_code": Value::Null,
        "error": Value::Null,
        "backend": backend_label,
        "items": items,
        "extract_urls": extract_urls,
        "summary": "search_result_set",
        "result_count": page_end.saturating_sub(page_start),
        "observed_candidate_count": observed_count,
        "citations": citations,
        "snapshot_id": snapshot_id,
        "page": {
            "cursor": input.cursor,
            "limit": input.top_k,
            "returned_count": page_end.saturating_sub(page_start),
            "total_count": Value::Null,
            "observed_candidate_count": observed_count,
            "has_more": has_more,
            "next_cursor": has_more.then_some(page_end),
            "previous_cursor": (input.cursor > 0)
                .then_some(input.cursor.saturating_sub(input.top_k)),
            "stability": "backend_best_effort"
        }
    })
}

fn search_failure(error: anyhow::Error) -> SearchError {
    SearchError::new("SEARCH_FAILED", error.to_string()).retryable()
}

fn candidate_window(input: &SearchInput) -> usize {
    input
        .cursor
        .saturating_add(input.top_k)
        .saturating_add(1)
        .min(MAX_CANDIDATE_WINDOW)
}

fn search_snapshot_id(input: &SearchInput, backend: &str, items: &[SearchItem]) -> String {
    let mut digest = Sha256::new();
    digest.update(input.query.as_bytes());
    digest.update([0]);
    digest.update(backend.as_bytes());
    for item in items {
        digest.update([0]);
        digest.update(item.url.as_bytes());
        digest.update([0]);
        digest.update(item.title.as_bytes());
    }
    format!("sha256:{:x}", digest.finalize())
}

fn resolve_backend(raw: Option<&str>) -> Result<Backend> {
    if let Some(name) = raw {
        if let Some(b) = Backend::from_name(name) {
            return Ok(b);
        }
        return Err(anyhow!("unsupported backend `{}`", name));
    }
    if env::var("SERPAPI_API_KEY").is_ok() {
        return Ok(Backend::SerpApi);
    }
    Ok(Backend::DuckDuckGoHtml)
}

fn backend_client(timeout_seconds: u64, allowed_hosts: &[&str]) -> Result<Client> {
    let allowed_hosts = allowed_hosts
        .iter()
        .map(|host| host.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let redirect_policy = Policy::custom(move |attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.error("backend redirect limit exceeded");
        }
        let allowed = attempt.url().scheme() == "https"
            && attempt
                .url()
                .host_str()
                .is_some_and(|host| allowed_hosts.iter().any(|allowed| host == allowed));
        if allowed {
            attempt.follow()
        } else {
            attempt.error("backend redirect target blocked")
        }
    });
    Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .redirect(redirect_policy)
        .build()
        .context("build search client failed")
}

fn read_backend_response(mut response: Response) -> Result<Vec<u8>> {
    let content_length = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    read_bounded_backend_body(&mut response, content_length)
}

fn read_bounded_backend_body(
    reader: &mut impl Read,
    content_length: Option<usize>,
) -> Result<Vec<u8>> {
    if content_length.is_some_and(|length| length > MAX_BACKEND_RESPONSE_BYTES) {
        return Err(anyhow!("search backend response exceeds byte limit"));
    }
    let mut body = Vec::with_capacity(64 * 1024);
    reader
        .take((MAX_BACKEND_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .context("search backend response read failed")?;
    if body.len() > MAX_BACKEND_RESPONSE_BYTES {
        return Err(anyhow!("search backend response exceeds byte limit"));
    }
    Ok(body)
}

fn read_backend_text(response: Response) -> Result<String> {
    Ok(String::from_utf8_lossy(&read_backend_response(response)?).into_owned())
}

fn search_serpapi(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let api_key =
        env::var("SERPAPI_API_KEY").context("SERPAPI_API_KEY missing for serpapi backend")?;
    let client = backend_client(20, &["serpapi.com"])?;

    let mut url = Url::parse("https://serpapi.com/search.json").expect("valid url");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("engine", "google");
        q.append_pair("q", &input.query);
        q.append_pair("num", &candidate_window(input).to_string());
        q.append_pair("api_key", &api_key);
        if let Some(lang) = &input.lang {
            q.append_pair("hl", lang);
        }
        if let Some(tr) = &input.time_range {
            if !tr.trim().is_empty() {
                q.append_pair("tbs", tr.trim());
            }
        }
    }
    let res = client
        .get(url)
        .send()
        .map_err(|_| anyhow!("serpapi request failed"))?
        .error_for_status()
        .map_err(|_| anyhow!("serpapi non-success response"))?;

    let body = read_backend_response(res)?;
    let v: Value = serde_json::from_slice(&body).context("serpapi json parse failed")?;
    let organic = v
        .get("organic_results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = vec![];
    for item in organic {
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let url = item
            .get("link")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let snippet = item
            .get("snippet")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let source = normalize_source_from_url(&url);
        out.push(SearchItem {
            title,
            url,
            snippet,
            source,
            rank: out.len() + 1,
        });
    }
    Ok(out)
}

fn search_duckduckgo_html(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let client = backend_client(20, &["html.duckduckgo.com"])?;
    let mut url = Url::parse("https://html.duckduckgo.com/html/").expect("valid url");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("q", &input.query);
        if let Some(lang) = &input.lang {
            q.append_pair("kl", lang);
        }
    }
    let response = client
        .get(url)
        .header(
            "user-agent",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
        )
        .send()
        .context("duckduckgo request failed")?
        .error_for_status()
        .context("duckduckgo non-success response")?;
    let html = read_backend_text(response)?;

    Ok(parse_duckduckgo_html_results(&html, input))
}

fn parse_duckduckgo_html_results(html: &str, input: &SearchInput) -> Vec<SearchItem> {
    let a_re = Regex::new(
        r#"(?is)<a[^>]*class="[^"]*\bresult__a\b[^"]*"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#,
    )
    .expect("regex");
    let sn_re = Regex::new(r#"(?is)<a[^>]*class="[^"]*\bresult__snippet\b[^"]*"[^>]*>(.*?)</a>|<div[^>]*class="[^"]*\bresult__snippet\b[^"]*"[^>]*>(.*?)</div>"#)
        .expect("regex");
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("regex");

    let mut out = vec![];
    let captures = a_re.captures_iter(html).collect::<Vec<_>>();
    for (idx, ac) in captures.iter().enumerate() {
        let href = ac.get(1).map(|m| m.as_str()).unwrap_or("").trim();
        let title_html = ac.get(2).map(|m| m.as_str()).unwrap_or("").trim();
        let title = tag_re
            .replace_all(title_html, " ")
            .to_string()
            .replace("&amp;", "&");
        let url = unwrap_ddg_redirect(href).unwrap_or_else(|| href.to_string());
        if title.trim().is_empty() || url.trim().is_empty() {
            continue;
        }
        let block_start = ac.get(0).map(|m| m.end()).unwrap_or(0);
        let block_end = captures
            .get(idx + 1)
            .and_then(|next| next.get(0).map(|m| m.start()))
            .unwrap_or(html.len());
        let block = html.get(block_start..block_end).unwrap_or_default();
        let snippet = sn_re.captures(block).and_then(|c| {
            let s = c
                .get(1)
                .or_else(|| c.get(2))
                .map(|m| m.as_str())
                .unwrap_or("");
            let cleaned = tag_re.replace_all(s, " ").to_string().replace("&amp;", "&");
            let trimmed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        out.push(SearchItem {
            title: title.split_whitespace().collect::<Vec<_>>().join(" "),
            url,
            snippet,
            source: "duckduckgo".to_string(),
            rank: out.len() + 1,
        });
        if out.len() >= candidate_window(input).saturating_mul(3) {
            break;
        }
    }
    out
}

fn search_fallback_sources(input: &SearchInput) -> Result<(String, Vec<SearchItem>)> {
    if domain_explicitly_allowed(input, "docs.rs") {
        if let Ok(items) = search_docs_rs(input) {
            if !items.is_empty() {
                return Ok(("docs_rs_search".to_string(), items));
            }
        }
    }
    if domain_allowed_by_filter(input, "github.com") {
        if let Ok(items) = search_github_repositories(input) {
            if !items.is_empty() {
                return Ok(("github_repositories".to_string(), items));
            }
        }
    }
    Err(anyhow!("no fallback search source returned candidates"))
}

fn domain_explicitly_allowed(input: &SearchInput, domain: &str) -> bool {
    input
        .domains_allow
        .iter()
        .any(|allowed| domain_matches(domain, allowed))
}

fn domain_allowed_by_filter(input: &SearchInput, domain: &str) -> bool {
    if input
        .domains_deny
        .iter()
        .any(|denied| domain_matches(domain, denied))
    {
        return false;
    }
    input.domains_allow.is_empty()
        || input
            .domains_allow
            .iter()
            .any(|allowed| domain_matches(domain, allowed))
}

fn search_github_repositories(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let client = backend_client(15, &["api.github.com"])?;
    let mut url = Url::parse("https://api.github.com/search/repositories").expect("valid url");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("q", &query_without_site_operators(&input.query));
        q.append_pair("per_page", &candidate_window(input).min(100).to_string());
    }
    let res = client
        .get(url)
        .header("user-agent", "rustclaw-web-search-extract")
        .send()
        .context("github search request failed")?
        .error_for_status()
        .context("github search non-success response")?;
    let body = read_backend_response(res)?;
    let payload: Value =
        serde_json::from_slice(&body).context("github search json parse failed")?;
    let mut out = Vec::new();
    for item in payload
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let full_name = item
            .get("full_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let url = item
            .get("html_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if full_name.is_empty() || url.is_empty() {
            continue;
        }
        let snippet = item
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let title = snippet
            .as_deref()
            .map(|description| format!("{full_name} - {description}"))
            .unwrap_or(full_name);
        out.push(SearchItem {
            title,
            url,
            snippet,
            source: "github.com".to_string(),
            rank: out.len() + 1,
        });
    }
    Ok(out)
}

fn search_docs_rs(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let client = backend_client(15, &["docs.rs"])?;
    let mut url = Url::parse("https://docs.rs/releases/search").expect("valid url");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("query", &query_without_site_operators(&input.query));
    }
    let response = client
        .get(url)
        .header(
            "user-agent",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
        )
        .send()
        .context("docs.rs search request failed")?
        .error_for_status()
        .context("docs.rs search non-success response")?;
    let html = read_backend_text(response)?;
    Ok(parse_docs_rs_results(
        &html,
        candidate_window(input).saturating_mul(3),
    ))
}

fn parse_docs_rs_results(html: &str, max_items: usize) -> Vec<SearchItem> {
    let row_re =
        Regex::new(r#"(?is)<a\s+href="([^"]+)"\s+class="release"\s*>(.*?)</a>"#).expect("regex");
    let name_re =
        Regex::new(r#"(?is)<div[^>]*class="[^"]*\bname\b[^"]*"[^>]*>(.*?)</div>"#).expect("regex");
    let desc_re = Regex::new(r#"(?is)<div[^>]*class="[^"]*\bdescription\b[^"]*"[^>]*>(.*?)</div>"#)
        .expect("regex");
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("regex");

    let mut out = Vec::new();
    for row in row_re.captures_iter(html) {
        let href = row.get(1).map(|m| m.as_str()).unwrap_or("").trim();
        let block = row.get(2).map(|m| m.as_str()).unwrap_or("");
        let title = name_re
            .captures(block)
            .and_then(|c| c.get(1))
            .map(|m| clean_html_text(m.as_str(), &tag_re))
            .unwrap_or_default();
        if title.is_empty() || href.is_empty() {
            continue;
        }
        let snippet = desc_re
            .captures(block)
            .and_then(|c| c.get(1))
            .map(|m| clean_html_text(m.as_str(), &tag_re))
            .filter(|value| !value.is_empty());
        out.push(SearchItem {
            title,
            url: format!("https://docs.rs{href}"),
            snippet,
            source: "docs.rs".to_string(),
            rank: out.len() + 1,
        });
        if out.len() >= max_items {
            break;
        }
    }
    out
}

fn search_bing_html(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let client = backend_client(20, &["www.bing.com"])?;
    let mut url = Url::parse("https://www.bing.com/search").expect("valid url");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("q", &input.query);
        q.append_pair("count", &candidate_window(input).to_string());
        if let Some(lang) = &input.lang {
            q.append_pair("setlang", lang);
        }
    }
    let response = client
        .get(url)
        .header(
            "user-agent",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
        )
        .send()
        .context("bing request failed")?
        .error_for_status()
        .context("bing non-success response")?;
    let html = read_backend_text(response)?;
    Ok(parse_bing_html_results(
        &html,
        candidate_window(input).saturating_mul(3),
    ))
}

fn parse_bing_html_results(html: &str, max_items: usize) -> Vec<SearchItem> {
    let row_re = Regex::new(r#"(?is)<li class="b_algo"[^>]*>(.*?)</li>"#).expect("regex");
    let a_re = Regex::new(r#"(?is)<h2[^>]*>\s*<a[^>]*href="([^"]+)"[^>]*>(.*?)</a>\s*</h2>"#)
        .expect("regex");
    let sn_re =
        Regex::new(r#"(?is)<div[^>]*class="b_caption"[^>]*>.*?<p[^>]*>(.*?)</p>"#).expect("regex");
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("regex");

    let mut out = vec![];
    for row in row_re.captures_iter(html) {
        let Some(block) = row.get(1).map(|m| m.as_str()) else {
            continue;
        };
        let Some(ac) = a_re.captures(block) else {
            continue;
        };
        let href = ac.get(1).map(|m| m.as_str()).unwrap_or("").trim();
        let title_html = ac.get(2).map(|m| m.as_str()).unwrap_or("").trim();
        let title = clean_html_text(title_html, &tag_re);
        if title.is_empty() || href.is_empty() {
            continue;
        }
        let snippet = sn_re.captures(block).and_then(|captures| {
            captures
                .get(1)
                .map(|m| clean_html_text(m.as_str(), &tag_re))
                .filter(|value| !value.is_empty())
        });
        out.push(SearchItem {
            title,
            url: href.to_string(),
            snippet,
            source: "bing".to_string(),
            rank: out.len() + 1,
        });
        if out.len() >= max_items {
            break;
        }
    }
    out
}

fn clean_html_text(raw: &str, tag_re: &Regex) -> String {
    decode_basic_html_entities(&tag_re.replace_all(raw, " "))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_basic_html_entities(raw: &str) -> String {
    raw.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
        .replace("&ensp;", " ")
        .replace("&emsp;", " ")
        .replace("&#0183;", "·")
}

fn unwrap_ddg_redirect(href: &str) -> Option<String> {
    let href = href.replace("&amp;", "&");
    let href = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href
    };
    let parsed = Url::parse(&href).ok()?;
    if parsed.domain() == Some("duckduckgo.com") && parsed.path() == "/l/" {
        let uddg = parsed
            .query_pairs()
            .find(|(k, _)| k == "uddg")
            .map(|(_, v)| v.to_string());
        return uddg;
    }
    Some(href.to_string())
}

fn site_domains_from_query(query: &str) -> Vec<String> {
    let re =
        Regex::new(r"(?i)(?:^|\s)site:([a-z0-9][a-z0-9.-]*\.[a-z]{2,})(?:\s|$)").expect("regex");
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for cap in re.captures_iter(query) {
        let Some(domain) = cap.get(1).map(|m| m.as_str().to_ascii_lowercase()) else {
            continue;
        };
        if seen.insert(domain.clone()) {
            out.push(domain);
        }
    }
    out
}

fn query_without_site_operators(query: &str) -> String {
    let re = Regex::new(r"(?i)(?:^|\s)site:[a-z0-9][a-z0-9.-]*\.[a-z]{2,}(?:\s|$)").expect("regex");
    re.replace_all(query, " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_and_filter(items: &mut Vec<SearchItem>, input: &SearchInput) {
    let mut seen = HashSet::new();
    let mut out = vec![];

    for it in items.drain(..) {
        let Some(norm_url) = normalize_url(&it.url) else {
            continue;
        };
        let host = host_of(&norm_url);
        if !input.domains_allow.is_empty()
            && !input
                .domains_allow
                .iter()
                .any(|domain| domain_matches(&host, domain))
        {
            continue;
        }
        if input
            .domains_deny
            .iter()
            .any(|domain| domain_matches(&host, domain))
        {
            continue;
        }
        if seen.insert(norm_url.clone()) {
            out.push(SearchItem {
                title: bounded_text(&it.title, MAX_TITLE_CHARS),
                snippet: it
                    .snippet
                    .as_deref()
                    .map(|value| bounded_text(value, MAX_SNIPPET_CHARS))
                    .filter(|value| !value.is_empty()),
                source: normalize_source_from_url(&norm_url),
                url: norm_url,
                rank: it.rank,
            });
        }
    }
    *items = out;
}

fn normalize_url(raw: &str) -> Option<String> {
    if raw.len() > MAX_URL_BYTES {
        return None;
    }
    let mut url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_fragment(None);
    let host = url.host_str()?.to_ascii_lowercase();
    if is_local_hostname(&host)
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| !is_public_ip(address))
    {
        return None;
    }
    url.set_host(Some(&host)).ok()?;
    if (url.scheme() == "http" && url.port() == Some(80))
        || (url.scheme() == "https" && url.port() == Some(443))
    {
        let _ = url.set_port(None);
    }
    let kept = url
        .query_pairs()
        .filter(|(k, _)| {
            let key = k.to_ascii_lowercase();
            !key.starts_with("utm_") && key != "gclid" && key != "fbclid"
        })
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<Vec<_>>();
    url.set_query(None);
    if !kept.is_empty() {
        {
            let mut q = url.query_pairs_mut();
            for (k, v) in kept {
                q.append_pair(&k, &v);
            }
        }
    }
    Some(url.to_string())
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn is_local_hostname(host: &str) -> bool {
    matches!(host, "localhost" | "localhost.localdomain")
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 198 && matches!(octets[1], 18 | 19))
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        || octets[0] >= 240)
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let segments = ip.segments();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn host_of(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_ascii_lowercase()))
        .unwrap_or_default()
}

fn normalize_source_from_url(url: &str) -> String {
    host_of(url)
}

fn unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use reqwest::blocking::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::env;
use std::io::{self, BufRead, Write};
use std::time::Duration;
use url::Url;

#[derive(Clone, Debug)]
struct SearchInput {
    request_id: String,
    action: String,
    query: String,
    top_k: usize,
    lang: Option<String>,
    time_range: Option<String>,
    domains_allow: Vec<String>,
    domains_deny: Vec<String>,
    backend: Option<String>,
    include_snippet: bool,
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
}

impl Backend {
    fn from_name(v: &str) -> Option<Self> {
        match v.to_ascii_lowercase().as_str() {
            "serpapi" => Some(Self::SerpApi),
            "duckduckgo_html" | "duckduckgo" | "ddg" => Some(Self::DuckDuckGoHtml),
            _ => None,
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            Self::SerpApi => "serpapi",
            Self::DuckDuckGoHtml => "duckduckgo_html",
        }
    }
}

fn main() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let req: Value = serde_json::from_str(&line).unwrap_or_else(|_| json!({"request_id":"unknown"}));
        let input = parse_input(&req);
        let text_payload = match handle(&input) {
            Ok(v) => v,
            Err(e) => json!({
                "status":"error",
                "error_code":"SEARCH_FAILED",
                "error": e.to_string(),
                "backend": Value::Null,
                "items": [],
                "extract_urls": [],
                "summary": "",
                "citations": []
            }),
        };

        let out = json!({
            "request_id": input.request_id,
            "status": "ok",
            "text": serde_json::to_string(&text_payload)?,
            "error_text": Value::Null,
            "extra": {
                "action": input.action,
                "backend_connected": text_payload.get("status").and_then(Value::as_str) == Some("ok")
            }
        });
        writeln!(stdout, "{}", serde_json::to_string(&out)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn parse_input(req: &Value) -> SearchInput {
    let args = req.get("args").unwrap_or(req);
    let request_id = req
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let action = args
        .get("action")
        .or_else(|| req.get("action"))
        .and_then(Value::as_str)
        .unwrap_or("search")
        .to_string();
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let top_k = args
        .get("top_k")
        .or_else(|| args.get("topK"))
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(5)
        .clamp(1, 20);
    let lang = args.get("lang").and_then(Value::as_str).map(|s| s.to_string());
    let time_range = args
        .get("time_range")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let domains_allow = get_string_array(args.get("domains_allow"));
    let domains_deny = get_string_array(args.get("domains_deny"));
    let backend = args
        .get("backend")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| env::var("WEB_SEARCH_BACKEND").ok());
    let include_snippet = args
        .get("include_snippet")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    SearchInput {
        request_id,
        action,
        query,
        top_k,
        lang,
        time_range,
        domains_allow,
        domains_deny,
        backend,
        include_snippet,
    }
}

fn get_string_array(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn handle(input: &SearchInput) -> Result<Value> {
    if input.query.is_empty() {
        return Ok(json!({
            "status":"error",
            "error_code":"INVALID_INPUT",
            "error":"query is required",
            "backend": Value::Null,
            "items": [],
            "extract_urls": [],
            "summary":"",
            "citations":[]
        }));
    }
    if input.action != "search" && input.action != "search_extract" {
        return Ok(json!({
            "status":"error",
            "error_code":"INVALID_ACTION",
            "error": format!("unsupported action: {}", input.action),
            "backend": Value::Null,
            "items": [],
            "extract_urls": [],
            "summary":"",
            "citations":[]
        }));
    }

    let backend = resolve_backend(input.backend.as_deref())?;
    let mut items = match backend {
        Backend::SerpApi => search_serpapi(input).or_else(|e| {
            if matches!(input.backend.as_deref().map(|s| s.to_ascii_lowercase()), Some(ref b) if b == "serpapi") {
                Err(e)
            } else {
                search_duckduckgo_html(input)
            }
        })?,
        Backend::DuckDuckGoHtml => search_duckduckgo_html(input)?,
    };

    normalize_and_filter(&mut items, input);
    if items.len() > input.top_k {
        items.truncate(input.top_k);
    }

    if !input.include_snippet {
        for it in &mut items {
            it.snippet = None;
        }
    }

    for (idx, it) in items.iter_mut().enumerate() {
        it.rank = idx + 1;
    }

    let extract_urls = items.iter().map(|x| x.url.clone()).collect::<Vec<_>>();
    let citations = extract_urls.clone();
    let summary = build_summary(&items, &input.query, backend.as_str());

    Ok(json!({
        "status":"ok",
        "error_code": Value::Null,
        "error": Value::Null,
        "backend": backend.as_str(),
        "items": items,
        "extract_urls": extract_urls,
        "summary": summary,
        "citations": citations,
        "notes": "search-only skill; use browser_web for page extraction"
    }))
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
    if env::var("WEB_SEARCH_ALLOW_DDG").ok().as_deref() == Some("1") {
        return Ok(Backend::DuckDuckGoHtml);
    }
    Err(anyhow!(
        "no search backend configured: set args.backend or WEB_SEARCH_BACKEND, or provide SERPAPI_API_KEY"
    ))
}

fn search_serpapi(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let api_key = env::var("SERPAPI_API_KEY").context("SERPAPI_API_KEY missing for serpapi backend")?;
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("build http client failed")?;

    let mut url = Url::parse("https://serpapi.com/search.json").expect("valid url");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("engine", "google");
        q.append_pair("q", &input.query);
        q.append_pair("num", &input.top_k.to_string());
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
        .context("serpapi request failed")?
        .error_for_status()
        .context("serpapi non-success response")?;

    let v: Value = res.json().context("serpapi json parse failed")?;
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
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("build http client failed")?;
    let mut url = Url::parse("https://duckduckgo.com/html/").expect("valid url");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("q", &input.query);
        if let Some(lang) = &input.lang {
            q.append_pair("kl", lang);
        }
    }
    let html = client
        .get(url)
        .header(
            "user-agent",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
        )
        .send()
        .context("duckduckgo request failed")?
        .error_for_status()
        .context("duckduckgo non-success response")?
        .text()
        .context("duckduckgo body read failed")?;

    let row_re = Regex::new(r#"(?is)<div class="result__body".*?</div>\s*</div>"#).expect("regex");
    let a_re = Regex::new(r#"(?is)<a[^>]*class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).expect("regex");
    let sn_re = Regex::new(r#"(?is)<a[^>]*class="result__snippet"[^>]*>(.*?)</a>|<div[^>]*class="result__snippet"[^>]*>(.*?)</div>"#)
        .expect("regex");
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("regex");

    let mut out = vec![];
    for row in row_re.find_iter(&html) {
        let block = row.as_str();
        let Some(ac) = a_re.captures(block) else { continue; };
        let href = ac.get(1).map(|m| m.as_str()).unwrap_or("").trim();
        let title_html = ac.get(2).map(|m| m.as_str()).unwrap_or("").trim();
        let title = tag_re.replace_all(title_html, " ").to_string().replace("&amp;", "&");
        let url = unwrap_ddg_redirect(href).unwrap_or_else(|| href.to_string());
        if title.trim().is_empty() || url.trim().is_empty() {
            continue;
        }
        let snippet = sn_re.captures(block).and_then(|c| {
            let s = c.get(1).or_else(|| c.get(2)).map(|m| m.as_str()).unwrap_or("");
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
        if out.len() >= input.top_k * 3 {
            break;
        }
    }
    Ok(out)
}

fn unwrap_ddg_redirect(href: &str) -> Option<String> {
    let parsed = Url::parse(href).ok()?;
    if parsed.domain() == Some("duckduckgo.com") && parsed.path() == "/l/" {
        let uddg = parsed.query_pairs().find(|(k, _)| k == "uddg").map(|(_, v)| v.to_string());
        return uddg;
    }
    Some(href.to_string())
}

fn normalize_and_filter(items: &mut Vec<SearchItem>, input: &SearchInput) {
    let mut seen = HashSet::new();
    let mut out = vec![];

    for it in items.drain(..) {
        let Some(norm_url) = normalize_url(&it.url) else { continue; };
        let host = host_of(&norm_url);
        if !input.domains_allow.is_empty()
            && !input
                .domains_allow
                .iter()
                .any(|d| host == *d || host.ends_with(&format!(".{}", d)))
        {
            continue;
        }
        if input
            .domains_deny
            .iter()
            .any(|d| host == *d || host.ends_with(&format!(".{}", d)))
        {
            continue;
        }
        if seen.insert(norm_url.clone()) {
            out.push(SearchItem {
                source: normalize_source_from_url(&norm_url),
                url: norm_url,
                ..it
            });
        }
    }
    *items = out;
}

fn normalize_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    url.set_fragment(None);
    let host = url.host_str()?.to_ascii_lowercase();
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

fn host_of(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_ascii_lowercase()))
        .unwrap_or_default()
}

fn normalize_source_from_url(url: &str) -> String {
    host_of(url)
}

fn build_summary(items: &[SearchItem], query: &str, backend: &str) -> String {
    if items.is_empty() {
        return format!("No results found for \"{}\" via {}", query, backend);
    }
    let sources = items
        .iter()
        .take(3)
        .map(|i| i.source.clone())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Found {} result(s) for \"{}\" via {} (top sources: {})",
        items.len(),
        query,
        backend,
        sources
    )
}

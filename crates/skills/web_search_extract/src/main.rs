use anyhow::{anyhow, Context, Result};
use regex::Regex;
use reqwest::blocking::{Client, Response};
use reqwest::header::CONTENT_LENGTH;
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::env;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const SKILL_NAME: &str = "web_search_extract";
const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 20;
const MAX_BACKEND_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_QUERY_CHARS: usize = 2_000;
const MAX_OPTION_CHARS: usize = 128;
const MAX_DOMAIN_FILTERS: usize = 32;
const MAX_TITLE_CHARS: usize = 300;
const MAX_SNIPPET_CHARS: usize = 1_000;
const MAX_URL_BYTES: usize = 4_096;
const PROVIDER_CONFIG_RELATIVE_PATH: &str = "configs/web_search_providers.toml";
const EMBEDDED_PROVIDER_CONFIG: &str =
    include_str!("../../../../configs/web_search_providers.toml");

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
    backend_policy: BackendPolicy,
    include_snippet: bool,
}

#[derive(Debug)]
struct SearchError {
    code: &'static str,
    detail: String,
    retryable: bool,
    failure_phase: &'static str,
    recovery_action: Option<&'static str>,
    backend_attempts: Vec<BackendAttempt>,
}

impl SearchError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
            retryable: false,
            failure_phase: "pre_dispatch",
            recovery_action: None,
            backend_attempts: Vec::new(),
        }
    }

    fn execution_failure(
        mut self,
        backend_attempts: Vec<BackendAttempt>,
        allow_source_replan: bool,
    ) -> Self {
        self.retryable = true;
        self.failure_phase = "execution_no_effect";
        self.recovery_action = allow_source_replan.then_some("replan_arguments");
        self.backend_attempts = backend_attempts;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    field_truncations: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    SerpApi,
    BaiduAi,
    Brave,
    SearXng,
    Tavily,
    Perplexity,
    Exa,
    You,
    Mojeek,
    Kagi,
    DuckDuckGoHtml,
    BingHtml,
    DocsRsSearch,
    GitHubRepositories,
}

#[derive(Clone, Debug, Deserialize)]
struct SearchProviderConfig {
    schema_version: u32,
    auto_provider_limit: usize,
    auto_order: Vec<String>,
    providers: BTreeMap<String, ProviderPolicy>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct ProviderPolicy {
    enabled: bool,
    auto_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackendPolicy {
    Auto,
    Strict,
}

impl BackendPolicy {
    fn from_name(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "auto" | "multi_source" | "aggregate" => Some(Self::Auto),
            "strict" => Some(Self::Strict),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Strict => "strict",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct BackendAttempt {
    backend: String,
    status: &'static str,
    result_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug)]
struct BackendOutcome {
    backend: Backend,
    result: std::result::Result<Vec<SearchItem>, String>,
}

#[derive(Debug)]
struct AggregatedSearch {
    items: Vec<SearchItem>,
    backends_used: Vec<String>,
    backend_attempts: Vec<BackendAttempt>,
}

impl Backend {
    fn from_name(v: &str) -> Option<Self> {
        match v.to_ascii_lowercase().as_str() {
            "serpapi" => Some(Self::SerpApi),
            "baidu_ai" | "baidu" | "baidu_search" => Some(Self::BaiduAi),
            "brave" | "brave_search" => Some(Self::Brave),
            "searxng" | "searx" => Some(Self::SearXng),
            "tavily" => Some(Self::Tavily),
            "perplexity" | "perplexity_search" => Some(Self::Perplexity),
            "exa" | "exa_search" => Some(Self::Exa),
            "you" | "you_search" | "you.com" => Some(Self::You),
            "mojeek" => Some(Self::Mojeek),
            "kagi" => Some(Self::Kagi),
            "duckduckgo_html" | "duckduckgo" | "ddg" => Some(Self::DuckDuckGoHtml),
            "bing_html" | "bing" => Some(Self::BingHtml),
            _ => None,
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            Self::SerpApi => "serpapi",
            Self::BaiduAi => "baidu_ai",
            Self::Brave => "brave",
            Self::SearXng => "searxng",
            Self::Tavily => "tavily",
            Self::Perplexity => "perplexity",
            Self::Exa => "exa",
            Self::You => "you",
            Self::Mojeek => "mojeek",
            Self::Kagi => "kagi",
            Self::DuckDuckGoHtml => "duckduckgo_html",
            Self::BingHtml => "bing_html",
            Self::DocsRsSearch => "docs_rs_search",
            Self::GitHubRepositories => "github_repositories",
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
            "error_code": error.code,
            "message_key": format!("skill.{}.{}", SKILL_NAME, error.code.to_ascii_lowercase()),
            "retryable": error.retryable,
            "failure_phase": error.failure_phase,
            "side_effect_applied": false,
            "recovery_action": error.recovery_action,
            "backend_attempts": error.backend_attempts,
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
        "backend_policy": input.backend_policy.as_str(),
        "backend": text_payload.get("backend").cloned().unwrap_or(Value::Null),
        "backends_used": text_payload
            .get("backends_used")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "backend_attempts": text_payload
            .get("backend_attempts")
            .cloned()
            .unwrap_or_else(|| json!([])),
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
            "backends": text_payload
                .get("backends_used")
                .cloned()
                .unwrap_or_else(|| json!([])),
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
    let cursor = parse_search_cursor(args, &query)?;
    let lang = optional_string(args.get("lang"), "lang")?;
    let time_range = optional_string(args.get("time_range"), "time_range")?;
    let mut domains_allow = get_string_array(args.get("domains_allow"), "domains_allow")?;
    if domains_allow.is_empty() {
        domains_allow = site_domains_from_query(&query);
    }
    let domains_deny = get_string_array(args.get("domains_deny"), "domains_deny")?;
    let backend = optional_string(args.get("backend"), "backend")?
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
    let backend_policy = optional_string(args.get("backend_policy"), "backend_policy")?
        .or_else(|| env::var("WEB_SEARCH_BACKEND_POLICY").ok())
        .map(|value| {
            BackendPolicy::from_name(&value).ok_or_else(|| {
                SearchError::new("INVALID_INPUT", "backend_policy must be auto or strict")
            })
        })
        .transpose()?
        .unwrap_or(BackendPolicy::Auto);
    if backend_policy == BackendPolicy::Strict && backend.is_none() {
        return Err(SearchError::new(
            "INVALID_INPUT",
            "backend is required when backend_policy is strict",
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
        backend_policy,
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
    let value = value
        .as_str()
        .ok_or_else(|| {
            SearchError::new(
                "INVALID_INPUT",
                format!("invalid_field_type:{field}:string"),
            )
        })?
        .trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > MAX_OPTION_CHARS {
        return Err(SearchError::new(
            "INVALID_INPUT",
            format!("field_length_exceeded:{field}:{MAX_OPTION_CHARS}"),
        ));
    }
    Ok(Some(value.to_string()))
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

fn load_provider_config() -> std::result::Result<SearchProviderConfig, SearchError> {
    let configured_path = env::var("WEB_SEARCH_PROVIDER_CONFIG")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let workspace_path = env::var("WORKSPACE_ROOT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .map(|root| root.join(PROVIDER_CONFIG_RELATIVE_PATH));
    let raw = match configured_path {
        Some(path) => fs::read_to_string(&path).map_err(|error| {
            SearchError::new(
                "SEARCH_CONFIG_INVALID",
                format!(
                    "cannot read WEB_SEARCH_PROVIDER_CONFIG {}: {error}",
                    path.display()
                ),
            )
        })?,
        None => match workspace_path.as_ref().filter(|path| path.is_file()) {
            Some(path) => fs::read_to_string(path).map_err(|error| {
                SearchError::new(
                    "SEARCH_CONFIG_INVALID",
                    format!("cannot read search provider config: {error}"),
                )
            })?,
            None => EMBEDDED_PROVIDER_CONFIG.to_string(),
        },
    };
    parse_provider_config(&raw)
}

fn parse_provider_config(raw: &str) -> std::result::Result<SearchProviderConfig, SearchError> {
    let config: SearchProviderConfig = toml::from_str(raw).map_err(|error| {
        SearchError::new(
            "SEARCH_CONFIG_INVALID",
            format!("invalid search provider config: {error}"),
        )
    })?;
    if config.schema_version != 1 {
        return Err(SearchError::new(
            "SEARCH_CONFIG_INVALID",
            "unsupported search provider config schema_version",
        ));
    }
    if config.auto_provider_limit == 0 || config.auto_provider_limit > config.auto_order.len() {
        return Err(SearchError::new(
            "SEARCH_CONFIG_INVALID",
            "auto_provider_limit must be between 1 and auto_order length",
        ));
    }
    let mut seen = HashSet::new();
    for name in &config.auto_order {
        let backend = Backend::from_name(name).ok_or_else(|| {
            SearchError::new(
                "SEARCH_CONFIG_INVALID",
                format!("unknown provider in auto_order: {name}"),
            )
        })?;
        if !backend.is_general() || !seen.insert(backend.as_str()) {
            return Err(SearchError::new(
                "SEARCH_CONFIG_INVALID",
                format!("invalid or duplicate automatic provider: {name}"),
            ));
        }
        if !config.providers.contains_key(backend.as_str()) {
            return Err(SearchError::new(
                "SEARCH_CONFIG_INVALID",
                format!("provider policy is missing for {name}"),
            ));
        }
    }
    Ok(config)
}

impl Backend {
    fn is_general(self) -> bool {
        !matches!(self, Self::DocsRsSearch | Self::GitHubRepositories)
    }

    fn credential_is_available(self) -> bool {
        let present = |name: &str| {
            env::var(name)
                .ok()
                .is_some_and(|value| !value.trim().is_empty())
        };
        match self {
            Self::SerpApi => present("SERPAPI_API_KEY"),
            Self::BaiduAi => present("BAIDU_AI_SEARCH_API_KEY"),
            Self::Brave => present("BRAVE_SEARCH_API_KEY"),
            Self::SearXng => present("SEARXNG_SEARCH_URL"),
            Self::Tavily => present("TAVILY_API_KEY"),
            Self::Perplexity => present("PERPLEXITY_API_KEY"),
            Self::Exa => present("EXA_API_KEY"),
            Self::You => present("YOU_SEARCH_API_KEY"),
            Self::Mojeek => present("MOJEEK_API_KEY"),
            Self::Kagi => present("KAGI_API_TOKEN"),
            Self::DuckDuckGoHtml
            | Self::BingHtml
            | Self::DocsRsSearch
            | Self::GitHubRepositories => true,
        }
    }
}

fn handle(input: &SearchInput) -> std::result::Result<Value, SearchError> {
    let provider_config = load_provider_config()?;
    let mut backend_plan = build_backend_plan(
        input.backend.as_deref(),
        input.backend_policy,
        &provider_config,
    )?;
    append_explicit_domain_backends(input, &mut backend_plan);
    let outcomes = execute_backend_plan(input, &backend_plan);
    let aggregated = aggregate_backend_outcomes(input, outcomes);

    if aggregated.items.is_empty() {
        let detail = aggregated
            .backend_attempts
            .iter()
            .map(|attempt| match attempt.error.as_deref() {
                Some(error) => format!("{}: {error}", attempt.backend),
                None => format!("{}: no candidates", attempt.backend),
            })
            .collect::<Vec<_>>()
            .join("; ");
        return Err(SearchError::new(
            "SEARCH_FAILED",
            format!("no search source returned candidates ({detail})"),
        )
        .execution_failure(
            aggregated.backend_attempts,
            input.backend_policy == BackendPolicy::Auto,
        ));
    }

    let backend_label = if aggregated.backends_used.len() > 1 {
        "multi_source".to_string()
    } else {
        aggregated
            .backends_used
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown".to_string())
    };
    let mut payload = build_backend_page_payload(input, &backend_label, aggregated.items);
    payload["backend_policy"] = json!(input.backend_policy.as_str());
    payload["backends_used"] = json!(aggregated.backends_used);
    payload["backend_attempts"] = json!(aggregated.backend_attempts);
    Ok(payload)
}

fn build_backend_plan(
    preferred_backend: Option<&str>,
    backend_policy: BackendPolicy,
    config: &SearchProviderConfig,
) -> std::result::Result<Vec<Backend>, SearchError> {
    let preferred = preferred_backend
        .map(|name| {
            Backend::from_name(name).ok_or_else(|| {
                SearchError::new("INVALID_INPUT", format!("unsupported backend `{name}`"))
            })
        })
        .transpose()?;
    let preferred = if let Some(backend) = preferred {
        let enabled = config
            .providers
            .get(backend.as_str())
            .is_some_and(|policy| policy.enabled);
        if !enabled && backend_policy == BackendPolicy::Strict {
            return Err(SearchError::new(
                "SEARCH_PROVIDER_UNAVAILABLE",
                format!("search provider `{}` is disabled", backend.as_str()),
            ));
        }
        if enabled && backend_policy == BackendPolicy::Strict && !backend.credential_is_available()
        {
            return Err(SearchError::new(
                "SEARCH_PROVIDER_UNAVAILABLE",
                format!(
                    "search provider `{}` is enabled but its runtime configuration is missing",
                    backend.as_str()
                ),
            ));
        }
        enabled.then_some(backend)
    } else {
        None
    };

    let mut plan = preferred.into_iter().collect::<Vec<_>>();
    if backend_policy == BackendPolicy::Strict {
        return Ok(plan);
    }
    for name in &config.auto_order {
        let backend = Backend::from_name(name).expect("validated provider config");
        let policy = config
            .providers
            .get(backend.as_str())
            .expect("validated provider policy");
        if policy.enabled
            && policy.auto_enabled
            && backend.credential_is_available()
            && !plan.contains(&backend)
        {
            plan.push(backend);
        }
        if plan.len() >= config.auto_provider_limit {
            break;
        }
    }
    if plan.is_empty() {
        return Err(SearchError::new(
            "SEARCH_PROVIDER_UNAVAILABLE",
            "no enabled search provider is currently available",
        ));
    }
    Ok(plan)
}

fn append_explicit_domain_backends(input: &SearchInput, plan: &mut Vec<Backend>) {
    if input.backend_policy != BackendPolicy::Auto {
        return;
    }
    for (domain, backend) in [
        ("docs.rs", Backend::DocsRsSearch),
        ("github.com", Backend::GitHubRepositories),
    ] {
        if domain_explicitly_allowed(input, domain) && !plan.contains(&backend) {
            plan.push(backend);
        }
    }
}

fn execute_backend_plan(input: &SearchInput, plan: &[Backend]) -> Vec<BackendOutcome> {
    std::thread::scope(|scope| {
        let handles = plan
            .iter()
            .copied()
            .map(|backend| {
                scope.spawn(move || BackendOutcome {
                    backend,
                    result: search_one_backend(input, backend).map_err(|error| error.to_string()),
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .zip(plan.iter().copied())
            .map(|(handle, backend)| {
                handle.join().unwrap_or_else(|_| BackendOutcome {
                    backend,
                    result: Err("search backend worker panicked".to_string()),
                })
            })
            .collect()
    })
}

fn search_one_backend(input: &SearchInput, backend: Backend) -> Result<Vec<SearchItem>> {
    match backend {
        Backend::SerpApi => search_serpapi(input),
        Backend::BaiduAi => search_baidu_ai(input),
        Backend::Brave => search_brave(input),
        Backend::SearXng => search_searxng(input),
        Backend::Tavily => search_tavily(input),
        Backend::Perplexity => search_perplexity(input),
        Backend::Exa => search_exa(input),
        Backend::You => search_you(input),
        Backend::Mojeek => search_mojeek(input),
        Backend::Kagi => search_kagi(input),
        Backend::DuckDuckGoHtml => search_duckduckgo_html(input),
        Backend::BingHtml => search_bing_html(input),
        Backend::DocsRsSearch => search_docs_rs(input),
        Backend::GitHubRepositories => search_github_repositories(input),
    }
}

fn aggregate_backend_outcomes(
    input: &SearchInput,
    outcomes: Vec<BackendOutcome>,
) -> AggregatedSearch {
    let mut queues = Vec::new();
    let mut backends_used = Vec::new();
    let mut backend_attempts = Vec::new();

    for outcome in outcomes {
        let backend_name = outcome.backend.as_str().to_string();
        match outcome.result {
            Ok(mut items) => {
                normalize_and_filter(&mut items, input);
                let result_count = items.len();
                backend_attempts.push(BackendAttempt {
                    backend: backend_name.clone(),
                    status: if result_count == 0 { "empty" } else { "ok" },
                    result_count,
                    error: None,
                });
                if result_count > 0 {
                    backends_used.push(backend_name);
                    queues.push(VecDeque::from(items));
                }
            }
            Err(error) => backend_attempts.push(BackendAttempt {
                backend: backend_name,
                status: "error",
                result_count: 0,
                error: Some(error),
            }),
        }
    }

    let mut items = Vec::new();
    let mut seen_urls = HashSet::new();
    while queues.iter().any(|queue| !queue.is_empty()) {
        for queue in &mut queues {
            while let Some(item) = queue.pop_front() {
                if seen_urls.insert(item.url.clone()) {
                    items.push(item);
                    break;
                }
            }
        }
    }

    AggregatedSearch {
        items,
        backends_used,
        backend_attempts,
    }
}

#[cfg(test)]
fn build_search_payload(input: &SearchInput, backend_label: &str, items: Vec<SearchItem>) -> Value {
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

fn build_backend_page_payload(
    input: &SearchInput,
    backend_label: &str,
    mut items: Vec<SearchItem>,
) -> Value {
    let observed_candidate_count = items.len();
    let has_more = observed_candidate_count > input.top_k;
    items.truncate(input.top_k);
    if !input.include_snippet {
        items.iter_mut().for_each(|item| item.snippet = None);
    }
    for (index, item) in items.iter_mut().enumerate() {
        item.rank = input.cursor + index + 1;
    }
    let returned_count = items.len();
    let next_cursor = has_more.then(|| input.cursor.saturating_add(returned_count));
    let snapshot_id = search_snapshot_id(input, backend_label, &items);
    let extract_urls = items
        .iter()
        .map(|item| item.url.clone())
        .collect::<Vec<_>>();
    json!({
        "status": "ok",
        "error_code": Value::Null,
        "error": Value::Null,
        "backend": backend_label,
        "items": items,
        "extract_urls": extract_urls,
        "summary": "search_result_set",
        "result_count": returned_count,
        "observed_candidate_count": observed_candidate_count,
        "citations": extract_urls,
        "snapshot_id": snapshot_id,
        "page": {
            "cursor": input.cursor,
            "limit": input.top_k,
            "returned_count": returned_count,
            "total_count": Value::Null,
            "observed_candidate_count": observed_candidate_count,
            "has_more": has_more,
            "next_cursor": next_cursor,
            "next_continuation": next_cursor.map(|offset| encode_search_continuation(&input.query, offset)),
            "previous_cursor": (input.cursor > 0)
                .then_some(input.cursor.saturating_sub(input.top_k)),
            "stability": "backend_best_effort"
        }
    })
}

fn candidate_window(input: &SearchInput) -> usize {
    input.top_k.saturating_add(1)
}

fn parse_search_cursor(
    args: &serde_json::Map<String, Value>,
    query: &str,
) -> std::result::Result<usize, SearchError> {
    if let Some(token) = args
        .get("continuation")
        .and_then(Value::as_str)
        .filter(|token| !token.trim().is_empty())
    {
        return decode_search_continuation(query, token);
    }
    let Some(value) = args.get("cursor") else {
        return Ok(0);
    };
    value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| SearchError::new("INVALID_INPUT", "cursor must be integer"))
}

fn search_query_sha256(query: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(query.trim().as_bytes()))
}

fn encode_search_continuation(query: &str, offset: usize) -> String {
    format!("web_search_v1:{offset}:{}", search_query_sha256(query))
}

fn decode_search_continuation(query: &str, token: &str) -> std::result::Result<usize, SearchError> {
    let mut parts = token.splitn(3, ':');
    if parts.next() != Some("web_search_v1") {
        return Err(SearchError::new(
            "INVALID_CONTINUATION",
            "continuation token is invalid",
        ));
    }
    let offset = parts
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| SearchError::new("INVALID_CONTINUATION", "continuation token is invalid"))?;
    if parts.next().unwrap_or_default() != search_query_sha256(query) {
        return Err(SearchError::new(
            "STALE_SNAPSHOT",
            "continuation token does not belong to this query",
        ));
    }
    Ok(offset)
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

fn required_env(
    value: std::result::Result<String, env::VarError>,
    name: &str,
    backend: &str,
) -> Result<String> {
    value
        .ok()
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{name} missing for {backend} backend"))
}

fn provider_language(input: &SearchInput) -> Option<String> {
    input.lang.as_deref().and_then(|lang| {
        let value = lang
            .split(['-', '_'])
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        (value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_alphabetic())).then_some(value)
    })
}

fn provider_time_range(input: &SearchInput) -> Option<&'static str> {
    match input
        .time_range
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "day" | "d" | "24h" => Some("day"),
        "week" | "w" | "7d" => Some("week"),
        "month" | "m" | "30d" => Some("month"),
        "year" | "y" | "365d" => Some("year"),
        _ => None,
    }
}

fn json_text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => {
            Some(value.trim().to_string()).filter(|value| !value.is_empty())
        }
        Some(Value::Array(values)) => {
            let joined = values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            (!joined.is_empty()).then_some(joined)
        }
        _ => None,
    }
}

fn first_json_text(item: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| json_text(item.get(*field)))
}

fn parse_json_result_items(
    items: &[Value],
    title_fields: &[&str],
    url_fields: &[&str],
    snippet_fields: &[&str],
) -> Vec<SearchItem> {
    let mut output = Vec::new();
    for item in items {
        let Some(title) = first_json_text(item, title_fields) else {
            continue;
        };
        let Some(url) = first_json_text(item, url_fields) else {
            continue;
        };
        output.push(SearchItem {
            title,
            source: normalize_source_from_url(&url),
            url,
            snippet: first_json_text(item, snippet_fields),
            rank: output.len() + 1,
            field_truncations: None,
        });
    }
    output
}

fn json_items_at<'a>(payload: &'a Value, pointer: &str) -> &'a [Value] {
    payload
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn parse_baidu_ai_response(payload: &Value) -> Vec<SearchItem> {
    parse_json_result_items(
        json_items_at(payload, "/references"),
        &["title"],
        &["url"],
        &["snippet", "content"],
    )
}

fn parse_brave_response(payload: &Value) -> Vec<SearchItem> {
    parse_json_result_items(
        json_items_at(payload, "/web/results"),
        &["title"],
        &["url"],
        &["description", "snippet"],
    )
}

fn parse_searxng_response(payload: &Value) -> Vec<SearchItem> {
    parse_json_result_items(
        json_items_at(payload, "/results"),
        &["title"],
        &["url"],
        &["content", "snippet"],
    )
}

fn parse_tavily_response(payload: &Value) -> Vec<SearchItem> {
    parse_json_result_items(
        json_items_at(payload, "/results"),
        &["title"],
        &["url"],
        &["content", "snippet"],
    )
}

fn parse_perplexity_response(payload: &Value) -> Vec<SearchItem> {
    parse_json_result_items(
        json_items_at(payload, "/results"),
        &["title"],
        &["url"],
        &["snippet", "content"],
    )
}

fn parse_exa_response(payload: &Value) -> Vec<SearchItem> {
    parse_json_result_items(
        json_items_at(payload, "/results"),
        &["title"],
        &["url"],
        &["text", "highlights"],
    )
}

fn parse_you_response(payload: &Value) -> Vec<SearchItem> {
    let mut values = Vec::new();
    for path in ["/results/web", "/results/news"] {
        values.extend(json_items_at(payload, path).iter().cloned());
    }
    parse_json_result_items(&values, &["title"], &["url"], &["description", "snippets"])
}

fn parse_mojeek_response(payload: &Value) -> Result<Vec<SearchItem>> {
    let status = payload
        .pointer("/response/status")
        .and_then(Value::as_str)
        .unwrap_or("OK");
    if status != "OK" {
        return Err(anyhow!("mojeek search provider returned an error status"));
    }
    Ok(parse_json_result_items(
        json_items_at(payload, "/response/results"),
        &["title"],
        &["url"],
        &["desc", "snippet"],
    ))
}

fn parse_kagi_response(payload: &Value) -> Vec<SearchItem> {
    let values = json_items_at(payload, "/data")
        .iter()
        .filter(|item| item.get("t").and_then(Value::as_i64).unwrap_or(0) == 0)
        .cloned()
        .collect::<Vec<_>>();
    parse_json_result_items(&values, &["title"], &["url"], &["snippet"])
}

fn search_baidu_ai(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let api_key = required_env(
        env::var("BAIDU_AI_SEARCH_API_KEY"),
        "BAIDU_AI_SEARCH_API_KEY",
        "baidu_ai",
    )?;
    let client = backend_client(20, &["qianfan.baidubce.com"])?;
    let mut body = json!({
        "messages": [{"role": "user", "content": input.query}],
        "search_source": "baidu_search_v2",
        "resource_type_filter": [{"type": "web", "top_k": candidate_window(input)}]
    });
    if !input.domains_allow.is_empty() {
        body["search_filter"] = json!({"match": {"site": input.domains_allow}});
    }
    if let Some(range @ ("week" | "month" | "year")) = provider_time_range(input) {
        body["search_recency_filter"] = json!(range);
    }
    let response = client
        .post("https://qianfan.baidubce.com/v2/ai_search/web_search")
        .header("X-Appbuilder-Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .context("baidu ai search request failed")?
        .error_for_status()
        .context("baidu ai search non-success response")?;
    let payload: Value = serde_json::from_slice(&read_backend_response(response)?)
        .context("baidu ai search json parse failed")?;
    Ok(parse_baidu_ai_response(&payload))
}

fn search_brave(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let api_key = required_env(
        env::var("BRAVE_SEARCH_API_KEY"),
        "BRAVE_SEARCH_API_KEY",
        "brave",
    )?;
    let client = backend_client(20, &["api.search.brave.com"])?;
    let mut url = Url::parse("https://api.search.brave.com/res/v1/web/search").expect("valid url");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("q", &input.query);
        query.append_pair("count", &candidate_window(input).min(20).to_string());
        query.append_pair("offset", &(input.cursor / 20).min(9).to_string());
        if let Some(lang) = provider_language(input) {
            query.append_pair("search_lang", &lang);
        }
        if let Some(range) = provider_time_range(input) {
            query.append_pair(
                "freshness",
                match range {
                    "day" => "pd",
                    "week" => "pw",
                    "month" => "pm",
                    "year" => "py",
                    _ => unreachable!(),
                },
            );
        }
    }
    let response = client
        .get(url)
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .send()
        .context("brave search request failed")?
        .error_for_status()
        .context("brave search non-success response")?;
    let payload: Value = serde_json::from_slice(&read_backend_response(response)?)
        .context("brave search json parse failed")?;
    Ok(parse_brave_response(&payload))
}

fn searxng_endpoint() -> Result<Url> {
    let raw = required_env(
        env::var("SEARXNG_SEARCH_URL"),
        "SEARXNG_SEARCH_URL",
        "searxng",
    )?;
    let mut url = Url::parse(raw.trim()).context("SEARXNG_SEARCH_URL is invalid")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(anyhow!(
            "SEARXNG_SEARCH_URL must be an HTTP(S) URL without credentials"
        ));
    }
    if matches!(url.path(), "" | "/") {
        url.set_path("/search");
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn search_searxng(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let mut url = searxng_endpoint()?;
    let host = url.host_str().expect("validated host").to_string();
    let client = backend_client(20, &[host.as_str()])?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("q", &input.query);
        query.append_pair("format", "json");
        query.append_pair(
            "pageno",
            &(input.cursor / input.top_k.max(1) + 1).to_string(),
        );
        if let Some(lang) = provider_language(input) {
            query.append_pair("language", &lang);
        }
        if let Some(range) = provider_time_range(input) {
            query.append_pair("time_range", range);
        }
    }
    let mut request = client.get(url).header("Accept", "application/json");
    if let Ok(api_key) = env::var("SEARXNG_API_KEY") {
        if !api_key.trim().is_empty() {
            request = request.bearer_auth(api_key);
        }
    }
    let response = request
        .send()
        .context("searxng search request failed")?
        .error_for_status()
        .context("searxng search non-success response")?;
    let payload: Value = serde_json::from_slice(&read_backend_response(response)?)
        .context("searxng search json parse failed")?;
    Ok(parse_searxng_response(&payload))
}

fn search_tavily(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let api_key = required_env(env::var("TAVILY_API_KEY"), "TAVILY_API_KEY", "tavily")?;
    let client = backend_client(20, &["api.tavily.com"])?;
    let mut body = json!({
        "query": input.query,
        "search_depth": "basic",
        "max_results": candidate_window(input).min(20),
        "include_answer": false,
        "include_raw_content": false
    });
    if let Some(range) = provider_time_range(input) {
        body["time_range"] = json!(range);
    }
    if !input.domains_allow.is_empty() {
        body["include_domains"] = json!(input.domains_allow);
    }
    if !input.domains_deny.is_empty() {
        body["exclude_domains"] = json!(input.domains_deny);
    }
    let response = client
        .post("https://api.tavily.com/search")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .context("tavily search request failed")?
        .error_for_status()
        .context("tavily search non-success response")?;
    let payload: Value = serde_json::from_slice(&read_backend_response(response)?)
        .context("tavily search json parse failed")?;
    Ok(parse_tavily_response(&payload))
}

fn search_perplexity(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let api_key = required_env(
        env::var("PERPLEXITY_API_KEY"),
        "PERPLEXITY_API_KEY",
        "perplexity",
    )?;
    let client = backend_client(20, &["api.perplexity.ai"])?;
    let mut body = json!({
        "query": input.query,
        "max_results": candidate_window(input).min(20)
    });
    if let Some(lang) = provider_language(input) {
        body["search_language_filter"] = json!([lang]);
    }
    if !input.domains_allow.is_empty() {
        body["search_domain_filter"] = json!(input.domains_allow);
    }
    if let Some(range) = provider_time_range(input) {
        body["search_recency_filter"] = json!(range);
    }
    let response = client
        .post("https://api.perplexity.ai/search")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .context("perplexity search request failed")?
        .error_for_status()
        .context("perplexity search non-success response")?;
    let payload: Value = serde_json::from_slice(&read_backend_response(response)?)
        .context("perplexity search json parse failed")?;
    Ok(parse_perplexity_response(&payload))
}

fn search_exa(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let api_key = required_env(env::var("EXA_API_KEY"), "EXA_API_KEY", "exa")?;
    let client = backend_client(20, &["api.exa.ai"])?;
    let mut body = json!({
        "query": input.query,
        "type": "auto",
        "numResults": candidate_window(input).min(20),
        "contents": {"text": {"maxCharacters": 1000}}
    });
    if !input.domains_allow.is_empty() {
        body["includeDomains"] = json!(input.domains_allow);
    }
    if !input.domains_deny.is_empty() {
        body["excludeDomains"] = json!(input.domains_deny);
    }
    let response = client
        .post("https://api.exa.ai/search")
        .header("x-api-key", api_key)
        .json(&body)
        .send()
        .context("exa search request failed")?
        .error_for_status()
        .context("exa search non-success response")?;
    let payload: Value = serde_json::from_slice(&read_backend_response(response)?)
        .context("exa search json parse failed")?;
    Ok(parse_exa_response(&payload))
}

fn search_you(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let api_key = required_env(env::var("YOU_SEARCH_API_KEY"), "YOU_SEARCH_API_KEY", "you")?;
    let client = backend_client(20, &["ydc-index.io"])?;
    let mut body = json!({
        "query": input.query,
        "count": candidate_window(input).min(20),
        "offset": (input.cursor / input.top_k.max(1)).min(9)
    });
    if !input.domains_allow.is_empty() {
        body["include_domains"] = json!(input.domains_allow);
    } else if !input.domains_deny.is_empty() {
        body["exclude_domains"] = json!(input.domains_deny);
    }
    if let Some(range) = provider_time_range(input) {
        body["freshness"] = json!(range);
    }
    if let Some(lang) = provider_language(input) {
        body["language"] = json!(lang);
    }
    let response = client
        .post("https://ydc-index.io/v1/search")
        .header("X-API-Key", api_key)
        .json(&body)
        .send()
        .context("you search request failed")?
        .error_for_status()
        .context("you search non-success response")?;
    let payload: Value = serde_json::from_slice(&read_backend_response(response)?)
        .context("you search json parse failed")?;
    Ok(parse_you_response(&payload))
}

fn search_mojeek(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let api_key = required_env(env::var("MOJEEK_API_KEY"), "MOJEEK_API_KEY", "mojeek")?;
    let client = backend_client(20, &["api.mojeek.com"])?;
    let mut url = Url::parse("https://api.mojeek.com/search").expect("valid url");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("api_key", &api_key);
        query.append_pair("q", &input.query);
        query.append_pair("s", &input.cursor.saturating_add(1).to_string());
        query.append_pair("t", &candidate_window(input).min(20).to_string());
        query.append_pair("fmt", "json");
        query.append_pair("date", "1");
        query.append_pair("safe", "1");
        if let Some(lang) = provider_language(input) {
            query.append_pair("lb", &lang.to_ascii_uppercase());
            query.append_pair("lbb", "100");
        }
        if let Some(range) = provider_time_range(input) {
            query.append_pair("since", range);
        }
        if !input.domains_allow.is_empty() {
            query.append_pair("fi", &input.domains_allow.join(","));
        }
        if !input.domains_deny.is_empty() {
            query.append_pair("fe", &input.domains_deny.join(","));
        }
    }
    let response = client
        .get(url)
        .send()
        .context("mojeek search request failed")?
        .error_for_status()
        .context("mojeek search non-success response")?;
    let payload: Value = serde_json::from_slice(&read_backend_response(response)?)
        .context("mojeek search json parse failed")?;
    parse_mojeek_response(&payload)
}

fn search_kagi(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let api_key = required_env(env::var("KAGI_API_TOKEN"), "KAGI_API_TOKEN", "kagi")?;
    let client = backend_client(20, &["kagi.com"])?;
    let mut url = Url::parse("https://kagi.com/api/v1/search").expect("valid url");
    url.query_pairs_mut().append_pair("q", &input.query);
    let response = client
        .get(url)
        .header("Authorization", format!("Bot {api_key}"))
        .send()
        .context("kagi search request failed")?
        .error_for_status()
        .context("kagi search non-success response")?;
    let payload: Value = serde_json::from_slice(&read_backend_response(response)?)
        .context("kagi search json parse failed")?;
    Ok(parse_kagi_response(&payload))
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
        q.append_pair("start", &input.cursor.to_string());
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
            field_truncations: None,
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
        q.append_pair("s", &input.cursor.to_string());
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
            field_truncations: None,
        });
        if out.len() >= candidate_window(input).saturating_mul(3) {
            break;
        }
    }
    out
}

fn domain_explicitly_allowed(input: &SearchInput, domain: &str) -> bool {
    input
        .domains_allow
        .iter()
        .any(|allowed| domain_matches(domain, allowed))
}

#[cfg(test)]
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
        q.append_pair("page", &(input.cursor / input.top_k.max(1) + 1).to_string());
    }
    let res = client
        .get(url)
        .header("user-agent", "agent-system-web-search-extract")
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
            field_truncations: None,
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
        q.append_pair("page", &(input.cursor / input.top_k.max(1) + 1).to_string());
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
            field_truncations: None,
        });
        if out.len() >= max_items {
            break;
        }
    }
    out
}

fn search_bing_html(input: &SearchInput) -> Result<Vec<SearchItem>> {
    let client = backend_client(20, &["www.bing.com", "cn.bing.com"])?;
    let mut url = Url::parse("https://www.bing.com/search").expect("valid url");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("q", &input.query);
        q.append_pair("count", &candidate_window(input).to_string());
        q.append_pair("first", &input.cursor.saturating_add(1).to_string());
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
            field_truncations: None,
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
            let title_chars = it.title.chars().count();
            let snippet_chars = it.snippet.as_deref().map(str::chars).map(Iterator::count);
            let field_truncations = json!({
                "title": (title_chars > MAX_TITLE_CHARS).then(|| json!({
                    "truncated": true,
                    "original_chars": title_chars,
                    "returned_chars": MAX_TITLE_CHARS,
                    "recovery": "open_result_url",
                })),
                "snippet": snippet_chars.filter(|count| *count > MAX_SNIPPET_CHARS).map(|count| json!({
                    "truncated": true,
                    "original_chars": count,
                    "returned_chars": MAX_SNIPPET_CHARS,
                    "recovery": "open_result_url",
                })),
            });
            let has_field_truncation = field_truncations
                .as_object()
                .is_some_and(|fields| fields.values().any(|value| !value.is_null()));
            let field_truncations = has_field_truncation.then_some(field_truncations);
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
                field_truncations,
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

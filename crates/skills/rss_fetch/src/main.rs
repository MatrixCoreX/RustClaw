use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 单个 active source 的失败状态（持久化在 config 的 source_entries 中）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SourceStateEntry {
    url: String,
    #[serde(default)]
    failure_count: u32,
    #[serde(default)]
    last_error: String,
    #[serde(default)]
    last_failed_at: String,
}

/// 废弃区的一条记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeprecatedEntry {
    url: String,
    category: String,
    reason: String,
    failure_count: u32,
    #[serde(default)]
    last_error: String,
    deprecated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DeprecatedSection {
    #[serde(default)]
    sources: Vec<DeprecatedEntry>,
}

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    error_text: Option<String>,
}

#[derive(Debug, Clone)]
struct FeedItem {
    title: String,
    link: String,
    date: String,
    source: String,
    layer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RootConfig {
    #[serde(default)]
    rss: RssConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RssConfig {
    #[serde(default)]
    default_category: Option<String>,
    #[serde(default)]
    default_limit: Option<u64>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    /// 连续失败达到此次数后，该 source 会被移入 deprecated，默认 3。
    #[serde(default)]
    deprecate_after_failures: Option<u32>,
    #[serde(default)]
    categories: HashMap<String, RssCategoryConfig>,
    /// 废弃的 RSS 地址列表；默认抓取时不参与。
    #[serde(default)]
    deprecated: Option<DeprecatedSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RssCategoryConfig {
    /// 当前语义：全量抓取该列表中的所有源；无 primary/secondary/fallback 分层。
    #[serde(default)]
    sources: Option<Vec<String>>,
    /// 每个 source 的失败计数与最近错误（持久化）；达到阈值后从 sources 移入 deprecated。
    #[serde(default)]
    source_entries: Option<Vec<SourceStateEntry>>,
    /// 兼容旧配置：若未配置 sources，则使用 primary + secondary + fallback 合并为全量列表。
    #[serde(default)]
    primary: Vec<String>,
    #[serde(default)]
    secondary: Vec<String>,
    #[serde(default)]
    fallback: Vec<String>,
    #[serde(default)]
    output_language: Option<String>,
    #[serde(default)]
    bilingual_summary: Option<bool>,
}

/// 返回配置中所有废弃 URL 的集合；默认抓取时不抓这些。
fn deprecated_urls(cfg: &RootConfig) -> HashSet<String> {
    let mut set = HashSet::new();
    if let Some(ref dep) = cfg.rss.deprecated {
        for e in &dep.sources {
            set.insert(e.url.clone());
        }
    }
    set
}

/// 返回该 category 下要全量抓取的所有 feed URL 及其状态（排除 deprecated）；兼容旧 primary/secondary/fallback。
fn all_sources_with_state_for_category(cfg: &RootConfig, category: &str) -> Vec<(String, SourceStateEntry)> {
    let dep = deprecated_urls(cfg);
    let cat = match cfg.rss.categories.get(category) {
        Some(c) => c,
        None => return Vec::new(),
    };
    let urls: Vec<String> = if let Some(ref s) = cat.sources {
        if s.is_empty() {
            let mut out = Vec::new();
            out.extend(cat.primary.clone());
            out.extend(cat.secondary.clone());
            out.extend(cat.fallback.clone());
            out
        } else {
            s.clone()
        }
    } else {
        let mut out = Vec::new();
        out.extend(cat.primary.clone());
        out.extend(cat.secondary.clone());
        out.extend(cat.fallback.clone());
        out
    };
    let state_by_url: HashMap<String, SourceStateEntry> = cat
        .source_entries
        .as_ref()
        .map(|v| v.iter().map(|e| (e.url.clone(), e.clone())).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for url in urls {
        if !dep.contains(&url) && is_safe_feed_url(&url) {
            let state = state_by_url
                .get(&url)
                .cloned()
                .unwrap_or_else(|| SourceStateEntry {
                    url: url.clone(),
                    failure_count: 0,
                    last_error: String::new(),
                    last_failed_at: String::new(),
                });
            out.push((url, state));
        }
    }
    out
}

/// 仅返回 URL 列表（兼容旧调用）；排除 deprecated。测试与外部可读配置时使用。
#[allow(dead_code)]
fn all_sources_for_category(cfg: &RootConfig, category: &str) -> Vec<String> {
    all_sources_with_state_for_category(cfg, category)
        .into_iter()
        .map(|(u, _)| u)
        .collect()
}

#[derive(Debug, Clone)]
struct TextCatalog {
    current: HashMap<String, String>,
}

impl TextCatalog {
    fn for_lang(lang: &str) -> Self {
        let mut current = default_i18n_dict(lang);
        let lang_tag = normalize_lang_tag(lang);
        let path = workspace_root().join(format!("configs/i18n/rss_fetch.{lang_tag}.toml"));
        if let Some(external) = load_external_i18n(&path) {
            current.extend(external);
        }
        Self { current }
    }

    fn text_or(&self, key: &str, default: &str) -> String {
        self.current
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }

    fn render(&self, key: &str, vars: &[(&str, String)], default: &str) -> String {
        let mut out = self.text_or(key, default);
        for (k, v) in vars {
            out = out.replace(&format!("{{{k}}}"), v);
        }
        out
    }
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut cfg = load_root_config();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => {
                let result = execute(&mut cfg, req.args);
                if let Err(e) = save_config(&cfg) {
                    let _ = std::io::stderr().write_fmt(format_args!("rss_fetch save_config failed: {}\n", e));
                }
                match result {
                    Ok(text) => Resp {
                        request_id: req.request_id,
                        status: "ok".to_string(),
                        text,
                        error_text: None,
                    },
                    Err(err) => Resp {
                        request_id: req.request_id,
                        status: "error".to_string(),
                        text: String::new(),
                        error_text: Some(err),
                    },
                }
            }
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

/// Legacy / mistaken `action` names from older callers or schedules; normalized before dispatch.
/// Canonical actions remain `fetch`, `latest`, `news` (see INTERFACE.md).
fn normalize_rss_legacy_actions(args: &mut serde_json::Map<String, Value>) -> Result<(), String> {
    let action_raw = match args.get("action").and_then(|v| v.as_str()) {
        Some(s) => s.trim().to_ascii_lowercase(),
        None => return Ok(()),
    };
    if action_raw.is_empty() {
        return Ok(());
    }
    match action_raw.as_str() {
        "fetch_crypto_news" => {
            args.insert("action".to_string(), Value::String("latest".to_string()));
            if !args.contains_key("category") {
                args.insert("category".to_string(), Value::String("crypto".to_string()));
            }
            Ok(())
        }
        "fetch_tech_news" => {
            args.insert("action".to_string(), Value::String("latest".to_string()));
            if !args.contains_key("category") {
                args.insert("category".to_string(), Value::String("tech".to_string()));
            }
            Ok(())
        }
        "fetch_news" => {
            args.insert("action".to_string(), Value::String("latest".to_string()));
            Ok(())
        }
        "fetch_feed" => {
            if direct_feed_selector_present(args) {
                args.insert("action".to_string(), Value::String("fetch".to_string()));
            } else {
                return Err(
                    "fetch_feed requires url, feed_url, or feed_urls (direct feed only); use action=latest or action=news for category feeds"
                        .to_string(),
                );
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// True if args carry a non-empty direct URL selector (same intent as `fetch`).
fn direct_feed_selector_present(obj: &serde_json::Map<String, Value>) -> bool {
    if let Some(v) = obj.get("url").or_else(|| obj.get("feed_url")) {
        if let Some(s) = v.as_str() {
            if !s.trim().is_empty() {
                return true;
            }
        }
    }
    if let Some(arr) = obj.get("feed_urls").and_then(|v| v.as_array()) {
        return arr.iter().any(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()));
    }
    false
}

fn execute(cfg: &mut RootConfig, args: Value) -> Result<String, String> {
    let mut obj = args
        .as_object()
        .cloned()
        .ok_or_else(|| "args must be object".to_string())?;
    normalize_rss_legacy_actions(&mut obj)?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("latest")
        .trim()
        .to_ascii_lowercase();

    match action.as_str() {
        "fetch" => fetch_direct_feeds(&obj),
        "latest" | "news" => fetch_layered_news(cfg, &obj),
        _ => Err("unsupported action; use fetch|latest|news".to_string()),
    }
}

/// Direct-feed only: requires `url`, `feed_url`, or non-empty `feed_urls` (http/https). No category fallback.
fn resolve_direct_feed_urls(obj: &serde_json::Map<String, Value>) -> Result<Vec<String>, String> {
    if let Some(v) = obj.get("url").or_else(|| obj.get("feed_url")) {
        let s = v
            .as_str()
            .ok_or_else(|| "url and feed_url must be strings".to_string())?
            .trim();
        if s.is_empty() {
            return Err("fetch requires a non-empty url or feed_url".to_string());
        }
        if !is_safe_feed_url(s) {
            return Err("url must start with http:// or https://".to_string());
        }
        return Ok(vec![s.to_string()]);
    }
    if let Some(arr) = obj.get("feed_urls").and_then(|v| v.as_array()) {
        if arr.is_empty() {
            return Err("fetch requires a non-empty feed_urls array or url/feed_url".to_string());
        }
        let mut out = Vec::new();
        for v in arr {
            let s = v
                .as_str()
                .ok_or_else(|| "feed_urls entries must be strings".to_string())?
                .trim();
            if s.is_empty() {
                continue;
            }
            if !is_safe_feed_url(s) {
                return Err(format!(
                    "invalid feed URL (must start with http:// or https://): {s}"
                ));
            }
            out.push(s.to_string());
        }
        if out.is_empty() {
            return Err(
                "fetch requires at least one non-empty http(s) URL in feed_urls".to_string(),
            );
        }
        return Ok(out);
    }
    Err("fetch requires url, feed_url, or feed_urls".to_string())
}

fn fetch_direct_feeds(obj: &serde_json::Map<String, Value>) -> Result<String, String> {
    let urls = resolve_direct_feed_urls(obj)?;
    if urls.len() == 1 {
        return fetch_single_feed(obj, &urls[0]);
    }
    let mut parts = Vec::new();
    for url in &urls {
        parts.push(fetch_single_feed(obj, url)?);
    }
    Ok(parts.join("\n\n"))
}

fn fetch_single_feed(obj: &serde_json::Map<String, Value>, url: &str) -> Result<String, String> {
    if !is_safe_feed_url(url) {
        return Err("url must start with http:// or https://".to_string());
    }
    let limit = obj
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .clamp(1, 50) as usize;
    let timeout_seconds = obj
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(15)
        .clamp(3, 60);
    let xml = fetch_feed_xml(url, timeout_seconds)?;
    Ok(render_feed(&xml, limit))
}

fn now_iso_secs() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn fetch_layered_news(cfg: &mut RootConfig, obj: &serde_json::Map<String, Value>) -> Result<String, String> {
    let default_category = cfg
        .rss
        .default_category
        .as_deref()
        .unwrap_or("general")
        .trim()
        .to_ascii_lowercase();
    let category = obj
        .get("category")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
        .unwrap_or(default_category);
    let limit = obj
        .get("limit")
        .and_then(|v| v.as_u64())
        .or(cfg.rss.default_limit)
        .unwrap_or(10)
        .clamp(1, 50) as usize;
    let timeout_seconds = obj
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .or(cfg.rss.timeout_seconds)
        .unwrap_or(20)
        .clamp(3, 60);
    let output_language = obj
        .get("output_language")
        .or_else(|| obj.get("lang"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            cfg.rss
                .categories
                .get(&category)
                .and_then(|c| c.output_language.clone())
        })
        .unwrap_or_else(|| "zh-CN".to_string());
    let classify = obj
        .get("classify")
        .or_else(|| obj.get("classify_news"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let bilingual_summary = obj
        .get("bilingual_summary")
        .or_else(|| obj.get("zh_summary"))
        .and_then(|v| v.as_bool())
        .or_else(|| {
            cfg.rss
                .categories
                .get(&category)
                .and_then(|c| c.bilingual_summary)
        })
        .unwrap_or(false);
    let output_i18n = TextCatalog::for_lang(&output_language);
    let summary_i18n = TextCatalog::for_lang("zh-CN");
    let feed_urls = explicit_feed_urls(obj);

    let threshold = cfg.rss.deprecate_after_failures.unwrap_or(3);

    let (urls_with_state, is_explicit) = if !feed_urls.is_empty() {
        let urls: Vec<(String, SourceStateEntry)> = feed_urls
            .into_iter()
            .filter(|u| is_safe_feed_url(u))
            .map(|u| {
                (
                    u.clone(),
                    SourceStateEntry {
                        url: u,
                        failure_count: 0,
                        last_error: String::new(),
                        last_failed_at: String::new(),
                    },
                )
            })
            .collect();
        (urls, true)
    } else {
        let with_state = all_sources_with_state_for_category(cfg, &category);
        (with_state, false)
    };

    if urls_with_state.is_empty() {
        return Err(format!("no configured feeds for category={category}"));
    }

    let per_feed_limit = (limit * 3).max(20).min(100);
    let mut items = Vec::new();
    let mut success_count = 0usize;
    let mut failed_count = 0usize;
    let mut state_updates: HashMap<String, SourceStateEntry> = HashMap::new();
    let mut to_deprecate: Vec<DeprecatedEntry> = Vec::new();

    for (url, state) in &urls_with_state {
        let mut state = state.clone();
        match fetch_feed_xml(url, timeout_seconds) {
            Ok(xml) => {
                success_count += 1;
                state.failure_count = 0;
                state.last_error.clear();
                state.last_failed_at.clear();
                if !is_explicit {
                    state_updates.insert(url.clone(), state);
                }
                for mut item in parse_feed_items(&xml, per_feed_limit) {
                    item.source = url.clone();
                    item.layer = "feed".to_string();
                    items.push(item);
                }
            }
            Err(err_msg) => {
                failed_count += 1;
                if !is_explicit {
                    state.failure_count += 1;
                    state.last_error = err_msg.chars().take(200).collect::<String>();
                    state.last_failed_at = now_iso_secs();
                    if state.failure_count >= threshold {
                        to_deprecate.push(DeprecatedEntry {
                            url: url.clone(),
                            category: category.clone(),
                            reason: "consecutive_fetch_failures".to_string(),
                            failure_count: state.failure_count,
                            last_error: state.last_error.clone(),
                            deprecated_at: state.last_failed_at.clone(),
                        });
                    } else {
                        state_updates.insert(url.clone(), state);
                    }
                }
            }
        }
    }

    if !is_explicit && (!to_deprecate.is_empty() || !state_updates.is_empty()) {
        apply_deprecation_and_state(cfg, &category, &state_updates, &to_deprecate);
    }

    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for item in items {
        let key = format!(
            "{}|{}",
            compact_text(&item.link).to_ascii_lowercase(),
            compact_text(&item.title).to_ascii_lowercase()
        );
        if seen.insert(key) {
            deduped.push(item);
        }
    }
    sort_feed_items_by_date(&mut deduped);
    let items: Vec<FeedItem> = deduped.into_iter().take(limit).collect();

    if items.is_empty() {
        return Err(format!(
            "no feed items available: all {} source(s) failed or returned no items",
            urls_with_state.len()
        ));
    }

    let header = format!(
        "sources_ok={} sources_failed={} items={}\n",
        success_count,
        failed_count,
        items.len()
    );
    let body = format_layered_news_output(
        items,
        classify,
        bilingual_summary,
        &output_i18n,
        &summary_i18n,
    );
    Ok(header + &body)
}

/// 将本次运行产生的废弃项与状态更新写回 config（仅修改内存；调用方负责 save_config）。
fn apply_deprecation_and_state(
    cfg: &mut RootConfig,
    category: &str,
    state_updates: &HashMap<String, SourceStateEntry>,
    to_deprecate: &[DeprecatedEntry],
) {
    let dep_urls: HashSet<String> = to_deprecate.iter().map(|e| e.url.clone()).collect();
    let cat = match cfg.rss.categories.get_mut(category) {
        Some(c) => c,
        None => return,
    };

    let mut active_urls: Vec<String> = Vec::new();
    let mut entries: Vec<SourceStateEntry> = Vec::new();

    let existing_urls: Vec<String> = cat
        .sources
        .as_ref()
        .map(|s| s.clone())
        .unwrap_or_else(|| {
            let mut u = Vec::new();
            u.extend(cat.primary.clone());
            u.extend(cat.secondary.clone());
            u.extend(cat.fallback.clone());
            u
        });

    for url in &existing_urls {
        if dep_urls.contains(url) {
            continue;
        }
        active_urls.push(url.clone());
        if let Some(updated) = state_updates.get(url) {
            entries.push(updated.clone());
        } else if let Some(ref prev) = cat.source_entries {
            if let Some(prev_entry) = prev.iter().find(|e| &e.url == url) {
                entries.push(prev_entry.clone());
            }
        }
    }

    cat.sources = Some(active_urls);
    cat.source_entries = if entries.is_empty() { None } else { Some(entries) };

    let existing_dep_urls: HashSet<String> = cfg
        .rss
        .deprecated
        .as_ref()
        .map(|d| d.sources.iter().map(|e| e.url.clone()).collect())
        .unwrap_or_default();

    for entry in to_deprecate {
        if existing_dep_urls.contains(&entry.url) {
            continue;
        }
        cfg.rss.deprecated.get_or_insert_with(DeprecatedSection::default).sources.push(entry.clone());
    }
}

fn explicit_feed_urls(obj: &serde_json::Map<String, Value>) -> Vec<String> {
    if let Some(url) = obj
        .get("url")
        .or_else(|| obj.get("feed_url"))
        .and_then(|v| v.as_str())
    {
        return vec![url.trim().to_string()];
    }
    obj.get("feed_urls")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// 按 date 字符串降序（新在前）；无 date 的排到末尾。
fn sort_feed_items_by_date(items: &mut [FeedItem]) {
    items.sort_by(|a, b| {
        let a_empty = a.date.trim().is_empty();
        let b_empty = b.date.trim().is_empty();
        match (a_empty, b_empty) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => b.date.trim().cmp(a.date.trim()),
        }
    });
}

fn fetch_feed_xml(url: &str, timeout_seconds: u64) -> Result<String, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|err| format!("build http client failed: {err}"))?;

    let resp = client
        .get(url)
        .header("User-Agent", "RustClaw-RSS-Fetch/1.0")
        .send()
        .map_err(|err| format!("http request failed: {err}"))?;

    if !resp.status().is_success() {
        return Err(format!("http status is {}", resp.status()));
    }
    resp.text()
        .map_err(|err| format!("read response body failed: {err}"))
}

fn render_feed(xml: &str, limit: usize) -> String {
    if xml.to_ascii_lowercase().contains("<feed") {
        render_atom(xml, limit)
    } else {
        render_rss(xml, limit)
    }
}

fn parse_feed_items(xml: &str, limit: usize) -> Vec<FeedItem> {
    if xml.to_ascii_lowercase().contains("<feed") {
        parse_atom_items(xml, limit)
    } else {
        parse_rss_items(xml, limit)
    }
}

fn parse_rss_items(xml: &str, limit: usize) -> Vec<FeedItem> {
    let channel = extract_first_block(xml, "channel").unwrap_or(xml);
    let mut items = Vec::new();
    for blk in extract_blocks(channel, "item").into_iter().take(limit) {
        let title = extract_tag_text(blk, "title").unwrap_or_else(|| "(no title)".to_string());
        let link = extract_tag_text(blk, "link").unwrap_or_default();
        let date = extract_tag_text(blk, "pubDate")
            .or_else(|| extract_tag_text(blk, "dc:date"))
            .unwrap_or_default();
        items.push(FeedItem {
            title,
            link,
            date,
            source: String::new(),
            layer: String::new(),
        });
    }
    items
}

fn parse_atom_items(xml: &str, limit: usize) -> Vec<FeedItem> {
    let mut items = Vec::new();
    for blk in extract_blocks(xml, "entry").into_iter().take(limit) {
        let title = extract_tag_text(blk, "title").unwrap_or_else(|| "(no title)".to_string());
        let link = extract_atom_link(blk)
            .or_else(|| extract_tag_text(blk, "id"))
            .unwrap_or_default();
        let date = extract_tag_text(blk, "updated")
            .or_else(|| extract_tag_text(blk, "published"))
            .unwrap_or_default();
        items.push(FeedItem {
            title,
            link,
            date,
            source: String::new(),
            layer: String::new(),
        });
    }
    items
}

fn format_layered_news_output(
    items: Vec<FeedItem>,
    classify: bool,
    bilingual_summary: bool,
    output_i18n: &TextCatalog,
    summary_i18n: &TextCatalog,
) -> String {
    if classify {
        return format_classified_news_output(items, bilingual_summary, output_i18n, summary_i18n);
    }
    let mut out = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let host = source_host(&item.source);
        let cls = classify_news_topic(&item.title);
        let mut line = format!(
            "{}. [{}][{}] {}",
            idx + 1,
            item.layer,
            host,
            compact_text(&item.title)
        );
        if !item.date.is_empty() {
            line.push_str(&format!(" [{}]", compact_text(&item.date)));
        }
        out.push(line);
        if bilingual_summary {
            out.push(format!(
                "   {}",
                output_i18n.text_or("rss.msg.summary_header", "🧾 Summary:")
            ));
            for line in build_summary_lines(item, &host, cls, summary_i18n) {
                out.push(format!("   {}", line));
            }
        }
        if !item.link.is_empty() {
            out.push(format!(
                "   {} {}",
                output_i18n.text_or("rss.msg.link_prefix", "🔗"),
                compact_text(&item.link)
            ));
        }
    }
    out.join("\n")
}

fn format_classified_news_output(
    items: Vec<FeedItem>,
    bilingual_summary: bool,
    output_i18n: &TextCatalog,
    summary_i18n: &TextCatalog,
) -> String {
    let mut buckets: HashMap<&'static str, Vec<FeedItem>> = HashMap::new();
    let mut order = Vec::new();
    for item in items {
        let cls = classify_news_topic(&item.title);
        if !buckets.contains_key(cls) {
            order.push(cls);
        }
        buckets.entry(cls).or_default().push(item);
    }

    let mut out = Vec::new();
    for cls in order {
        let Some(group) = buckets.get(cls) else {
            continue;
        };
        let icon = class_emoji(cls);
        out.push(output_i18n.render(
            "rss.msg.class_header",
            &[
                ("icon", icon.to_string()),
                ("label", localized_class_label(cls, output_i18n)),
                ("count", group.len().to_string()),
            ],
            "{icon} {label} · {count} items",
        ));
        for (idx, item) in group.iter().enumerate() {
            let host = source_host(&item.source);
            let mut line = format!(
                "{}. [{}][{}] {}",
                idx + 1,
                item.layer,
                host,
                compact_text(&item.title)
            );
            if !item.date.is_empty() {
                line.push_str(&format!(" [{}]", compact_text(&item.date)));
            }
            out.push(line);
            if bilingual_summary {
                out.push(format!(
                    "   {}",
                    output_i18n.text_or("rss.msg.summary_header", "🧾 Summary:")
                ));
                for line in build_summary_lines(item, &host, cls, summary_i18n) {
                    out.push(format!("   {}", line));
                }
            }
            if !item.link.is_empty() {
                out.push(format!(
                    "   {} {}",
                    output_i18n.text_or("rss.msg.link_prefix", "🔗"),
                    compact_text(&item.link)
                ));
            }
        }
    }
    out.join("\n")
}

fn classify_news_topic(title: &str) -> &'static str {
    let t = title.to_ascii_lowercase();
    if has_any(&t, &["sec", "senate", "bill", "policy", "regulat", "ban", "cbdc"]) {
        return "policy_regulation";
    }
    if has_any(&t, &["hack", "exploit", "breach", "attack", "drain", "vulnerability"]) {
        return "security_incident";
    }
    if has_any(&t, &["ipo", "earnings", "results", "revenue", "funding", "acquire", "merger"]) {
        return "company_business";
    }
    if has_any(&t, &["etf", "inflation", "fed", "interest rate", "macro", "economy", "jobs"]) {
        return "macro_market";
    }
    if has_any(&t, &["upgrade", "launch", "mainnet", "protocol", "rollup", "layer 2", "l2"]) {
        return "tech_ecosystem";
    }
    "other"
}

fn has_any(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|k| text.contains(k))
}

fn localized_class_label(class_key: &str, i18n: &TextCatalog) -> String {
    i18n.text_or(&format!("rss.topic.{class_key}"), class_key)
}

fn class_emoji(class_key: &str) -> &'static str {
    match class_key {
        "policy_regulation" => "⚖️",
        "security_incident" => "🛡️",
        "company_business" => "🏢",
        "macro_market" => "📈",
        "tech_ecosystem" => "🧪",
        _ => "📰",
    }
}

fn source_host(url: &str) -> String {
    let trimmed = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    trimmed
        .split('/')
        .next()
        .map(str::to_string)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn build_summary_lines(item: &FeedItem, host: &str, class_key: &str, i18n: &TextCatalog) -> Vec<String> {
    let class_label = localized_class_label(class_key, i18n);
    let mut lines = Vec::new();
    lines.push(i18n.render(
        "rss.msg.summary_from",
        &[("host", host.to_string())],
        "From {host}.",
    ));
    if item.date.is_empty() {
        lines.push(i18n.render(
            "rss.msg.summary_topic",
            &[("topic", class_label.clone())],
            "Topic: {topic}.",
        ));
    } else {
        lines.push(i18n.render(
            "rss.msg.summary_time",
            &[("time", compact_text(&item.date))],
            "Time: {time}.",
        ));
        lines.push(i18n.render(
            "rss.msg.summary_topic",
            &[("topic", class_label)],
            "Topic: {topic}.",
        ));
    }
    lines.push(i18n.text_or(
        "rss.msg.summary_tip",
        "Please check the original title and link for details.",
    ));
    lines
}

fn render_rss(xml: &str, limit: usize) -> String {
    let channel = extract_first_block(xml, "channel").unwrap_or(xml);
    let feed_title = extract_tag_text(channel, "title").unwrap_or_else(|| "(untitled)".to_string());
    let feed_link = extract_tag_text(channel, "link").unwrap_or_default();
    let mut items = Vec::new();
    for blk in extract_blocks(channel, "item").into_iter().take(limit) {
        let title = extract_tag_text(blk, "title").unwrap_or_else(|| "(no title)".to_string());
        let link = extract_tag_text(blk, "link").unwrap_or_default();
        let date = extract_tag_text(blk, "pubDate")
            .or_else(|| extract_tag_text(blk, "dc:date"))
            .unwrap_or_default();
        items.push(FeedItem {
            title,
            link,
            date,
            source: String::new(),
            layer: String::new(),
        });
    }
    format_feed_output(feed_title, feed_link, items)
}

fn render_atom(xml: &str, limit: usize) -> String {
    let feed_title = extract_tag_text(xml, "title").unwrap_or_else(|| "(untitled)".to_string());
    let feed_link = extract_atom_link(xml).unwrap_or_default();
    let mut items = Vec::new();
    for blk in extract_blocks(xml, "entry").into_iter().take(limit) {
        let title = extract_tag_text(blk, "title").unwrap_or_else(|| "(no title)".to_string());
        let link = extract_atom_link(blk)
            .or_else(|| extract_tag_text(blk, "id"))
            .unwrap_or_default();
        let date = extract_tag_text(blk, "updated")
            .or_else(|| extract_tag_text(blk, "published"))
            .unwrap_or_default();
        items.push(FeedItem {
            title,
            link,
            date,
            source: String::new(),
            layer: String::new(),
        });
    }
    format_feed_output(feed_title, feed_link, items)
}

fn format_feed_output(feed_title: String, feed_link: String, items: Vec<FeedItem>) -> String {
    let mut out = Vec::new();
    out.push(format!("feed_title={}", compact_text(&feed_title)));
    if !feed_link.is_empty() {
        out.push(format!("feed_link={}", compact_text(&feed_link)));
    }
    out.push(format!("item_count={}", items.len()));
    out.push("items:".to_string());

    for (idx, item) in items.iter().enumerate() {
        let mut line = format!("{}. {}", idx + 1, compact_text(&item.title));
        if !item.date.is_empty() {
            line.push_str(&format!(" [{}]", compact_text(&item.date)));
        }
        out.push(line);
        if !item.link.is_empty() {
            out.push(format!("   🔗 {}", compact_text(&item.link)));
        }
    }

    out.join("\n")
}

fn is_safe_feed_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn extract_first_block<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    extract_blocks(xml, tag).into_iter().next()
}

fn extract_blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut pos = 0usize;
    while let Some(start_rel) = xml[pos..].find(&open) {
        let start = pos + start_rel;
        let gt_rel = match xml[start..].find('>') {
            Some(v) => v,
            None => break,
        };
        let body_start = start + gt_rel + 1;
        let end_rel = match xml[body_start..].find(&close) {
            Some(v) => v,
            None => break,
        };
        let end = body_start + end_rel;
        out.push(&xml[body_start..end]);
        pos = end + close.len();
    }
    out
}

fn extract_tag_text(xml: &str, tag: &str) -> Option<String> {
    extract_blocks(xml, tag)
        .into_iter()
        .next()
        .map(|v| {
            let raw = v.trim();
            let stripped = strip_cdata(raw);
            xml_unescape(stripped.trim())
        })
}

fn extract_atom_link(xml: &str) -> Option<String> {
    let mut pos = 0usize;
    while let Some(start_rel) = xml[pos..].find("<link") {
        let start = pos + start_rel;
        let end_rel = xml[start..].find('>')?;
        let head = &xml[start..start + end_rel + 1];
        if let Some(href) = extract_attr(head, "href") {
            if !href.is_empty() {
                return Some(xml_unescape(&href));
            }
        }
        pos = start + end_rel + 1;
    }
    None
}

fn extract_attr(tag_text: &str, attr: &str) -> Option<String> {
    let key = format!("{attr}=\"");
    let start = tag_text.find(&key)? + key.len();
    let rem = &tag_text[start..];
    let end = rem.find('"')?;
    Some(rem[..end].to_string())
}

fn compact_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_cdata(input: &str) -> &str {
    input
        .strip_prefix("<![CDATA[")
        .and_then(|s| s.strip_suffix("]]>"))
        .unwrap_or(input)
}

fn xml_unescape(input: &str) -> String {
    input
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn normalize_lang_tag(lang: &str) -> &'static str {
    let l = lang.to_ascii_lowercase();
    if l.starts_with("zh") {
        "zh-CN"
    } else {
        "en-US"
    }
}

fn default_i18n_dict(_lang: &str) -> HashMap<String, String> {
    let mut current = HashMap::new();
    current.insert("rss.msg.summary_header".to_string(), "🧾 Summary:".to_string());
    current.insert("rss.msg.summary_from".to_string(), "From {host}.".to_string());
    current.insert("rss.msg.summary_time".to_string(), "Time: {time}.".to_string());
    current.insert("rss.msg.summary_topic".to_string(), "Topic: {topic}.".to_string());
    current.insert(
        "rss.msg.summary_tip".to_string(),
        "Please check the original title and link for details.".to_string(),
    );
    current.insert(
        "rss.msg.class_header".to_string(),
        "{icon} {label} · {count} items".to_string(),
    );
    current.insert("rss.msg.link_prefix".to_string(), "🔗".to_string());
    current.insert(
        "rss.topic.policy_regulation".to_string(),
        "Policy & Regulation".to_string(),
    );
    current.insert(
        "rss.topic.security_incident".to_string(),
        "Security Incident".to_string(),
    );
    current.insert(
        "rss.topic.company_business".to_string(),
        "Company & Business".to_string(),
    );
    current.insert("rss.topic.macro_market".to_string(), "Macro & Market".to_string());
    current.insert("rss.topic.tech_ecosystem".to_string(), "Tech & Ecosystem".to_string());
    current.insert("rss.topic.other".to_string(), "Other".to_string());
    current
}

fn flatten_toml_table(prefix: &str, table: &toml::map::Map<String, toml::Value>, out: &mut HashMap<String, String>) {
    for (k, v) in table {
        let key = if prefix.is_empty() {
            k.to_string()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            toml::Value::String(text) => {
                out.insert(key, text.to_string());
            }
            toml::Value::Table(child) => {
                flatten_toml_table(&key, child, out);
            }
            _ => {}
        }
    }
}

fn load_external_i18n(path: &Path) -> Option<HashMap<String, String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: toml::Value = toml::from_str(&raw).ok()?;
    let mut out = HashMap::new();
    if let Some(dict) = value.get("dict").and_then(|v| v.as_table()) {
        flatten_toml_table("", dict, &mut out);
        return Some(out);
    }
    if let Some(root) = value.as_table() {
        flatten_toml_table("", root, &mut out);
        if out.is_empty() {
            return None;
        }
        return Some(out);
    }
    None
}

fn load_root_config() -> RootConfig {
    let root = workspace_root();
    let rss_path = root.join("configs/rss.toml");
    if let Ok(raw) = std::fs::read_to_string(&rss_path) {
        if let Ok(parsed) = toml::from_str::<RootConfig>(&raw) {
            return parsed;
        }
    }
    RootConfig::default()
}

fn save_config(cfg: &RootConfig) -> Result<(), String> {
    let path = workspace_root().join("configs/rss.toml");
    let s = toml::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, s).map_err(|e| e.to_string())?;
    Ok(())
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cfg_with_sources(category: &str, sources: Vec<String>) -> RootConfig {
        let mut categories = HashMap::new();
        let mut cat = RssCategoryConfig::default();
        cat.sources = Some(sources);
        categories.insert(category.to_string(), cat);
        RootConfig {
            rss: RssConfig {
                default_category: Some("general".to_string()),
                default_limit: Some(10),
                timeout_seconds: Some(20),
                deprecate_after_failures: Some(3),
                categories,
                deprecated: None,
            },
        }
    }

    fn make_cfg_legacy_only(category: &str, primary: Vec<String>, secondary: Vec<String>, fallback: Vec<String>) -> RootConfig {
        let mut categories = HashMap::new();
        let cat = RssCategoryConfig {
            sources: None,
            source_entries: None,
            primary,
            secondary,
            fallback,
            output_language: None,
            bilingual_summary: None,
        };
        categories.insert(category.to_string(), cat);
        RootConfig {
            rss: RssConfig {
                default_category: Some("general".to_string()),
                default_limit: Some(10),
                timeout_seconds: Some(20),
                deprecate_after_failures: None,
                categories,
                deprecated: None,
            },
        }
    }

    #[test]
    fn all_sources_for_category_uses_sources_when_present() {
        let cfg = make_cfg_with_sources("crypto", vec!["https://a.com/feed".to_string(), "https://b.com/rss".to_string()]);
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
            FeedItem { title: "a".into(), link: "l1".into(), date: "2020-01-01".into(), source: String::new(), layer: String::new() },
            FeedItem { title: "b".into(), link: "l2".into(), date: "2022-06-15".into(), source: String::new(), layer: String::new() },
            FeedItem { title: "c".into(), link: "l3".into(), date: "2021-03-10".into(), source: String::new(), layer: String::new() },
        ];
        sort_feed_items_by_date(&mut items);
        assert_eq!(items[0].date, "2022-06-15");
        assert_eq!(items[1].date, "2021-03-10");
        assert_eq!(items[2].date, "2020-01-01");
    }

    #[test]
    fn sort_feed_items_by_date_empty_dates_at_end() {
        let mut items = vec![
            FeedItem { title: "a".into(), link: "l1".into(), date: "2022-01-01".into(), source: String::new(), layer: String::new() },
            FeedItem { title: "b".into(), link: "l2".into(), date: "".into(), source: String::new(), layer: String::new() },
            FeedItem { title: "c".into(), link: "l3".into(), date: "2021-01-01".into(), source: String::new(), layer: String::new() },
        ];
        sort_feed_items_by_date(&mut items);
        assert_eq!(items[0].date, "2022-01-01");
        assert_eq!(items[1].date, "2021-01-01");
        assert_eq!(items[2].date, "");
    }

    #[test]
    fn all_sources_fail_returns_err() {
        let mut cfg = make_cfg_with_sources("fail_cat", vec!["https://nonexistent.invalid.example/feed".to_string()]);
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
    fn legacy_fetch_crypto_news_matches_latest_crypto_category() {
        let mut cfg_a = make_cfg_with_sources(
            "crypto",
            vec!["https://nonexistent.invalid.example/feed".to_string()],
        );
        let a = serde_json::json!({
            "action": "fetch_crypto_news",
            "timeout_seconds": 1,
            "limit": 3
        });
        let ra = execute(
            &mut cfg_a,
            serde_json::Value::Object(a.as_object().unwrap().clone()),
        );
        let mut cfg_b = make_cfg_with_sources(
            "crypto",
            vec!["https://nonexistent.invalid.example/feed".to_string()],
        );
        let b = serde_json::json!({
            "action": "latest",
            "category": "crypto",
            "timeout_seconds": 1,
            "limit": 3
        });
        let rb = execute(
            &mut cfg_b,
            serde_json::Value::Object(b.as_object().unwrap().clone()),
        );
        assert_eq!(ra.is_err(), rb.is_err());
        let ea = ra.unwrap_err();
        let eb = rb.unwrap_err();
        assert!(ea.contains("no feed items") || ea.contains("failed") || ea.contains("all"));
        assert!(eb.contains("no feed items") || eb.contains("failed") || eb.contains("all"));
    }

    #[test]
    fn legacy_fetch_tech_news_sets_tech_category() {
        let mut cfg_a = make_cfg_with_sources(
            "tech",
            vec!["https://nonexistent.invalid.example/feed".to_string()],
        );
        let a = serde_json::json!({
            "action": "fetch_tech_news",
            "timeout_seconds": 1,
            "limit": 3
        });
        let ra = execute(
            &mut cfg_a,
            serde_json::Value::Object(a.as_object().unwrap().clone()),
        );
        let mut cfg_b = make_cfg_with_sources(
            "tech",
            vec!["https://nonexistent.invalid.example/feed".to_string()],
        );
        let b = serde_json::json!({
            "action": "latest",
            "category": "tech",
            "timeout_seconds": 1,
            "limit": 3
        });
        let rb = execute(
            &mut cfg_b,
            serde_json::Value::Object(b.as_object().unwrap().clone()),
        );
        assert_eq!(ra.is_err(), rb.is_err());
    }

    #[test]
    fn legacy_fetch_news_matches_latest_same_category() {
        let mut cfg_a = make_cfg_with_sources(
            "general",
            vec!["https://nonexistent.invalid.example/feed".to_string()],
        );
        let a = serde_json::json!({
            "action": "fetch_news",
            "timeout_seconds": 1,
            "limit": 3
        });
        let ra = execute(
            &mut cfg_a,
            serde_json::Value::Object(a.as_object().unwrap().clone()),
        );
        let mut cfg_b = make_cfg_with_sources(
            "general",
            vec!["https://nonexistent.invalid.example/feed".to_string()],
        );
        let b = serde_json::json!({
            "action": "latest",
            "category": "general",
            "timeout_seconds": 1,
            "limit": 3
        });
        let rb = execute(
            &mut cfg_b,
            serde_json::Value::Object(b.as_object().unwrap().clone()),
        );
        assert_eq!(ra.is_err(), rb.is_err());
    }

    #[test]
    fn legacy_fetch_feed_without_url_errors() {
        let mut cfg = make_cfg_with_sources("g", vec!["https://example.com/f".to_string()]);
        let args = serde_json::json!({ "action": "fetch_feed" });
        let r = execute(&mut cfg, args);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("fetch_feed"));
    }

    #[test]
    fn legacy_fetch_feed_with_url_uses_direct_fetch_path() {
        let mut cfg = make_cfg_with_sources("unused", vec![]);
        let args = serde_json::json!({
            "action": "fetch_feed",
            "url": "https://nonexistent.invalid.example/feed.xml",
            "timeout_seconds": 1
        });
        let r = execute(&mut cfg, serde_json::Value::Object(args.as_object().unwrap().clone()));
        assert!(r.is_err());
        let err = r.unwrap_err();
        assert!(
            !err.contains("unsupported action"),
            "should normalize to fetch, not reject action: {err}"
        );
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
        let mut cfg = make_cfg_with_sources("omitted_cat", vec!["https://nonexistent.invalid.example/feed".to_string()]);
        let args = serde_json::json!({
            "category": "omitted_cat",
            "limit": 5,
            "timeout_seconds": 1
        });
        let result = execute(&mut cfg, serde_json::Value::Object(args.as_object().unwrap().clone()));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("no feed items") || err.contains("all") || err.contains("failed"),
            "expected layered-news failure, got: {err}"
        );
    }

    #[test]
    fn news_action_same_error_shape_as_latest_for_failed_sources() {
        let mut cfg_latest = make_cfg_with_sources("cat", vec!["https://nonexistent.invalid.example/feed".to_string()]);
        let latest_args = serde_json::json!({"action": "latest", "category": "cat", "timeout_seconds": 1, "limit": 3});
        let r_latest = execute(
            &mut cfg_latest,
            serde_json::Value::Object(latest_args.as_object().unwrap().clone()),
        );
        let mut cfg_news = make_cfg_with_sources("cat", vec!["https://nonexistent.invalid.example/feed".to_string()]);
        let news_args = serde_json::json!({"action": "news", "category": "cat", "timeout_seconds": 1, "limit": 3});
        let r_news = execute(
            &mut cfg_news,
            serde_json::Value::Object(news_args.as_object().unwrap().clone()),
        );
        assert_eq!(r_latest.is_err(), r_news.is_err());
    }

    #[test]
    fn registry_prompt_file_rss_fetch_md_exists() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let p = repo.join("prompts/vendors/default/skills/rss_fetch.md");
        assert!(
            p.is_file(),
            "skills_registry prompt_file should exist: {}",
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
        let text = result.unwrap();
        assert!(!text.trim().is_empty());
    }

    #[test]
    fn deprecated_urls_excluded_from_all_sources() {
        let mut cfg = make_cfg_with_sources("c", vec!["https://a.com".to_string(), "https://b.com".to_string()]);
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
        let mut cfg = make_cfg_with_sources("one", vec!["https://nonexistent.invalid.example/feed".to_string()]);
        cfg.rss.deprecate_after_failures = Some(3);
        let args = serde_json::json!({"action": "latest", "category": "one", "limit": 5, "timeout_seconds": 1});
        let _ = execute(&mut cfg, serde_json::Value::Object(args.as_object().unwrap().clone()));
        assert!(cfg.rss.deprecated.as_ref().map(|d| d.sources.is_empty()).unwrap_or(true));
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
        let dep_count = cfg.rss.deprecated.as_ref().map(|d| d.sources.len()).unwrap_or(0);
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
        let entries = cfg.rss.categories.get("c").and_then(|c| c.source_entries.as_ref()).unwrap();
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
            let _ = execute(&mut cfg, serde_json::Value::Object(args.as_object().unwrap().clone()));
        }
        assert!(
            cfg.rss.deprecated.as_ref().map(|d| d.sources.len()).unwrap_or(0) >= 1,
            "expected url to be in deprecated after 3 consecutive failures"
        );
        let urls = all_sources_for_category(&cfg, "fail3");
        assert!(urls.is_empty(), "deprecated source should no longer be in active sources");
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
        assert!(result.is_ok(), "expected ok when at least one source succeeds: {:?}", result);
        let text = result.unwrap();
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
        let text = result.unwrap();
        assert!(text.starts_with("sources_ok="));
        assert!(text.contains("items=2") || text.contains("items=1"));
    }
}

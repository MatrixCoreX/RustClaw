use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, Deserialize, Default)]
struct RootConfig {
    #[serde(default)]
    rss: RssConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RssConfig {
    #[serde(default)]
    default_category: Option<String>,
    #[serde(default)]
    default_limit: Option<u64>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    categories: HashMap<String, RssCategoryConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RssCategoryConfig {
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
    let cfg = load_root_config();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(&cfg, req.args) {
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
            },
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

fn execute(cfg: &RootConfig, args: Value) -> Result<String, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("fetch")
        .trim()
        .to_ascii_lowercase();

    match action.as_str() {
        "fetch" => {
            if let Some(url) = obj
                .get("url")
                .or_else(|| obj.get("feed_url"))
                .and_then(|v| v.as_str())
            {
                fetch_single_feed(obj, url)
            } else {
                fetch_layered_news(cfg, obj)
            }
        }
        "latest" | "news" => fetch_layered_news(cfg, obj),
        _ => Err("unsupported action; use fetch|latest|news".to_string()),
    }
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

fn fetch_layered_news(cfg: &RootConfig, obj: &serde_json::Map<String, Value>) -> Result<String, String> {
    let default_category = cfg
        .rss
        .default_category
        .as_deref()
        .unwrap_or("crypto")
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
        .unwrap_or(5)
        .clamp(1, 50) as usize;
    let timeout_seconds = obj
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .or(cfg.rss.timeout_seconds)
        .unwrap_or(15)
        .clamp(3, 60);
    let source_layer = obj
        .get("source_layer")
        .or_else(|| obj.get("layer"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "all".to_string());
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

    let mut layered_sources = if !feed_urls.is_empty() {
        vec![("custom".to_string(), feed_urls)]
    } else {
        build_layered_sources(cfg, &category, &source_layer)
    };
    if layered_sources.is_empty() {
        return Err(format!("no configured feeds for category={category} layer={source_layer}"));
    }

    let mut dedupe = HashSet::new();
    let mut items = Vec::new();
    for (layer, urls) in layered_sources.drain(..) {
        for url in urls {
            if !is_safe_feed_url(&url) {
                continue;
            }
            let xml = match fetch_feed_xml(&url, timeout_seconds) {
                Ok(v) => v,
                Err(_) => continue,
            };
            for mut item in parse_feed_items(&xml, limit) {
                item.layer = layer.clone();
                item.source = url.clone();
                let key = format!(
                    "{}|{}",
                    compact_text(&item.title).to_ascii_lowercase(),
                    compact_text(&item.link).to_ascii_lowercase()
                );
                if dedupe.insert(key) {
                    items.push(item);
                    if items.len() >= limit {
                        return Ok(format_layered_news_output(
                            items,
                            classify,
                            bilingual_summary,
                            &output_i18n,
                            &summary_i18n,
                        ));
                    }
                }
            }
        }
    }
    if items.is_empty() {
        return Err("no feed items available from configured sources".to_string());
    }
    Ok(format_layered_news_output(
        items,
        classify,
        bilingual_summary,
        &output_i18n,
        &summary_i18n,
    ))
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

fn build_layered_sources(cfg: &RootConfig, category: &str, source_layer: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let cat = cfg.rss.categories.get(category);
    let primary = cat.map(|c| c.primary.clone()).unwrap_or_default();
    let secondary = cat.map(|c| c.secondary.clone()).unwrap_or_default();
    let fallback = cat.map(|c| c.fallback.clone()).unwrap_or_default();
    match source_layer {
        "primary" | "tier1" => {
            if !primary.is_empty() {
                out.push(("primary".to_string(), primary));
            }
        }
        "secondary" | "tier2" => {
            if !secondary.is_empty() {
                out.push(("secondary".to_string(), secondary));
            }
        }
        "fallback" | "tier3" => {
            if !fallback.is_empty() {
                out.push(("fallback".to_string(), fallback));
            }
        }
        _ => {
            if !primary.is_empty() {
                out.push(("primary".to_string(), primary));
            }
            if !secondary.is_empty() {
                out.push(("secondary".to_string(), secondary));
            }
            if !fallback.is_empty() {
                out.push(("fallback".to_string(), fallback));
            }
        }
    }
    out
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

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf()))
}

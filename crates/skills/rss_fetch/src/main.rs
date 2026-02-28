use std::io::{self, BufRead, Write};
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
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args) {
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

fn execute(args: Value) -> Result<String, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("fetch");

    match action {
        "fetch" => {
            let url = obj
                .get("url")
                .or_else(|| obj.get("feed_url"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "url is required".to_string())?;
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
            let text = render_feed(&xml, limit);
            Ok(text)
        }
        _ => Err("unsupported action; use fetch".to_string()),
    }
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
        items.push(FeedItem { title, link, date });
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
        items.push(FeedItem { title, link, date });
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
            out.push(format!("   {}", compact_text(&item.link)));
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
        .map(|v| xml_unescape(v.trim()))
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

fn xml_unescape(input: &str) -> String {
    input
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

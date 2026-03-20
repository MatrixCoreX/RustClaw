use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

#[derive(Debug, Deserialize)]
struct Request {
    request_id: String,
    args: Value,
    #[allow(dead_code)]
    context: Option<Value>,
    #[allow(dead_code)]
    user_id: i64,
    #[allow(dead_code)]
    chat_id: i64,
}

#[derive(Debug, Serialize)]
struct Response {
    request_id: String,
    status: String,
    text: String,
    error_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    buttons: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct OpenExtractArgs {
    action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    urls: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_pages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    save_screenshot: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screenshot_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_text_chars: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fail_fast: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait_map_path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SearchPageArgs {
    action: String,
    query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    engine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lang: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SearchExtractArgs {
    action: String,
    query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    engine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extract_top_n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summarize: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_text_chars: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fail_fast: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lang: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Request, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => handle(req),
            Err(err) => Response {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                error_text: Some(format!("invalid input: {err}")),
                buttons: None,
                extra: None,
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }

    Ok(())
}

fn handle(req: Request) -> Response {
    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Fix blocker 1: Handle args.as_object() with consistent return type
    let obj = match req.args.as_object() {
        Some(o) => o,
        None => {
            return Response {
                request_id: req.request_id,
                status: "error".to_string(),
                text: String::new(),
                error_text: Some("args must be object".to_string()),
                buttons: None,
                extra: None,
            };
        }
    };

    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "action is required".to_string());

    let action = match action {
        Ok(a) => a,
        Err(err) => {
            return Response {
                request_id: req.request_id,
                status: "error".to_string(),
                text: String::new(),
                error_text: Some(err),
                buttons: None,
                extra: None,
            };
        }
    };

    let result = match action {
        "open_extract" => {
            let args = parse_open_extract_args(obj);
            match args {
                Ok(a) => open_extract_action(&workspace_root, a),
                Err(e) => Err(e),
            }
        }
        "search_page" => {
            let args = parse_search_page_args(obj);
            match args {
                Ok(a) => search_page_action(&workspace_root, a),
                Err(e) => Err(e),
            }
        }
        "search_extract" => {
            let args = parse_search_extract_args(obj);
            match args {
                Ok(a) => search_extract_action(&workspace_root, a),
                Err(e) => Err(e),
            }
        }
        other => Err(format!("unknown action: {other}; allowed: open_extract|search_page|search_extract")),
    };

    match result {
        Ok(text) => Response {
            request_id: req.request_id,
            status: "ok".to_string(),
            text,
            error_text: None,
            buttons: None,
            extra: None,
        },
        Err(err) => Response {
            request_id: req.request_id,
            status: "error".to_string(),
            text: String::new(),
            error_text: Some(err),
            buttons: None,
            extra: None,
        },
    }
}

fn parse_open_extract_args(obj: &serde_json::Map<String, Value>) -> Result<OpenExtractArgs, String> {
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "action is required".to_string())?
        .to_string();

    let url = obj.get("url").and_then(|v| v.as_str()).map(|s| s.to_string());
    let urls = obj
        .get("urls")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });

    if url.is_none() && urls.as_ref().map_or(true, Vec::is_empty) {
        return Err("at least one of url or urls is required".to_string());
    }

    let max_pages = if let Some(v) = obj.get("max_pages") {
        let val = v.as_u64().ok_or_else(|| "max_pages must be an integer".to_string())?;
        if val < 1 || val > 10 {
            return Err(format!("max_pages must be between 1 and 10, got {}", val));
        }
        val as u32
    } else {
        3
    };

    let wait_until = obj
        .get("wait_until")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "domcontentloaded".to_string());

    if !matches!(wait_until.as_str(), "domcontentloaded" | "load" | "networkidle") {
        return Err(format!("wait_until must be one of: domcontentloaded, load, networkidle"));
    }

    let save_screenshot = obj.get("save_screenshot").and_then(|v| v.as_bool()).unwrap_or(true);

    let screenshot_dir = obj
        .get("screenshot_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("image/browser_web")
        .to_string();
    let content_mode = obj
        .get("content_mode")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "clean".to_string());
    if !matches!(content_mode.as_str(), "clean" | "raw") {
        return Err("content_mode must be one of: clean, raw".to_string());
    }
    let max_text_chars = if let Some(v) = obj.get("max_text_chars") {
        let val = v
            .as_u64()
            .ok_or_else(|| "max_text_chars must be an integer".to_string())?;
        if val < 100 || val > 200_000 {
            return Err(format!(
                "max_text_chars must be between 100 and 200000, got {}",
                val
            ));
        }
        val as u32
    } else {
        12_000
    };
    let fail_fast = obj
        .get("fail_fast")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let wait_map_path = obj
        .get("wait_map_path")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    Ok(OpenExtractArgs {
        action,
        url,
        urls,
        max_pages: Some(max_pages),
        wait_until: Some(wait_until),
        save_screenshot: Some(save_screenshot),
        screenshot_dir: Some(screenshot_dir),
        content_mode: Some(content_mode),
        max_text_chars: Some(max_text_chars),
        fail_fast: Some(fail_fast),
        wait_map_path,
    })
}

fn parse_search_page_args(obj: &serde_json::Map<String, Value>) -> Result<SearchPageArgs, String> {
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "action is required".to_string())?
        .to_string();

    let query = obj
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "query is required".to_string())?
        .trim()
        .to_string();

    if query.is_empty() {
        return Err("query must not be empty".to_string());
    }

    let engine = obj
        .get("engine")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "google".to_string());

    if engine != "google" {
        return Err(format!("unsupported engine: {engine}; only 'google' is supported"));
    }

    let top_k = if let Some(v) = obj.get("top_k") {
        let val = v.as_u64().ok_or_else(|| "top_k must be an integer".to_string())?;
        if val < 1 || val > 20 {
            return Err(format!("top_k must be between 1 and 20, got {}", val));
        }
        val as u32
    } else {
        5
    };
    let region = obj
        .get("region")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let lang = obj
        .get("lang")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| Some("en".to_string()));

    Ok(SearchPageArgs {
        action,
        query,
        engine: Some(engine),
        top_k: Some(top_k),
        region,
        lang,
    })
}

fn parse_search_extract_args(obj: &serde_json::Map<String, Value>) -> Result<SearchExtractArgs, String> {
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "action is required".to_string())?
        .to_string();

    let query = obj
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "query is required".to_string())?
        .trim()
        .to_string();

    if query.is_empty() {
        return Err("query must not be empty".to_string());
    }

    let engine = obj
        .get("engine")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "google".to_string());

    if engine != "google" {
        return Err(format!("unsupported engine: {engine}; only 'google' is supported"));
    }

    let top_k = if let Some(v) = obj.get("top_k") {
        let val = v.as_u64().ok_or_else(|| "top_k must be an integer".to_string())?;
        if val < 1 || val > 20 {
            return Err(format!("top_k must be between 1 and 20, got {}", val));
        }
        val as u32
    } else {
        5
    };

    let extract_top_n = if let Some(v) = obj.get("extract_top_n") {
        let val = v.as_u64().ok_or_else(|| "extract_top_n must be an integer".to_string())?;
        if val < 1 || val > 10 {
            return Err(format!("extract_top_n must be between 1 and 10, got {}", val));
        }
        val as u32
    } else {
        3
    };

    let wait_until = obj
        .get("wait_until")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "domcontentloaded".to_string());

    if !matches!(wait_until.as_str(), "domcontentloaded" | "load" | "networkidle") {
        return Err(format!("wait_until must be one of: domcontentloaded, load, networkidle"));
    }
    let summarize = obj.get("summarize").and_then(|v| v.as_bool()).unwrap_or(true);
    let content_mode = obj
        .get("content_mode")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "clean".to_string());
    if !matches!(content_mode.as_str(), "clean" | "raw") {
        return Err("content_mode must be one of: clean, raw".to_string());
    }
    let max_text_chars = if let Some(v) = obj.get("max_text_chars") {
        let val = v
            .as_u64()
            .ok_or_else(|| "max_text_chars must be an integer".to_string())?;
        if val < 100 || val > 200_000 {
            return Err(format!(
                "max_text_chars must be between 100 and 200000, got {}",
                val
            ));
        }
        val as u32
    } else {
        12_000
    };
    let fail_fast = obj
        .get("fail_fast")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let region = obj
        .get("region")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let lang = obj
        .get("lang")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| Some("en".to_string()));

    Ok(SearchExtractArgs {
        action,
        query,
        engine: Some(engine),
        top_k: Some(top_k),
        extract_top_n: Some(extract_top_n),
        wait_until: Some(wait_until),
        summarize: Some(summarize),
        content_mode: Some(content_mode),
        max_text_chars: Some(max_text_chars),
        fail_fast: Some(fail_fast),
        region,
        lang,
    })
}

fn open_extract_action(workspace_root: &PathBuf, args: OpenExtractArgs) -> Result<String, String> {
    let mut urls = Vec::new();
    if let Some(url) = args.url {
        urls.push(url);
    }
    if let Some(urls_list) = args.urls {
        urls.extend(urls_list);
    }

    if urls.is_empty() {
        return Err("at least one URL is required".to_string());
    }

    // Validate URLs
    for url_str in &urls {
        let url = Url::parse(url_str).map_err(|e| format!("invalid URL '{}': {}", url_str, e))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(format!("URL must be http or https: {}", url_str));
        }
    }

    // Limit to max_pages
    let max_pages = args.max_pages.unwrap_or(3);
    urls.truncate(max_pages as usize);

    let helper_input = json!({
        "action": "openExtract",
        "urls": urls,
        "waitUntil": args.wait_until.unwrap_or_else(|| "domcontentloaded".to_string()),
        "saveScreenshot": args.save_screenshot.unwrap_or(true),
        "screenshotDir": args.screenshot_dir.unwrap_or_else(|| "image/browser_web".to_string()),
        "contentMode": args.content_mode.unwrap_or_else(|| "clean".to_string()),
        "maxTextChars": args.max_text_chars.unwrap_or(12_000),
        "failFast": args.fail_fast.unwrap_or(false),
        "waitMapPath": args.wait_map_path,
    });

    call_browser_helper(workspace_root, helper_input)
}

fn search_page_action(workspace_root: &PathBuf, args: SearchPageArgs) -> Result<String, String> {
    let helper_input = json!({
        "action": "searchPage",
        "query": args.query,
        "engine": args.engine.unwrap_or_else(|| "google".to_string()),
        "topK": args.top_k.unwrap_or(5),
        "region": args.region,
        "lang": args.lang.unwrap_or_else(|| "en".to_string()),
    });

    call_browser_helper(workspace_root, helper_input)
}

fn search_extract_action(workspace_root: &PathBuf, args: SearchExtractArgs) -> Result<String, String> {
    let helper_input = json!({
        "action": "searchExtract",
        "query": args.query,
        "engine": args.engine.unwrap_or_else(|| "google".to_string()),
        "topK": args.top_k.unwrap_or(5),
        "extractTopN": args.extract_top_n.unwrap_or(3),
        "waitUntil": args.wait_until.unwrap_or_else(|| "domcontentloaded".to_string()),
        "summarize": args.summarize.unwrap_or(true),
        "contentMode": args.content_mode.unwrap_or_else(|| "clean".to_string()),
        "maxTextChars": args.max_text_chars.unwrap_or(12_000),
        "failFast": args.fail_fast.unwrap_or(false),
        "region": args.region,
        "lang": args.lang.unwrap_or_else(|| "en".to_string()),
    });

    call_browser_helper(workspace_root, helper_input)
}

fn call_browser_helper(workspace_root: &PathBuf, input: Value) -> Result<String, String> {
    let helper_path = workspace_root
        .join("crates")
        .join("skills")
        .join("browser_web")
        .join("browser_web.js");

    if !helper_path.exists() {
        return Err(format!(
            "browser helper not found at {}; ensure browser_web.js exists and Node.js/Playwright are installed (see package.json in skill directory)",
            helper_path.display()
        ));
    }

    let mut cmd = Command::new("node");
    cmd.arg(helper_path.as_os_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // On Windows, avoid popping up an extra terminal window for Node helper.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("Node.js not found; please install Node.js to use browser_web skill. Helper path: {}", helper_path.display())
        } else {
            format!("failed to start browser helper: {}", e)
        }
    })?;

    let input_json = serde_json::to_string(&input)
        .map_err(|e| format!("failed to serialize helper input: {}", e))?;

    {
        let mut stdin = child.stdin.take().ok_or_else(|| "failed to take stdin".to_string())?;
        writeln!(stdin, "{}", input_json).map_err(|e| format!("failed to write to helper stdin: {}", e))?;
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait for helper: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_trimmed = stderr.trim();
        let parsed_code = stderr_trimmed
            .split_whitespace()
            .next()
            .and_then(|tok| tok.strip_prefix('['))
            .and_then(|tok| tok.strip_suffix(']'))
            .filter(|tok| !tok.is_empty())
            .unwrap_or("HELPER_ERROR");
        return Err(format!(
            "browser helper failed (code={}, exit={}): {}; ensure Playwright is installed (run 'npm install' in skill directory)",
            parsed_code,
            output.status.code().unwrap_or(-1),
            stderr_trimmed
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("helper output is not valid UTF-8: {}", e))?;

    Ok(stdout.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_args_non_object_returns_error() {
        let req = Request {
            request_id: "test-1".to_string(),
            args: json!("not an object"),
            context: None,
            user_id: 1,
            chat_id: 1,
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
        assert!(args.unwrap_err().contains("at least one of url or urls is required"));
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
        assert!(args.unwrap_err().contains("max_pages must be between 1 and 10"));
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
        assert!(args.unwrap_err().contains("max_pages must be between 1 and 10"));
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
        assert!(args.unwrap_err().contains("extract_top_n must be between 1 and 10"));
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
        assert!(args.unwrap_err().contains("extract_top_n must be between 1 and 10"));
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
            "fail_fast": true,
            "wait_map_path": "configs/browser_web_wait_map.json"
        })
        .as_object()
        .unwrap()
        .clone();

        let args = parse_open_extract_args(&obj).unwrap();
        assert_eq!(args.content_mode, Some("raw".to_string()));
        assert_eq!(args.max_text_chars, Some(4096));
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
            "fail_fast": true
        })
        .as_object()
        .unwrap()
        .clone();

        let args = parse_search_extract_args(&obj).unwrap();
        assert_eq!(args.summarize, Some(false));
        assert_eq!(args.content_mode, Some("raw".to_string()));
        assert_eq!(args.max_text_chars, Some(1600));
        assert_eq!(args.fail_fast, Some(true));
    }
}

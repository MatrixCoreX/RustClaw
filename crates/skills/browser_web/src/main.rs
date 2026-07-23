use std::io::{self, BufRead, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

const SKILL_NAME: &str = "browser_web";
const MAX_URL_BYTES: usize = 4_096;
const MAX_DOMAIN_FILTERS: usize = 32;

#[derive(Debug, Deserialize)]
struct Request {
    request_id: String,
    args: Value,
    #[serde(rename = "context")]
    _context: Option<Value>,
    #[serde(rename = "user_id")]
    _user_id: i64,
    #[serde(rename = "chat_id")]
    _chat_id: i64,
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

#[derive(Debug)]
struct SkillFailure {
    code: String,
    message: String,
    retryable: bool,
    details: Option<Value>,
}

impl SkillFailure {
    fn machine(code: &str) -> Self {
        Self {
            code: code.to_string(),
            message: code.to_string(),
            retryable: false,
            details: None,
        }
    }

    fn with_message(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            retryable: false,
            details: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct HelperFailureEnvelope {
    error_code: String,
    error_text: String,
    #[serde(default)]
    retryable: bool,
    #[serde(default)]
    details: Option<Value>,
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
    capture_images: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screenshot_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_text_chars: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_content_chars: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fail_fast: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait_map_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    domains_allow: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    domains_deny: Option<Vec<String>>,
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
                extra: Some(error_extra("INVALID_INPUT", false, None)),
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

    let obj = match req.args.as_object() {
        Some(o) => o,
        None => {
            return error_response(
                req.request_id,
                SkillFailure::with_message("INVALID_INPUT", "args must be object"),
            );
        }
    };

    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "action is required".to_string());

    let action = match action {
        Ok(a) => a,
        Err(err) => {
            return error_response(
                req.request_id,
                SkillFailure::with_message("INVALID_INPUT", err),
            );
        }
    };

    let result = match action {
        "open_extract" => parse_open_extract_args(obj)
            .map_err(|message| SkillFailure::with_message("INVALID_INPUT", message))
            .and_then(|args| open_extract_action(&workspace_root, args)),
        _ => Err(SkillFailure::with_message(
            "INVALID_ACTION",
            "unsupported_action",
        )),
    };

    match result {
        Ok(text) => Response {
            request_id: req.request_id,
            status: "ok".to_string(),
            extra: browser_web_success_extra(&text),
            text,
            error_text: None,
            buttons: None,
        },
        Err(err) => error_response(req.request_id, err),
    }
}

fn error_response(request_id: String, failure: SkillFailure) -> Response {
    Response {
        request_id,
        status: "error".to_string(),
        text: String::new(),
        error_text: Some(failure.message),
        buttons: None,
        extra: Some(error_extra(
            &failure.code,
            failure.retryable,
            failure.details.as_ref(),
        )),
    }
}

fn error_extra(error_code: &str, retryable: bool, details: Option<&Value>) -> Value {
    let mut extra = json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_code,
        "error_code": error_code,
        "message_key": format!(
            "skill.{}.{}",
            SKILL_NAME,
            error_code.to_ascii_lowercase()
        ),
        "retryable": retryable,
    });
    if let (Some(object), Some(details)) = (extra.as_object_mut(), details) {
        object.insert("details".to_string(), details.clone());
    }
    extra
}

fn browser_web_success_extra(text: &str) -> Option<Value> {
    let mut value: Value = serde_json::from_str(text).ok()?;
    if let Some(object) = value.as_object_mut() {
        object
            .entry("schema_version".to_string())
            .or_insert_with(|| json!(1));
        object
            .entry("source_skill".to_string())
            .or_insert_with(|| json!("browser_web"));
        object
            .entry("status".to_string())
            .or_insert_with(|| json!("ok"));
        object.entry("trust".to_string()).or_insert_with(|| {
            json!({
                "classification": "untrusted_web_content",
                "instructions_executable": false
            })
        });
        let items = object
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let source_refs = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let url = item
                    .get("final_url")
                    .or_else(|| item.get("url"))
                    .and_then(Value::as_str)?;
                Some(json!({
                    "url": url,
                    "title": item.get("title").cloned().unwrap_or(Value::Null),
                    "rank": index + 1,
                    "kind": "browser_page",
                    "content_sha256": item
                        .get("content_sha256")
                        .cloned()
                        .unwrap_or(Value::Null)
                }))
            })
            .collect::<Vec<_>>();
        object
            .entry("source_refs".to_string())
            .or_insert_with(|| json!(source_refs));
        object.entry("page".to_string()).or_insert_with(|| {
            json!({
                "cursor": 0,
                "limit": items.len(),
                "returned_count": items.len(),
                "total_count": items.len(),
                "has_more": false,
                "next_cursor": Value::Null,
                "previous_cursor": Value::Null
            })
        });
        let truncated = items.iter().any(|item| {
            item.get("text_truncated").and_then(Value::as_bool) == Some(true)
                || item.get("raw_html_truncated").and_then(Value::as_bool) == Some(true)
        });
        object
            .entry("truncated".to_string())
            .or_insert_with(|| json!(truncated));
    }
    Some(value)
}

fn parse_open_extract_args(
    obj: &serde_json::Map<String, Value>,
) -> Result<OpenExtractArgs, String> {
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "action is required".to_string())?
        .to_string();

    let url = optional_string(obj.get("url"), "url")?;
    let urls = match obj.get("urls") {
        None | Some(Value::Null) => None,
        Some(Value::Array(values)) => Some(
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| "urls_items_invalid".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err("urls_invalid".to_string()),
    };

    if url.is_none() && urls.as_ref().is_none_or(Vec::is_empty) {
        return Err("at least one of url or urls is required".to_string());
    }

    let max_pages = if let Some(v) = obj.get("max_pages") {
        let val = v
            .as_u64()
            .ok_or_else(|| "max_pages must be an integer".to_string())?;
        if !(1..=10).contains(&val) {
            return Err(format!("max_pages must be between 1 and 10, got {}", val));
        }
        val as u32
    } else {
        3
    };

    let wait_until = optional_string(obj.get("wait_until"), "wait_until")?
        .unwrap_or_else(|| "domcontentloaded".to_string());

    if !matches!(
        wait_until.as_str(),
        "domcontentloaded" | "load" | "networkidle"
    ) {
        return Err("wait_until must be one of: domcontentloaded, load, networkidle".to_string());
    }

    let save_screenshot =
        optional_bool(obj.get("save_screenshot"), "save_screenshot")?.unwrap_or(true);
    let capture_images =
        optional_bool(obj.get("capture_images"), "capture_images")?.unwrap_or(false);

    let screenshot_dir = optional_string(obj.get("screenshot_dir"), "screenshot_dir")?
        .unwrap_or_else(|| "skills_output/browser_web/screenshots".to_string());
    let content_mode = optional_string(obj.get("content_mode"), "content_mode")?
        .unwrap_or_else(|| "clean".to_string());
    if !matches!(content_mode.as_str(), "clean" | "raw") {
        return Err("content_mode must be one of: clean, raw".to_string());
    }
    let max_text_chars = if let Some(v) = obj.get("max_text_chars") {
        let val = v
            .as_u64()
            .ok_or_else(|| "max_text_chars must be an integer".to_string())?;
        if !(100..=200_000).contains(&val) {
            return Err(format!(
                "max_text_chars must be between 100 and 200000, got {}",
                val
            ));
        }
        val as u32
    } else {
        12_000
    };
    let min_content_chars = if let Some(v) = obj.get("min_content_chars") {
        let val = v
            .as_u64()
            .ok_or_else(|| "min_content_chars must be an integer".to_string())?;
        if !(20..=10_000).contains(&val) {
            return Err(format!(
                "min_content_chars must be between 20 and 10000, got {}",
                val
            ));
        }
        val as u32
    } else {
        200
    };
    let fail_fast = optional_bool(obj.get("fail_fast"), "fail_fast")?.unwrap_or(false);
    let wait_map_path = optional_string(obj.get("wait_map_path"), "wait_map_path")?;
    let domains_allow = parse_domain_list(obj.get("domains_allow"), "domains_allow")?;
    let domains_deny = parse_domain_list(obj.get("domains_deny"), "domains_deny")?;

    Ok(OpenExtractArgs {
        action,
        url,
        urls,
        max_pages: Some(max_pages),
        wait_until: Some(wait_until),
        save_screenshot: Some(save_screenshot),
        capture_images: Some(capture_images),
        screenshot_dir: Some(screenshot_dir),
        content_mode: Some(content_mode),
        max_text_chars: Some(max_text_chars),
        min_content_chars: Some(min_content_chars),
        fail_fast: Some(fail_fast),
        wait_map_path,
        domains_allow: Some(domains_allow),
        domains_deny: Some(domains_deny),
    })
}

fn optional_string(value: Option<&Value>, field: &str) -> Result<Option<String>, String> {
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
        .map(str::to_string)
        .map(Some)
        .ok_or_else(|| format!("{field}_invalid"))
}

fn optional_bool(value: Option<&Value>, field: &str) -> Result<Option<bool>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| format!("{field}_invalid"))
}

fn parse_domain_list(value: Option<&Value>, field: &str) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| format!("{field}_invalid"))?;
    if values.len() > MAX_DOMAIN_FILTERS {
        return Err(format!("{field}_too_many"));
    }
    values
        .iter()
        .map(|value| {
            let domain = value
                .as_str()
                .map(str::trim)
                .map(|value| value.trim_start_matches('.').trim_end_matches('.'))
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("{field}_item_invalid"))?;
            if domain.len() > 253
                || !domain
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
            {
                return Err(format!("{field}_item_invalid"));
            }
            Ok(domain.to_ascii_lowercase())
        })
        .collect()
}

fn open_extract_action(
    workspace_root: &Path,
    args: OpenExtractArgs,
) -> Result<String, SkillFailure> {
    let mut urls = Vec::new();
    if let Some(url) = args.url {
        urls.push(url);
    }
    if let Some(urls_list) = args.urls {
        urls.extend(urls_list);
    }

    if urls.is_empty() {
        return Err(SkillFailure::with_message(
            "INVALID_INPUT",
            "at least one URL is required",
        ));
    }

    let max_pages = args.max_pages.unwrap_or(3);
    urls.truncate(max_pages as usize);
    let domains_allow = args.domains_allow.unwrap_or_default();
    let domains_deny = args.domains_deny.unwrap_or_default();
    let mut validated_urls = Vec::with_capacity(urls.len());
    for url in urls {
        let url = validate_browser_target(&url, &domains_allow, &domains_deny)?;
        if !validated_urls.contains(&url) {
            validated_urls.push(url);
        }
    }

    let screenshot_dir = resolve_workspace_directory(
        workspace_root,
        &args
            .screenshot_dir
            .unwrap_or_else(|| "skills_output/browser_web/screenshots".to_string()),
    )?;
    let wait_map_path = args
        .wait_map_path
        .as_deref()
        .map(|path| resolve_workspace_file(workspace_root, path))
        .transpose()?;

    let helper_input = json!({
        "action": "openExtract",
        "urls": validated_urls,
        "waitUntil": args.wait_until.unwrap_or_else(|| "domcontentloaded".to_string()),
        "saveScreenshot": args.save_screenshot.unwrap_or(true),
        "captureImages": args.capture_images.unwrap_or(false),
        "screenshotDir": screenshot_dir,
        "contentMode": args.content_mode.unwrap_or_else(|| "clean".to_string()),
        "maxTextChars": args.max_text_chars.unwrap_or(12_000),
        "minContentChars": args.min_content_chars.unwrap_or(200),
        "failFast": args.fail_fast.unwrap_or(false),
        "waitMapPath": wait_map_path,
        "domainsAllow": domains_allow,
        "domainsDeny": domains_deny,
        "workspaceRoot": workspace_root,
    });

    call_browser_helper(workspace_root, helper_input)
}

fn validate_browser_target(
    raw: &str,
    domains_allow: &[String],
    domains_deny: &[String],
) -> Result<String, SkillFailure> {
    if raw.is_empty() || raw.len() > MAX_URL_BYTES {
        return Err(SkillFailure::machine("URL_INVALID"));
    }
    let mut url = Url::parse(raw).map_err(|_| SkillFailure::machine("URL_INVALID"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(SkillFailure::machine("URL_SCHEME_BLOCKED"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SkillFailure::machine("URL_CREDENTIALS_BLOCKED"));
    }
    url.set_fragment(None);
    let host = url
        .host_str()
        .map(|value| value.trim_end_matches('.').to_ascii_lowercase())
        .ok_or_else(|| SkillFailure::machine("URL_HOST_MISSING"))?;
    if domains_deny
        .iter()
        .any(|domain| domain_matches(&host, domain))
    {
        return Err(SkillFailure::machine("DOMAIN_BLOCKED"));
    }
    if !domains_allow.is_empty()
        && !domains_allow
            .iter()
            .any(|domain| domain_matches(&host, domain))
    {
        return Err(SkillFailure::machine("DOMAIN_NOT_ALLOWED"));
    }
    if is_local_hostname(&host) {
        return Err(SkillFailure::machine("PRIVATE_NETWORK_BLOCKED"));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| SkillFailure::machine("URL_PORT_INVALID"))?;
    let addresses = if let Ok(address) = host.parse::<IpAddr>() {
        vec![address]
    } else {
        (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|_| SkillFailure::machine("DNS_RESOLUTION_FAILED"))?
            .map(|address| address.ip())
            .collect::<Vec<_>>()
    };
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !is_public_or_proxy_synthetic(*address, &host, url.scheme()))
    {
        return Err(SkillFailure::machine("PRIVATE_NETWORK_BLOCKED"));
    }
    Ok(url.to_string())
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

fn is_public_or_proxy_synthetic(address: IpAddr, host: &str, scheme: &str) -> bool {
    if is_public_ip(address) {
        return true;
    }
    host.parse::<IpAddr>().is_err()
        && proxy_configured(scheme)
        && !host_matches_no_proxy(host)
        && is_proxy_synthetic_ip(address)
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_broadcast()
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 198 && matches!(octets[1], 18 | 19))
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        || octets[0] >= 240)
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let segments = address.segments();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

fn is_proxy_synthetic_ip(address: IpAddr) -> bool {
    matches!(
        address,
        IpAddr::V4(address)
            if {
                let octets = address.octets();
                octets[0] == 198 && matches!(octets[1], 18 | 19)
            }
    )
}

fn proxy_configured(scheme: &str) -> bool {
    let names: &[&str] = if scheme == "https" {
        &["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        &["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    };
    names.iter().any(|name| {
        std::env::var(name)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    })
}

fn host_matches_no_proxy(host: &str) -> bool {
    ["NO_PROXY", "no_proxy"]
        .iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .is_some_and(|value| {
            value.split(',').map(str::trim).any(|entry| {
                if entry == "*" {
                    return true;
                }
                let entry = entry
                    .split_once(':')
                    .map(|(host, _)| host)
                    .unwrap_or(entry)
                    .trim_start_matches('.')
                    .trim_end_matches('.')
                    .to_ascii_lowercase();
                !entry.is_empty() && domain_matches(host, &entry)
            })
        })
}

fn normalize_workspace_relative(path: &str) -> Result<PathBuf, SkillFailure> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Err(SkillFailure::machine("WORKSPACE_PATH_OUTSIDE"));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(SkillFailure::machine("WORKSPACE_PATH_OUTSIDE"));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(SkillFailure::machine("WORKSPACE_PATH_INVALID"));
    }
    Ok(normalized)
}

fn resolve_workspace_directory(
    workspace_root: &Path,
    requested: &str,
) -> Result<PathBuf, SkillFailure> {
    let workspace = workspace_root
        .canonicalize()
        .map_err(|_| SkillFailure::machine("WORKSPACE_UNAVAILABLE"))?;
    let candidate = if Path::new(requested).is_absolute() {
        let requested = Path::new(requested);
        let relative = requested
            .strip_prefix(&workspace)
            .map_err(|_| SkillFailure::machine("WORKSPACE_PATH_OUTSIDE"))?;
        let relative = relative
            .to_str()
            .ok_or_else(|| SkillFailure::machine("WORKSPACE_PATH_INVALID"))?;
        workspace.join(normalize_workspace_relative(relative)?)
    } else {
        workspace.join(normalize_workspace_relative(requested)?)
    };
    std::fs::create_dir_all(&candidate)
        .map_err(|_| SkillFailure::machine("WORKSPACE_PATH_CREATE_FAILED"))?;
    let canonical = candidate
        .canonicalize()
        .map_err(|_| SkillFailure::machine("WORKSPACE_PATH_INVALID"))?;
    if !canonical.starts_with(&workspace) {
        return Err(SkillFailure::machine("WORKSPACE_PATH_OUTSIDE"));
    }
    Ok(canonical)
}

fn resolve_workspace_file(workspace_root: &Path, requested: &str) -> Result<PathBuf, SkillFailure> {
    let workspace = workspace_root
        .canonicalize()
        .map_err(|_| SkillFailure::machine("WORKSPACE_UNAVAILABLE"))?;
    let candidate = if Path::new(requested).is_absolute() {
        PathBuf::from(requested)
    } else {
        workspace.join(normalize_workspace_relative(requested)?)
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|_| SkillFailure::machine("WORKSPACE_FILE_UNAVAILABLE"))?;
    if !canonical.starts_with(&workspace) || !canonical.is_file() {
        return Err(SkillFailure::machine("WORKSPACE_PATH_OUTSIDE"));
    }
    Ok(canonical)
}

fn call_browser_helper(workspace_root: &Path, input: Value) -> Result<String, SkillFailure> {
    let helper_path = workspace_root
        .join("crates")
        .join("skills")
        .join("browser_web")
        .join("browser_web.js");

    if !helper_path.exists() {
        return Err(SkillFailure::with_message(
            "DEPENDENCY_MISSING",
            format!("browser helper not found at {}", helper_path.display()),
        ));
    }

    let mut cmd = Command::new("node");
    cmd.arg(helper_path.as_os_str())
        .current_dir(workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // On Windows, avoid popping up an extra terminal window for Node helper.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SkillFailure::with_message("DEPENDENCY_MISSING", "node executable not found")
        } else {
            SkillFailure::with_message("HELPER_SPAWN_FAILED", error.to_string())
        }
    })?;

    let input_json = serde_json::to_string(&input).map_err(|error| {
        SkillFailure::with_message("HELPER_INPUT_SERIALIZATION_FAILED", error.to_string())
    })?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| SkillFailure::machine("HELPER_STDIN_UNAVAILABLE"))?;
        writeln!(stdin, "{}", input_json).map_err(|error| {
            SkillFailure::with_message("HELPER_STDIN_WRITE_FAILED", error.to_string())
        })?;
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .map_err(|error| SkillFailure::with_message("HELPER_WAIT_FAILED", error.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if let Some(envelope) = stderr
            .lines()
            .rev()
            .find_map(|line| serde_json::from_str::<HelperFailureEnvelope>(line).ok())
        {
            return Err(SkillFailure {
                code: envelope.error_code,
                message: envelope.error_text,
                retryable: envelope.retryable,
                details: envelope.details,
            });
        }
        return Err(SkillFailure::with_message("HELPER_ERROR", stderr.trim()));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| SkillFailure::with_message("HELPER_OUTPUT_INVALID", error.to_string()))?;

    Ok(stdout.trim().to_string())
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

use std::collections::HashMap;
use std::io::{self, BufRead, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SKILL_NAME: &str = "http_basic";
const DEFAULT_TIMEOUT_SECONDS: u64 = 20;
const MAX_TIMEOUT_SECONDS: u64 = 120;
const DEFAULT_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_REDIRECTS: usize = 5;
const MAX_REDIRECTS: usize = 10;
const PREVIEW_CHARS: usize = 8_000;

#[derive(Debug)]
struct HttpBasicError {
    code: &'static str,
    detail: String,
    retryable: bool,
    extra: Option<Value>,
}

impl HttpBasicError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
            retryable: false,
            extra: None,
        }
    }

    fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    fn with_extra(mut self, extra: Value) -> Self {
        self.extra = Some(extra);
        self
    }
}

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    context: Option<Value>,
    user_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    extra: Option<Value>,
    error_text: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => {
                let req_ui_key = request_ui_key(&req);
                match execute(req.args, req_ui_key.as_deref()) {
                    Ok((text, extra)) => Resp {
                        request_id: req.request_id,
                        status: "ok".to_string(),
                        text,
                        extra: Some(extra),
                        error_text: None,
                    },
                    Err(err) => Resp {
                        request_id: req.request_id,
                        status: "error".to_string(),
                        text: String::new(),
                        extra: Some(error_extra_with_detail(err.code, err.retryable, err.extra)),
                        error_text: Some(err.detail),
                    },
                }
            }
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(error_extra("invalid_input")),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }

    Ok(())
}

fn error_extra(error_kind: &str) -> Value {
    error_extra_with_detail(error_kind, false, None)
}

fn error_extra_with_detail(error_kind: &str, retryable: bool, detail: Option<Value>) -> Value {
    let mut extra = json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_kind,
        "error_code": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": retryable,
    });
    if let (Some(fields), Some(detail)) = (extra.as_object_mut(), detail) {
        fields.insert("detail".to_string(), detail);
    }
    extra
}

fn request_ui_key(req: &Req) -> Option<String> {
    req.user_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            req.context
                .as_ref()
                .and_then(|v| v.as_object())
                .and_then(|m| m.get("user_key"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

fn should_inject_rustclaw_key(url: &str) -> bool {
    Url::parse(url).ok().is_some_and(|url| {
        url.scheme() == "http"
            && url.port_or_known_default() == Some(8787)
            && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
    })
}

#[derive(Debug, Clone)]
struct FetchPolicy {
    timeout: Duration,
    max_response_bytes: usize,
    max_redirects: usize,
    domains_allow: Vec<String>,
    domains_deny: Vec<String>,
}

#[derive(Debug)]
struct ValidatedTarget {
    url: Url,
    resolved: Vec<SocketAddr>,
    rustclaw_local: bool,
    proxy_mediated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestMethod {
    Get,
    PostJson,
}

fn execute(args: Value, req_user_key: Option<&str>) -> Result<(String, Value), HttpBasicError> {
    let obj = args
        .as_object()
        .ok_or_else(|| HttpBasicError::new("invalid_args", "args must be object"))?;

    let action = optional_string(obj, "action", "invalid_action")?.unwrap_or("get");
    let mut method = match action {
        "get" | "download" => RequestMethod::Get,
        "post_json" => RequestMethod::PostJson,
        _ => {
            return Err(HttpBasicError::new("invalid_action", "unsupported action"));
        }
    };
    let url = obj
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| HttpBasicError::new("url_missing", "url is required"))?;
    let output_path = optional_string(obj, "output_path", "output_path_invalid")?;
    let download_requested = optional_bool(obj, "download")?.unwrap_or(false);
    if action == "get" && (download_requested || output_path.is_some()) {
        return Err(HttpBasicError::new(
            "download_action_required",
            "file output requires action=download",
        ));
    }
    let policy = parse_fetch_policy(obj)?;

    let mut headers = HashMap::new();
    if let Some(value) = obj.get("headers") {
        let map = value
            .as_object()
            .ok_or_else(|| HttpBasicError::new("headers_invalid", "headers must be object"))?;
        for (name, value) in map {
            let value = value.as_str().ok_or_else(|| {
                HttpBasicError::new("headers_invalid", "header values must be strings")
            })?;
            if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("x-rustclaw-key") {
                return Err(HttpBasicError::new(
                    "header_blocked",
                    "protected header cannot be overridden",
                ));
            }
            headers.insert(name.to_string(), value.to_string());
        }
    }

    let body = obj.get("body").cloned().unwrap_or(Value::Null);
    let allow_rustclaw_local = req_user_key
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let original = validate_target_url(url, &policy, allow_rustclaw_local)?;
    let original_origin = origin_key(&original.url);
    let permit_local_redirect_target = original.rustclaw_local;
    let mut current = original;
    let mut redirects = Vec::new();

    let mut resp = loop {
        let client = client_for_target(&current, &policy)?;
        let mut request = match method {
            RequestMethod::Get => client.get(current.url.clone()),
            RequestMethod::PostJson => client.post(current.url.clone()),
        }
        .header("User-Agent", "RustClaw/1.0");

        let same_origin = origin_key(&current.url) == original_origin;
        for (name, value) in &headers {
            if should_forward_header(name, same_origin) {
                request = request.header(name, value);
            }
        }
        if current.rustclaw_local {
            if let Some(user_key) = req_user_key
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                request = request.header("X-RustClaw-Key", user_key);
            }
        }
        if method == RequestMethod::PostJson {
            request = request.json(&body);
        }

        let response = request.send().map_err(|error| {
            HttpBasicError::new("request_failed", format!("http request failed: {error}"))
                .retryable()
                .with_extra(json!({"url": current.url.as_str()}))
        })?;
        if !response.status().is_redirection() {
            break response;
        }
        let Some(location) = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
        else {
            break response;
        };
        if redirects.len() >= policy.max_redirects {
            return Err(
                HttpBasicError::new("redirect_limit_exceeded", "redirect limit exceeded")
                    .with_extra(json!({"max_redirects": policy.max_redirects})),
            );
        }
        let next_url = current
            .url
            .join(location)
            .map_err(|error| HttpBasicError::new("redirect_url_invalid", error.to_string()))?;
        if current.url.scheme() == "https" && next_url.scheme() != "https" {
            return Err(HttpBasicError::new(
                "redirect_scheme_downgrade",
                "https redirect cannot downgrade to http",
            )
            .with_extra(json!({
                "from": current.url.as_str(),
                "to": next_url.as_str(),
            })));
        }
        redirects.push(json!({
            "status_code": response.status().as_u16(),
            "from": current.url.as_str(),
            "to": next_url.as_str(),
        }));
        if redirect_switches_to_get(response.status(), method) {
            method = RequestMethod::Get;
        }
        current = validate_target_url(next_url.as_str(), &policy, permit_local_redirect_target)?;
    };

    let status = resp.status().as_u16();
    let success = resp.status().is_success();
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if let Some(content_length) = resp
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
    {
        if content_length > policy.max_response_bytes {
            return Err(response_too_large(
                content_length,
                policy.max_response_bytes,
            ));
        }
    }
    let body = read_limited(&mut resp, policy.max_response_bytes)?;
    let output_requested = action == "download" || download_requested || output_path.is_some();
    let textual = is_textual_content(content_type.as_deref(), &body);
    if !textual && !output_requested {
        return Err(HttpBasicError::new(
            "content_type_blocked",
            "binary response requires download=true or output_path",
        )
        .with_extra(json!({
            "content_type": content_type,
            "size_bytes": body.len(),
        })));
    }
    let (preview, preview_truncated) = if textual {
        let text = String::from_utf8_lossy(&body);
        (
            bounded_preview(&text, PREVIEW_CHARS),
            text.chars().count() > PREVIEW_CHARS,
        )
    } else {
        (String::new(), false)
    };
    let body_sha256 = format!("sha256:{:x}", Sha256::digest(&body));
    let artifact = if output_requested {
        let workspace = workspace_root()
            .map_err(|error| HttpBasicError::new("workspace_unavailable", error.to_string()))?;
        let output_path = resolve_output_path(&workspace, "document/http/download", output_path)?;
        write_workspace_artifact(&workspace, &output_path, &body)?;
        Some(HttpArtifact {
            output_path: output_path.to_string_lossy().to_string(),
            size_bytes: body.len() as u64,
            content_type: content_type.clone(),
            sha256: body_sha256.clone(),
        })
    } else {
        None
    };

    Ok(http_observation(HttpObservationInput {
        action,
        requested_url: url,
        final_url: current.url.as_str(),
        status,
        success_status: success,
        content_type: content_type.as_deref(),
        size_bytes: body.len(),
        body_sha256: &body_sha256,
        redirects,
        network_route: if current.proxy_mediated {
            "trusted_egress_proxy"
        } else {
            "direct"
        },
        preview: &preview,
        preview_truncated,
        artifact: artifact.as_ref(),
    }))
}

fn parse_fetch_policy(obj: &serde_json::Map<String, Value>) -> Result<FetchPolicy, HttpBasicError> {
    let timeout_seconds = bounded_u64(
        obj.get("timeout_seconds"),
        DEFAULT_TIMEOUT_SECONDS,
        1,
        MAX_TIMEOUT_SECONDS,
        "timeout_invalid",
    )?;
    let max_response_bytes = bounded_u64(
        obj.get("max_response_bytes"),
        DEFAULT_RESPONSE_BYTES as u64,
        1,
        MAX_RESPONSE_BYTES as u64,
        "response_limit_invalid",
    )? as usize;
    let max_redirects = bounded_u64(
        obj.get("max_redirects"),
        DEFAULT_REDIRECTS as u64,
        0,
        MAX_REDIRECTS as u64,
        "redirect_limit_invalid",
    )? as usize;
    Ok(FetchPolicy {
        timeout: Duration::from_secs(timeout_seconds),
        max_response_bytes,
        max_redirects,
        domains_allow: parse_domain_list(obj.get("domains_allow"))?,
        domains_deny: parse_domain_list(obj.get("domains_deny"))?,
    })
}

fn bounded_u64(
    value: Option<&Value>,
    default: u64,
    minimum: u64,
    maximum: u64,
    code: &'static str,
) -> Result<u64, HttpBasicError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let value = value
        .as_u64()
        .ok_or_else(|| HttpBasicError::new(code, code))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(HttpBasicError::new(code, code));
    }
    Ok(value)
}

fn parse_domain_list(value: Option<&Value>) -> Result<Vec<String>, HttpBasicError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| HttpBasicError::new("domain_policy_invalid", "domain list must be array"))?;
    values
        .iter()
        .map(|value| {
            let domain = value
                .as_str()
                .map(str::trim)
                .map(|value| value.trim_start_matches('.').trim_end_matches('.'))
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    HttpBasicError::new("domain_policy_invalid", "domain must be a string")
                })?;
            if !domain
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
            {
                return Err(HttpBasicError::new(
                    "domain_policy_invalid",
                    "domain contains unsupported characters",
                ));
            }
            Ok(domain.to_ascii_lowercase())
        })
        .collect()
}

fn validate_target_url(
    raw: &str,
    policy: &FetchPolicy,
    allow_rustclaw_local: bool,
) -> Result<ValidatedTarget, HttpBasicError> {
    let mut url =
        Url::parse(raw).map_err(|error| HttpBasicError::new("url_invalid", error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(HttpBasicError::new(
            "url_scheme_blocked",
            "only http and https URLs are supported",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(HttpBasicError::new(
            "url_credentials_blocked",
            "URL userinfo is not allowed",
        ));
    }
    url.set_fragment(None);
    let host = url
        .host_str()
        .map(|value| value.trim_end_matches('.').to_ascii_lowercase())
        .ok_or_else(|| HttpBasicError::new("url_host_missing", "URL host is required"))?;
    enforce_domain_policy(&host, policy)?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| HttpBasicError::new("url_port_invalid", "URL port is unavailable"))?;
    let rustclaw_local = allow_rustclaw_local && should_inject_rustclaw_key(url.as_str());
    let resolved = resolve_host(&host, port)?;
    let proxy_mediated =
        host.parse::<IpAddr>().is_err() && proxy_applies_to_host(url.scheme(), &host);
    if !rustclaw_local
        && resolved.iter().any(|address| {
            !is_public_ip(address.ip()) && !(proxy_mediated && is_proxy_synthetic_ip(address.ip()))
        })
    {
        return Err(HttpBasicError::new(
            "private_network_blocked",
            "target resolves to a non-public network address",
        )
        .with_extra(json!({"host": host})));
    }
    Ok(ValidatedTarget {
        url,
        resolved,
        rustclaw_local,
        proxy_mediated,
    })
}

fn enforce_domain_policy(host: &str, policy: &FetchPolicy) -> Result<(), HttpBasicError> {
    if policy
        .domains_deny
        .iter()
        .any(|domain| domain_matches(host, domain))
    {
        return Err(HttpBasicError::new(
            "domain_blocked",
            "target domain is blocked",
        ));
    }
    if !policy.domains_allow.is_empty()
        && !policy
            .domains_allow
            .iter()
            .any(|domain| domain_matches(host, domain))
    {
        return Err(HttpBasicError::new(
            "domain_not_allowed",
            "target domain is not allowed",
        ));
    }
    Ok(())
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn resolve_host(host: &str, port: u16) -> Result<Vec<SocketAddr>, HttpBasicError> {
    let mut addresses = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        (host, port)
            .to_socket_addrs()
            .map_err(|error| {
                HttpBasicError::new("dns_resolution_failed", error.to_string()).retryable()
            })?
            .collect::<Vec<_>>()
    };
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty() {
        return Err(
            HttpBasicError::new("dns_resolution_failed", "host resolved to no addresses")
                .retryable(),
        );
    }
    Ok(addresses)
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

fn is_proxy_synthetic_ip(ip: IpAddr) -> bool {
    matches!(
        ip,
        IpAddr::V4(ip)
            if {
                let octets = ip.octets();
                octets[0] == 198 && matches!(octets[1], 18 | 19)
            }
    )
}

fn proxy_applies_to_host(scheme: &str, host: &str) -> bool {
    let configured = match scheme {
        "https" => first_non_empty_env(&["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]),
        "http" => first_non_empty_env(&["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]),
        _ => None,
    }
    .is_some();
    configured && !host_matches_no_proxy(host, no_proxy_value().as_deref())
}

fn first_non_empty_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn no_proxy_value() -> Option<String> {
    first_non_empty_env(&["NO_PROXY", "no_proxy"])
}

fn host_matches_no_proxy(host: &str, no_proxy: Option<&str>) -> bool {
    no_proxy.is_some_and(|entries| {
        entries.split(',').map(str::trim).any(|entry| {
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

fn client_for_target(
    target: &ValidatedTarget,
    policy: &FetchPolicy,
) -> Result<Client, HttpBasicError> {
    let mut builder = Client::builder()
        .timeout(policy.timeout)
        .redirect(Policy::none());
    if !target.proxy_mediated
        && target
            .url
            .host_str()
            .is_some_and(|host| host.parse::<IpAddr>().is_err())
    {
        builder = builder.resolve(
            target.url.host_str().expect("validated host"),
            target.resolved[0],
        );
    }
    builder
        .build()
        .map_err(|error| HttpBasicError::new("client_build_failed", error.to_string()))
}

fn origin_key(url: &Url) -> (String, String, Option<u16>) {
    (
        url.scheme().to_string(),
        url.host_str().unwrap_or_default().to_ascii_lowercase(),
        url.port_or_known_default(),
    )
}

fn is_sensitive_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization"
            | "cookie"
            | "proxy-authorization"
            | "x-api-key"
            | "x-auth-token"
            | "x-rustclaw-key"
    )
}

fn should_forward_header(name: &str, same_origin: bool) -> bool {
    if is_sensitive_header(name) {
        return same_origin;
    }
    same_origin
        || matches!(
            name.to_ascii_lowercase().as_str(),
            "accept" | "accept-language" | "cache-control" | "if-none-match" | "if-modified-since"
        )
}

fn redirect_switches_to_get(status: StatusCode, method: RequestMethod) -> bool {
    status == StatusCode::SEE_OTHER
        || (method == RequestMethod::PostJson
            && matches!(status, StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND))
}

fn read_limited(
    reader: &mut impl Read,
    max_response_bytes: usize,
) -> Result<Vec<u8>, HttpBasicError> {
    let take_limit = max_response_bytes.saturating_add(1) as u64;
    let mut body = Vec::with_capacity(max_response_bytes.min(64 * 1024));
    reader
        .take(take_limit)
        .read_to_end(&mut body)
        .map_err(|error| {
            HttpBasicError::new("response_read_failed", error.to_string()).retryable()
        })?;
    if body.len() > max_response_bytes {
        return Err(response_too_large(body.len(), max_response_bytes));
    }
    Ok(body)
}

fn response_too_large(observed: usize, limit: usize) -> HttpBasicError {
    HttpBasicError::new("response_too_large", "response exceeds byte limit").with_extra(json!({
        "observed_bytes_at_least": observed,
        "max_response_bytes": limit,
    }))
}

fn is_textual_content(content_type: Option<&str>, body: &[u8]) -> bool {
    let Some(content_type) = content_type else {
        return std::str::from_utf8(body).is_ok();
    };
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();
    mime.starts_with("text/")
        || mime.ends_with("+json")
        || mime.ends_with("+xml")
        || matches!(
            mime.as_str(),
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/x-javascript"
                | "application/x-www-form-urlencoded"
                | "application/graphql"
        )
}

#[derive(Debug)]
struct HttpArtifact {
    output_path: String,
    size_bytes: u64,
    content_type: Option<String>,
    sha256: String,
}

struct HttpObservationInput<'a> {
    action: &'a str,
    requested_url: &'a str,
    final_url: &'a str,
    status: u16,
    success_status: bool,
    content_type: Option<&'a str>,
    size_bytes: usize,
    body_sha256: &'a str,
    redirects: Vec<Value>,
    network_route: &'a str,
    preview: &'a str,
    preview_truncated: bool,
    artifact: Option<&'a HttpArtifact>,
}

fn http_observation(observation: HttpObservationInput<'_>) -> (String, Value) {
    let HttpObservationInput {
        action,
        requested_url,
        final_url,
        status,
        success_status,
        content_type,
        size_bytes,
        body_sha256,
        redirects,
        network_route,
        preview,
        preview_truncated,
        artifact,
    } = observation;
    let output = match artifact {
        Some(artifact) => format!(
            "status={status}\noutput_path={}\n{preview}",
            artifact.output_path
        ),
        None => format!("status={status}\n{preview}"),
    };
    let mut extra = json!({
        "schema_version": 1,
        "action": action,
        "url": requested_url,
        "requested_url": requested_url,
        "final_url": final_url,
        "status_code": status,
        "success_status": success_status,
        "content_type": content_type,
        "size_bytes": size_bytes,
        "body_sha256": body_sha256,
        "body_preview": preview,
        "preview_truncated": preview_truncated,
        "redirects": redirects,
        "redirect_count": redirects.len(),
        "network_route": network_route,
        "source_refs": [{
            "url": final_url,
            "kind": "http_response"
        }],
        "citations": [final_url],
        "trust": {
            "classification": "untrusted_external_content",
            "instructions_executable": false,
            "source_url": final_url
        },
        "provenance": {
            "source": "http",
            "requested_url": requested_url,
            "final_url": final_url,
            "observed_at": unix_ts()
        }
    });
    if let (Some(obj), Some(artifact)) = (extra.as_object_mut(), artifact) {
        obj.insert("downloaded".to_string(), json!(true));
        obj.insert("output_path".to_string(), json!(artifact.output_path));
        obj.insert("artifact_path".to_string(), json!(artifact.output_path));
        obj.insert("size_bytes".to_string(), json!(artifact.size_bytes));
        obj.insert("artifact_sha256".to_string(), json!(artifact.sha256));
        if let Some(content_type) = artifact.content_type.as_deref() {
            obj.insert("content_type".to_string(), json!(content_type));
        }
    }
    (output.clone(), extra)
}

fn optional_string<'a>(
    obj: &'a serde_json::Map<String, Value>,
    key: &str,
    code: &'static str,
) -> Result<Option<&'a str>, HttpBasicError> {
    let Some(value) = obj.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_str()
        .ok_or_else(|| HttpBasicError::new(code, code))?
        .trim();
    Ok((!value.is_empty()).then_some(value))
}

fn optional_bool(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<bool>, HttpBasicError> {
    let Some(value) = obj.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| HttpBasicError::new("boolean_argument_invalid", key))
}

fn bounded_preview(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn workspace_root() -> Result<PathBuf, std::io::Error> {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)
}

fn resolve_output_path(
    workspace_root: &Path,
    default_dir: &str,
    requested: Option<&str>,
) -> Result<PathBuf, HttpBasicError> {
    if let Some(path) = requested.map(str::trim).filter(|value| !value.is_empty()) {
        let out = normalize_workspace_path(workspace_root, path)?;
        return Ok(out);
    }
    Ok(workspace_root
        .join(default_dir)
        .join(format!("http-{}.body", unix_ts())))
}

fn normalize_workspace_path(
    workspace_root: &Path,
    raw_path: &str,
) -> Result<PathBuf, HttpBasicError> {
    if raw_path.is_empty() || raw_path.len() > 4096 {
        return Err(HttpBasicError::new(
            "output_path_invalid",
            "output path is invalid",
        ));
    }
    let workspace = workspace_root
        .canonicalize()
        .map_err(|error| HttpBasicError::new("workspace_unavailable", error.to_string()))?;
    let path = Path::new(raw_path);
    let relative = if path.is_absolute() {
        path.strip_prefix(&workspace).map_err(|_| {
            HttpBasicError::new(
                "output_path_outside_workspace",
                "output_path is outside workspace",
            )
        })?
    } else {
        path
    };
    let mut normalized = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(HttpBasicError::new(
                    "output_path_outside_workspace",
                    "output_path is outside workspace",
                ));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(HttpBasicError::new(
            "output_path_invalid",
            "output path is invalid",
        ));
    }
    let output = workspace.join(normalized);
    if output.exists() {
        let metadata = std::fs::symlink_metadata(&output)
            .map_err(|error| HttpBasicError::new("output_path_invalid", error.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(HttpBasicError::new(
                "output_path_symlink_blocked",
                "output path cannot be a symlink",
            ));
        }
        let canonical = output
            .canonicalize()
            .map_err(|error| HttpBasicError::new("output_path_invalid", error.to_string()))?;
        if !canonical.starts_with(&workspace) {
            return Err(HttpBasicError::new(
                "output_path_outside_workspace",
                "output_path is outside workspace",
            ));
        }
    }
    Ok(output)
}

fn write_workspace_artifact(
    workspace_root: &Path,
    path: &Path,
    bytes: &[u8],
) -> Result<(), HttpBasicError> {
    let workspace = workspace_root
        .canonicalize()
        .map_err(|error| HttpBasicError::new("workspace_unavailable", error.to_string()))?;
    let parent = path
        .parent()
        .ok_or_else(|| HttpBasicError::new("output_path_invalid", "output path has no parent"))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| HttpBasicError::new("artifact_write_failed", error.to_string()))?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|error| HttpBasicError::new("artifact_write_failed", error.to_string()))?;
    if !canonical_parent.starts_with(&workspace) {
        return Err(HttpBasicError::new(
            "output_path_outside_workspace",
            "output_path is outside workspace",
        ));
    }
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(HttpBasicError::new(
            "output_path_symlink_blocked",
            "output path cannot be a symlink",
        ));
    }
    let temporary =
        canonical_parent.join(format!(".http-{}-{}.tmp", std::process::id(), unix_ts()));
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| HttpBasicError::new("artifact_write_failed", error.to_string()))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| HttpBasicError::new("artifact_write_failed", error.to_string()))?;
    }
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(HttpBasicError::new(
            "artifact_write_failed",
            error.to_string(),
        ));
    }
    Ok(())
}

fn unix_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

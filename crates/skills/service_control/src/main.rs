//! service_control skill: unified, safe, structured service lifecycle control.
//! Supports: status, start, stop, restart, reload, logs, verify, diagnose.
//! Managers: rustclaw (HTTP), systemd, service; others return unimplemented.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------- Constants ----------

const RUSTCLAW_SERVICES: &[&str] = &[
    "clawd",
    "telegramd",
    "whatsappd",
    "whatsapp_webd",
    "feishud",
    "larkd",
];

const ALLOWED_ACTIONS: &[&str] = &[
    "status",
    "start",
    "stop",
    "restart",
    "reload",
    "logs",
    "verify",
    "diagnose_start_failure",
    "diagnose_unhealthy_state",
];

const MANAGER_TYPES: &[&str] = &[
    "systemd",
    "service",
    "docker_compose",
    "docker_container",
    "supervisor",
    "process_only",
    "rustclaw",
    "unknown",
];

/// Targets that are too vague for high-risk actions (stop/restart).
const AMBIGUOUS_TARGETS: &[&str] = &[
    "\u{540E}\u{7AEF}",
    "\u{670D}\u{52A1}\u{4EEC}",
    "\u{90A3}\u{51E0}\u{4E2A}",
    "\u{90A3}\u{51E0}\u{4E2A}\u{670D}\u{52A1}",
    "\u{5168}\u{90E8}",
    "\u{5168}\u{90E8}\u{670D}\u{52A1}",
    "all",
    "*",
    "\u{670D}\u{52A1}",
];

const TAIL_LINES_DEFAULT: usize = 100;
const TAIL_LINES_MAX: usize = 500;
const VERIFY_WAIT_SECONDS: u64 = 2;

// ---------- Alias normalization (skill-internal) ----------

/// Strip common service suffix (Chinese and English) for alias lookup; does not change core name.
fn strip_service_suffix(s: &str) -> &str {
    let s = s.trim();
    let s_lower = s.to_lowercase();
    if s.ends_with("\u{670D}\u{52A1}") {
        s[..s.len().saturating_sub("\u{670D}\u{52A1}".len())].trim()
    } else if s_lower.ends_with(" service") {
        s_lower.rfind(" service").map(|i| s[..i].trim()).unwrap_or(s)
    } else if s_lower.ends_with(".service") {
        s[..s.len().saturating_sub(".service".len())].trim()
    } else {
        s
    }
}

/// Normalize common service aliases and strip service suffix. Only affects target string for lookup; does not change action.
pub(crate) fn normalize_target_alias(input: &str) -> String {
    let s = strip_service_suffix(input).trim().to_lowercase();
    if s.is_empty() {
        return input.trim().to_string();
    }
    // Small, stable alias map (extensible): alias -> canonical unit/service name
    let canonical = match s.as_str() {
        "nginx" => "nginx",
        "mysql" | "mysqld" => "mysql",
        "redis" | "redis-server" => "redis",
        "postgres" | "postgresql" => "postgresql",
        "docker" | "dockerd" => "docker",
        "caddy" => "caddy",
        "ssh" | "sshd" => "sshd",
        "cron" | "crond" => "cron",
        _ => return s,
    };
    canonical.to_string()
}

// ---------- Service discovery (pre-execution) ----------

const DISCOVER_CANDIDATES_MAX: usize = 20;

/// List systemd service unit names that match `target` (exact > prefix > contains), limited.
pub(crate) fn discover_systemd_candidates(target: &str) -> Vec<String> {
    let target = target.trim().to_lowercase();
    if target.is_empty() {
        return Vec::new();
    }
    let out = match Command::new("systemctl")
        .args(["list-unit-files", "--type=service", "--no-legend"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    let mut contains = Vec::new();
    let line = String::from_utf8_lossy(&out.stdout);
    for raw in line.lines() {
        let unit = raw.split_whitespace().next().unwrap_or("").trim();
        let name = unit.strip_suffix(".service").unwrap_or(unit);
        if name.is_empty() {
            continue;
        }
        let name_lower = name.to_lowercase();
        if name_lower == target || name_lower == format!("{}.service", target) {
            exact.push(name.to_string());
        } else if name_lower.starts_with(&target) || target.starts_with(&name_lower) {
            prefix.push(name.to_string());
        } else if name_lower.contains(&target) {
            contains.push(name.to_string());
        }
    }
    exact.sort();
    prefix.sort();
    contains.sort();
    let mut out_vec = Vec::new();
    out_vec.extend(exact);
    out_vec.extend(prefix);
    out_vec.extend(contains);
    out_vec.truncate(DISCOVER_CANDIDATES_MAX);
    out_vec
}

/// List service(8) visible names that match `target` (exact > prefix > contains), limited.
pub(crate) fn discover_service_candidates(target: &str) -> Vec<String> {
    let target = target.trim().to_lowercase();
    if target.is_empty() {
        return Vec::new();
    }
    let out = match Command::new("service").args(["--status-all"]).output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let line = String::from_utf8_lossy(&out.stdout);
    let mut names = Vec::new();
    for raw in line.lines() {
        // Typical: "[ + ] nginx" or "[ - ] cron" 鈥?name is after the first ']'
        let rest = raw.trim();
        if let Some(idx) = rest.find(']') {
            let name = rest[idx + 1..].trim();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                names.push(name.to_string());
            }
        }
    }
    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    let mut contains = Vec::new();
    for name in names {
        let name_lower = name.to_lowercase();
        if name_lower == target {
            exact.push(name);
        } else if name_lower.starts_with(&target) || target.starts_with(&name_lower) {
            prefix.push(name);
        } else if name_lower.contains(&target) {
            contains.push(name);
        }
    }
    exact.sort();
    prefix.sort();
    contains.sort();
    let mut out_vec = Vec::new();
    out_vec.extend(exact);
    out_vec.extend(prefix);
    out_vec.extend(contains);
    out_vec.truncate(DISCOVER_CANDIDATES_MAX);
    out_vec
}

/// Merge systemd + service candidates, dedup, preserve order (exact > prefix > contains), limit.
fn discover_all_candidates(normalized_target: &str) -> Vec<String> {
    let sys = discover_systemd_candidates(normalized_target);
    let svc = discover_service_candidates(normalized_target);
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();
    for name in sys.into_iter().chain(svc) {
        if seen.insert(name.clone()) {
            merged.push(name);
        }
    }
    merged.truncate(DISCOVER_CANDIDATES_MAX);
    merged
}

fn command_output_text(outp: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&outp.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&outp.stdout).trim().to_string();
    if !stderr.is_empty() && !stdout.is_empty() {
        format!("stderr: {}; stdout: {}", stderr, stdout)
    } else if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        "no output".to_string()
    }
}

fn looks_like_permission_error(message: &str) -> bool {
    let m = message.to_lowercase();
    [
        "permission denied",
        "operation not permitted",
        "access denied",
        "must be root",
        "authentication is required",
        "interactive authentication required",
        "not in the sudoers",
        "a password is required",
        "password is required",
        "no tty present",
        "pkttyagent",
        "authorization failed",
        "polkit",
        "permission",
        "denied",
    ]
    .iter()
    .any(|k| m.contains(k))
}

/// Safe unit/target name: alphanumeric, dot, dash, underscore, @ (for systemd units).
pub(crate) fn is_safe_target(s: &str) -> bool {
    if s.is_empty() || s.len() > 256 {
        return false;
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' || c == '@')
}

pub(crate) fn is_ambiguous_target(target: &str) -> bool {
    let t = target.trim().to_lowercase();
    if t.is_empty() {
        return true;
    }
    AMBIGUOUS_TARGETS
        .iter()
        .any(|a| t == *a || t.contains(&a.to_lowercase()))
}

pub(crate) fn is_high_risk_action(action: &str) -> bool {
    matches!(action, "stop" | "restart")
}

// ---------- Request / Response (skill-runner protocol) ----------

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    #[serde(default)]
    user_key: Option<String>,
    #[serde(default)]
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    error_text: Option<String>,
}

// ---------- Input contract ----------

#[derive(Debug, Clone)]
struct SkillInput {
    action: String,
    target: Option<String>,
    manager_type: Option<String>,
    tail_lines: usize,
    verify: bool,
    allow_risky: bool,
    suggested_target: Option<String>,
    suggest_once: bool,
}

fn normalize_non_empty_string(v: Option<&Value>) -> Option<String> {
    v.and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn extract_suggested_target(obj: &serde_json::Map<String, Value>) -> Option<String> {
    let from_suggested_params = obj
        .get("suggested_params")
        .and_then(|v| v.as_object())
        .and_then(|m| {
            normalize_non_empty_string(m.get("target"))
                .or_else(|| normalize_non_empty_string(m.get("service")))
                .or_else(|| normalize_non_empty_string(m.get("service_name")))
                .or_else(|| normalize_non_empty_string(m.get("candidate_target")))
        });
    from_suggested_params.or_else(|| normalize_non_empty_string(obj.get("llm_suggested_target")))
}

fn extract_suggest_once(obj: &serde_json::Map<String, Value>) -> bool {
    obj.get("suggest_once")
        .and_then(|v| v.as_bool())
        .or_else(|| obj.get("llm_suggest_once").and_then(|v| v.as_bool()))
        .unwrap_or(true)
}

fn parse_input(args: &Value) -> Result<SkillInput, String> {
    let obj = args.as_object().ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "action is required".to_string())?
        .trim()
        .to_string();
    if !ALLOWED_ACTIONS.contains(&action.as_str()) {
        return Err(format!(
            "unsupported action: {}; allowed: {}",
            action,
            ALLOWED_ACTIONS.join(", ")
        ));
    }
    let target = obj
        .get("target")
        .or_else(|| obj.get("service"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let manager_type = obj
        .get("manager_type")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let tail_lines = obj
        .get("tail_lines")
        .or_else(|| obj.get("lines"))
        .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|i| i as u64)))
        .unwrap_or(TAIL_LINES_DEFAULT as u64)
        .min(TAIL_LINES_MAX as u64) as usize;
    let verify = obj
        .get("verify")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let allow_risky = obj.get("allow_risky").and_then(|v| v.as_bool()).unwrap_or(false);
    let suggested_target = extract_suggested_target(obj);
    let suggest_once = extract_suggest_once(obj);

    Ok(SkillInput {
        action,
        target,
        manager_type,
        tail_lines,
        verify,
        allow_risky,
        suggested_target,
        suggest_once,
    })
}

// ---------- Output contract (structured result) ----------

#[derive(Debug, Default, Clone, Serialize)]
struct OutputContract {
    status: String,
    service_name: String,
    manager_type: String,
    requested_action: String,
    executed_actions: Vec<String>,
    pre_state: String,
    post_state: String,
    verified: bool,
    key_evidence: Vec<String>,
    failure_reason: String,
    next_step: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    summary: String,
}

impl OutputContract {
    fn ok_summary(&mut self, msg: &str) {
        self.status = "ok".to_string();
        self.summary = msg.to_string();
    }
    fn fail(&mut self, reason: &str) {
        self.status = "error".to_string();
        self.failure_reason = reason.to_string();
    }
    fn add_evidence(&mut self, s: impl AsRef<str>) {
        self.key_evidence.push(s.as_ref().to_string());
    }
}

// ---------- Manager detection ----------

/// Lightweight probe: try systemctl then service for a safe target. Returns None on any failure.
fn detect_manager_for_target(target: &str) -> Option<&'static str> {
    if !is_safe_target(target) {
        return None;
    }
    // Try systemctl is-active (read-only)
    if let Ok(cmd_out) = Command::new("systemctl").args(["is-active", target]).output() {
        if cmd_out.status.code().is_some() {
            let stdout_str = String::from_utf8_lossy(&cmd_out.stdout).into_owned();
            let s = stdout_str.trim();
            if !s.is_empty() && s.len() < 50 && s.chars().all(|c| c.is_ascii_alphabetic() || c == ' ' || c == '(' || c == ')') {
                return Some("systemd");
            }
        }
    }
    // Try service status (read-only)
    if let Ok(out) = Command::new("service").args([target, "status"]).output() {
        if out.status.code().is_some() {
            return Some("service");
        }
    }
    None
}

fn resolve_manager(input: &SkillInput, effective_target: Option<&str>) -> String {
    if let Some(ref mt) = input.manager_type {
        if MANAGER_TYPES.contains(&mt.as_str()) {
            return mt.clone();
        }
    }
    let t = effective_target.or_else(|| input.target.as_deref());
    if let Some(t) = t {
        if RUSTCLAW_SERVICES.contains(&t) {
            return "rustclaw".to_string();
        }
        return detect_manager_for_target(t).unwrap_or("unknown").to_string();
    }
    "unknown".to_string()
}

// ---------- Main entry ----------

/// Builds runner response from execute result. Business failure (out.status == "error") becomes runner status "error".
fn build_runner_response(request_id: String, result: Result<OutputContract, String>) -> Resp {
    match result {
        Ok(out) => {
            let text = serde_json::to_string(&out).unwrap_or_default();
            let is_business_error = out.status == "error";
            Resp {
                request_id,
                status: if is_business_error { "error".to_string() } else { "ok".to_string() },
                text: text.clone(),
                error_text: if is_business_error {
                    Some(if out.failure_reason.is_empty() { "skill reported error".to_string() } else { out.failure_reason })
                } else {
                    None
                },
            }
        }
        Err(err) => Resp {
            request_id,
            status: "error".to_string(),
            text: String::new(),
            error_text: Some(err),
        },
    }
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
        .or_else(ui_key)
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => {
                let request_id = req.request_id.clone();
                let req_ui_key = request_ui_key(&req);
                build_runner_response(
                    request_id.clone(),
                    execute(request_id, req.args, req_ui_key.as_deref()),
                )
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

fn execute(
    _request_id: String,
    args: Value,
    req_user_key: Option<&str>,
) -> Result<OutputContract, String> {
    let input = parse_input(&args)?;

    // Target required for all actions except status (all) and when action is status with no target
    let needs_target = input.action != "status"
        || input.target.is_some()
        || matches!(
            input.action.as_str(),
            "start" | "stop" | "restart" | "reload" | "logs" | "verify"
                | "diagnose_start_failure" | "diagnose_unhealthy_state"
        );
    let target_opt = input.target.as_deref();

    if needs_target && target_opt.map_or(true, |t| t.is_empty()) {
        let mut out = OutputContract::default();
        out.service_name = input.target.clone().unwrap_or_default();
        out.requested_action = input.action.clone();
        out.fail("target (service name) is required for this action and must not be empty");
        out.next_step = "Provide a specific service name in args.target or args.service.".to_string();
        return Ok(out);
    }

    if let Some(t) = target_opt {
        if is_ambiguous_target(t) && is_high_risk_action(&input.action) && !input.allow_risky {
            let mut out = OutputContract::default();
            out.service_name = t.to_string();
            out.requested_action = input.action.clone();
            out.fail("target is ambiguous or too broad for high-risk action (stop/restart); refuse to execute");
            out.next_step = "Use a specific service name and avoid vague targets like backend/services/all.".to_string();
            return Ok(out);
        }
        if input.manager_type.as_deref() != Some("rustclaw") && !RUSTCLAW_SERVICES.contains(&t) {
            if !is_safe_target(t) {
                let mut out = OutputContract::default();
                out.service_name = t.to_string();
                out.fail("target contains invalid characters; use only alphanumeric, dot, dash, underscore");
                return Ok(out);
            }
        }
    }

    // Service discovery (non-rustclaw): normalize alias -> 0 candidates fail, >1 ambiguous, 1 proceed. Skip discovery when manager_type is explicit (caller trusts the name).
    let mut suggestion_used = false;
    let mut suggestion_target = String::new();
    let effective_target_opt: Option<String> = if let Some(t) = target_opt {
        if RUSTCLAW_SERVICES.contains(&t.as_ref()) {
            Some(t.to_string())
        } else if input.manager_type.is_some() {
            Some(normalize_target_alias(t))
        } else {
            let normalized = normalize_target_alias(t);
            let mut candidates = discover_all_candidates(&normalized);
            if candidates.len() != 1 && input.suggest_once {
                if let Some(s) = input.suggested_target.as_deref() {
                    let suggested = normalize_target_alias(s);
                    if !suggested.is_empty() && suggested != normalized {
                        let suggested_candidates = discover_all_candidates(&suggested);
                        if suggested_candidates.len() == 1 {
                            candidates = suggested_candidates;
                            suggestion_used = true;
                            suggestion_target = suggested;
                        }
                    }
                }
            }
            if candidates.is_empty() {
                let mut out = OutputContract::default();
                out.service_name = t.to_string();
                out.requested_action = input.action.clone();
                out.fail("no matching service found for the given target");
                out.next_step = "Provide a more specific service name, or confirm the service exists on this host.".to_string();
                return Ok(out);
            }
            if candidates.len() > 1 {
                let mut out = OutputContract::default();
                out.service_name = t.to_string();
                out.requested_action = input.action.clone();
                out.fail("ambiguous: multiple matching services");
                out.next_step = format!(
                    "Select one concrete service name and retry. candidates: {}",
                    candidates.join(", ")
                );
                return Ok(out);
            }
            Some(candidates[0].clone())
        }
    } else {
        None
    };

    let effective_target = effective_target_opt.as_deref();
    let manager = resolve_manager(&input, effective_target);
    let mut executed = Vec::new();
    let mut out = OutputContract {
        service_name: effective_target.unwrap_or("").to_string(),
        manager_type: manager.clone(),
        requested_action: input.action.clone(),
        executed_actions: Vec::new(),
        ..Default::default()
    };
    if suggestion_used {
        out.add_evidence(format!(
            "used suggested_params fallback: {}",
            suggestion_target
        ));
    }

    // Diagnose actions expand to status + logs
    let (action, do_verify, do_logs_after_fail) = match input.action.as_str() {
        "diagnose_start_failure" | "diagnose_unhealthy_state" => {
            executed.push("status".to_string());
            let (pre_state, evidence) =
                run_status_inner(&input, &manager, effective_target, req_user_key, &mut out);
            out.pre_state = pre_state;
            for e in evidence {
                out.add_evidence(e);
            }
            executed.push("logs".to_string());
            if let Some(t) = effective_target {
                let log_evidence = fetch_logs_inner(t, &manager, input.tail_lines);
                for e in log_evidence {
                    out.add_evidence(e);
                }
            }
            out.executed_actions = executed;
            out.post_state = out.pre_state.clone();
            out.verified = false;
            if out.failure_reason.is_empty() {
                out.ok_summary("Diagnosis: status and recent logs collected.");
            }
            return Ok(out);
        }
        "status" => {
            let (pre_state, evidence) =
                run_status_inner(&input, &manager, effective_target, req_user_key, &mut out);
            out.pre_state = pre_state.clone();
            out.post_state = pre_state;
            for e in evidence {
                out.add_evidence(e);
            }
            out.executed_actions = vec!["status".to_string()];
            out.verified = true;
            if out.failure_reason.is_empty() {
                out.ok_summary(&format!("Status: {}", out.pre_state));
            }
            return Ok(out);
        }
        "logs" => {
            let t = effective_target.ok_or_else(|| "target required for logs".to_string())?;
            let evidence = fetch_logs_inner(t, &manager, input.tail_lines);
            for e in &evidence {
                out.add_evidence(e.clone());
            }
            out.executed_actions = vec!["logs".to_string()];
            out.ok_summary(&format!("Retrieved {} log evidence lines.", evidence.len()));
            return Ok(out);
        }
        "verify" => {
            let t = effective_target.ok_or_else(|| "target required for verify".to_string())?;
            let (state, evidence) = run_verify_inner(t, &manager, req_user_key, &mut out);
            out.post_state = state.clone();
            for e in evidence {
                out.add_evidence(e);
            }
            out.executed_actions = vec!["verify".to_string()];
            out.verified = !state.is_empty() && (state == "active" || state == "running" || state == "active (running)");
            if out.failure_reason.is_empty() {
                out.ok_summary(&format!("Verify: {}", state));
            }
            return Ok(out);
        }
        a => {
            let do_verify = input.verify && matches!(a, "start" | "restart" | "reload");
            (a, do_verify, true)
        }
    };

    let target = effective_target.ok_or_else(|| "target required".to_string())?;

    // Pre-state for state-changing actions
    if matches!(action, "start" | "stop" | "restart" | "reload") {
        executed.push("status".to_string());
        let (pre_state, _) =
            run_status_inner(&input, &manager, Some(target), req_user_key, &mut out);
        out.pre_state = pre_state;
    }

    // Execute control action
    executed.push(action.to_string());
    let control_result = run_control_inner(action, target, &manager, req_user_key, &mut out);
    if !control_result.is_ok() {
        if do_logs_after_fail {
            let evidence = fetch_logs_inner(target, &manager, input.tail_lines);
            for e in evidence {
                out.add_evidence(e);
            }
        }
        out.executed_actions = executed;
        return Ok(out);
    }

    // Optional verify after start/restart/reload
    if do_verify {
        std::thread::sleep(Duration::from_secs(VERIFY_WAIT_SECONDS));
        executed.push("verify".to_string());
        let (post_state, evidence) = run_verify_inner(target, &manager, req_user_key, &mut out);
        out.post_state = post_state.clone();
        for e in evidence {
            out.add_evidence(e);
        }
        let healthy = post_state == "active" || post_state == "running" || post_state == "active (running)";
        out.verified = healthy;
        if !healthy {
            out.fail("Post-action verification failed: service did not reach active/running state.");
            if do_logs_after_fail {
                let log_ev = fetch_logs_inner(target, &manager, input.tail_lines);
                for e in log_ev {
                    out.add_evidence(e);
                }
            }
        }
    } else if matches!(action, "start" | "restart" | "reload") {
        let (post_state, _) = run_verify_inner(target, &manager, req_user_key, &mut out);
        out.post_state = post_state;
    }

    out.executed_actions = executed;
    if out.failure_reason.is_empty() {
        out.ok_summary(&format!("{} completed for {}", action, target));
    }
    Ok(out)
}

// ---------- RustClaw (HTTP) ----------

fn clawd_base_url() -> String {
    std::env::var("CLAWD_BASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:8787".to_string())
}

fn ui_key() -> Option<String> {
    std::env::var("RUSTCLAW_UI_KEY")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn rustclaw_health(
    client: &reqwest::blocking::Client,
    req_user_key: Option<&str>,
) -> Result<Value, String> {
    let base = clawd_base_url();
    let mut req = client.get(format!("{base}/v1/health"));
    let fallback_ui_key = ui_key();
    if let Some(k) = req_user_key.or(fallback_ui_key.as_deref()) {
        req = req.header("x-rustclaw-key", k);
    }
    let resp = req.send().map_err(|e| format!("health request failed: {e}"))?;
    if resp.status().as_u16() == 401 {
        return Err("clawd API 401; missing valid user key".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("health failed: {}", resp.status()));
    }
    let data: Value = resp.json().map_err(|e| format!("health json: {e}"))?;
    Ok(data.get("data").cloned().unwrap_or(data))
}

fn rustclaw_service_state(data: &Value, service: &str) -> (bool, Option<usize>, Option<u64>) {
    match service {
        "clawd" => {
            let rss = data.get("memory_rss_bytes").and_then(|v| v.as_u64());
            (true, Some(1), rss)
        }
        "telegramd" => (
            data.get("telegramd_healthy").and_then(|v| v.as_bool()).unwrap_or(false),
            data.get("telegramd_process_count").and_then(|v| v.as_u64()).map(|u| u as usize),
            data.get("telegramd_memory_rss_bytes").and_then(|v| v.as_u64()),
        ),
        "whatsappd" => (
            data.get("whatsappd_healthy").and_then(|v| v.as_bool()).unwrap_or(false),
            data.get("whatsappd_process_count").and_then(|v| v.as_u64()).map(|u| u as usize),
            data.get("whatsappd_memory_rss_bytes").and_then(|v| v.as_u64()),
        ),
        "whatsapp_webd" => (
            data.get("whatsapp_web_healthy").and_then(|v| v.as_bool()).unwrap_or(false),
            data.get("whatsapp_web_process_count").and_then(|v| v.as_u64()).map(|u| u as usize),
            data.get("whatsapp_web_memory_rss_bytes").and_then(|v| v.as_u64()),
        ),
        "feishud" => (
            data.get("feishud_healthy").and_then(|v| v.as_bool()).unwrap_or(false),
            data.get("feishud_process_count").and_then(|v| v.as_u64()).map(|u| u as usize),
            data.get("feishud_memory_rss_bytes").and_then(|v| v.as_u64()),
        ),
        "larkd" => (
            data.get("larkd_healthy").and_then(|v| v.as_bool()).unwrap_or(false),
            data.get("larkd_process_count").and_then(|v| v.as_u64()).map(|u| u as usize),
            data.get("larkd_memory_rss_bytes").and_then(|v| v.as_u64()),
        ),
        _ => (false, None, None),
    }
}

fn run_status_inner(
    _input: &SkillInput,
    manager: &str,
    target: Option<&str>,
    req_user_key: Option<&str>,
    out: &mut OutputContract,
) -> (String, Vec<String>) {
    let mut evidence = Vec::new();
    match manager {
        "rustclaw" => {
            let client = match reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    out.fail(&format!("http client: {e}"));
                    return ("unknown".to_string(), evidence);
                }
            };
            let data = match rustclaw_health(&client, req_user_key) {
                Ok(d) => d,
                Err(e) => {
                    out.fail(&e);
                    return ("unknown".to_string(), evidence);
                }
            };
            let services: Vec<&str> = target
                .map(|t| vec![t])
                .unwrap_or_else(|| RUSTCLAW_SERVICES.to_vec());
            let mut parts = Vec::new();
            for s in &services {
                let (running, count, rss) = rustclaw_service_state(&data, s);
                let state = if running { "running" } else { "stopped" };
                parts.push(format!("{}={}", s, state));
                evidence.push(format!("{} process_count={} memory_rss_bytes={:?}", s, count.unwrap_or(0), rss));
            }
            let pre_state = parts.join(", ");
            (pre_state, evidence)
        }
        "systemd" => {
            let t = target.unwrap_or("");
            if !is_safe_target(t) {
                out.fail("invalid target for systemd");
                return ("unknown".to_string(), evidence);
            }
            let o = Command::new("systemctl")
                .args(["is-active", t])
                .output();
            match o {
                Ok(outp) => {
                    let s = String::from_utf8_lossy(&outp.stdout).trim().to_string();
                    if s.is_empty() {
                        let e = String::from_utf8_lossy(&outp.stderr);
                        evidence.push(e.to_string());
                        ("inactive".to_string(), evidence)
                    } else {
                        evidence.push(format!("systemctl is-active: {}", s));
                        (s, evidence)
                    }
                }
                Err(e) => {
                    out.fail(&format!("systemctl failed: {e}"));
                    ("unknown".to_string(), evidence)
                }
            }
        }
        "service" => {
            let t = target.unwrap_or("");
            if !is_safe_target(t) {
                out.fail("invalid target for service");
                return ("unknown".to_string(), evidence);
            }
            let o = Command::new("service").args([t, "status"]).output();
            match o {
                Ok(outp) => {
                    let s = String::from_utf8_lossy(&outp.stdout);
                    let first = s.lines().next().unwrap_or("").to_string();
                    evidence.push(first.clone());
                    let state = if outp.status.success() { "running" } else { "stopped" };
                    (state.to_string(), evidence)
                }
                Err(e) => {
                    out.fail(&format!("service status failed: {e}"));
                    ("unknown".to_string(), evidence)
                }
            }
        }
        _ => {
            out.fail(&format!("manager {} not implemented for status", manager));
            ("unknown".to_string(), evidence)
        }
    }
}

fn run_verify_inner(
    target: &str,
    manager: &str,
    req_user_key: Option<&str>,
    out: &mut OutputContract,
) -> (String, Vec<String>) {
    let mut evidence = Vec::new();
    match manager {
        "rustclaw" => {
            let client = match reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
            {
                Ok(c) => c,
                Err(_) => return ("unknown".to_string(), evidence),
            };
            let data = match rustclaw_health(&client, req_user_key) {
                Ok(d) => d,
                Err(_) => return ("unknown".to_string(), evidence),
            };
            let (running, _, _) = rustclaw_service_state(&data, target);
            let state = if running { "running" } else { "stopped" };
            evidence.push(format!("health check: {}", state));
            (state.to_string(), evidence)
        }
        "systemd" => {
            if !is_safe_target(target) {
                return ("unknown".to_string(), evidence);
            }
            let o = Command::new("systemctl").args(["is-active", target]).output();
            match o {
                Ok(outp) => {
                    let s = String::from_utf8_lossy(&outp.stdout).trim().to_string();
                    evidence.push(format!("systemctl is-active: {}", s));
                    (s, evidence)
                }
                Err(_) => ("unknown".to_string(), evidence),
            }
        }
        "service" => {
            if !is_safe_target(target) {
                return ("unknown".to_string(), evidence);
            }
            let o = Command::new("service").args([target, "status"]).output();
            match o {
                Ok(outp) => {
                    let state = if outp.status.success() { "running" } else { "stopped" };
                    (state.to_string(), evidence)
                }
                Err(_) => ("unknown".to_string(), evidence),
            }
        }
        _ => {
            out.fail(&format!("manager {} not implemented for verify", manager));
            ("unknown".to_string(), evidence)
        }
    }
}

fn run_control_inner(
    action: &str,
    target: &str,
    manager: &str,
    req_user_key: Option<&str>,
    out: &mut OutputContract,
) -> Result<(), ()> {
    let effective_action = if action == "reload" && manager == "rustclaw" {
        "restart"
    } else {
        action
    };

    match manager {
        "rustclaw" => {
            if !RUSTCLAW_SERVICES.contains(&target) {
                out.fail(&format!("service {} not in RustClaw whitelist", target));
                return Err(());
            }
            if target == "clawd" && matches!(effective_action, "start" | "stop" | "restart") {
                out.fail("clawd cannot be started/stopped/restarted via this skill");
                return Err(());
            }
            let base = clawd_base_url();
            let url = format!("{base}/v1/services/{target}/{effective_action}");
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(|e| {
                    out.fail(&format!("http client: {e}"));
            })?;
            let mut req = client.post(&url);
            let fallback_ui_key = ui_key();
            if let Some(k) = req_user_key.or(fallback_ui_key.as_deref()) {
                req = req.header("x-rustclaw-key", k);
            }
            let resp = req.send().map_err(|e| {
                out.fail(&format!("request failed: {e}"));
            })?;
            if resp.status().as_u16() == 401 {
                out.fail("clawd API 401; missing valid user key");
                return Err(());
            }
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().unwrap_or_default();
                out.fail(&format!("{} {}", status, body));
                return Err(());
            }
            let data: Value = resp.json().unwrap_or_default();
            let msg = data
                .get("data")
                .and_then(|d| d.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or("done");
            out.add_evidence(msg);
            Ok(())
        }
        "systemd" => {
            if !is_safe_target(target) {
                out.fail("invalid target for systemd");
                return Err(());
            }
            let cmd = match effective_action {
                "start" => "start",
                "stop" => "stop",
                "restart" => "restart",
                "reload" => "reload",
                _ => {
                    out.fail(&format!("action {} not supported for systemd", effective_action));
                    return Err(());
                }
            };
            let o = Command::new("systemctl")
                .args(["--no-ask-password", cmd, target])
                .output();
            match o {
                Ok(outp) => {
                    if outp.status.success() {
                        out.add_evidence(format!("systemctl {} {}", cmd, target));
                        Ok(())
                    } else {
                        let message = command_output_text(&outp);
                        if looks_like_permission_error(&message) {
                            let o2 = Command::new("sudo")
                                .args(["-n", "systemctl", "--no-ask-password", cmd, target])
                                .output();
                            match o2 {
                                Ok(outp2) => {
                                    if outp2.status.success() {
                                        out.add_evidence(format!("systemctl {} {}", cmd, target));
                                        Ok(())
                                    } else {
                                        let sudo_message = command_output_text(&outp2);
                                        out.fail("unable to execute via sudo");
                                        out.add_evidence(format!("sudo failed: {}", sudo_message));
                                        out.next_step = "Use an account with sudo privileges, or run the command manually.".to_string();
                                        Err(())
                                    }
                                }
                                Err(e) => {
                                    out.fail("unable to execute via sudo");
                                    out.add_evidence(format!("sudo launch failed: {e}"));
                                    out.next_step = "Use an account with sudo privileges, or run the command manually.".to_string();
                                    Err(())
                                }
                            }
                        } else {
                            out.fail(&format!("systemctl {} failed: {}", cmd, message));
                            Err(())
                        }
                    }
                }
                Err(e) => {
                    out.fail(&format!("systemctl: {e}"));
                    Err(())
                }
            }
        }
        "service" => {
            if !is_safe_target(target) {
                out.fail("invalid target for service");
                return Err(());
            }
            let cmd = match effective_action {
                "start" => "start",
                "stop" => "stop",
                "restart" => "restart",
                "reload" => "reload",
                _ => {
                    out.fail(&format!("action {} not supported for service", effective_action));
                    return Err(());
                }
            };
            let o = Command::new("service").args([target, cmd]).output();
            match o {
                Ok(outp) => {
                    if outp.status.success() {
                        out.add_evidence(format!("service {} {}", target, cmd));
                        Ok(())
                    } else {
                        let message = command_output_text(&outp);
                        if looks_like_permission_error(&message) {
                            let o2 = Command::new("sudo")
                                .args(["-n", "service", target, cmd])
                                .output();
                            match o2 {
                                Ok(outp2) => {
                                    if outp2.status.success() {
                                        out.add_evidence(format!("service {} {}", target, cmd));
                                        Ok(())
                                    } else {
                                        let sudo_message = command_output_text(&outp2);
                                        out.fail("unable to execute via sudo");
                                        out.add_evidence(format!("sudo failed: {}", sudo_message));
                                        out.next_step = "Use an account with sudo privileges, or run the command manually.".to_string();
                                        Err(())
                                    }
                                }
                                Err(e) => {
                                    out.fail("unable to execute via sudo");
                                    out.add_evidence(format!("sudo launch failed: {e}"));
                                    out.next_step = "Use an account with sudo privileges, or run the command manually.".to_string();
                                    Err(())
                                }
                            }
                        } else {
                            out.fail(&format!("service {} {} failed: {}", target, cmd, message));
                            Err(())
                        }
                    }
                }
                Err(e) => {
                    out.fail(&format!("service: {e}"));
                    Err(())
                }
            }
        }
        _ => {
            out.fail(&format!("manager {} does not support lifecycle control", manager));
            Err(())
        }
    }
}

fn fetch_logs_inner(target: &str, manager: &str, tail_lines: usize) -> Vec<String> {
    let mut evidence = Vec::new();
    match manager {
        "rustclaw" => {
            if !RUSTCLAW_SERVICES.contains(&target) {
                evidence.push(format!("service {} not in whitelist, no log path", target));
                return evidence;
            }
            let root = workspace_root();
            let path = root.join("logs").join(format!("{}.log", target));
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let lines: Vec<&str> = content.lines().collect();
                    let from = lines.len().saturating_sub(tail_lines);
                    let slice = &lines[from..];
                    let summary: String = slice
                        .iter()
                        .rev()
                        .take(20)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("; ");
                    evidence.push(format!("last {} lines (total {}); recent: {}", slice.len(), lines.len(), if summary.len() > 400 { format!("{}...", &summary[..400]) } else { summary }));
                }
                Err(e) => {
                    evidence.push(format!("read log failed: {} ({})", path.display(), e));
                }
            }
        }
        "systemd" => {
            if !is_safe_target(target) {
                return evidence;
            }
            let o = Command::new("journalctl")
                .args(["-u", target, "-n", &tail_lines.to_string(), "--no-pager"])
                .output();
            if let Ok(outp) = o {
                let s = String::from_utf8_lossy(&outp.stdout);
                let last: String = s.lines().rev().take(10).collect::<Vec<_>>().join(" ");
                evidence.push(format!("journalctl last {} lines; recent: {}", s.lines().count(), if last.len() > 300 { format!("{}...", &last[..300]) } else { last }));
            }
        }
        _ => {
            evidence.push(format!("manager {} logs not implemented", manager));
        }
    }
    evidence
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn target_missing_returns_structured_error() {
        let args = json!({"action": "start"});
        let out = execute("req-1".to_string(), args).expect("execute must return Ok(OutputContract)");
        assert_eq!(out.status, "error");
        assert!(!out.failure_reason.is_empty(), "failure_reason must be set");
        assert!(!out.next_step.is_empty());
    }

    #[test]
    fn ambiguous_target_blocks_high_risk_action() {
        let args = json!({"action": "restart", "target": "\u{540E}\u{7AEF}"});
        let out = execute("req-2".to_string(), args).expect("execute must return Ok(OutputContract)");
        assert_eq!(out.status, "error");
        assert!(
            out.failure_reason.contains("ambiguous") || out.failure_reason.contains("high-risk"),
            "expected ambiguous/high-risk refusal: {}",
            out.failure_reason
        );
    }

    #[test]
    fn business_failure_produces_runner_error() {
        let args = json!({"action": "start"});
        let out = execute("req-bf".to_string(), args).unwrap();
        assert_eq!(out.status, "error");
        let resp = build_runner_response("req-bf".to_string(), Ok(out));
        assert_eq!(resp.status, "error");
        assert!(resp.error_text.is_some());
    }

    #[test]
    fn status_failure_not_overwritten_by_ok_summary() {
        let args = json!({"action": "status", "target": "nonexistent_xyz_123"});
        let out = execute("req-status".to_string(), args).unwrap();
        assert_eq!(out.status, "error", "unknown manager or status failure must set status=error");
        assert!(!out.failure_reason.is_empty());
    }

    #[test]
    fn verify_failure_not_overwritten_by_ok_summary() {
        let args = json!({"action": "verify", "target": "nonexistent_xyz_456"});
        let out = execute("req-verify".to_string(), args).unwrap();
        assert_eq!(out.status, "error", "unknown manager for verify must set status=error");
        assert!(!out.failure_reason.is_empty());
    }

    #[test]
    fn manager_rustclaw_whitelist() {
        let args = json!({"action": "status", "target": "clawd"});
        let out = execute("req-m1".to_string(), args).unwrap();
        assert_eq!(out.manager_type, "rustclaw");
    }

    #[test]
    fn manager_explicit_type() {
        let args = json!({"action": "status", "target": "nginx", "manager_type": "systemd"});
        let out = execute("req-m2".to_string(), args).unwrap();
        assert_eq!(out.manager_type, "systemd");
    }

    #[test]
    fn manager_unknown_or_detected() {
        let args = json!({"action": "status", "target": "nonexistent_svc_xyz_789"});
        let out = execute("req-m3".to_string(), args).unwrap();
        assert!(
            out.manager_type == "unknown" || out.manager_type == "systemd" || out.manager_type == "service",
            "fallback or detected: {}",
            out.manager_type
        );
    }

    #[test]
    fn output_contract_has_required_keys() {
        let args = json!({"action": "start"});
        let out = execute("req-3".to_string(), args).unwrap();
        let text = serde_json::to_string(&out).unwrap();
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let required = ["status", "service_name", "manager_type", "requested_action", "executed_actions", "key_evidence", "failure_reason"];
        for key in required {
            assert!(parsed.get(key).is_some(), "output must contain key: {}", key);
        }
    }

    #[test]
    fn safe_target_accepts_alphanumeric_and_dots() {
        assert!(is_safe_target("nginx"));
        assert!(is_safe_target("redis-server"));
        assert!(is_safe_target("unit@.service"));
        assert!(!is_safe_target(""));
        assert!(!is_safe_target("a;b"));
        assert!(!is_safe_target("/etc/passwd"));
    }

    #[test]
    fn ambiguous_target_detection() {
        assert!(is_ambiguous_target("\u{540E}\u{7AEF}"));
        assert!(is_ambiguous_target("\u{670D}\u{52A1}\u{4EEC}"));
        assert!(is_ambiguous_target("all"));
        assert!(is_ambiguous_target("  ALL  "));
        assert!(!is_ambiguous_target("nginx"));
        assert!(!is_ambiguous_target("clawd"));
    }

    #[test]
    fn high_risk_actions() {
        assert!(is_high_risk_action("stop"));
        assert!(is_high_risk_action("restart"));
        assert!(!is_high_risk_action("start"));
        assert!(!is_high_risk_action("status"));
    }

    #[test]
    fn normalize_target_alias_maps_common_aliases() {
        assert_eq!(normalize_target_alias("nginx"), "nginx");
        assert_eq!(normalize_target_alias("mysql"), "mysql");
        assert_eq!(normalize_target_alias("mysqld"), "mysql");
        assert_eq!(normalize_target_alias("redis"), "redis");
        assert_eq!(normalize_target_alias("redis-server"), "redis");
        assert_eq!(normalize_target_alias("postgres"), "postgresql");
        assert_eq!(normalize_target_alias("postgresql"), "postgresql");
        assert_eq!(normalize_target_alias("docker"), "docker");
        assert_eq!(normalize_target_alias("dockerd"), "docker");
        assert_eq!(normalize_target_alias("sshd"), "sshd");
        assert_eq!(normalize_target_alias("ssh"), "sshd");
        assert_eq!(normalize_target_alias("cron"), "cron");
        assert_eq!(normalize_target_alias("crond"), "cron");
    }

    #[test]
    fn normalize_target_alias_strips_service_suffix() {
        assert_eq!(normalize_target_alias("nginx\u{670D}\u{52A1}"), "nginx");
        assert_eq!(normalize_target_alias("mysql \u{670D}\u{52A1}"), "mysql");
        assert_eq!(normalize_target_alias("redis service"), "redis");
    }
}

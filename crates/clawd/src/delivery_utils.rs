use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde_json::Value;

use crate::intent_router::{IntentOutputContract, OutputResponseShape};
use crate::AppState;

pub(crate) fn extract_delivery_file_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("FILE:") {
            out.push(format!("FILE:{}", rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("IMAGE_FILE:") {
            out.push(format!("FILE:{}", rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("IMAGE_URL:") {
            out.push(format!("IMAGE_URL:{}", rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("VIDEO_URL:") {
            out.push(format!("VIDEO_URL:{}", rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("FILE_URL:") {
            out.push(format!("FILE_URL:{}", rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("MEDIA_URL:") {
            out.push(format!("MEDIA_URL:{}", rest.trim()));
        }
    }
    out
}

pub(crate) fn intercept_response_text_for_delivery(text: &str) -> String {
    text.trim().to_string()
}

pub(crate) fn intercept_response_payload_for_delivery(
    state: &AppState,
    user_request: &str,
    wants_file_delivery: bool,
    output_contract: &IntentOutputContract,
    text: String,
    messages: Vec<String>,
) -> (String, Vec<String>) {
    let mut seen = HashSet::new();
    let mut normalized_messages = messages
        .into_iter()
        .filter_map(|msg| normalize_delivery_message(state, &msg))
        .filter(|msg| !msg.is_empty())
        .filter(|msg| seen.insert(msg.clone()))
        .collect::<Vec<_>>();
    let mut normalized_text = normalize_delivery_message(state, &text)
        .or_else(|| normalized_messages.first().cloned())
        .unwrap_or_default();
    enforce_explicit_path_delivery_contract(
        state,
        user_request,
        wants_file_delivery
            || output_contract.delivery_required
            || matches!(output_contract.response_shape, OutputResponseShape::FileToken),
        &mut normalized_text,
        &mut normalized_messages,
    );
    enforce_output_contract(
        state,
        user_request,
        output_contract,
        &mut normalized_text,
        &mut normalized_messages,
    );
    (normalized_text, normalized_messages)
}

fn enforce_output_contract(
    state: &AppState,
    user_request: &str,
    output_contract: &IntentOutputContract,
    normalized_text: &mut String,
    normalized_messages: &mut Vec<String>,
) {
    match output_contract.response_shape {
        OutputResponseShape::OneSentence => {
            *normalized_text = take_first_sentence(normalized_text);
        }
        OutputResponseShape::Scalar => {
            if let Some(scalar) = extract_scalar_literal(normalized_text) {
                *normalized_text = scalar;
            }
        }
        _ => {}
    }

    let file_contract = output_contract.delivery_required
        || matches!(output_contract.response_shape, OutputResponseShape::FileToken);
    if !file_contract || response_has_any_delivery_token(normalized_text, normalized_messages) {
        return;
    }

    if let Some(path) = find_resolvable_path(state, user_request)
        .or_else(|| find_resolvable_path(state, normalized_text))
    {
        let token = format!("FILE:{}", path.display());
        *normalized_text = token.clone();
        if !normalized_messages.iter().any(|m| m == &token) {
            normalized_messages.push(token);
        }
        return;
    }

    *normalized_text = "File not found.".to_string();
    normalized_messages.retain(|msg| !msg.trim_start().starts_with("FILE:"));
}

fn enforce_explicit_path_delivery_contract(
    state: &AppState,
    user_request: &str,
    wants_file_delivery: bool,
    normalized_text: &mut String,
    normalized_messages: &mut Vec<String>,
) {
    if !wants_file_delivery {
        return;
    }
    let Some(raw_path) = extract_explicit_path_from_request(user_request) else {
        return;
    };
    let path_token = trim_path_token(&raw_path);
    if path_token.is_empty() {
        return;
    }
    if let Some(resolved) = resolve_existing_delivery_path(state, &path_token) {
        let expected = format!("FILE:{}", resolved.display());
        if !response_has_same_file_token(normalized_text, normalized_messages, &resolved) {
            *normalized_text = expected.clone();
            if !normalized_messages.iter().any(|v| v == &expected) {
                normalized_messages.push(expected);
            }
        }
        return;
    }
    *normalized_text = format!("File not found: {}", path_token);
    normalized_messages.retain(|msg| !msg.trim_start().starts_with("FILE:"));
}

fn extract_explicit_path_from_request(input: &str) -> Option<String> {
    for token in input.split_whitespace() {
        let trimmed = trim_path_token(token);
        if trimmed.starts_with('/') || trimmed.starts_with("./") || trimmed.starts_with("../") {
            return Some(trimmed);
        }
    }
    None
}

fn find_resolvable_path(state: &AppState, text: &str) -> Option<PathBuf> {
    let path = extract_explicit_path_from_request(text)?;
    resolve_existing_delivery_path(state, &path)
}

fn response_has_any_delivery_token(text: &str, messages: &[String]) -> bool {
    !extract_delivery_file_tokens(text).is_empty()
        || messages
            .iter()
            .any(|m| !extract_delivery_file_tokens(m).is_empty())
}

fn take_first_sentence(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut buf = String::new();
    for ch in trimmed.chars() {
        if ch == '\n' || ch == '\r' {
            break;
        }
        buf.push(ch);
        if matches!(ch, '。' | '.' | '!' | '?' | '！' | '？') {
            break;
        }
    }
    let out = buf.trim();
    if out.is_empty() {
        trimmed.to_string()
    } else {
        out.to_string()
    }
}

fn extract_scalar_literal(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if is_scalar_literal(trimmed) {
        return Some(trimmed.to_string());
    }
    for token in trimmed.split_whitespace() {
        if is_scalar_literal(token) {
            return Some(token.to_string());
        }
    }
    None
}

fn is_scalar_literal(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let s = s.trim();
    if s.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    s.parse::<f64>().is_ok()
}

fn response_has_same_file_token(text: &str, messages: &[String], expected: &Path) -> bool {
    let expected_str = expected.to_string_lossy().to_string();
    let mut candidates = Vec::with_capacity(messages.len() + 1);
    candidates.push(text.to_string());
    candidates.extend_from_slice(messages);
    candidates.iter().any(|msg| {
        extract_delivery_file_tokens(msg).iter().any(|token| {
            extract_file_path_from_delivery_token(token)
                .map(|path| {
                    let p = if Path::new(&path).is_absolute() {
                        PathBuf::from(&path)
                    } else {
                        expected
                            .parent()
                            .map(|parent| parent.join(&path))
                            .unwrap_or_else(|| PathBuf::from(&path))
                    };
                    p.canonicalize()
                        .ok()
                        .map(|cp| cp == expected)
                        .unwrap_or_else(|| path == expected_str)
                })
                .unwrap_or(false)
        })
    })
}

fn normalize_delivery_message(state: &AppState, text: &str) -> Option<String> {
    let normalized = intercept_response_text_for_delivery(text);
    if normalized.is_empty() {
        return None;
    }
    let trimmed = normalized.trim();
    if looks_like_tool_call_artifact(trimmed) {
        return None;
    }
    if let Some(path) = trimmed
        .strip_prefix("FILE:")
        .or_else(|| trimmed.strip_prefix("IMAGE_FILE:"))
    {
        let resolved = resolve_existing_delivery_path(state, path)?;
        return Some(format!("FILE:{}", resolved.display()));
    }
    if let Some(url) = trimmed
        .strip_prefix("IMAGE_URL:")
        .or_else(|| trimmed.strip_prefix("VIDEO_URL:"))
        .or_else(|| trimmed.strip_prefix("FILE_URL:"))
        .or_else(|| trimmed.strip_prefix("MEDIA_URL:"))
    {
        let url = trim_path_token(url);
        if url.is_empty() {
            return None;
        }
        let prefix = if trimmed.starts_with("IMAGE_URL:") {
            "IMAGE_URL:"
        } else if trimmed.starts_with("VIDEO_URL:") {
            "VIDEO_URL:"
        } else if trimmed.starts_with("FILE_URL:") {
            "FILE_URL:"
        } else {
            "MEDIA_URL:"
        };
        return Some(format!("{prefix}{url}"));
    }
    Some(normalized)
}

fn looks_like_tool_call_artifact(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with("[TOOL_CALL]")
        || trimmed.contains("[/TOOL_CALL]")
        || (trimmed.contains("{tool =>") && trimmed.contains("args =>"))
}

fn resolve_existing_delivery_path(state: &AppState, raw_path: &str) -> Option<PathBuf> {
    let trimmed = trim_path_token(raw_path);
    if trimmed.is_empty() {
        return None;
    }
    let candidate = if Path::new(&trimmed).is_absolute() {
        PathBuf::from(&trimmed)
    } else {
        state.workspace_root.join(&trimmed)
    };
    let canonical = candidate.canonicalize().ok()?;
    if !canonical.is_file() {
        return None;
    }
    Some(canonical)
}

pub(crate) fn collect_recent_image_candidates(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    limit: usize,
) -> Vec<String> {
    let Some(user_key) = user_key.map(str::trim).filter(|v| !v.is_empty()) else {
        return Vec::new();
    };
    let db = match state.db.lock() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let mut mem_stmt = match db.prepare(
        "SELECT content
         FROM memories
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3 AND role = 'assistant'
         ORDER BY id DESC
         LIMIT 120",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    if let Ok(rows) = mem_stmt.query_map(params![user_id, chat_id, user_key], |row| {
        row.get::<_, String>(0)
    }) {
        for row in rows.flatten() {
            let tokens = extract_delivery_file_tokens(&row);
            for t in tokens {
                if let Some(reference) = extract_image_reference_from_delivery_token(&t) {
                    if seen.insert(reference.clone()) {
                        out.push(reference);
                    }
                }
            }
        }
    }

    let mut task_stmt = match db.prepare(
        "SELECT payload_json, result_json
         FROM tasks
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3 AND kind = 'run_skill' AND status = 'succeeded'
         ORDER BY rowid DESC
         LIMIT ?4",
    ) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = task_stmt
        .query_map(params![user_id, chat_id, user_key, limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
    {
        for row in rows.flatten() {
            let (payload_json, result_json) = row;
            if let Ok(payload) = serde_json::from_str::<Value>(&payload_json) {
                collect_image_paths_from_task_payload(&payload, &mut out, &mut seen);
            }
            if let Some(result) = result_json {
                if let Ok(v) = serde_json::from_str::<Value>(&result) {
                    if let Some(text) = v.get("text").and_then(|x| x.as_str()) {
                        for t in extract_delivery_file_tokens(text) {
                            if let Some(reference) = extract_image_reference_from_delivery_token(&t)
                            {
                                if seen.insert(reference.clone()) {
                                    out.push(reference);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

fn extract_file_path_from_delivery_token(token: &str) -> Option<String> {
    token
        .strip_prefix("FILE:")
        .or_else(|| token.strip_prefix("IMAGE_FILE:"))
        .map(trim_path_token)
        .filter(|s| !s.is_empty())
}

fn extract_image_reference_from_delivery_token(token: &str) -> Option<String> {
    if let Some(path) = extract_file_path_from_delivery_token(token) {
        if is_image_file_path(&path) {
            return Some(path);
        }
    }
    token
        .strip_prefix("IMAGE_URL:")
        .map(trim_path_token)
        .filter(|s| is_remote_image_url(s))
}

fn trim_path_token(token: &str) -> String {
    token
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '`' | '，' | ',' | ':' | '：' | ';' | '。' | ')' | '(' | '）' | '（'
            )
        })
        .to_string()
}

fn is_image_file_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
        || lower.ends_with(".bmp")
}

fn is_remote_image_url(url: &str) -> bool {
    let lower = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")) && is_image_file_path(&lower)
}

fn merge_image_candidate_paths_from_args(
    args: &serde_json::Map<String, Value>,
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if let Some(images) = args.get("images").and_then(|v| v.as_array()) {
        for item in images {
            let path = item
                .as_object()
                .and_then(|m| m.get("path"))
                .and_then(|v| v.as_str())
                .or_else(|| item.as_str());
            if let Some(path) = path {
                let p = path.trim().to_string();
                if is_image_file_path(&p) && seen.insert(p.clone()) {
                    out.push(p);
                }
            }
        }
    }
    let path = args
        .get("image")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("path"))
        .and_then(|v| v.as_str())
        .or_else(|| args.get("image").and_then(|v| v.as_str()));
    if let Some(path) = path {
        let p = path.trim().to_string();
        if is_image_file_path(&p) && seen.insert(p.clone()) {
            out.push(p);
        }
    }
    if let Some(path) = args.get("output_path").and_then(|v| v.as_str()) {
        let p = path.trim().to_string();
        if is_image_file_path(&p) && seen.insert(p.clone()) {
            out.push(p);
        }
    }
}

fn collect_image_paths_from_task_payload(
    payload: &Value,
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    let Some(args) = payload.get("args").and_then(|v| v.as_object()) else {
        return;
    };
    merge_image_candidate_paths_from_args(args, out, seen);
}

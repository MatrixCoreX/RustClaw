use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde_json::Value;

use crate::AppState;

pub(crate) fn extract_delivery_file_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("FILE:") {
            out.push(format!("FILE:{}", rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("IMAGE_FILE:") {
            out.push(format!("FILE:{}", rest.trim()));
        }
    }
    out
}

pub(crate) fn intercept_response_text_for_delivery(text: &str) -> String {
    text.trim().to_string()
}

pub(crate) fn intercept_response_payload_for_delivery(
    state: &AppState,
    text: String,
    messages: Vec<String>,
) -> (String, Vec<String>) {
    let mut seen = HashSet::new();
    let normalized_messages = messages
        .into_iter()
        .filter_map(|msg| normalize_delivery_message(state, &msg))
        .filter(|msg| !msg.is_empty())
        .filter(|msg| seen.insert(msg.clone()))
        .collect::<Vec<_>>();
    let normalized_text =
        normalize_delivery_message(state, &text).unwrap_or_else(|| "File not found.".to_string());
    (normalized_text, normalized_messages)
}

fn normalize_delivery_message(state: &AppState, text: &str) -> Option<String> {
    let normalized = intercept_response_text_for_delivery(text);
    if normalized.is_empty() {
        return None;
    }
    let trimmed = normalized.trim();
    if let Some(path) = trimmed
        .strip_prefix("FILE:")
        .or_else(|| trimmed.strip_prefix("IMAGE_FILE:"))
    {
        let resolved = resolve_existing_delivery_path(state, path)?;
        return Some(format!("FILE:{}", resolved.display()));
    }
    Some(normalized)
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
                if let Some(path) = extract_file_path_from_delivery_token(&t) {
                    if is_image_file_path(&path) && seen.insert(path.clone()) {
                        out.push(path);
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
                            if let Some(path) = extract_file_path_from_delivery_token(&t) {
                                if is_image_file_path(&path) && seen.insert(path.clone()) {
                                    out.push(path);
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

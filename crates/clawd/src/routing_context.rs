use regex::Regex;
use rusqlite::params;
use serde_json::Value;

use crate::{AppState, ClaimedTask};

#[derive(Debug, Clone)]
struct ExecutionAnchor {
    ts: String,
    skill: String,
    domain: String,
    subject: Option<String>,
    symbol: Option<String>,
    request: String,
    result: String,
}

fn query_recent_execution_rows(
    db: &rusqlite::Connection,
    user_id: i64,
    chat_id: i64,
    user_key: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<(String, String, String, String)>> {
    let mut stmt = db.prepare(
        "SELECT kind, payload_json, result_json, updated_at
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND status = 'succeeded'
           AND (
             (?3 IS NOT NULL AND user_key = ?3)
             OR (?3 IS NULL AND (user_key IS NULL OR TRIM(user_key) = ''))
           )
         ORDER BY CAST(updated_at AS INTEGER) DESC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![user_id, chat_id, user_key, limit as i64], |row| {
        let kind: String = row.get(0)?;
        let payload_json: String = row.get(1)?;
        let result_json: String = row.get(2)?;
        let updated_at: String = row.get(3)?;
        Ok((kind, payload_json, result_json, updated_at))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(crate) fn build_recent_execution_context(
    state: &AppState,
    task: &ClaimedTask,
    limit: usize,
) -> String {
    let rows = load_recent_execution_rows(state, task, limit);
    if rows.is_empty() {
        return "<none>".to_string();
    }
    render_recent_execution_context(&rows, limit)
}

pub(crate) fn build_recent_execution_anchor_context(state: &AppState, task: &ClaimedTask) -> String {
    let rows = load_recent_execution_rows(state, task, 4);
    if rows.is_empty() {
        return "<none>".to_string();
    }
    render_recent_execution_anchor_context(&rows)
}

fn load_recent_execution_rows(
    state: &AppState,
    task: &ClaimedTask,
    limit: usize,
) -> Vec<(String, String, String, String)> {
    let user_key = task
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let legacy_chat_id = user_key
        .map(crate::stable_i64_from_key)
        .filter(|legacy| *legacy != task.chat_id);
    let db = match state.db.lock() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let rows = match query_recent_execution_rows(&db, task.user_id, task.chat_id, user_key, limit) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    if !rows.is_empty() {
        return rows;
    }

    if let Some(legacy_chat_id) = legacy_chat_id {
        return query_recent_execution_rows(&db, task.user_id, legacy_chat_id, user_key, limit)
            .unwrap_or_default();
    }

    Vec::new()
}

fn render_recent_execution_context(rows: &[(String, String, String, String)], limit: usize) -> String {
    let mut sections = Vec::new();
    let anchor_block = render_recent_execution_anchor_context(rows);
    if anchor_block != "<none>" {
        sections.push(anchor_block);
    }

    let mut items = Vec::new();
    for (kind, payload_json, result_json, updated_at) in rows.iter().take(limit) {
        let req = task_payload_summary(kind, payload_json);
        let result = task_result_summary(result_json);
        items.push(format!(
            "- ts={updated_at} kind={kind} request={} result={}",
            truncate_snippet(&req, 220),
            truncate_snippet(&result, 320)
        ));
    }
    if !items.is_empty() {
        sections.push(format!(
            "### RECENT_EXECUTION_EVENTS\n{}",
            items.join("\n")
        ));
    }

    if sections.is_empty() {
        "<none>".to_string()
    } else {
        sections.join("\n\n")
    }
}

fn render_recent_execution_anchor_context(rows: &[(String, String, String, String)]) -> String {
    let Some(anchor) = rows
        .iter()
        .find_map(|(kind, payload_json, result_json, updated_at)| {
            extract_execution_anchor(kind, payload_json, result_json, updated_at)
        })
    else {
        return "<none>".to_string();
    };

    let mut lines = vec![
        format!("- latest_succeeded_ts={}", anchor.ts),
        format!("- latest_succeeded_skill={}", anchor.skill),
        format!("- latest_domain={}", anchor.domain),
    ];
    if let Some(subject) = anchor.subject.as_deref().filter(|v| !v.is_empty()) {
        lines.push(format!("- latest_subject={subject}"));
    }
    if let Some(symbol) = anchor.symbol.as_deref().filter(|v| !v.is_empty()) {
        lines.push(format!("- latest_symbol={symbol}"));
    }
    lines.push(format!(
        "- latest_request={}",
        truncate_snippet(&anchor.request, 180)
    ));
    lines.push(format!(
        "- latest_result={}",
        truncate_snippet(&anchor.result, 220)
    ));
    lines.push(
        "- anchor_rule=For short follow-up requests without an explicit new target, continue from this latest successful subject/domain instead of switching to older memory.".to_string(),
    );
    format!("### RECENT_EXECUTION_ANCHOR\n{}", lines.join("\n"))
}

fn extract_execution_anchor(
    kind: &str,
    payload_json: &str,
    result_json: &str,
    updated_at: &str,
) -> Option<ExecutionAnchor> {
    let request = task_payload_summary(kind, payload_json);
    let result = task_result_summary(result_json);
    let payload = serde_json::from_str::<Value>(payload_json).ok();

    let skill_from_payload = payload
        .as_ref()
        .and_then(|v| v.get("skill_name"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let skill_from_result = extract_skill_from_result(&result);
    let skill = skill_from_payload
        .or(skill_from_result)
        .unwrap_or_else(|| infer_skill_from_text(&request, &result));

    let symbol = extract_symbol_from_payload(payload.as_ref()).or_else(|| extract_symbol_from_text(&result));
    let subject = extract_subject_from_result(&result).or_else(|| extract_subject_from_request(&request));
    let domain = infer_domain(&skill, symbol.as_deref(), subject.as_deref(), &request, &result);

    if skill == "unknown" && symbol.is_none() && subject.is_none() {
        return None;
    }

    Some(ExecutionAnchor {
        ts: updated_at.to_string(),
        skill,
        domain,
        subject,
        symbol,
        request,
        result,
    })
}

fn extract_skill_from_result(result: &str) -> Option<String> {
    let re = Regex::new(r"skill\(([^)]+)\)").ok()?;
    re.captures(result)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string())
        .filter(|v| !v.is_empty())
}

fn infer_skill_from_text(request: &str, result: &str) -> String {
    let combined = format!("{request}\n{result}").to_ascii_lowercase();
    if combined.contains("btcusdt")
        || combined.contains("ethusdt")
        || combined.contains("crypto")
        || combined.contains("加密")
        || combined.contains("币圈")
    {
        "crypto".to_string()
    } else if combined.contains("股票")
        || combined.contains("a股")
        || combined.contains("上证")
        || combined.contains("深证")
        || Regex::new(r"\b[036]\d{5}\b")
            .ok()
            .is_some_and(|re| re.is_match(&combined))
    {
        "stock".to_string()
    } else {
        "unknown".to_string()
    }
}

fn extract_symbol_from_payload(payload: Option<&Value>) -> Option<String> {
    payload
        .and_then(|v| v.get("args"))
        .and_then(|args| {
            args.get("symbol")
                .or_else(|| args.get("code"))
                .and_then(|v| v.as_str())
        })
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn extract_symbol_from_text(text: &str) -> Option<String> {
    let stock_re = Regex::new(r"\[(?:SH|SZ)?(\d{6})\]").ok()?;
    if let Some(symbol) = stock_re
        .captures(text)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
    {
        return Some(symbol);
    }
    let crypto_re = Regex::new(r"\b([A-Z]{2,12}(?:USDT|USD))\b").ok()?;
    crypto_re
        .captures(text)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

fn extract_subject_from_result(result: &str) -> Option<String> {
    let stock_re =
        Regex::new(r"\[(?:SH|SZ)?\d{6}\]\s*([^\s\[]+?)\s+(?:现价|今开|昨收|涨跌幅|最高|最低|成交量|日期)")
            .ok()?;
    if let Some(subject) = stock_re
        .captures(result)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string())
        .filter(|v| !v.is_empty())
    {
        return Some(subject);
    }
    None
}

fn extract_subject_from_request(request: &str) -> Option<String> {
    let trimmed = request.trim();
    let markers = ["查询", "分析", "看看", "查看", "帮我看", "帮我查"];
    let suffixes = [
        "今天", "今日", "涨跌", "行情", "走势", "情况", "后面", "后续", "现在", "当前", "一下",
    ];
    for marker in markers {
        if let Some(idx) = trimmed.find(marker) {
            let mut candidate = trimmed[idx + marker.len()..].trim().to_string();
            for suffix in suffixes {
                if let Some(end) = candidate.find(suffix) {
                    candidate = candidate[..end].trim().to_string();
                    break;
                }
            }
            if !candidate.is_empty() && candidate.chars().count() <= 16 {
                return Some(candidate);
            }
        }
    }
    None
}

fn infer_domain(
    skill: &str,
    symbol: Option<&str>,
    subject: Option<&str>,
    request: &str,
    result: &str,
) -> String {
    let skill_lower = skill.to_ascii_lowercase();
    let combined = format!("{request}\n{result}").to_ascii_lowercase();
    if skill_lower.contains("crypto")
        || symbol.is_some_and(|v| v.ends_with("USDT") || v.ends_with("USD"))
        || combined.contains("btcusdt")
        || combined.contains("ethusdt")
    {
        "crypto".to_string()
    } else if skill_lower.contains("stock")
        || skill_lower.contains("a_stock")
        || subject.is_some_and(|v| v.contains('股'))
        || combined.contains("a股")
        || combined.contains("上证")
        || combined.contains("深证")
        || symbol.is_some_and(|v| v.len() == 6 && v.chars().all(|ch| ch.is_ascii_digit()))
    {
        "cn_stock".to_string()
    } else {
        "general".to_string()
    }
}

fn task_payload_summary(kind: &str, payload_json: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(payload_json) else {
        return payload_json.to_string();
    };
    match kind {
        "ask" => v
            .get("text")
            .and_then(|x| x.as_str())
            .unwrap_or(payload_json)
            .to_string(),
        "run_skill" => {
            let skill = v
                .get("skill_name")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown");
            format!(
                "run_skill:{skill} args={}",
                v.get("args").cloned().unwrap_or(Value::Null)
            )
        }
        _ => payload_json.to_string(),
    }
}

fn task_result_summary(result_json: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(result_json) else {
        return sanitize_result_summary(result_json);
    };
    let text = v
        .get("text")
        .and_then(|x| x.as_str())
        .or_else(|| v.as_str())
        .unwrap_or(result_json)
        .to_string();
    sanitize_result_summary(&text)
}

fn sanitize_result_summary(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let refusal_markers = [
        "no code generation",
        "all executable code is forbidden",
        "不能提供可执行代码",
        "禁止生成任何可执行代码",
        "当前策略明确禁止",
        "不提供java代码示例",
        "不提供可执行的java",
    ];
    if refusal_markers.iter().any(|m| lower.contains(m)) {
        "<assistant policy-style refusal omitted from routing context>".to_string()
    } else {
        text.to_string()
    }
}

fn truncate_snippet(text: &str, max_chars: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max_chars {
        return t.to_string();
    }
    let mut out = String::new();
    for (idx, ch) in t.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        out.push(ch);
    }
    out.push_str("...(truncated)");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_stock_anchor_from_result_text() {
        let anchor = extract_execution_anchor(
            "ask",
            r#"{"text":"查询中芯国际今天涨跌情况"}"#,
            r#"{"text":"subtask#1 skill(stock): success [SH688981] 中芯国际 现价106.020 今开108.540 昨收108.600"}"#,
            "1710668477",
        )
        .expect("anchor");
        assert_eq!(anchor.skill, "stock");
        assert_eq!(anchor.domain, "cn_stock");
        assert_eq!(anchor.symbol.as_deref(), Some("688981"));
        assert_eq!(anchor.subject.as_deref(), Some("中芯国际"));
    }

    #[test]
    fn extracts_crypto_anchor_from_result_text() {
        let anchor = extract_execution_anchor(
            "ask",
            r#"{"text":"分析下行情"}"#,
            r#"{"text":"subtask#1 skill(crypto): success BTCUSDT RSI(14)=54.2"}"#,
            "1710668477",
        )
        .expect("anchor");
        assert_eq!(anchor.skill, "crypto");
        assert_eq!(anchor.domain, "crypto");
        assert_eq!(anchor.symbol.as_deref(), Some("BTCUSDT"));
    }
}

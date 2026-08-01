use super::*;

#[derive(Default)]
pub(super) struct ObservedEvidenceCollector {
    pub(super) items: Vec<Value>,
    pub(super) total_count: usize,
}

impl ObservedEvidenceCollector {
    pub(super) fn push(&mut self, item: Value) {
        self.total_count += 1;
        if self.items.len() < MAX_OBSERVED_EVIDENCE_ITEMS {
            self.items.push(item);
        }
    }
}

pub(super) fn prioritize_observed_evidence_for_storage(items: &mut Vec<Value>) {
    let mut prioritized = Vec::new();
    for leaf in OBSERVED_EVIDENCE_STORAGE_PRIORITY_LEAVES {
        let selected = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let field = item.get("field")?.as_str()?;
                (field.rsplit('.').next() == Some(*leaf))
                    .then_some((index, field.matches('.').count()))
            })
            .min_by_key(|(_, depth)| *depth)
            .map(|(index, _)| index);
        if let Some(index) = selected {
            prioritized.push(items.remove(index));
        }
    }
    prioritized.append(items);
    *items = prioritized;
}

pub(super) fn collect_json_observed_evidence(
    collector: &mut ObservedEvidenceCollector,
    source: &str,
    prefix: &str,
    value: &Value,
    depth: usize,
) {
    if depth > MAX_OBSERVED_EVIDENCE_DEPTH {
        return;
    }
    match value {
        Value::Object(map) => {
            if depth > 0 {
                collector.push(json_observed_evidence_item(source, prefix, value));
            }
            collect_structured_missing_search_evidence(collector, source, prefix, map);
            let mut emitted_priority_keys = BTreeSet::new();
            for key in JSON_EVIDENCE_PRIORITY_KEYS {
                if let Some(child) = map.get(*key) {
                    let field = if prefix.is_empty() {
                        (*key).to_string()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    if *key == "entries" && depth == 0 && prefix.is_empty() {
                        collector.push(json_observed_evidence_item(source, &field, child));
                    } else {
                        collect_json_object_child(collector, source, depth, prefix, key, child);
                        emitted_priority_keys.insert((*key).to_string());
                    }
                }
            }
            for (key, child) in map {
                if emitted_priority_keys.contains(key.as_str()) {
                    continue;
                }
                collect_json_object_child(collector, source, depth, prefix, key, child);
            }
        }
        Value::Array(items) => {
            if depth == 0 || prefix.is_empty() {
                collector.push(json_observed_evidence_item(source, "value", value));
            }
            for (idx, child) in items.iter().take(MAX_OBSERVED_ARRAY_SAMPLES).enumerate() {
                let field = if prefix.is_empty() {
                    format!("[{idx}]")
                } else {
                    format!("{prefix}[{idx}]")
                };
                collector.push(json_observed_evidence_item(source, &field, child));
                if depth < MAX_OBSERVED_EVIDENCE_DEPTH
                    && matches!(child, Value::Object(_) | Value::Array(_))
                {
                    collect_json_observed_evidence(collector, source, &field, child, depth + 1);
                }
            }
        }
        _ => collector.push(json_observed_evidence_item(source, "value", value)),
    }
}

pub(super) fn collect_embedded_http_json_body_evidence(
    collector: &mut ObservedEvidenceCollector,
    value: &Value,
) {
    let collected_preview = value
        .pointer("/extra/body_preview")
        .and_then(Value::as_str)
        .is_some_and(|body| {
            collect_embedded_json_body_string_evidence(
                collector,
                "json_output.extra.body_json",
                body,
            )
        });
    if collected_preview {
        return;
    }
}

pub(super) fn collect_embedded_json_body_string_evidence(
    collector: &mut ObservedEvidenceCollector,
    source: &str,
    body: &str,
) -> bool {
    let body = body.trim();
    if body.is_empty() {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    collect_priority_json_status_scalar_evidence(collector, source, "body", &value, 0);
    collect_json_observed_evidence(collector, source, "body", &value, 0);
    true
}

pub(super) fn collect_priority_json_status_scalar_evidence(
    collector: &mut ObservedEvidenceCollector,
    source: &str,
    prefix: &str,
    value: &Value,
    depth: usize,
) {
    if depth > MAX_OBSERVED_EVIDENCE_DEPTH {
        return;
    }
    match value {
        Value::Object(map) => {
            collect_priority_json_log_status_fields(collector, source, prefix, map);
            let mut emitted_priority_keys = BTreeSet::new();
            for key in JSON_STATUS_SCALAR_PRIORITY_KEYS {
                let Some(child) = map.get(*key) else {
                    continue;
                };
                let field = if prefix.is_empty() {
                    (*key).to_string()
                } else {
                    format!("{prefix}.{key}")
                };
                if json_status_scalar_field_is_priority(&field, child) {
                    collector.push(json_observed_evidence_item(source, &field, child));
                }
                if matches!(child, Value::Object(_) | Value::Array(_)) {
                    collect_priority_json_status_scalar_evidence(
                        collector,
                        source,
                        &field,
                        child,
                        depth + 1,
                    );
                }
                emitted_priority_keys.insert((*key).to_string());
            }
            for (key, child) in map {
                if emitted_priority_keys.contains(key.as_str()) {
                    continue;
                }
                let field = if prefix.is_empty() {
                    key.to_string()
                } else {
                    format!("{prefix}.{key}")
                };
                if json_status_scalar_field_is_priority(&field, child) {
                    collector.push(json_observed_evidence_item(source, &field, child));
                }
                if matches!(child, Value::Object(_) | Value::Array(_)) {
                    collect_priority_json_status_scalar_evidence(
                        collector,
                        source,
                        &field,
                        child,
                        depth + 1,
                    );
                }
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().take(MAX_OBSERVED_ARRAY_SAMPLES).enumerate() {
                let field = if prefix.is_empty() {
                    format!("[{idx}]")
                } else {
                    format!("{prefix}[{idx}]")
                };
                if json_status_scalar_field_is_priority(&field, child) {
                    collector.push(json_observed_evidence_item(source, &field, child));
                }
                if matches!(child, Value::Object(_) | Value::Array(_)) {
                    collect_priority_json_status_scalar_evidence(
                        collector,
                        source,
                        &field,
                        child,
                        depth + 1,
                    );
                }
            }
        }
        _ => {}
    }
}

fn collect_priority_json_log_status_fields(
    collector: &mut ObservedEvidenceCollector,
    source: &str,
    prefix: &str,
    map: &serde_json::Map<String, Value>,
) {
    for (key, child) in map {
        if !normalize_evidence_field(key).ends_with("_log") {
            continue;
        }
        let Some(log_fields) = child.as_object() else {
            continue;
        };
        let log_prefix = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        for field in ["keyword_error_count", "size_bytes"] {
            let Some(value) = log_fields.get(field) else {
                continue;
            };
            if matches!(
                value,
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
            ) {
                collector.push(json_observed_evidence_item(
                    source,
                    &format!("{log_prefix}.{field}"),
                    value,
                ));
            }
        }
    }
}

const JSON_STATUS_SCALAR_PRIORITY_KEYS: &[&str] = &[
    "ok",
    "status",
    "status_code",
    "success_status",
    "healthy",
    "version",
    "worker_state",
    "uptime_seconds",
    "running_length",
    "queue_length",
    "memory_rss_bytes",
    "running_oldest_age_seconds",
    "clawd_process_count",
    "clawd_health_port_open",
    "clawd_visible",
    "db_available",
    "overall_status",
    "telegramd_healthy",
    "telegramd_process_count",
    "channel_gateway_healthy",
    "channel_gateway_process_count",
    "telegram_bot_healthy",
    "telegram_bot_process_count",
    "telegram_configured_bot_count",
    "whatsappd_healthy",
    "whatsappd_process_count",
    "whatsapp_cloud_healthy",
    "whatsapp_cloud_process_count",
    "whatsapp_web_healthy",
    "whatsapp_web_process_count",
    "webd_healthy",
    "webd_process_count",
    "wechatd_healthy",
    "wechatd_process_count",
    "feishud_healthy",
    "feishud_process_count",
    "larkd_healthy",
    "larkd_process_count",
    "user_count",
    "bound_channel_count",
    "hostname",
    "kernel_release",
    "os_family",
    "arch",
    "cpu_count",
    "service_manager",
    "load_avg_1m",
    "load_avg_5m",
    "load_avg_15m",
    "memory_available_bytes",
    "memory_total_bytes",
    "disk_root_available_bytes",
    "disk_root_total_bytes",
];

pub(super) fn json_status_scalar_field_is_priority(field: &str, value: &Value) -> bool {
    let normalized = normalize_evidence_field(field);
    let leaf = normalized_field_leaf(&normalized);
    if leaf == "warnings" && normalized.contains("system_health") && value.is_array() {
        return true;
    }
    if normalized.contains("_log.")
        && matches!(
            leaf,
            "exists" | "keyword_error_count" | "modified_ts" | "size_bytes"
        )
    {
        return matches!(
            value,
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
        );
    }
    if !matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    ) {
        return false;
    }
    matches!(
        leaf,
        "ok" | "status"
            | "status_code"
            | "success_status"
            | "healthy"
            | "version"
            | "worker_state"
            | "uptime_seconds"
            | "running_length"
            | "queue_length"
            | "memory_rss_bytes"
            | "clawd_visible"
            | "db_available"
            | "overall_status"
            | "user_count"
            | "bound_channel_count"
            | "hostname"
            | "kernel_release"
            | "os_family"
            | "arch"
            | "cpu_count"
            | "service_manager"
            | "load_avg_1m"
            | "load_avg_5m"
            | "load_avg_15m"
            | "memory_available_bytes"
            | "memory_total_bytes"
            | "disk_root_available_bytes"
            | "disk_root_total_bytes"
    ) || leaf.ends_with("_healthy")
        || leaf.ends_with("_process_count")
        || leaf.ends_with("_memory_rss_bytes")
        || leaf.ends_with("_status")
        || leaf.ends_with("_state")
        || (matches!(leaf, "name" | "kind" | "scope") && normalized.contains("statuses["))
}

pub(super) fn collect_structured_missing_search_evidence(
    collector: &mut ObservedEvidenceCollector,
    source: &str,
    prefix: &str,
    map: &serde_json::Map<String, Value>,
) {
    let Some(locator) = structured_missing_search_locator(map) else {
        return;
    };
    let field_prefix = if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}.")
    };
    collector.push(text_extracted_evidence_item_with_source(
        &format!("{field_prefix}path"),
        source,
        &locator,
    ));
    collector.push(json_observed_evidence_item(
        source,
        &format!("{field_prefix}exists"),
        &json!(false),
    ));
}

pub(super) fn structured_missing_search_locator(
    map: &serde_json::Map<String, Value>,
) -> Option<String> {
    let action = map
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_evidence_field)?;
    if !matches!(action.as_str(), "find_entries" | "find_name" | "find_path") {
        return None;
    }
    if map.get("count").and_then(Value::as_u64) != Some(0) {
        return None;
    }
    if map
        .get("results")
        .and_then(Value::as_array)
        .is_some_and(|results| !results.is_empty())
    {
        return None;
    }
    map.get("patterns")
        .and_then(Value::as_array)
        .and_then(|patterns| patterns.iter().find_map(structured_search_pattern_locator))
}

pub(super) fn structured_search_pattern_locator(value: &Value) -> Option<String> {
    let locator = value.as_str()?.trim();
    if locator.is_empty()
        || locator.len() > MAX_OBSERVED_EVIDENCE_EXCERPT_CHARS
        || locator.contains(|ch| matches!(ch, '\n' | '\r' | '\0'))
    {
        return None;
    }
    Some(locator.to_string())
}

pub(super) fn collect_json_object_child(
    collector: &mut ObservedEvidenceCollector,
    source: &str,
    depth: usize,
    prefix: &str,
    key: &str,
    child: &Value,
) {
    if key == "_matrix_admission" {
        return;
    }
    let field = if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    };
    collector.push(json_observed_evidence_item(source, &field, child));
    collect_multiline_excerpt_line_evidence(collector, source, &field, child);
    if depth < MAX_OBSERVED_EVIDENCE_DEPTH && matches!(child, Value::Object(_) | Value::Array(_)) {
        let child_source = if depth == 0 && key == "extra" {
            "json_output.extra"
        } else {
            source
        };
        collect_json_observed_evidence(collector, child_source, &field, child, depth + 1);
    }
}

pub(super) fn collect_multiline_excerpt_line_evidence(
    collector: &mut ObservedEvidenceCollector,
    source: &str,
    field: &str,
    value: &Value,
) {
    let Some(text) = value.as_str() else {
        return;
    };
    if !json_field_should_split_multiline_excerpt(field) || !text.contains('\n') {
        return;
    }
    for (idx, line) in sampled_multiline_excerpt_lines(text) {
        collector.push(json!({
            "field": "content_excerpt",
            "source": source,
            "kind": "text",
            "origin_field": field,
            "line_index": idx,
            "excerpt": redacted_text_excerpt(line),
            "hash": stable_trace_hash(line),
        }));
    }
}

pub(super) fn sampled_multiline_excerpt_lines(text: &str) -> Vec<(usize, &str)> {
    let lines = text
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let line = line.trim();
            (!line.is_empty()).then_some((idx, line))
        })
        .collect::<Vec<_>>();
    if lines.len() <= MAX_OBSERVED_MULTILINE_EXCERPT_LINES {
        return lines;
    }

    let mut selected = std::collections::BTreeSet::new();
    for (idx, line) in &lines {
        if line_has_diagnostic_severity_signal(line) {
            selected.insert(*idx);
            if selected.len() >= MAX_OBSERVED_MULTILINE_EXCERPT_LINES {
                break;
            }
        }
    }

    let head_count = MAX_OBSERVED_MULTILINE_EXCERPT_LINES / 2;
    for (idx, _) in lines.iter().take(head_count) {
        if selected.len() >= MAX_OBSERVED_MULTILINE_EXCERPT_LINES {
            break;
        }
        selected.insert(*idx);
    }

    let tail_count = MAX_OBSERVED_MULTILINE_EXCERPT_LINES - head_count;
    let tail_start = lines.len().saturating_sub(tail_count);
    for (idx, _) in lines.iter().skip(tail_start) {
        if selected.len() >= MAX_OBSERVED_MULTILINE_EXCERPT_LINES {
            break;
        }
        selected.insert(*idx);
    }

    for (idx, _) in &lines {
        if selected.len() >= MAX_OBSERVED_MULTILINE_EXCERPT_LINES {
            break;
        }
        selected.insert(*idx);
    }

    lines
        .iter()
        .copied()
        .filter(|(idx, _)| selected.contains(idx))
        .collect()
}

fn line_has_diagnostic_severity_signal(line: &str) -> bool {
    line.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .any(|token| {
            token.eq_ignore_ascii_case("warn")
                || token.eq_ignore_ascii_case("warning")
                || token.eq_ignore_ascii_case("error")
                || token.eq_ignore_ascii_case("fatal")
                || token.eq_ignore_ascii_case("critical")
                || token.eq_ignore_ascii_case("panic")
        })
}

pub(super) fn json_field_should_split_multiline_excerpt(field: &str) -> bool {
    let leaf = normalized_field_leaf(field);
    matches!(leaf, "excerpt" | "content_excerpt")
}

pub(super) fn json_observed_evidence_item(source: &str, field: &str, value: &Value) -> Value {
    let sensitive_field = evidence_field_is_sensitive(field);
    let mut item = serde_json::Map::new();
    item.insert("field".to_string(), json!(field));
    item.insert("source".to_string(), json!(source));
    item.insert("kind".to_string(), json!(json_value_kind(value)));
    match value {
        Value::Object(map) => {
            item.insert(
                "keys".to_string(),
                json!(map
                    .keys()
                    .take(MAX_OBSERVED_EVIDENCE_KEYS)
                    .collect::<Vec<_>>()),
            );
            item.insert("key_count".to_string(), json!(map.len()));
        }
        Value::Array(items) => {
            item.insert("count".to_string(), json!(items.len()));
            item.insert(
                "sample_kinds".to_string(),
                json!(items
                    .iter()
                    .take(MAX_OBSERVED_EVIDENCE_KEYS)
                    .map(json_value_kind)
                    .collect::<Vec<_>>()),
            );
            let sample_keys = items
                .iter()
                .filter_map(Value::as_object)
                .flat_map(|map| map.keys())
                .take(MAX_OBSERVED_EVIDENCE_KEYS)
                .collect::<BTreeSet<_>>();
            if !sample_keys.is_empty() {
                item.insert(
                    "sample_keys".to_string(),
                    json!(sample_keys.into_iter().collect::<Vec<_>>()),
                );
            }
            if !sensitive_field {
                let mut redacted_sample_values = 0_usize;
                let sample_values = items
                    .iter()
                    .take(MAX_OBSERVED_ARRAY_VALUE_SAMPLES)
                    .filter_map(|value| {
                        provider_safe_array_sample_value(field, value, &mut redacted_sample_values)
                    })
                    .collect::<Vec<_>>();
                if !sample_values.is_empty() {
                    item.insert("sample_values".to_string(), json!(sample_values));
                    item.insert(
                        "sample_values_truncated".to_string(),
                        json!(items.len() > MAX_OBSERVED_ARRAY_VALUE_SAMPLES),
                    );
                }
                if redacted_sample_values > 0 {
                    item.insert(
                        "redacted_sample_values".to_string(),
                        json!(redacted_sample_values),
                    );
                }
            }
        }
        Value::Null => {
            item.insert("excerpt".to_string(), json!("null"));
            item.insert("hash".to_string(), json!(stable_trace_hash("null")));
        }
        Value::Bool(value) => {
            let text = value.to_string();
            item.insert("excerpt".to_string(), json!(text));
            item.insert("hash".to_string(), json!(stable_trace_hash(&text)));
        }
        Value::Number(value) => {
            let text = value.to_string();
            item.insert("excerpt".to_string(), json!(text));
            item.insert("hash".to_string(), json!(stable_trace_hash(&text)));
        }
        Value::String(value) => {
            if sensitive_field {
                item.insert("redacted".to_string(), json!(true));
            } else if text_looks_sensitive(value) && !evidence_field_allows_redacted_excerpt(field)
            {
                item.insert("redacted".to_string(), json!(true));
            } else {
                let excerpt = if text_looks_sensitive(value) {
                    redacted_text_excerpt(value)
                } else {
                    evidence_excerpt(value)
                };
                item.insert("excerpt".to_string(), json!(excerpt));
                item.insert("hash".to_string(), json!(stable_trace_hash(value)));
            }
        }
    }
    Value::Object(item)
}

pub(super) fn provider_safe_array_sample_value(
    field: &str,
    value: &Value,
    redacted_count: &mut usize,
) -> Option<Value> {
    match value {
        Value::String(value) => {
            if text_looks_sensitive(value) {
                if evidence_field_allows_redacted_excerpt(field) {
                    Some(json!(redacted_text_excerpt(value)))
                } else {
                    *redacted_count += 1;
                    None
                }
            } else {
                Some(json!(evidence_excerpt(value)))
            }
        }
        Value::Number(_) | Value::Bool(_) | Value::Null => Some(value.clone()),
        Value::Object(map) => {
            let mut sampled = serde_json::Map::new();
            for key in [
                "name",
                "path",
                "resolved_path",
                "kind",
                "local_address",
                "local_endpoint",
                "size_bytes",
                "modified_ts",
                "port",
                "bind_scope",
                "is_wildcard",
                "is_loopback",
                "process_name",
                "pid",
            ] {
                let Some(child) = map.get(key) else {
                    continue;
                };
                if evidence_field_is_sensitive(key) {
                    continue;
                }
                match child {
                    Value::String(text) => {
                        if text_looks_sensitive(text) {
                            *redacted_count += 1;
                        } else {
                            sampled.insert(key.to_string(), json!(evidence_excerpt(text)));
                        }
                    }
                    Value::Number(_) | Value::Bool(_) | Value::Null => {
                        sampled.insert(key.to_string(), child.clone());
                    }
                    _ => {}
                }
            }
            (!sampled.is_empty()).then(|| Value::Object(sampled))
        }
        Value::Array(_) => None,
    }
}

pub(super) fn text_extracted_evidence_item_with_source(
    field: &str,
    source: &str,
    value: &str,
) -> Value {
    let excerpt = redacted_text_excerpt(value);
    json!({
        "field": field,
        "source": source,
        "kind": "text",
        "excerpt": excerpt,
        "hash": stable_trace_hash(value),
    })
}

pub(super) fn text_line_looks_like_path(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty()
        && line.len() <= MAX_OBSERVED_EVIDENCE_EXCERPT_CHARS
        && !line.contains(|ch| matches!(ch, '\n' | '\r' | '\0'))
        && !line.contains("://")
        && !line.ends_with(['.', '。'])
        && (line.starts_with('/')
            || line.starts_with("./")
            || line.starts_with("../")
            || line.contains('/'))
}

pub(super) fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub(super) fn evidence_field_is_sensitive(field: &str) -> bool {
    let normalized = field.to_ascii_lowercase().replace(['-', '.'], "_");
    [
        "secret",
        "token",
        "password",
        "passwd",
        "credential",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
        "cookie",
        "authorization",
        "auth_header",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub(super) fn evidence_field_allows_redacted_excerpt(field: &str) -> bool {
    let leaf = normalized_field_leaf(field);
    matches!(
        leaf,
        "body"
            | "body_preview"
            | "content"
            | "content_excerpt"
            | "description"
            | "excerpt"
            | "snippet"
            | "summary"
            | "text"
            | "title"
            | "titles"
    )
}

pub(super) fn evidence_excerpt(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() <= MAX_OBSERVED_EVIDENCE_EXCERPT_CHARS {
        return collapsed;
    }
    let mut out =
        crate::utf8_safe_prefix(&collapsed, MAX_OBSERVED_EVIDENCE_EXCERPT_CHARS).to_string();
    out.push_str("...(truncated)");
    out
}

pub(super) fn redacted_text_excerpt(text: &str) -> String {
    let redacted = text
        .split_whitespace()
        .map(|token| {
            if text_looks_sensitive(token) {
                "[redacted]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    evidence_excerpt(&redacted)
}

pub(super) fn text_looks_sensitive(text: &str) -> bool {
    if text
        .to_ascii_lowercase()
        .contains(claw_core::secrets::SECRET_TOKEN_REFERENCE_PREFIX)
    {
        return true;
    }
    let trimmed =
        text.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-');
    if looks_like_safe_file_token(trimmed) {
        return false;
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return false;
    }
    if trimmed.len() < 24 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("sk-") || lower.starts_with("sk_") {
        return true;
    }
    let dense_chars = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '+'))
        .count();
    dense_chars * 100 / trimmed.len().max(1) >= 85
}

pub(super) fn looks_like_safe_file_token(text: &str) -> bool {
    let Some((stem, ext)) = text.rsplit_once('.') else {
        return false;
    };
    if stem.is_empty()
        || ext.is_empty()
        || ext.len() > 12
        || !ext.chars().all(|ch| ch.is_ascii_alphanumeric())
    {
        return false;
    }
    let ext = ext.to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "bash"
            | "bmp"
            | "csv"
            | "db"
            | "gif"
            | "gz"
            | "html"
            | "jpeg"
            | "jpg"
            | "json"
            | "lock"
            | "log"
            | "md"
            | "mp3"
            | "pdf"
            | "png"
            | "rs"
            | "sh"
            | "sqlite"
            | "svg"
            | "tar"
            | "toml"
            | "ts"
            | "tsx"
            | "txt"
            | "wav"
            | "webp"
            | "yaml"
            | "yml"
            | "zip"
    )
}

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
    "task_timeout_seconds",
    "running_oldest_age_seconds",
    "clawd_process_count",
    "clawd_health_port_open",
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

pub(super) fn text_observed_evidence_item(output: &str) -> Value {
    let excerpt = redacted_text_excerpt(output);
    json!({
        "field": "text_excerpt",
        "source": "text_output",
        "kind": "text",
        "excerpt": excerpt,
        "hash": stable_trace_hash(output),
    })
}

pub(super) fn collect_text_observed_evidence(
    collector: &mut ObservedEvidenceCollector,
    output: &str,
) {
    collector.push(text_observed_evidence_item(output));
    collect_text_observed_evidence_fields(collector, output);
}

pub(super) fn collect_text_observed_evidence_for_extractor(
    collector: &mut ObservedEvidenceCollector,
    output: &str,
    _extractor: EvidenceExtractorSpec,
) {
    collector.push(text_observed_evidence_item(output));
    collect_text_observed_evidence_fields(collector, output);
}

pub(super) fn collect_text_observed_evidence_fields(
    collector: &mut ObservedEvidenceCollector,
    output: &str,
) {
    if let Some(count) = text_count_evidence(output) {
        collector.push(json_observed_evidence_item(
            "text_output.extractor",
            "count",
            &json!(count),
        ));
    }
    if let Some(path) = text_path_evidence(output) {
        collector.push(text_extracted_evidence_item("path", &path));
    }
    collect_text_machine_key_value_evidence(collector, output);
    collect_status_prefixed_json_body_evidence(collector, output);
    let lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() > 1
        && lines
            .iter()
            .all(|line| text_line_looks_like_list_item(line))
    {
        collector.push(json_observed_evidence_item(
            "text_output.extractor",
            "count",
            &json!(lines.len()),
        ));
        let hidden_count = lines
            .iter()
            .filter(|line| text_line_looks_like_hidden_entry(line))
            .count();
        if hidden_count > 0 {
            collector.push(json_observed_evidence_item(
                "text_output.extractor",
                "hidden_count",
                &json!(hidden_count),
            ));
        }
        for (idx, line) in lines.iter().take(MAX_OBSERVED_EVIDENCE_ITEMS).enumerate() {
            collector.push(text_extracted_evidence_item(
                &format!("results[{idx}]"),
                line,
            ));
        }
    }
}

pub(super) fn collect_status_prefixed_json_body_evidence(
    collector: &mut ObservedEvidenceCollector,
    output: &str,
) {
    let mut non_empty_lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let Some(first_line) = non_empty_lines.next() else {
        return;
    };
    if !first_line.starts_with("status=") {
        return;
    }
    let body = non_empty_lines.collect::<Vec<_>>().join("\n");
    if body.is_empty() {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(&body) else {
        return;
    };
    collect_priority_json_status_scalar_evidence(
        collector,
        "text_output.body_json",
        "body",
        &value,
        0,
    );
    collect_json_observed_evidence(collector, "text_output.body_json", "body", &value, 0);
}

pub(super) fn collect_text_machine_key_value_evidence(
    collector: &mut ObservedEvidenceCollector,
    output: &str,
) {
    let mut seen = BTreeSet::new();
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = normalize_evidence_field(raw_key);
        let value = raw_value.trim();
        let has_following_pair = value
            .split_whitespace()
            .skip(1)
            .any(|token| token.contains('='));
        if !has_following_pair
            && machine_key_value_evidence_key_allowed(&key)
            && !evidence_field_is_sensitive(&key)
            && !value.is_empty()
            && !text_looks_sensitive(value)
            && seen.insert((key.clone(), value.to_string()))
        {
            collector.push(text_extracted_evidence_item(&key, value));
        }
    }
    for token in output.lines().flat_map(str::split_whitespace) {
        let token = token
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
                )
            })
            .trim();
        let Some((raw_key, raw_value)) = token.split_once('=') else {
            continue;
        };
        let key = normalize_evidence_field(raw_key);
        if !machine_key_value_evidence_key_allowed(&key) || evidence_field_is_sensitive(&key) {
            continue;
        }
        let value = raw_value
            .trim()
            .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | ';'));
        if value.is_empty() || text_looks_sensitive(value) {
            continue;
        }
        if seen.insert((key.clone(), value.to_string())) {
            collector.push(text_extracted_evidence_item(&key, value));
        }
    }
}

pub(super) fn machine_key_value_evidence_key_allowed(key: &str) -> bool {
    matches!(
        key,
        "field_value"
            | "value"
            | "status"
            | "state"
            | "version"
            | "schema_version"
            | "package_manager"
            | "manager"
            | "subject"
            | "branch"
            | "commit"
            | "valid"
            | "available"
            | "size_bytes"
            | "bytes"
            | "exit"
            | "exit_code"
            | "error_kind"
            | "datetime"
            | "timezone"
            | "title"
    )
}

pub(super) fn text_extracted_evidence_item(field: &str, value: &str) -> Value {
    text_extracted_evidence_item_with_source(field, "text_output.extractor", value)
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

pub(super) fn text_count_evidence(output: &str) -> Option<i64> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = trimmed.parse::<i64>() {
        return Some(value);
    }
    let normalized = trimmed
        .replace(',', " ")
        .replace(':', " ")
        .replace(';', " ");
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    let mut counts = BTreeSet::new();
    for window in tokens.windows(2) {
        let number = window[0].parse::<i64>().ok();
        let unit = window[1].trim_matches(|ch: char| !ch.is_ascii_alphabetic());
        if let Some(value) = number {
            let unit = unit.to_ascii_lowercase();
            if matches!(
                unit.as_str(),
                "file" | "files" | "item" | "items" | "entry" | "entries" | "row" | "rows"
            ) {
                counts.insert(value);
            }
        }
    }
    (counts.len() == 1).then(|| *counts.iter().next().expect("single count"))
}

pub(super) fn text_path_evidence(output: &str) -> Option<String> {
    let lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() == 1 && text_line_looks_like_standalone_path(lines[0]) {
        return Some(lines[0].to_string());
    }
    if let Some(path) = labeled_text_path_evidence(output) {
        return Some(path);
    }
    let mut paths = BTreeSet::new();
    for token in output.split_whitespace() {
        let candidate = token
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '。' | '，'
                )
            })
            .trim();
        if text_line_looks_like_path(candidate) {
            paths.insert(candidate.to_string());
            continue;
        }
        if let Some((_, rhs)) = candidate.split_once('=') {
            let rhs = rhs.trim();
            if text_line_looks_like_path(rhs) {
                paths.insert(rhs.to_string());
            }
        }
    }
    (paths.len() == 1).then(|| paths.into_iter().next().expect("single path"))
}

pub(super) fn labeled_text_path_evidence(output: &str) -> Option<String> {
    let mut paths = BTreeSet::new();
    for token in output.split_whitespace() {
        let candidate = token
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '。' | '，'
                )
            })
            .trim();
        let Some((key, rhs)) = candidate.split_once('=') else {
            continue;
        };
        let key = normalize_evidence_field(key);
        if !matches!(
            key.as_str(),
            "path"
                | "archive"
                | "archive_path"
                | "output"
                | "output_path"
                | "dest"
                | "dest_path"
                | "destination"
        ) {
            continue;
        }
        let rhs = rhs.trim();
        if text_line_looks_like_path(rhs) {
            paths.insert(rhs.to_string());
        }
    }
    (paths.len() == 1).then(|| paths.into_iter().next().expect("single labeled path"))
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

pub(super) fn text_line_looks_like_standalone_path(line: &str) -> bool {
    text_line_looks_like_path(line) && line.split_whitespace().count() == 1
}

pub(super) fn text_line_looks_like_list_item(line: &str) -> bool {
    let line = line.trim();
    if line == "." {
        return true;
    }
    !line.is_empty()
        && line.len() <= MAX_OBSERVED_EVIDENCE_EXCERPT_CHARS
        && !line.contains(|ch| matches!(ch, '\n' | '\r' | '\0'))
        && !line.contains("://")
        && !line.ends_with(['.', '。', ':', '：'])
        && line.split_whitespace().count() <= 4
}

pub(super) fn text_line_looks_like_hidden_entry(line: &str) -> bool {
    let leaf = line
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | ';'))
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim();
    leaf.starts_with('.') && leaf != "." && leaf != ".."
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
    if known_non_secret_config_risk_label(trimmed) {
        return false;
    }
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

pub(super) fn known_non_secret_config_risk_label(text: &str) -> bool {
    let Some((field, value)) = text.split_once('=') else {
        return false;
    };
    let field = field.trim().to_ascii_lowercase();
    let value = value
        .trim()
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`'));
    if !matches!(
        field.as_str(),
        "tools.allow"
            | "tools.allow_sudo"
            | "tools.allow_path_outside_workspace"
            | "telegram.sendfile.full_access"
            | "server.listen"
            | "worker.task_timeout_seconds"
    ) {
        return false;
    }
    if value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("false")
        || value.parse::<i64>().is_ok()
        || value.parse::<f64>().is_ok()
    {
        return true;
    }
    field == "tools.allow" && value == "[\"*\"]"
        || field == "server.listen" && (value == "0.0.0.0" || value.starts_with("0.0.0.0:"))
}

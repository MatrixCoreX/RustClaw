use super::*;

pub(super) fn source_journal(result: &Value) -> Option<&Value> {
    [
        "/task_journal",
        "/final_result_json/task_journal",
        "/result/task_journal",
    ]
    .iter()
    .find_map(|path| {
        result.pointer(path).filter(|journal| {
            journal
                .pointer("/trace/capability_results")
                .is_some_and(Value::is_array)
                || journal.get("canonical_capabilities").is_some()
        })
    })
}

#[cfg(test)]
pub(super) fn select_context(
    source: &TaskDeliveryRecord,
    request: &RecallRequest,
    binding: &str,
    now: i64,
) -> Result<Value, &'static str> {
    select_context_with_capabilities(source, request, binding, now, None)
}

pub(super) fn select_context_with_capabilities(
    source: &TaskDeliveryRecord,
    request: &RecallRequest,
    binding: &str,
    now: i64,
    canonical: Option<&Vec<Value>>,
) -> Result<Value, &'static str> {
    if request.field_paths.is_empty() || request.field_paths.len() > 8 {
        return Err("context_fields_invalid");
    }
    let page_size = request.page_size.unwrap_or(64);
    let has_index = request
        .field_paths
        .iter()
        .any(|path| path == "/capabilities");
    if !(1..=64).contains(&page_size)
        || (!has_index && (request.cursor.is_some() || request.page_size.is_some()))
    {
        return Err("context_page_invalid");
    }
    let payload: Value =
        serde_json::from_str(&source.task.payload_json).map_err(|_| "context_payload_invalid")?;
    let result = source
        .result_json
        .as_ref()
        .ok_or("context_result_unavailable")?;
    let snapshot = source_digest(&json!({"source_task_id": source.task.task_id,
        "status": source.status, "payload": payload, "result": result}))?;
    if request
        .source_digest
        .as_ref()
        .is_some_and(|expected| expected != &snapshot)
    {
        return Err("context_snapshot_changed");
    }
    let selection = source_digest(&json!(request.field_paths))?;
    let offset = request
        .cursor
        .as_deref()
        .map(|token| cursor::offset(token, binding, &snapshot, &selection, page_size, now))
        .transpose()?
        .unwrap_or(0);
    let trace = source_journal(result).and_then(|journal| journal.get("trace"));
    let complete = canonical.is_some()
        || trace.is_some_and(crate::task_journal::retained_capability_results_complete);
    let capabilities = canonical.or_else(|| {
        trace
            .and_then(|t| t.get("capability_results"))
            .and_then(Value::as_array)
    });
    let mut fields = serde_json::Map::new();
    let mut pagination = Value::Null;
    for path in &request.field_paths {
        if !path.starts_with('/') || path.len() > 256 {
            return Err("context_fields_invalid");
        }
        if path
            .split('/')
            .any(|part| sensitive_key(&part.replace("~1", "/").replace("~0", "~")))
        {
            return Err("context_field_forbidden");
        }
        if path == "/capabilities" || path.starts_with("/capabilities/") {
            if capabilities.is_none() {
                return Err("context_source_unavailable");
            }
            if !complete {
                return Err("context_source_compacted");
            }
        }
        if path == "/capabilities" {
            let items = capabilities.expect("checked source");
            if offset > items.len() {
                return Err("context_cursor_invalid");
            }
            let end = offset.saturating_add(page_size).min(items.len());
            let index = items[offset..end]
                .iter()
                .enumerate()
                .map(|(position, item)| {
                    let index = offset + position;
                    redact(
                        &json!({"index": index, "capability": item.get("capability"),
                    "status": item.get("status"), "args_ref": format!("/capabilities/{index}/args"),
                    "data_ref": format!("/capabilities/{index}/data")}),
                        0,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let has_more = end < items.len();
            let next = if has_more {
                Some(cursor::issue(
                    binding, &snapshot, &selection, end, page_size, now,
                )?)
            } else {
                None
            };
            pagination = json!({"offset": offset, "returned": index.len(), "total": items.len(),
                "has_more": has_more, "next_cursor": next, "page_size": page_size,
                "index_complete": true});
            fields.insert(path.clone(), json!(index));
        } else {
            let value = historical_field(&payload, capabilities, path)?;
            check_json_size(value)?;
            fields.insert(path.clone(), redact(value, 0)?);
        }
    }
    let mut value = json!({"schema_version": 1, "action": "recall_context", "status": "ok",
        "message_key": "task_control.recall_context.ok", "source_task_id": source.task.task_id,
        "field_value": {"source_task_id": source.task.task_id, "source_status": source.status},
        "source_status": source.status, "source_digest": snapshot,
        "capability_evidence_source": if request.field_paths.iter().any(|path|
            path == "/capabilities" || path.starts_with("/capabilities/")) {
            if canonical.is_some() { "canonical_archive" } else { "retained_trace" }
        } else { "not_selected" },
        "instruction_authority": "none", "current_execution_evidence": false,
        "scope": "prior_task_context", "fields": fields});
    if has_index {
        value["pagination"] = pagination;
    }
    check_json_size(&value)?;
    Ok(value)
}

fn historical_field<'a>(
    payload: &'a Value,
    capabilities: Option<&'a Vec<Value>>,
    path: &str,
) -> Result<&'a Value, &'static str> {
    if path == "/request" {
        return payload
            .get("text")
            .or_else(|| payload.get("user_message"))
            .ok_or("context_field_unavailable");
    }
    let mut parts = path
        .strip_prefix("/capabilities/")
        .ok_or("context_field_unavailable")?
        .splitn(3, '/');
    let index = parts
        .next()
        .and_then(|index| index.parse::<usize>().ok())
        .ok_or("context_field_unavailable")?;
    let capability = capabilities
        .and_then(|items| items.get(index))
        .ok_or("context_field_unavailable")?;
    let field = match parts.next().ok_or("context_field_unavailable")? {
        "args" => {
            if capability
                .pointer("/provenance/continuation_args_projection/truncated")
                .and_then(Value::as_bool)
                == Some(true)
            {
                return Err("context_field_compacted");
            }
            capability.pointer("/provenance/continuation_args")
        }
        name @ ("data" | "capability" | "status") => capability.get(name),
        _ => None,
    }
    .ok_or("context_field_unavailable")?;
    match parts.next() {
        Some(suffix) => field
            .pointer(&format!("/{suffix}"))
            .ok_or("context_field_unavailable"),
        None => Ok(field),
    }
}

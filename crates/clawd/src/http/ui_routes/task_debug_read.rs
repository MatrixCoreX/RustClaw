async fn task_debug_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<String>,
    Query(query): Query<TeachingTraceQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    // Ownership checks, log scans, catalog parsing and redaction are all
    // synchronous. Keep their existing order on a request-owned worker.
    match tokio::task::spawn_blocking(move || {
        read_task_debug_detail(&state, &headers, &task_id, query)
    })
    .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                panicked = error.is_panic(),
                cancelled = error.is_cancelled(),
                "teaching_trace_read_join_failed"
            );
            teaching_trace_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "teaching_trace_read_failed",
            )
        }
    }
}

fn read_task_debug_detail(
    state: &AppState,
    headers: &HeaderMap,
    task_id: &str,
    query: TeachingTraceQuery,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(state, headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let normalized_task_id = task_id.trim();
    if normalized_task_id.is_empty() {
        return teaching_trace_error(StatusCode::BAD_REQUEST, "task_id_required");
    }
    if query.teaching != Some(true) {
        return teaching_trace_error(StatusCode::BAD_REQUEST, "teaching_trace_opt_in_required");
    }
    let Some((task_user_key, _channel)) =
        (match task_access_meta_for_debug(state, normalized_task_id) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(
                    "teaching trace task owner lookup failed error={}",
                    crate::truncate_for_log(&err.to_string())
                );
                return teaching_trace_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "teaching_trace_owner_lookup_failed",
                );
            }
        })
    else {
        return teaching_trace_error(StatusCode::NOT_FOUND, "teaching_trace_task_not_found");
    };
    let access_scope = match teaching_trace_access_scope(&identity, task_user_key.as_deref(), true)
    {
        Ok(scope) => scope,
        Err(error_code) => {
            return teaching_trace_error(StatusCode::FORBIDDEN, error_code);
        }
    };
    let entries_page = match read_task_debug_entries_page(
        state,
        normalized_task_id,
        query.limit.unwrap_or(20),
        query.offset.unwrap_or(0),
    ) {
        Ok(page) => page,
        Err(err) => {
            tracing::warn!(
                "teaching trace model log read failed error={}",
                crate::truncate_for_log(&err.to_string())
            );
            return teaching_trace_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "teaching_trace_model_log_read_failed",
            );
        }
    };
    let mut entries = entries_page.entries;
    entries.sort_by(|a, b| {
        (a.ts.unwrap_or(0), a.call_id.as_deref().unwrap_or_default())
            .cmp(&(b.ts.unwrap_or(0), b.call_id.as_deref().unwrap_or_default()))
    });
    let mut redacted_fields = redact_task_debug_entries(&mut entries);
    let calls = numbered_task_debug_calls_from(&entries, entries_page.offset);
    let flow_summary = build_task_debug_flow_summary(&calls);
    let task_result_json = match read_task_result_json_for_debug(state, normalized_task_id) {
        Ok(value) => value,
        Err(err) => {
            tracing::warn!(
                "teaching trace task result read failed error={}",
                crate::truncate_for_log(&err.to_string())
            );
            return teaching_trace_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "teaching_trace_task_result_read_failed",
            );
        }
    };
    let memory_trace = task_result_json
        .as_ref()
        .and_then(|(_, result_json)| extract_memory_trace_for_debug(result_json));
    let resume_trace = task_result_json
        .as_ref()
        .and_then(|(db_status, result_json)| {
            extract_resume_trace_for_debug(db_status, result_json)
        });
    let model_catalog_trace = build_model_catalog_trace_for_debug(state, &entries);
    let trace_availability = teaching_trace_availability(
        &entries,
        task_result_json
            .as_ref()
            .map(|(db_status, _)| db_status.as_str()),
    );
    let mut runtime_decisions = json!({
        "memory_trace": memory_trace,
        "model_catalog_trace": model_catalog_trace,
        "resume_trace": resume_trace,
    });
    redacted_fields += redact_teaching_value(&mut runtime_decisions, None, 0);
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "task_id": normalized_task_id,
                "trace_schema_version": 2,
                "access": {
                    "opt_in": true,
                    "scope": access_scope,
                },
                "redaction": {
                    "applied": redacted_fields > 0,
                    "field_count": redacted_fields,
                    "policy": "teaching_trace_v1",
                },
                "trace_layers": teaching_trace_layers(),
                "call_count": entries_page.total,
                "pagination": {
                    "limit": entries_page.limit,
                    "offset": entries_page.offset,
                    "total": entries_page.total,
                    "has_more": entries_page.has_more,
                },
                "trace_availability": trace_availability,
                "flow_summary": flow_summary,
                "calls": calls,
                "entries": entries,
                "memory_trace": runtime_decisions.get("memory_trace").cloned(),
                "model_catalog_trace": runtime_decisions.get("model_catalog_trace").cloned(),
                "resume_trace": runtime_decisions.get("resume_trace").cloned(),
            })),
            error: None,
        }),
    )
}

fn normalize_log_file_name(raw: Option<&str>) -> String {
    let fallback = "agent_trace.log".to_string();
    let candidate = raw.unwrap_or("").trim();
    if candidate.is_empty() {
        return fallback;
    }
    let allowed = [
        "agent_trace.log",
        "model_io.log",
        "routing.log",
        "act_plan.log",
        "clawd.log",
        "nni.log",
        "nni-server.log",
        "channel-gateway.log",
        "telegramd.log",
        "whatsappd.log",
        "whatsapp_webd.log",
    ];
    if allowed.iter().any(|v| v.eq_ignore_ascii_case(candidate)) {
        return candidate.to_string();
    }
    fallback
}

fn read_last_lines(path: &std::path::Path, limit_lines: usize) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let total_size = file.metadata()?.len();
    let max_tail_bytes: u64 = 512 * 1024;
    let read_from = total_size.saturating_sub(max_tail_bytes);
    if read_from > 0 {
        file.seek(SeekFrom::Start(read_from))?;
    }
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let content = String::from_utf8_lossy(&buf);
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Ok(String::new());
    }
    let start = lines.len().saturating_sub(limit_lines);
    Ok(lines[start..].join("\n"))
}

fn canonical_bound_channel_name(raw: &str) -> String {
    let channel = raw.trim().to_ascii_lowercase();
    match channel.as_str() {
        "" => String::new(),
        "telegram_bot" => "telegram".to_string(),
        "whatsapp_cloud" | "whatsapp-cloud" | "whatsapp_web" | "whatsapp-web" | "wa_cloud"
        | "wa-cloud" | "wa_web" | "wa-web" => "whatsapp".to_string(),
        "wechat_bot" | "openclaw-weixin" | "weixin" => "wechat".to_string(),
        other => other.to_string(),
    }
}

fn auth_user_summary_counts(state: &AppState) -> anyhow::Result<(usize, usize, Vec<String>)> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let user_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM auth_keys WHERE enabled = 1",
        [],
        |row| row.get(0),
    )?;
    let mut stmt = db.prepare(
        "SELECT DISTINCT channel FROM channel_bindings WHERE TRIM(COALESCE(channel, '')) != ''",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut bound_channels = Vec::new();
    for row in rows {
        let channel = canonical_bound_channel_name(&row?);
        if !channel.is_empty() && !bound_channels.iter().any(|existing| existing == &channel) {
            bound_channels.push(channel);
        }
    }
    let channel_order = |channel: &str| match channel {
        "telegram" => 0,
        "whatsapp" => 1,
        "wechat" => 2,
        "feishu" => 3,
        "lark" => 4,
        "ui" => 5,
        _ => 99,
    };
    bound_channels.sort_by(|a, b| {
        channel_order(a)
            .cmp(&channel_order(b))
            .then_with(|| a.cmp(b))
    });
    let bound_channel_count = bound_channels.len();
    Ok((
        user_count.max(0) as usize,
        bound_channel_count,
        bound_channels,
    ))
}

async fn logs_latest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LogsLatestQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let file_name = normalize_log_file_name(query.file.as_deref());
    let lines = query.lines.unwrap_or(200).clamp(20, 2000);
    let path = state.skill_rt.workspace_root.join("logs").join(&file_name);
    let raw = match read_last_lines(&path, lines) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read log failed: {err}")),
                }),
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "file": file_name,
                "lines": lines,
                "text": raw,
            })),
            error: None,
        }),
    )
}

fn channel_allows_shared_ui_task_access(channel: &str) -> bool {
    matches!(
        channel,
        "telegram" | "whatsapp" | "wechat" | "feishu" | "lark"
    )
}

fn task_access_meta_for_debug(
    state: &AppState,
    task_id: &str,
) -> anyhow::Result<Option<(Option<String>, String)>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    db.query_row(
        "SELECT user_key, channel FROM tasks WHERE task_id = ?1 LIMIT 1",
        [task_id],
        |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
    )
    .optional()
    .map_err(Into::into)
}

fn preview_text(raw: &str, limit: usize) -> Option<String> {
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut preview = String::new();
    let mut count = 0usize;
    for ch in trimmed.chars() {
        if count >= limit {
            break;
        }
        preview.push(ch);
        count += 1;
    }
    if trimmed.chars().count() > limit {
        preview.push_str("...");
    }
    Some(preview)
}

fn preview_text_from_json(raw: Option<&str>, preferred_keys: &[&str]) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let value = serde_json::from_str::<Value>(raw).ok()?;
    for key in preferred_keys {
        if let Some(text) = value.get(*key).and_then(|v| v.as_str()) {
            if let Some(preview) = preview_text(text, 180) {
                return Some(preview);
            }
        }
    }
    if let Some(text) = value.as_str() {
        return preview_text(text, 180);
    }
    None
}

fn payload_telegram_bot_name(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let value = serde_json::from_str::<Value>(raw).ok()?;
    value
        .get("telegram_bot_name")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn payload_request_text(raw: Option<&str>) -> Option<String> {
    preview_text_from_json(raw, &["text"])
}

fn usage_record_visible_to_identity(identity: &AuthIdentity, meta: &UsageTaskMeta) -> bool {
    if meta.channel == "ui" {
        let expected_key = meta
            .user_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        return expected_key == Some(identity.user_key.trim());
    }
    channel_allows_shared_ui_task_access(&meta.channel)
}

fn usage_chain_entry_from_entry(entry: &TaskDebugEntry) -> UsageHistoryChainEntry {
    let prompt_tokens = entry
        .usage
        .as_ref()
        .and_then(|usage| usage.prompt_tokens.or(usage.input_tokens));
    let completion_tokens = entry
        .usage
        .as_ref()
        .and_then(|usage| usage.completion_tokens.or(usage.output_tokens));
    let total_tokens = entry.usage.as_ref().and_then(|usage| usage.total_tokens);
    UsageHistoryChainEntry {
        ts: entry.ts,
        vendor: entry.vendor.clone(),
        provider: entry.provider.clone(),
        provider_type: entry.provider_type.clone(),
        model: entry.model.clone(),
        model_kind: entry.model_kind.clone(),
        prompt_file: entry.prompt_file.clone(),
        prompt_tokens,
        completion_tokens,
        total_tokens,
        status: entry.status.clone(),
        error: entry.error.clone(),
        prompt: entry.prompt.clone(),
        request_payload: entry.request_payload.clone(),
        raw_response: entry.raw_response.clone(),
        clean_response: entry.clean_response.clone().or(entry.response.clone()),
    }
}

fn summarize_usage_task(
    task_id: String,
    meta: UsageTaskMeta,
    entries: &[TaskDebugEntry],
) -> UsageHistoryRecordSummary {
    let mut prompt_tokens = 0u64;
    let mut completion_tokens = 0u64;
    let mut total_tokens = 0u64;
    let mut latest_entry: Option<&TaskDebugEntry> = None;
    for entry in entries {
        let chain_entry = usage_chain_entry_from_entry(entry);
        prompt_tokens += chain_entry.prompt_tokens.unwrap_or(0);
        completion_tokens += chain_entry.completion_tokens.unwrap_or(0);
        total_tokens += chain_entry.total_tokens.unwrap_or_else(|| {
            chain_entry.prompt_tokens.unwrap_or(0) + chain_entry.completion_tokens.unwrap_or(0)
        });
        let replace = latest_entry
            .map(|current| entry.ts.unwrap_or(0) >= current.ts.unwrap_or(0))
            .unwrap_or(true);
        if replace {
            latest_entry = Some(entry);
        }
    }
    let latest = latest_entry.cloned().unwrap_or(TaskDebugEntry {
        ts: None,
        task_id: Some(task_id.clone()),
        vendor: None,
        provider: None,
        provider_type: None,
        model: None,
        model_kind: None,
        status: None,
        prompt_file: None,
        prompt: None,
        request_payload: None,
        response: None,
        raw_response: None,
        clean_response: None,
        sanitized: None,
        error: None,
        usage: None,
    });
    UsageHistoryRecordSummary {
        record_id: task_id.clone(),
        task_id,
        ts: latest.ts,
        channel: Some(meta.channel),
        kind: Some(meta.kind),
        task_status: Some(meta.task_status),
        telegram_bot_name: meta.telegram_bot_name,
        external_user_id: meta.external_user_id,
        external_chat_id: meta.external_chat_id,
        request_text: meta.request_text,
        vendor: latest.vendor,
        provider: latest.provider,
        provider_type: latest.provider_type,
        model: latest.model,
        model_kind: latest.model_kind,
        prompt_file: latest.prompt_file,
        prompt_tokens: Some(prompt_tokens),
        completion_tokens: Some(completion_tokens),
        total_tokens: Some(total_tokens),
        llm_call_count: entries.len(),
        status: latest.status,
        error: latest.error,
    }
}

fn usage_stats_add(stats: &mut UsageHistoryStats, record: &UsageHistoryRecordSummary) {
    stats.total_requests += 1;
    if record.status.as_deref() == Some("ok") {
        stats.success_requests += 1;
    } else {
        stats.failed_requests += 1;
    }
    stats.prompt_tokens += record.prompt_tokens.unwrap_or(0);
    stats.completion_tokens += record.completion_tokens.unwrap_or(0);
    stats.total_tokens += record.total_tokens.unwrap_or_else(|| {
        record.prompt_tokens.unwrap_or(0) + record.completion_tokens.unwrap_or(0)
    });
}

fn usage_channel_matches(query_channel: Option<&str>, record: &UsageHistoryRecordSummary) -> bool {
    let Some(query_channel) = query_channel
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    record.channel.as_deref().unwrap_or_default() == query_channel
}

fn usage_status_matches(query_status: Option<&str>, record: &UsageHistoryRecordSummary) -> bool {
    let Some(query_status) = query_status
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    match query_status {
        "success" => record.status.as_deref() == Some("ok"),
        "failed" => record.status.as_deref() != Some("ok"),
        _ => true,
    }
}

fn usage_search_matches(query: Option<&str>, record: &UsageHistoryRecordSummary) -> bool {
    let Some(query) = query.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    let query = query.to_lowercase();
    let haystack = [
        Some(record.task_id.as_str()),
        record.request_text.as_deref(),
        record.model.as_deref(),
        record.vendor.as_deref(),
        record.provider.as_deref(),
        record.telegram_bot_name.as_deref(),
        record.external_user_id.as_deref(),
        record.external_chat_id.as_deref(),
        record.prompt_file.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase();
    haystack.contains(&query)
}

fn task_usage_meta(state: &AppState, task_id: &str) -> anyhow::Result<Option<UsageTaskMeta>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    db.query_row(
        "SELECT channel, kind, status, user_key, external_user_id, external_chat_id, payload_json
         FROM tasks
         WHERE task_id = ?1
         LIMIT 1",
        [task_id],
        |row| {
            let payload_json: Option<String> = row.get(6)?;
            Ok(UsageTaskMeta {
                channel: row.get(0)?,
                kind: row.get(1)?,
                task_status: row.get(2)?,
                user_key: row.get(3)?,
                external_user_id: row.get(4)?,
                external_chat_id: row.get(5)?,
                telegram_bot_name: payload_telegram_bot_name(payload_json.as_deref()),
                request_text: payload_request_text(payload_json.as_deref()),
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

async fn recent_robot_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RecentRobotTasksQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let limit = query.limit.unwrap_or(12).clamp(1, 50);

    let read_result = (|| -> anyhow::Result<Vec<RecentRobotTaskSummary>> {
        let db = state
            .core
            .db
            .get()
            .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
        let mut stmt = db.prepare(
            "SELECT task_id, status, kind, channel, external_user_id, external_chat_id, payload_json, result_json, error_text,
                    CAST(NULLIF(created_at, '') AS INTEGER) AS created_ts,
                    CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) AS updated_ts
             FROM tasks
             WHERE channel IN ('telegram', 'whatsapp', 'wechat', 'feishu', 'lark')
             ORDER BY updated_ts DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(rusqlite::params![limit as i64], |row| {
            let payload_json: Option<String> = row.get(6)?;
            let result_json: Option<String> = row.get(7)?;
            Ok(RecentRobotTaskSummary {
                task_id: row.get(0)?,
                status: row.get(1)?,
                kind: row.get(2)?,
                channel: row.get(3)?,
                external_user_id: row.get(4)?,
                external_chat_id: row.get(5)?,
                telegram_bot_name: payload_telegram_bot_name(payload_json.as_deref()),
                request_text: preview_text_from_json(payload_json.as_deref(), &["text"]),
                result_text: preview_text_from_json(result_json.as_deref(), &["text"]),
                error_text: row.get(8)?,
                created_at: row.get::<_, Option<i64>>(9)?.map(|v| v.max(0) as u64),
                updated_at: row.get::<_, Option<i64>>(10)?.map(|v| v.max(0) as u64),
            })
        })?;

        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    })();

    match read_result {
        Ok(tasks) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "tasks": tasks })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read recent robot tasks failed: {err}")),
            }),
        ),
    }
}

async fn usage_records(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<UsageRecordsQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let page_size = query.page_size.unwrap_or(20).clamp(10, 100);
    let page = query.page.unwrap_or(1).max(1);
    let search = query.search.as_deref();
    let channel = query.channel.as_deref().filter(|value| *value != "all");
    let status = query.status.as_deref().filter(|value| *value != "all");
    let log_path = state
        .skill_rt
        .workspace_root
        .join("logs")
        .join("model_io.log");
    if !log_path.exists() {
        return (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "stats": UsageHistoryStats {
                        total_requests: 0,
                        success_requests: 0,
                        failed_requests: 0,
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: 0,
                    },
                    "records": Vec::<UsageHistoryRecordSummary>::new(),
                    "pagination": UsageHistoryPage {
                        page,
                        page_size,
                        total_records: 0,
                        total_pages: 0,
                    },
                })),
                error: None,
            }),
        );
    }

    let file = match std::fs::File::open(&log_path) {
        Ok(file) => file,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("open usage log failed: {err}")),
                }),
            );
        }
    };
    let reader = std::io::BufReader::new(file);
    let mut meta_cache: HashMap<String, Option<UsageTaskMeta>> = HashMap::new();
    let mut tasks_by_id: HashMap<String, (UsageTaskMeta, Vec<TaskDebugEntry>)> = HashMap::new();
    let mut stats = UsageHistoryStats {
        total_requests: 0,
        success_requests: 0,
        failed_requests: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
    };
    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<TaskDebugEntry>(trimmed) else {
            continue;
        };
        let Some(task_id) = entry
            .task_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
        else {
            continue;
        };
        let meta = if let Some(existing) = meta_cache.get(&task_id) {
            existing.clone()
        } else {
            let loaded = match task_usage_meta(&state, &task_id) {
                Ok(value) => value,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some(format!("load usage task meta failed: {err}")),
                        }),
                    );
                }
            };
            meta_cache.insert(task_id.clone(), loaded.clone());
            loaded
        };
        let Some(meta) = meta else {
            continue;
        };
        if !usage_record_visible_to_identity(&identity, &meta) {
            continue;
        }
        tasks_by_id
            .entry(task_id)
            .and_modify(|(_, entries)| entries.push(entry.clone()))
            .or_insert_with(|| (meta, vec![entry]));
    }
    let mut matched_records = Vec::new();
    for (task_id, (meta, mut entries)) in tasks_by_id {
        entries.sort_by(|a, b| (a.ts.unwrap_or(0)).cmp(&b.ts.unwrap_or(0)));
        let summary = summarize_usage_task(task_id, meta, &entries);
        if !usage_channel_matches(channel, &summary) {
            continue;
        }
        if !usage_status_matches(status, &summary) {
            continue;
        }
        if !usage_search_matches(search, &summary) {
            continue;
        }
        usage_stats_add(&mut stats, &summary);
        matched_records.push(summary);
    }
    matched_records.sort_by(|a, b| (b.ts.unwrap_or(0)).cmp(&a.ts.unwrap_or(0)));
    let total_records = matched_records.len();
    let total_pages = if total_records == 0 {
        0
    } else {
        total_records.div_ceil(page_size)
    };
    let safe_page = if total_pages == 0 {
        1
    } else {
        page.min(total_pages)
    };
    let start = (safe_page.saturating_sub(1)) * page_size;
    let records = matched_records
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "stats": stats,
                "records": records,
                "pagination": UsageHistoryPage {
                    page: safe_page,
                    page_size,
                    total_records,
                    total_pages,
                },
            })),
            error: None,
        }),
    )
}

async fn usage_record_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let task_id = task_id.trim();
    if task_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("invalid task id".to_string()),
            }),
        );
    }
    let meta = match task_usage_meta(&state, task_id) {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("usage record not found".to_string()),
                }),
            );
        }
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("load usage task meta failed: {err}")),
                }),
            );
        }
    };
    if !usage_record_visible_to_identity(&identity, &meta) {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("usage record access denied".to_string()),
            }),
        );
    }

    let mut entries = match read_task_debug_entries(&state, task_id) {
        Ok(entries) => entries,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read usage chain failed: {err}")),
                }),
            );
        }
    };
    if entries.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("usage record detail not found".to_string()),
            }),
        );
    }
    entries.sort_by(|a, b| (a.ts.unwrap_or(0)).cmp(&b.ts.unwrap_or(0)));
    let summary = summarize_usage_task(task_id.to_string(), meta, &entries);
    let record = UsageHistoryRecordDetail {
        summary,
        entries: entries.iter().map(usage_chain_entry_from_entry).collect(),
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!(record)),
            error: None,
        }),
    )
}

fn read_task_debug_entries(state: &AppState, task_id: &str) -> anyhow::Result<Vec<TaskDebugEntry>> {
    let path = state
        .skill_rt
        .workspace_root
        .join("logs")
        .join("model_io.log");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut entries = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<TaskDebugEntry>(trimmed) else {
            continue;
        };
        if entry.task_id.as_deref() == Some(task_id) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

async fn task_debug_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let normalized_task_id = task_id.trim();
    if normalized_task_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("task_id is required".to_string()),
            }),
        );
    }
    let Some((task_user_key, channel)) =
        (match task_access_meta_for_debug(&state, normalized_task_id) {
            Ok(value) => value,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("read task owner failed: {err}")),
                    }),
                );
            }
        })
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Task not found".to_string()),
            }),
        );
    };
    let expected_key = task_user_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    if !channel_allows_shared_ui_task_access(&channel)
        && expected_key.is_some()
        && identity.user_key.trim() != expected_key.unwrap_or_default()
    {
        return ui_auth_error("Task owner mismatch");
    }
    let entries = match read_task_debug_entries(&state, normalized_task_id) {
        Ok(entries) => entries,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read task debug failed: {err}")),
                }),
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "task_id": normalized_task_id,
                "entries": entries,
            })),
            error: None,
        }),
    )
}

#[cfg(test)]
mod logs_usage_debug_tests {
    use super::normalize_log_file_name;

    #[test]
    fn logs_latest_allows_device_side_and_server_nni_logs() {
        assert_eq!(normalize_log_file_name(Some("nni.log")), "nni.log");
        assert_eq!(
            normalize_log_file_name(Some("nni-server.log")),
            "nni-server.log"
        );
    }
}

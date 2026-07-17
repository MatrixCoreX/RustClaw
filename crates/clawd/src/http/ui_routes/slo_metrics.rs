#[derive(Debug, Deserialize)]
struct SloMetricsQuery {
    window_hours: Option<u64>,
    limit: Option<usize>,
}

#[derive(Debug)]
struct SloTaskRow {
    status: String,
    result: Option<Value>,
    result_parse_error: bool,
    claim_attempt: u64,
}

#[derive(Debug, Default)]
struct SloAggregate {
    terminal_tasks: u64,
    succeeded_tasks: u64,
    resumed_terminal_tasks: u64,
    resumed_succeeded_tasks: u64,
    journal_tasks: u64,
    result_parse_errors: u64,
    journal_unavailable_tasks: u64,
    planner_latency_ms: Vec<u64>,
    tool_latency_ms: Vec<u64>,
    logical_llm_calls: u64,
    provider_selections: u64,
    provider_retries: u64,
    prompt_truncations: u64,
    prompt_bytes_before: u64,
    prompt_bytes_after: u64,
    duplicate_mutations_suppressed: u64,
    stale_running_recoveries: u64,
    known_cost_usd_nanos: u64,
    unknown_cost_tasks: u64,
}

#[derive(Debug, Default)]
struct LocalAsyncJobHealth {
    scanned_jobs: u64,
    running_jobs: u64,
    terminal_jobs: u64,
    orphaned_jobs: u64,
    indeterminate_jobs: u64,
}

async fn observability_slo_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SloMetricsQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return slo_metrics_error(StatusCode::FORBIDDEN, "admin_required");
    }
    let window_hours = query.window_hours.unwrap_or(24).clamp(1, 24 * 30);
    let limit = query.limit.unwrap_or(5_000).clamp(1, 10_000);
    let cutoff_ts = crate::now_ts_u64().saturating_sub(window_hours.saturating_mul(3_600));
    let rows = match read_slo_task_rows(&state, cutoff_ts, limit.saturating_add(1)) {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                "observability slo query failed error={}",
                crate::truncate_for_log(&error.to_string())
            );
            return slo_metrics_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "slo_metrics_unavailable",
            );
        }
    };
    let truncated = rows.len() > limit;
    let sampled_rows = rows.into_iter().take(limit).collect::<Vec<_>>();
    let aggregate = aggregate_slo_rows(&sampled_rows);
    let async_jobs = inspect_local_async_jobs(&state.skill_rt.workspace_root, crate::now_ts_u64());
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(slo_metrics_json(
                &aggregate,
                window_hours,
                cutoff_ts,
                sampled_rows.len(),
                limit,
                truncated,
                &async_jobs,
            )),
            error: None,
        }),
    )
}

fn slo_metrics_error(
    status: StatusCode,
    error_code: &'static str,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: Some(json!({
                "owner_layer": "ui_observability",
                "error_code": error_code,
                "message_key": format!("clawd.ui.observability.{error_code}"),
            })),
            error: Some(error_code.to_string()),
        }),
    )
}

fn read_slo_task_rows(
    state: &AppState,
    cutoff_ts: u64,
    limit: usize,
) -> anyhow::Result<Vec<SloTaskRow>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
    let mut stmt = db.prepare(
        "SELECT status, result_json, COALESCE(claim_attempt, 0)
         FROM tasks
         WHERE CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) >= ?1
         ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![cutoff_ts as i64, limit as i64],
        |row| {
            let raw_result: Option<String> = row.get(1)?;
            let parsed_result = raw_result
                .as_deref()
                .filter(|raw| !raw.trim().is_empty())
                .map(serde_json::from_str::<Value>);
            let result_parse_error = parsed_result.as_ref().is_some_and(Result::is_err);
            let result = parsed_result.and_then(Result::ok);
            Ok(SloTaskRow {
                status: row.get(0)?,
                result,
                result_parse_error,
                claim_attempt: row.get::<_, i64>(2)?.max(0) as u64,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn aggregate_slo_rows(rows: &[SloTaskRow]) -> SloAggregate {
    let mut aggregate = SloAggregate::default();
    for row in rows {
        let terminal = matches!(
            row.status.as_str(),
            "succeeded" | "failed" | "canceled" | "timeout"
        );
        aggregate.terminal_tasks += u64::from(terminal);
        aggregate.succeeded_tasks += u64::from(row.status == "succeeded");
        if terminal && row.claim_attempt > 1 {
            aggregate.resumed_terminal_tasks += 1;
            aggregate.resumed_succeeded_tasks += u64::from(row.status == "succeeded");
        }
        if row.result_parse_error {
            aggregate.result_parse_errors += 1;
            continue;
        }
        let Some(result) = row.result.as_ref() else {
            continue;
        };
        let Some(journal) = find_task_journal(result, 0) else {
            aggregate.journal_unavailable_tasks += 1;
            aggregate.stale_running_recoveries +=
                count_machine_record(result, "source", "worker_stale_recovery");
            continue;
        };
        aggregate.journal_tasks += 1;
        let summary = journal.get("summary").unwrap_or(journal);
        let trace = journal.get("trace").unwrap_or(journal);
        collect_prompt_metrics(summary, &mut aggregate);
        collect_tool_latencies(trace, &mut aggregate.tool_latency_ms);
        aggregate.duplicate_mutations_suppressed +=
            count_machine_record(trace, "reason_code", "mutation_already_completed");
        aggregate.stale_running_recoveries +=
            count_machine_record(result, "source", "worker_stale_recovery");
        collect_cost_metrics(summary, &mut aggregate);
    }
    aggregate
}

fn find_task_journal(value: &Value, depth: usize) -> Option<&Value> {
    if depth > 6 {
        return None;
    }
    if value.get("summary").is_some() && value.get("trace").is_some() {
        return Some(value);
    }
    if let Some(journal) = value.get("task_journal").filter(|item| item.is_object()) {
        return Some(journal);
    }
    ["result", "result_json", "final_result_json", "data"]
        .iter()
        .find_map(|key| value.get(*key).and_then(|child| find_task_journal(child, depth + 1)))
}

fn collect_prompt_metrics(summary: &Value, aggregate: &mut SloAggregate) {
    let Some(metrics) = summary.get("task_metrics") else {
        return;
    };
    aggregate.prompt_truncations = aggregate.prompt_truncations.saturating_add(
        metrics
            .get("prompt_truncation_count")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    );
    if let Some(routing) = metrics.get("provider_routing") {
        aggregate.logical_llm_calls = aggregate.logical_llm_calls.saturating_add(
            routing
                .get("logical_calls")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        );
        aggregate.provider_selections = aggregate.provider_selections.saturating_add(
            routing
                .get("provider_selections")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        );
        aggregate.provider_retries = aggregate.provider_retries.saturating_add(
            routing
                .get("provider_retries")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        );
    }
    let Some(by_prompt) = metrics.get("by_prompt").and_then(Value::as_object) else {
        return;
    };
    for (label, bucket) in by_prompt {
        let count = bucket.get("count").and_then(Value::as_u64).unwrap_or(0);
        let elapsed = bucket
            .get("elapsed_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if label == "plan" && count > 0 {
            aggregate.planner_latency_ms.push(elapsed / count);
        }
        let before = bucket
            .get("prompt_bytes_before_max")
            .and_then(Value::as_u64);
        let after = bucket
            .get("prompt_bytes_after_max")
            .and_then(Value::as_u64);
        if let (Some(before), Some(after)) = (before, after) {
            aggregate.prompt_bytes_before =
                aggregate.prompt_bytes_before.saturating_add(before);
            aggregate.prompt_bytes_after = aggregate
                .prompt_bytes_after
                .saturating_add(after.min(before));
        }
    }
}

fn collect_tool_latencies(trace: &Value, latencies: &mut Vec<u64>) {
    let Some(steps) = trace.get("step_results").and_then(Value::as_array) else {
        return;
    };
    for step in steps {
        let started = step.get("started_at").and_then(Value::as_u64);
        let finished = step.get("finished_at").and_then(Value::as_u64);
        if let (Some(started), Some(finished)) = (started, finished) {
            latencies.push(finished.saturating_sub(started).saturating_mul(1_000));
        }
    }
}

fn collect_cost_metrics(summary: &Value, aggregate: &mut SloAggregate) {
    let Some(cost) = summary.pointer("/task_metrics/llm_cost") else {
        return;
    };
    aggregate.known_cost_usd_nanos = aggregate.known_cost_usd_nanos.saturating_add(
        cost.get("estimated_cost_usd_nanos")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    );
    aggregate.unknown_cost_tasks += u64::from(
        cost.get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| status == "unknown"),
    );
}

fn count_machine_record(value: &Value, key: &str, token: &str) -> u64 {
    match value {
        Value::Object(map) => {
            let own = u64::from(map.get(key).and_then(Value::as_str) == Some(token));
            map.values().fold(own, |count, child| {
                count.saturating_add(count_machine_record(child, key, token))
            })
        }
        Value::Array(items) => items.iter().fold(0_u64, |count, child| {
            count.saturating_add(count_machine_record(child, key, token))
        }),
        _ => 0,
    }
}

fn inspect_local_async_jobs(workspace_root: &Path, now_ts: u64) -> LocalAsyncJobHealth {
    const ORPHAN_GRACE_SECONDS: u64 = 300;
    const MAX_SCANNED_JOBS: usize = 10_000;
    let mut health = LocalAsyncJobHealth::default();
    let root = workspace_root.join(".rustclaw").join("async_jobs");
    let Ok(entries) = fs::read_dir(root) else {
        return health;
    };
    for entry in entries.flatten().take(MAX_SCANNED_JOBS) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        health.scanned_jobs += 1;
        if path.join("finished_at").is_file() || path.join("exit_code").is_file() {
            health.terminal_jobs += 1;
            continue;
        }
        let started_at = read_u64_file(&path.join("started_at"));
        let pid = read_u64_file(&path.join("pid")).and_then(|value| u32::try_from(value).ok());
        match pid.and_then(local_process_alive) {
            Some(true) => health.running_jobs += 1,
            Some(false)
                if started_at
                    .is_some_and(|started| now_ts.saturating_sub(started) >= ORPHAN_GRACE_SECONDS) =>
            {
                health.orphaned_jobs += 1;
            }
            _ => health.indeterminate_jobs += 1,
        }
    }
    health
}

fn read_u64_file(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(target_os = "linux")]
fn local_process_alive(pid: u32) -> Option<bool> {
    Some(Path::new("/proc").join(pid.to_string()).is_dir())
}

#[cfg(not(target_os = "linux"))]
fn local_process_alive(_pid: u32) -> Option<bool> {
    None
}

fn slo_metrics_json(
    aggregate: &SloAggregate,
    window_hours: u64,
    cutoff_ts: u64,
    sampled_rows: usize,
    limit: usize,
    truncated: bool,
    async_jobs: &LocalAsyncJobHealth,
) -> Value {
    let compaction_saved = aggregate
        .prompt_bytes_before
        .saturating_sub(aggregate.prompt_bytes_after);
    json!({
        "schema_version": 1,
        "window": {
            "hours": window_hours,
            "cutoff_ts": cutoff_ts,
            "sampled_task_count": sampled_rows,
            "task_limit": limit,
            "truncated": truncated,
        },
        "outcomes": {
            "terminal_tasks": aggregate.terminal_tasks,
            "succeeded_tasks": aggregate.succeeded_tasks,
            "success_rate_millis": ratio_millis(aggregate.succeeded_tasks, aggregate.terminal_tasks),
            "resumed_terminal_tasks": aggregate.resumed_terminal_tasks,
            "resumed_succeeded_tasks": aggregate.resumed_succeeded_tasks,
            "resume_success_rate_millis": ratio_millis(
                aggregate.resumed_succeeded_tasks,
                aggregate.resumed_terminal_tasks,
            ),
        },
        "latency": {
            "planner_ms": percentile_summary(&aggregate.planner_latency_ms, "prompt_bucket_average"),
            "tool_ms": percentile_summary(&aggregate.tool_latency_ms, "step_wall_clock"),
        },
        "reliability": {
            "logical_llm_calls": aggregate.logical_llm_calls,
            "provider_selections": aggregate.provider_selections,
            "provider_retries": aggregate.provider_retries,
            "retry_amplification_millis": ratio_millis(
                aggregate.provider_selections,
                aggregate.logical_llm_calls,
            ),
            "prompt_truncations": aggregate.prompt_truncations,
            "duplicate_mutations_suppressed": aggregate.duplicate_mutations_suppressed,
            "duplicate_side_effects_observed": Value::Null,
            "duplicate_side_effect_observation_status": "indirect_only",
            "stale_running_recoveries": aggregate.stale_running_recoveries,
            "local_async_jobs": {
                "scanned_jobs": async_jobs.scanned_jobs,
                "running_jobs": async_jobs.running_jobs,
                "terminal_jobs": async_jobs.terminal_jobs,
                "orphaned_jobs": async_jobs.orphaned_jobs,
                "indeterminate_jobs": async_jobs.indeterminate_jobs,
            },
        },
        "context": {
            "prompt_bytes_before_compaction": aggregate.prompt_bytes_before,
            "prompt_bytes_after_compaction": aggregate.prompt_bytes_after,
            "compaction_saved_bytes": compaction_saved,
            "compaction_ratio_millis": ratio_millis(
                compaction_saved,
                aggregate.prompt_bytes_before,
            ),
        },
        "cost": {
            "known_cost_usd_nanos": aggregate.known_cost_usd_nanos,
            "unknown_cost_tasks": aggregate.unknown_cost_tasks,
            "known_cost_per_success_usd_nanos": (aggregate.succeeded_tasks > 0)
                .then(|| aggregate.known_cost_usd_nanos / aggregate.succeeded_tasks),
        },
        "coverage": {
            "journal_tasks": aggregate.journal_tasks,
            "result_parse_errors": aggregate.result_parse_errors,
            "journal_unavailable_tasks": aggregate.journal_unavailable_tasks,
        },
    })
}

fn ratio_millis(numerator: u64, denominator: u64) -> Option<u64> {
    (denominator > 0).then(|| numerator.saturating_mul(1_000) / denominator)
}

fn percentile_summary(samples: &[u64], sample_unit: &'static str) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    json!({
        "sample_count": sorted.len(),
        "sample_unit": sample_unit,
        "p50": percentile(&sorted, 50),
        "p95": percentile(&sorted, 95),
    })
}

fn percentile(sorted: &[u64], percentile: usize) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = percentile
        .saturating_mul(sorted.len())
        .saturating_add(99)
        / 100;
    sorted.get(rank.saturating_sub(1)).copied()
}

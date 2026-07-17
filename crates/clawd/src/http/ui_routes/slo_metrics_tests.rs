use super::*;

fn row(status: &str, claim_attempt: u64, summary: Value, trace: Value) -> SloTaskRow {
    SloTaskRow {
        status: status.to_string(),
        claim_attempt,
        result_parse_error: false,
        result: Some(json!({
            "task_journal": {
                "summary": summary,
                "trace": trace,
            }
        })),
    }
}

fn summary(
    logical_calls: u64,
    selections: u64,
    retries: u64,
    truncations: u64,
    cost_status: &str,
    cost: u64,
) -> Value {
    json!({
        "task_metrics": {
            "prompt_truncation_count": truncations,
            "provider_routing": {
                "logical_calls": logical_calls,
                "provider_selections": selections,
                "provider_retries": retries,
            },
            "by_prompt": {
                "plan": {
                    "count": 2,
                    "elapsed_ms": 400,
                    "prompt_bytes_before_max": 1000,
                    "prompt_bytes_after_max": 600,
                }
            },
            "llm_cost": {
                "status": cost_status,
                "estimated_cost_usd_nanos": cost,
            }
        }
    })
}

#[test]
fn slo_aggregate_projects_outcome_latency_reliability_context_and_cost() {
    let rows = vec![
        row(
            "succeeded",
            2,
            summary(2, 3, 1, 1, "known", 900),
            json!({
                "step_results": [
                    {"started_at": 10, "finished_at": 12},
                    {
                        "started_at": 20,
                        "finished_at": 25,
                        "output": {
                            "source": "task_mutation_ledger",
                            "reason_code": "mutation_already_completed"
                        }
                    }
                ]
            }),
        ),
        row(
            "failed",
            2,
            summary(1, 1, 0, 0, "unknown", 0),
            json!({"step_results": [{"started_at": 30, "finished_at": 31}]}),
        ),
    ];

    let aggregate = aggregate_slo_rows(&rows);
    let async_jobs = LocalAsyncJobHealth {
        scanned_jobs: 3,
        running_jobs: 1,
        terminal_jobs: 1,
        orphaned_jobs: 1,
        indeterminate_jobs: 0,
    };
    let report = slo_metrics_json(&aggregate, 24, 100, rows.len(), 5_000, false, &async_jobs);

    assert_eq!(report["outcomes"]["success_rate_millis"], 500);
    assert_eq!(report["outcomes"]["resume_success_rate_millis"], 500);
    assert_eq!(report["failure_classes"]["agent_failure"], 1);
    assert_eq!(report["latency"]["planner_ms"]["p50"], 200);
    assert_eq!(report["latency"]["tool_ms"]["p50"], 2_000);
    assert_eq!(report["latency"]["tool_ms"]["p95"], 5_000);
    assert_eq!(report["reliability"]["retry_amplification_millis"], 1_333);
    assert_eq!(report["reliability"]["prompt_truncations"], 1);
    assert_eq!(report["reliability"]["duplicate_mutations_suppressed"], 1);
    assert_eq!(
        report["reliability"]["duplicate_side_effects_observed"],
        Value::Null
    );
    assert_eq!(
        report["reliability"]["local_async_jobs"]["orphaned_jobs"],
        1
    );
    assert_eq!(report["context"]["compaction_saved_bytes"], 800);
    assert_eq!(report["context"]["compaction_ratio_millis"], 400);
    assert_eq!(report["cost"]["known_cost_per_success_usd_nanos"], 900);
    assert_eq!(report["cost"]["unknown_cost_tasks"], 1);
    assert_eq!(report["coverage"]["journal_tasks"], 2);
}

#[test]
fn slo_failure_classes_use_structured_machine_fields_only() {
    let rows = vec![
        row(
            "failed",
            1,
            json!({"final_failure_attribution": "provider_error"}),
            json!({}),
        ),
        row(
            "failed",
            1,
            json!({"final_failure_attribution": "contract_gap"}),
            json!({}),
        ),
        row(
            "timeout",
            1,
            json!({"final_failure_attribution": "tool_gap"}),
            json!({"step_results": [{"status": "error"}]}),
        ),
        row(
            "failed",
            1,
            json!({"final_failure_attribution": "tool_gap"}),
            json!({"permission_decision": {"denied_by_policy": true}}),
        ),
    ];

    let aggregate = aggregate_slo_rows(&rows);
    let report = slo_metrics_json(
        &aggregate,
        1,
        100,
        rows.len(),
        100,
        false,
        &LocalAsyncJobHealth::default(),
    );

    assert_eq!(report["outcomes"]["failed_tasks"], 4);
    assert_eq!(report["failure_classes"]["model_failure"], 1);
    assert_eq!(report["failure_classes"]["agent_failure"], 1);
    assert_eq!(report["failure_classes"]["tool_failure"], 1);
    assert_eq!(report["failure_classes"]["policy_block"], 1);
    assert_eq!(report["failure_classes"]["classified_failure_count"], 4);
}

#[test]
fn slo_aggregate_uses_machine_recovery_records_without_text_matching() {
    let rows = vec![SloTaskRow {
        status: "timeout".to_string(),
        claim_attempt: 1,
        result_parse_error: false,
        result: Some(json!({
            "task_lifecycle": {
                "source": "worker_stale_recovery",
                "reason_code": "worker_lease_expired"
            }
        })),
    }];

    let aggregate = aggregate_slo_rows(&rows);
    let report = slo_metrics_json(
        &aggregate,
        1,
        100,
        rows.len(),
        100,
        false,
        &LocalAsyncJobHealth::default(),
    );

    assert_eq!(report["reliability"]["stale_running_recoveries"], 1);
    assert_eq!(report["coverage"]["journal_tasks"], 0);
    assert_eq!(report["coverage"]["result_parse_errors"], 0);
    assert_eq!(report["coverage"]["journal_unavailable_tasks"], 1);
    assert_eq!(report["latency"]["planner_ms"]["p50"], Value::Null);
    assert_eq!(report["outcomes"]["success_rate_millis"], 0);
}

#[test]
fn percentile_uses_nearest_rank_and_empty_samples_are_unknown() {
    assert_eq!(percentile(&[], 95), None);
    assert_eq!(percentile(&[10, 20, 30, 40], 50), Some(20));
    assert_eq!(percentile(&[10, 20, 30, 40], 95), Some(40));
}

#[test]
fn slo_task_reader_applies_window_and_reports_malformed_result_json() {
    let state = AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            result_json TEXT,
            claim_attempt INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )
    .expect("create task fixture table");
    db.execute(
        "INSERT INTO tasks (
            task_id, status, result_json, claim_attempt, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params!["task-slo-reader", "succeeded", "{invalid", 2, "100", "200"],
    )
    .expect("insert task fixture");
    drop(db);

    let rows = read_slo_task_rows(&state, 150, 10).expect("read SLO rows");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "succeeded");
    assert_eq!(rows[0].claim_attempt, 2);
    assert!(rows[0].result.is_none());
    assert!(rows[0].result_parse_error);
    let aggregate = aggregate_slo_rows(&rows);
    assert_eq!(aggregate.result_parse_errors, 1);
}

#[test]
fn slo_error_response_exposes_only_machine_contract_fields() {
    let (status, Json(response)) = slo_metrics_error(StatusCode::FORBIDDEN, "admin_required");

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("admin_required"));
    let data = response.data.expect("error contract");
    assert_eq!(data["error_code"], "admin_required");
    assert_eq!(data["message_key"], "clawd.ui.observability.admin_required");
}

#[test]
fn local_async_job_health_distinguishes_terminal_and_stale_dead_jobs() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-slo-async-jobs-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let async_root = root.join(".rustclaw").join("async_jobs");
    let terminal = async_root.join("terminal");
    let orphaned = async_root.join("orphaned");
    fs::create_dir_all(&terminal).expect("terminal dir");
    fs::create_dir_all(&orphaned).expect("orphaned dir");
    fs::write(terminal.join("finished_at"), "900").expect("terminal marker");
    fs::write(orphaned.join("started_at"), "100").expect("started marker");
    fs::write(orphaned.join("pid"), "4294967294").expect("dead pid");

    let health = inspect_local_async_jobs(&root, 1_000);

    assert_eq!(health.scanned_jobs, 2);
    assert_eq!(health.terminal_jobs, 1);
    #[cfg(target_os = "linux")]
    assert_eq!(health.orphaned_jobs, 1);
    #[cfg(not(target_os = "linux"))]
    assert_eq!(health.indeterminate_jobs, 1);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn slo_handler_requires_admin_and_returns_windowed_machine_metrics() {
    let state = AppState::test_default_with_fixture_provider();
    let now = crate::now_ts_u64();
    let db = state.core.db.get().expect("db");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("auth schema");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            result_json TEXT,
            claim_attempt INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )
    .expect("task schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES ('slo-admin', 'admin', 1, '1', NULL)",
        [],
    )
    .expect("admin identity");
    db.execute(
        "INSERT INTO tasks (
            task_id, status, result_json, claim_attempt, created_at, updated_at
         ) VALUES (?1, 'succeeded', ?2, 1, ?3, ?3)",
        rusqlite::params![
            "task-slo-handler",
            json!({
                "task_journal": {
                    "summary": summary(1, 1, 0, 0, "known", 100),
                    "trace": {"step_results": []}
                }
            })
            .to_string(),
            now.to_string(),
        ],
    )
    .expect("task row");
    drop(db);
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-rustclaw-key",
        axum::http::HeaderValue::from_static("slo-admin"),
    );

    let (status, Json(response)) = observability_slo_metrics(
        State(state),
        headers,
        Query(SloMetricsQuery {
            window_hours: Some(1),
            limit: Some(10),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(response.ok);
    let data = response.data.expect("SLO data");
    assert_eq!(data["schema_version"], 1);
    assert_eq!(data["window"]["sampled_task_count"], 1);
    assert_eq!(data["outcomes"]["succeeded_tasks"], 1);
    assert_eq!(data["cost"]["known_cost_usd_nanos"], 100);
}

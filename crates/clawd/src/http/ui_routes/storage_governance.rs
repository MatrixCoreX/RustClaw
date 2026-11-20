// Administrator-only inventory. No artifact deletion, vacuum, or raw-path exposure.
const STORAGE_SCAN_MAX_ENTRIES: usize = 80_000;
#[derive(Default, Serialize, Deserialize)]
struct StorageCategory {
    id: String,
    bytes: u64,
    files: u64,
    partial: bool,
    protected: bool,
}
#[derive(Serialize, Deserialize)]
struct StorageInventory {
    schema_version: u32,
    collected_at_ts: i64,
    categories: Vec<StorageCategory>,
    result_compaction: Value,
    history: Vec<Value>,
    growth: Vec<Value>,
    volumes: Vec<StorageVolume>,
    capacity_forecast: Vec<Value>,
}

#[derive(Serialize, Deserialize)]
struct StorageVolume {
    role: String,
    total_bytes: u64,
    available_bytes: u64,
}

fn storage_volumes(root: &Path) -> Vec<StorageVolume> {
    [
        ("system", root.ancestors().last().unwrap_or(root)),
        ("workspace", root),
    ]
    .into_iter()
    .filter_map(|(role, path)| {
        let (Some(total), Some(available)) = storage_capacity_bytes(path)? else {
            return None;
        };
        Some(StorageVolume {
            role: role.into(),
            total_bytes: total,
            available_bytes: available,
        })
    })
    .collect()
}

fn scan_storage_category(
    id: &str,
    root: &Path,
    allowed: &Path,
    budget: &mut usize,
) -> StorageCategory {
    let mut category = StorageCategory {
        id: id.into(),
        protected: true,
        ..Default::default()
    };
    if !root.exists() {
        return category;
    }
    let Ok(canonical) = root.canonicalize() else {
        category.partial = true;
        return category;
    };
    if !canonical.starts_with(allowed) {
        category.partial = true;
        return category;
    }
    // Fixed roots and all descendants are inspected without following symlinks.
    let mut pending = vec![root.to_path_buf()];
    let started = std::time::Instant::now();
    while let Some(path) = pending.pop() {
        if *budget == 0 || started.elapsed() > Duration::from_millis(350) {
            category.partial = true;
            break;
        }
        *budget -= 1;
        let Ok(meta) = fs::symlink_metadata(&path) else {
            category.partial = true;
            continue;
        };
        if meta.file_type().is_symlink() {
            category.partial = true;
            continue;
        }
        if meta.is_file() {
            category.bytes = category.bytes.saturating_add(meta.len());
            category.files += 1;
        } else if meta.is_dir() {
            let Ok(entries) = fs::read_dir(&path) else {
                category.partial = true;
                continue;
            };
            for entry in entries {
                if pending.len() >= (*budget).min(STORAGE_SCAN_MAX_ENTRIES)
                    || started.elapsed() > Duration::from_millis(350)
                {
                    category.partial = true;
                    break;
                }
                match entry {
                    Ok(entry) => pending.push(entry.path()),
                    Err(_) => category.partial = true,
                }
            }
        }
    }
    category
}

fn scan_storage_categories(
    roots: &[(&str, PathBuf)],
    allowed: &Path,
    budget: &mut usize,
) -> Vec<StorageCategory> {
    let mut categories = Vec::with_capacity(roots.len());
    for (index, (id, path)) in roots.iter().enumerate() {
        // Reserve a share for every remaining category, including small roots
        // after large evidence/package trees. Unused entries remain available.
        let allocated = *budget / (roots.len() - index);
        let mut category_budget = allocated;
        categories.push(scan_storage_category(
            id,
            path,
            allowed,
            &mut category_budget,
        ));
        *budget -= allocated - category_budget;
    }
    categories
}

fn storage_inventory(state: &AppState, persist: bool) -> anyhow::Result<StorageInventory> {
    let root = state.skill_rt.workspace_root.canonicalize()?;
    let private = claw_core::workspace_state::workspace_state_root(&root);
    let mut budget = STORAGE_SCAN_MAX_ENTRIES;
    let roots = [
        ("user_outputs", private.join("artifacts")),
        ("validation_evidence", private.join("optimization-progress")),
        (
            "operator_verification",
            root.join("data/operator-verification"),
        ),
        ("packages", root.join("data/skill-packages")),
        ("skill_data", root.join("data/skills")),
        ("logs", root.join("logs")),
        ("models", root.join("data/models")),
        ("backups", root.join("data/backups")),
        ("release_backups", root.join(".release-backups")),
        ("release_downloads", root.join("data/runtime-upgrades")),
        ("prebuilt_packages", root.join("prebuilt/skill-packages")),
        (
            "runtime_dependencies",
            root.join("data/runtime-dependencies"),
        ),
        ("runtime_binaries", root.join("target/release")),
    ];
    let mut categories = scan_storage_categories(&roots, &root, &mut budget);
    // Database paths come from SQLite, never a caller-controlled filesystem parameter.
    let db = state
        .core
        .db
        .get()
        .map_err(|_| anyhow::anyhow!("storage_database_unavailable"))?;
    let path: String = db.query_row(
        "SELECT file FROM pragma_database_list WHERE name='main'",
        [],
        |r| r.get(0),
    )?;
    let mut database = StorageCategory {
        id: "database".into(),
        protected: true,
        ..Default::default()
    };
    for path in [path.clone(), format!("{path}-wal"), format!("{path}-shm")] {
        if let Ok(meta) = fs::symlink_metadata(path) {
            if meta.is_file() {
                database.bytes += meta.len();
                database.files += 1;
            } else {
                database.partial = true;
            }
        }
    }
    categories.push(database);
    let result_compaction = storage_compaction_stats(&db)?;
    let collected_at_ts = now_unix_seconds();
    let volumes = storage_volumes(&root);
    if persist {
        db.execute_batch("CREATE TABLE IF NOT EXISTS runtime_storage_inventory_samples(sample_id INTEGER PRIMARY KEY,collected_at INTEGER NOT NULL,snapshot_json TEXT NOT NULL)")?;
        let tx = db.unchecked_transaction()?;
        tx.execute("INSERT INTO runtime_storage_inventory_samples(collected_at,snapshot_json) VALUES(?1,?2)",rusqlite::params![collected_at_ts,serde_json::to_string(&json!({"schema_version":2,"categories":categories,"volumes":volumes}))?])?;
        tx.execute("DELETE FROM runtime_storage_inventory_samples WHERE sample_id NOT IN (SELECT sample_id FROM runtime_storage_inventory_samples ORDER BY sample_id DESC LIMIT 24)",[])?;
        tx.commit()?;
    }
    let history = storage_inventory_history(&db)?;
    let growth = storage_growth_forecast(&categories, &history, collected_at_ts);
    let capacity_forecast = storage_capacity_forecast(&volumes, &history, collected_at_ts);
    Ok(StorageInventory {
        schema_version: 1,
        collected_at_ts,
        categories,
        result_compaction,
        history,
        growth,
        volumes,
        capacity_forecast,
    })
}

// Partial scans and short observation periods are not growth evidence. Compare
// complete samples only, and keep decreases visible instead of clamping to zero.
fn storage_growth_forecast(
    categories: &[StorageCategory],
    history: &[Value],
    now: i64,
) -> Vec<Value> {
    categories.iter().filter(|current| !current.partial).filter_map(|current| {
        let prior = history.iter().filter_map(|sample| {
            let ts = sample.get("collected_at_ts")?.as_i64()?;
            let elapsed = now.checked_sub(ts)?;
            if !(3600..=7 * 86400).contains(&elapsed) { return None; }
            let previous = sample.get("categories")?.as_array()?.iter().find(|category| {
                category.get("id").and_then(Value::as_str) == Some(current.id.as_str())
                    && category.get("partial").and_then(Value::as_bool) == Some(false)
            })?;
            Some((ts, previous.get("bytes")?.as_u64()?))
        }).min_by_key(|(ts, _)| *ts)?;
        let elapsed = now - prior.0;
        let delta = i128::from(current.bytes) - i128::from(prior.1);
        let rate = (delta * 86400 / i128::from(elapsed)).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
        Some(json!({"id":current.id,"observation_seconds":elapsed,"bytes_per_day":rate,"basis":"complete_category_samples","retention":"preserve_user_data"}))
    }).collect()
}
fn storage_capacity_forecast(volumes: &[StorageVolume], history: &[Value], now: i64) -> Vec<Value> {
    volumes.iter().filter_map(|current| {
        let previous=history.iter().filter_map(|sample| {
            let ts=sample.get("collected_at_ts")?.as_i64()?;
            if !(3600..=7*86400).contains(&now.checked_sub(ts)?) { return None; }
            let volume=sample.get("volumes")?.as_array()?.iter().find(|v| v.get("role").and_then(Value::as_str)==Some(current.role.as_str()) && v.get("total_bytes").and_then(Value::as_u64)==Some(current.total_bytes))?;
            Some((ts,volume.get("available_bytes")?.as_u64()?))
        }).min_by_key(|(ts,_)|*ts)?;
        let consumed=i128::from(previous.1)-i128::from(current.available_bytes);
        let rate=(consumed*86400/i128::from(now-previous.0)).clamp(i128::from(i64::MIN),i128::from(i64::MAX)) as i64;
        let guard=(512_u64*1024*1024).max(current.total_bytes/100);
        let days=(rate>0).then(|| current.available_bytes.saturating_sub(guard) as f64/rate as f64);
        Some(json!({"role":current.role,"bytes_per_day":rate,"days_to_write_guard":days,"write_guard_bytes":guard,"observation_seconds":now-previous.0,"estimate_only":true}))
    }).collect()
}

fn storage_compaction_stats(db: &rusqlite::Connection) -> anyhow::Result<Value> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='runtime_result_object_maintenance')",
        [],
        |row| row.get(0),
    )?;
    let stats: Option<Value> = if exists {
        db.query_row(
            "SELECT last_rowid,saved_bytes,compacted_rows FROM runtime_result_object_maintenance WHERE singleton=1",
            [], |row| Ok(json!({
                "cursor":row.get::<_,i64>(0)?,
                "saved_logical_bytes":row.get::<_,u64>(1)?,
                "compacted_rows":row.get::<_,u64>(2)?,
            })),
        ).optional()?
    } else {
        None
    };
    let mut value =
        stats.unwrap_or_else(|| json!({"cursor":0,"saved_logical_bytes":0,"compacted_rows":0}));
    let pages: u64 = db.query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
    let page_bytes: u64 = db.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    value["database_reusable_bytes"] = json!(pages.saturating_mul(page_bytes));
    Ok(value)
}
fn storage_inventory_history(db: &rusqlite::Connection) -> anyhow::Result<Vec<Value>> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='runtime_storage_inventory_samples')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(Vec::new());
    }
    let mut query=db.prepare("SELECT collected_at,snapshot_json FROM runtime_storage_inventory_samples ORDER BY sample_id DESC LIMIT 24")?;
    let rows = query
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(ts, raw)| {
            let value: Value = serde_json::from_str(&raw).ok()?;
            let categories: Vec<StorageCategory> = serde_json::from_value(if value.is_array() {
                value.clone()
            } else {
                value.get("categories")?.clone()
            })
            .ok()?;
            let volumes: Vec<StorageVolume> =
                serde_json::from_value(value.get("volumes").cloned().unwrap_or_else(|| json!([])))
                    .ok()?;
            Some(json!({"collected_at_ts":ts,"categories":categories,"volumes":volumes}))
        })
        .collect())
}
async fn get_storage_inventory(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    run_storage_inventory(state, headers, false).await
}

async fn get_checkpoint_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    let data = state.skill_rt.workspace_root.join("data");
    let path = data.join("runtime-checkpoint-status.json");
    let value = tokio::task::spawn_blocking(move || -> Option<Value> {
        let mut safe = read_operator_status_projection(
            &path,
            &[
                "schema_version",
                "status",
                "error_code",
                "created_at_ts",
                "attempted_at_ts",
                "bytes",
                "database_count",
            ],
        )
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_else(|| {
            serde_json::Map::from_iter([
                ("schema_version".into(), json!(1)),
                ("status".into(), json!("unavailable")),
            ])
        });
        for (name, file, fields) in [
            (
                "schedule",
                "runtime-backup-schedule.json",
                &[
                    "phase",
                    "attempt",
                    "next_retry_at_ts",
                    "error_code",
                    "notice_key",
                    "cycle_started_at_ts",
                ][..],
            ),
            (
                "vault",
                "runtime-vault-status.json",
                &[
                    "status",
                    "encrypted",
                    "retained_versions",
                    "verified_at_ts",
                    "payload_scopes",
                    "uploaded_bytes",
                    "reused_bytes",
                    "file_count",
                    "restore_verified_at_ts",
                ][..],
            ),
        ] {
            if let Some(report) = read_operator_status_projection(&data.join(file), fields) {
                safe.insert(name.into(), report);
            }
        }
        Some(Value::Object(safe))
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| json!({"schema_version":1,"status":"unavailable"}));
    crate::api_ok(value)
}

fn read_operator_status_projection(path: &Path, fields: &[&str]) -> Option<Value> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 65536 {
        return None;
    }
    let value: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let safe = fields
        .iter()
        .filter_map(|key| {
            let value = value.get(*key)?;
            let valid = match *key {
                "phase" | "status" | "notice_key" | "error_code" => {
                    value.as_str().is_some_and(|s| {
                        s.len() <= 128
                            && s.bytes().all(|c| {
                                c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-')
                            })
                    })
                }
                "payload_scopes" => value.as_array().is_some_and(|items| {
                    items.len() <= 3
                        && items.iter().all(|item| {
                            matches!(item.as_str(), Some("packages" | "models" | "attachments"))
                        })
                }),
                "encrypted" => value.is_boolean(),
                _ => value.as_u64().is_some(),
            };
            valid.then(|| ((*key).to_string(), value.clone()))
        })
        .collect::<serde_json::Map<_, _>>();
    Some(Value::Object(safe))
}
async fn record_storage_inventory(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    run_storage_inventory(state, headers, true).await
}
async fn run_storage_inventory(
    state: AppState,
    headers: HeaderMap,
    persist: bool,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    match tokio::task::spawn_blocking(move || storage_inventory(&state, persist)).await {
        Ok(Ok(value)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!(value)),
                error: None,
            }),
        ),
        _ => crate::api_err(
            StatusCode::SERVICE_UNAVAILABLE,
            "storage_inventory_unavailable",
        ),
    }
}
async fn compact_stored_results(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    match tokio::task::spawn_blocking(move || {
        let mut db = state
            .core
            .db
            .get()
            .map_err(|_| anyhow::anyhow!("storage_database_unavailable"))?;
        crate::task_result_objects::compact_batch(&mut db, 64)
    })
    .await
    {
        Ok(Ok(value)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!(value)),
                error: None,
            }),
        ),
        _ => crate::api_err(
            StatusCode::SERVICE_UNAVAILABLE,
            "result_compaction_unavailable",
        ),
    }
}
#[cfg(test)]
#[path = "storage_governance_tests.rs"]
mod storage_governance_tests;

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
    ];
    let mut categories = roots
        .iter()
        .map(|(id, path)| scan_storage_category(id, path, &root, &mut budget))
        .collect::<Vec<_>>();
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
    let result_compaction=db.query_row("SELECT last_rowid,saved_bytes,compacted_rows FROM runtime_result_object_maintenance WHERE singleton=1",[],|r|Ok(json!({"cursor":r.get::<_,i64>(0)?,"saved_logical_bytes":r.get::<_,u64>(1)?,"compacted_rows":r.get::<_,u64>(2)?}))).unwrap_or_else(|_|json!({"cursor":0,"saved_logical_bytes":0,"compacted_rows":0}));
    let collected_at_ts = now_unix_seconds();
    if persist {
        db.execute_batch("CREATE TABLE IF NOT EXISTS runtime_storage_inventory_samples(sample_id INTEGER PRIMARY KEY,collected_at INTEGER NOT NULL,snapshot_json TEXT NOT NULL)")?;
        let tx = db.unchecked_transaction()?;
        tx.execute("INSERT INTO runtime_storage_inventory_samples(collected_at,snapshot_json) VALUES(?1,?2)",rusqlite::params![collected_at_ts,serde_json::to_string(&categories)?])?;
        tx.execute("DELETE FROM runtime_storage_inventory_samples WHERE sample_id NOT IN (SELECT sample_id FROM runtime_storage_inventory_samples ORDER BY sample_id DESC LIMIT 24)",[])?;
        tx.commit()?;
    }
    let history=(||->rusqlite::Result<Vec<Value>> {
        let mut query=db.prepare("SELECT collected_at,snapshot_json FROM runtime_storage_inventory_samples ORDER BY sample_id DESC LIMIT 24")?;
        let rows=query.query_map([],|r| {let raw:String=r.get(1)?;Ok(json!({"collected_at_ts":r.get::<_,i64>(0)?,"categories":serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null)}))})?.collect();
        rows
    })().unwrap_or_default();
    Ok(StorageInventory {
        schema_version: 1,
        collected_at_ts,
        categories,
        result_compaction,
        history,
    })
}
async fn get_storage_inventory(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    run_storage_inventory(state, headers, false).await
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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::result_pagination::{paginate, CursorInput};

const CACHE_TTL_SECONDS: i64 = 60;
const CACHE_MAX_ENTRIES: i64 = 16;
const CACHE_MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(super) struct SnapshotCache {
    database_path: PathBuf,
}

#[derive(Debug, Clone)]
pub(super) struct CachedSnapshot {
    pub(super) template: Value,
    pub(super) primary_items: Vec<Value>,
    pub(super) stale: bool,
    pub(super) age_seconds: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachePayload {
    template: Value,
    primary_items: Vec<Value>,
    validation: Vec<PathStamp>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PathStamp {
    path: String,
    kind: String,
    size_bytes: u64,
    modified_unix_ns: Option<String>,
}

impl SnapshotCache {
    pub(super) fn from_context(context: Option<&Value>) -> Result<Option<Self>, String> {
        let Some(storage) = context.and_then(|value| value.get("skill_storage")) else {
            return Ok(None);
        };
        if storage.get("storage_kind").and_then(Value::as_str) != Some("sqlite")
            || storage.get("skill_name").and_then(Value::as_str) != Some("fs_search")
            || storage.get("schema_version").and_then(Value::as_u64) != Some(1)
        {
            return Err("skill_storage_invalid".to_string());
        }
        let path = storage
            .get("database_path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| "skill_storage_invalid".to_string())?;
        let cache = Self {
            database_path: path,
        };
        cache.initialize()?;
        Ok(Some(cache))
    }

    pub(super) fn load(
        &self,
        action: &str,
        query_sha256: &str,
        snapshot_sha256: &str,
    ) -> Result<Option<CachedSnapshot>, String> {
        let now = now_seconds();
        let db = self.open()?;
        let row = db
            .query_row(
                "SELECT payload_json, created_at FROM fs_search_snapshots
                 WHERE action = ?1 AND query_sha256 = ?2 AND snapshot_sha256 = ?3",
                params![action, query_sha256, snapshot_sha256],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|_| "snapshot_cache_read_failed".to_string())?;
        let Some((payload, created_at)) = row else {
            return Ok(None);
        };
        let age_seconds = now.saturating_sub(created_at);
        if age_seconds > CACHE_TTL_SECONDS {
            db.execute(
                "DELETE FROM fs_search_snapshots
                 WHERE action = ?1 AND query_sha256 = ?2 AND snapshot_sha256 = ?3",
                params![action, query_sha256, snapshot_sha256],
            )
            .map_err(|_| "snapshot_cache_prune_failed".to_string())?;
            return Ok(None);
        }
        let payload: CachePayload = serde_json::from_slice(&payload)
            .map_err(|_| "snapshot_cache_payload_invalid".to_string())?;
        let stale = payload
            .validation
            .iter()
            .any(|stamp| !stamp.matches_current());
        db.execute(
            "UPDATE fs_search_snapshots SET last_accessed_at = ?1
             WHERE action = ?2 AND query_sha256 = ?3 AND snapshot_sha256 = ?4",
            params![now, action, query_sha256, snapshot_sha256],
        )
        .map_err(|_| "snapshot_cache_touch_failed".to_string())?;
        Ok(Some(CachedSnapshot {
            template: payload.template,
            primary_items: payload.primary_items,
            stale,
            age_seconds,
        }))
    }

    pub(super) fn store(
        &self,
        action: &str,
        query_sha256: &str,
        snapshot_sha256: &str,
        root: &Path,
        workspace_root: &Path,
        template: &Value,
        primary_items: &[Value],
    ) -> Result<&'static str, String> {
        let validation = validation_stamps(root, workspace_root, primary_items);
        let payload = serde_json::to_vec(&CachePayload {
            template: template.clone(),
            primary_items: primary_items.to_vec(),
            validation,
        })
        .map_err(|_| "snapshot_cache_payload_encode_failed".to_string())?;
        if payload.len() > CACHE_MAX_PAYLOAD_BYTES {
            return Ok("not_stored_too_large");
        }
        let now = now_seconds();
        let db = self.open()?;
        db.execute(
            "INSERT INTO fs_search_snapshots
             (action, query_sha256, snapshot_sha256, payload_json, payload_bytes,
              created_at, last_accessed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(action, query_sha256, snapshot_sha256) DO UPDATE SET
               payload_json = excluded.payload_json,
               payload_bytes = excluded.payload_bytes,
               created_at = excluded.created_at,
               last_accessed_at = excluded.last_accessed_at",
            params![
                action,
                query_sha256,
                snapshot_sha256,
                payload,
                payload.len() as i64,
                now
            ],
        )
        .map_err(|_| "snapshot_cache_write_failed".to_string())?;
        self.prune(&db, now)?;
        Ok("stored")
    }

    fn initialize(&self) -> Result<(), String> {
        let db = self.open()?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS fs_search_snapshots (
               action TEXT NOT NULL,
               query_sha256 TEXT NOT NULL,
               snapshot_sha256 TEXT NOT NULL,
               payload_json BLOB NOT NULL,
               payload_bytes INTEGER NOT NULL,
               created_at INTEGER NOT NULL,
               last_accessed_at INTEGER NOT NULL,
               PRIMARY KEY(action, query_sha256, snapshot_sha256)
             );
             CREATE INDEX IF NOT EXISTS idx_fs_search_snapshots_accessed
               ON fs_search_snapshots(last_accessed_at);",
        )
        .map_err(|_| "snapshot_cache_schema_failed".to_string())
    }

    fn open(&self) -> Result<Connection, String> {
        Connection::open(&self.database_path).map_err(|_| "snapshot_cache_open_failed".to_string())
    }

    fn prune(&self, db: &Connection, now: i64) -> Result<(), String> {
        db.execute(
            "DELETE FROM fs_search_snapshots
             WHERE created_at < ?1",
            params![now.saturating_sub(CACHE_TTL_SECONDS)],
        )
        .map_err(|_| "snapshot_cache_prune_failed".to_string())?;
        db.execute(
            "DELETE FROM fs_search_snapshots WHERE rowid IN (
               SELECT rowid FROM fs_search_snapshots
               ORDER BY last_accessed_at DESC LIMIT -1 OFFSET ?1
             )",
            params![CACHE_MAX_ENTRIES],
        )
        .map_err(|_| "snapshot_cache_prune_failed".to_string())?;
        Ok(())
    }
}

pub(super) fn render_cached(
    mut cached: CachedSnapshot,
    cursor: &CursorInput,
    limit: usize,
    query_sha256: &str,
) -> Result<Value, String> {
    if cached.stale {
        return Ok(render_stale(cached.template, cached.age_seconds));
    }
    let scan_truncated = cached
        .template
        .pointer("/page/scan_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let page = paginate(
        &cached.primary_items,
        cursor,
        limit,
        scan_truncated,
        query_sha256,
    )?;
    let action = cached
        .template
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let output_mode = cached
        .template
        .get("output_mode")
        .and_then(Value::as_str)
        .unwrap_or("content")
        .to_string();
    let object = cached
        .template
        .as_object_mut()
        .ok_or_else(|| "snapshot_cache_template_invalid".to_string())?;
    object.insert("page".to_string(), page.metadata.clone());
    object.insert(
        "snapshot_sha256".to_string(),
        Value::String(page.snapshot_sha256),
    );
    object.insert("has_more".to_string(), Value::Bool(page.has_more));
    object.insert("truncated".to_string(), Value::Bool(page.has_more));
    object.insert(
        "continuation".to_string(),
        page.metadata
            .get("next_cursor")
            .and_then(Value::as_str)
            .map(|cursor| json!({"kind":"next_page","safe_to_continue":true,"cursor":cursor}))
            .unwrap_or(Value::Null),
    );
    match action.as_str() {
        "find_name" | "find_ext" => {
            object.insert("results".to_string(), Value::Array(page.items));
            update_counts(object, page.returned_count, page.total_count);
        }
        "find_images" => {
            let results = page
                .items
                .iter()
                .filter_map(|item| item.get("path").cloned())
                .collect::<Vec<_>>();
            object.insert("results".to_string(), Value::Array(results));
            object.insert("images".to_string(), Value::Array(page.items));
            update_counts(object, page.returned_count, page.total_count);
        }
        "grep_text" if output_mode == "paths" => {
            object.insert("results".to_string(), Value::Array(page.items));
            object.insert("matches".to_string(), Value::Array(Vec::new()));
            object.insert("count".to_string(), json!(page.returned_count));
            object.insert("known_match_count".to_string(), json!(page.total_count));
        }
        "grep_text" => {
            let results = unique_match_paths(&page.items);
            let result_count = results.len();
            object.insert("results".to_string(), Value::Array(results));
            object.insert("matches".to_string(), Value::Array(page.items));
            object.insert("count".to_string(), json!(result_count));
            object.insert("match_count".to_string(), json!(page.returned_count));
            object.insert("total_match_count".to_string(), json!(page.total_count));
            object.insert("known_match_count".to_string(), json!(page.total_count));
        }
        _ => return Err("snapshot_cache_action_invalid".to_string()),
    }
    object.insert("cache_reused".to_string(), Value::Bool(true));
    object.insert("cache_age_seconds".to_string(), json!(cached.age_seconds));
    if let Some(scan) = object.get_mut("scan").and_then(Value::as_object_mut) {
        scan.insert("cache_reused".to_string(), Value::Bool(true));
    }
    Ok(cached.template)
}

pub(super) fn render_missing_snapshot(
    action: &str,
    query_sha256: &str,
    snapshot_sha256: &str,
    limit: usize,
) -> Value {
    json!({
        "schema_version": 2,
        "action": action,
        "status": "ok",
        "source_skill": "fs_search",
        "completeness": "stale_snapshot",
        "total_count_is_complete": false,
        "results": [],
        "matches": [],
        "images": [],
        "count": 0,
        "returned_count": 0,
        "known_match_count": 0,
        "has_more": false,
        "truncated": true,
        "cache_reused": false,
        "cache_status": "miss_or_expired",
        "snapshot_sha256": snapshot_sha256,
        "continuation": {
            "kind": "new_snapshot",
            "safe_to_continue": true,
            "reason_code": "snapshot_cache_miss"
        },
        "page": {
            "cursor": 0,
            "cursor_token": Value::Null,
            "limit": limit,
            "returned_count": 0,
            "known_match_count": 0,
            "total_count": 0,
            "has_more": false,
            "next_cursor": Value::Null,
            "previous_cursor": Value::Null,
            "legacy_next_offset": Value::Null,
            "scan_truncated": true,
            "stale_snapshot": true,
            "query_sha256": query_sha256,
            "snapshot_sha256": snapshot_sha256,
            "cache_reused": false
        },
        "scan": {
            "completeness": "stale_snapshot",
            "cache_reused": false,
            "cache_status": "miss_or_expired"
        }
    })
}

fn render_stale(mut template: Value, age_seconds: i64) -> Value {
    if let Some(object) = template.as_object_mut() {
        object.insert(
            "completeness".to_string(),
            Value::String("stale_snapshot".to_string()),
        );
        object.insert("total_count_is_complete".to_string(), Value::Bool(false));
        object.insert("has_more".to_string(), Value::Bool(false));
        object.insert("truncated".to_string(), Value::Bool(true));
        object.insert("results".to_string(), Value::Array(Vec::new()));
        object.insert("matches".to_string(), Value::Array(Vec::new()));
        object.insert("images".to_string(), Value::Array(Vec::new()));
        object.insert("count".to_string(), json!(0));
        object.insert("returned_count".to_string(), json!(0));
        object.insert("match_count".to_string(), json!(0));
        object.insert("cache_reused".to_string(), Value::Bool(true));
        object.insert("cache_age_seconds".to_string(), json!(age_seconds));
        object.insert(
            "continuation".to_string(),
            json!({"kind":"new_snapshot","safe_to_continue":true,"reason_code":"stale_snapshot"}),
        );
        if let Some(page) = object.get_mut("page").and_then(Value::as_object_mut) {
            page.insert("returned_count".to_string(), json!(0));
            page.insert("has_more".to_string(), Value::Bool(false));
            page.insert("next_cursor".to_string(), Value::Null);
            page.insert("stale_snapshot".to_string(), Value::Bool(true));
            page.insert("scan_truncated".to_string(), Value::Bool(true));
            page.insert("cache_reused".to_string(), Value::Bool(true));
        }
        if let Some(scan) = object.get_mut("scan").and_then(Value::as_object_mut) {
            scan.insert(
                "completeness".to_string(),
                Value::String("stale_snapshot".to_string()),
            );
            scan.insert("cache_reused".to_string(), Value::Bool(true));
        }
    }
    template
}

fn update_counts(object: &mut serde_json::Map<String, Value>, returned: usize, total: usize) {
    for key in ["count", "returned_count"] {
        object.insert(key.to_string(), json!(returned));
    }
    for key in ["total_count", "known_match_count"] {
        object.insert(key.to_string(), json!(total));
    }
}

fn unique_match_paths(items: &[Value]) -> Vec<Value> {
    let mut paths = BTreeSet::new();
    for item in items {
        if let Some(path) = item.get("path").and_then(Value::as_str) {
            paths.insert(path.to_string());
        }
    }
    paths.into_iter().map(Value::String).collect()
}

fn validation_stamps(root: &Path, workspace_root: &Path, items: &[Value]) -> Vec<PathStamp> {
    let mut paths = BTreeSet::new();
    insert_directory_validation_paths(&mut paths, root, root);
    for item in items {
        let path = item
            .as_str()
            .or_else(|| item.get("path").and_then(Value::as_str));
        let Some(path) = path else {
            continue;
        };
        let absolute = workspace_root.join(path);
        paths.insert(absolute.clone());
        let mut parent = absolute.parent();
        while let Some(value) = parent {
            if !value.starts_with(root) {
                break;
            }
            insert_directory_validation_paths(&mut paths, value, root);
            if value == root {
                break;
            }
            parent = value.parent();
        }
    }
    paths
        .into_iter()
        .map(|path| PathStamp::capture_or_missing(&path))
        .collect()
}

fn insert_directory_validation_paths(paths: &mut BTreeSet<PathBuf>, directory: &Path, root: &Path) {
    paths.insert(directory.to_path_buf());
    paths.insert(directory.join(".gitignore"));
    paths.insert(directory.join(".ignore"));
    if directory == root {
        paths.insert(directory.join(".git/info/exclude"));
    }
}

impl PathStamp {
    fn capture(path: &Path) -> Option<Self> {
        let metadata = std::fs::symlink_metadata(path).ok()?;
        let kind = if metadata.is_dir() {
            "dir"
        } else if metadata.is_file() {
            "file"
        } else {
            "other"
        };
        let modified_unix_ns = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_nanos().to_string());
        Some(Self {
            path: path.display().to_string(),
            kind: kind.to_string(),
            size_bytes: metadata.len(),
            modified_unix_ns,
        })
    }

    fn capture_or_missing(path: &Path) -> Self {
        Self::capture(path).unwrap_or_else(|| Self {
            path: path.display().to_string(),
            kind: "missing".to_string(),
            size_bytes: 0,
            modified_unix_ns: None,
        })
    }

    fn matches_current(&self) -> bool {
        let current = Self::capture_or_missing(Path::new(&self.path));
        current.kind == self.kind
            && current.size_bytes == self.size_bytes
            && current.modified_unix_ns == self.modified_unix_ns
    }
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

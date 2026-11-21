//! Offline, resumable task-event maintenance. Use a stopped runtime and a verified backup.
use anyhow::{ensure, Context};
use rusqlite::{types::ValueRef, Connection, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io::Write, path::PathBuf, time::Duration};

#[path = "../event_json_chunks.rs"]
mod event_json_chunks;
#[path = "../event_json_storage.rs"]
mod event_json_storage;
#[path = "../task_event_storage_maintenance.rs"]
mod task_event_storage_maintenance;

fn main() {
    if let Err(error) = run() {
        eprintln!(
            "{}",
            json!({"ok":false,"error_code":"event_maintenance_failed","detail":error.to_string()})
        );
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let action = args
        .first()
        .context("action_required: prepare|verify|pack|prune|vacuum")?;
    let argument = |key: &str| args.windows(2).find(|w| w[0] == key).map(|w| w[1].clone());
    let path = PathBuf::from(argument("--database").context("database_required")?);
    let flags = if action == "verify" {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let mut db = Connection::open_with_flags(&path, flags)?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.pragma_update(None, "cache_size", -8192)?;
    let batch: usize = argument("--batch-size")
        .unwrap_or_else(|| "128".into())
        .parse()?;
    match action.as_str() {
        "prepare" => {
            task_event_storage_maintenance::ensure_schema(&db)?;
            println!("{}", json!({"ok":true,"action":action}));
        }
        "pack-chunks" => {
            task_event_storage_maintenance::ensure_schema(&db)?;
            db.pragma_update(None, "synchronous", "FULL")?;
            db.pragma_update(None, "wal_autocheckpoint", 256)?;
            let mut total = 0usize;
            loop {
                let rows = event_json_chunks::compact_batch(&mut db, batch)?;
                if rows == 0 {
                    break;
                }
                total += rows;
                println!("{}", json!({"ok":true,"action":action,"total_rows":total}));
                std::io::stdout().flush()?;
            }
            checkpoint(&db)?;
        }
        "pack" => {
            task_event_storage_maintenance::ensure_schema(&db)?;
            db.pragma_update(None, "synchronous", "FULL")?;
            db.pragma_update(None, "wal_autocheckpoint", 256)?;
            let mut rows = 0;
            loop {
                let report = task_event_storage_maintenance::compact_batch(&mut db, batch)?;
                if report.rows == 0 {
                    break;
                }
                rows += report.rows;
                println!(
                    "{}",
                    json!({"ok":true,"action":action,"total_rows":rows,"batch":report})
                );
                std::io::stdout().flush()?;
            }
            checkpoint(&db)?;
            println!(
                "{}",
                json!({"ok":true,"action":action,"total_rows":rows,"pending_rows":task_event_storage_maintenance::pending_rows(&db)?})
            );
        }
        "prune" => {
            let before: i64 = argument("--before-ms")
                .context("before_ms_required")?
                .parse()?;
            let mut total = 0;
            loop {
                let rows = task_event_storage_maintenance::prune_hot_batch(&mut db, before, batch)?;
                total += rows;
                if rows == 0 {
                    break;
                }
                println!("{}", json!({"ok":true,"action":action,"total_rows":total}));
                std::io::stdout().flush()?;
            }
            checkpoint(&db)?;
            println!(
                "{}",
                json!({"ok":true,"action":action,"total_rows":total,"complete":true})
            );
        }
        "verify" => {
            // One read transaction makes all per-table fingerprints a consistent snapshot.
            db.execute_batch("BEGIN")?;
            let mut output = serde_json::Map::new();
            for table in [
                "task_event_archive",
                "task_event_artifacts",
                "task_event_snapshots",
                "tasks",
                "memories",
                "long_term_memories",
                "user_preferences",
                "auth_keys",
                "credential_bindings",
                "principals",
            ] {
                output.insert(table.into(), fingerprint(&db, table)?);
            }
            db.execute_batch("COMMIT")?;
            println!("{}", json!({"ok":true,"fingerprints":output}));
        }
        "vacuum" => {
            let destination = PathBuf::from(argument("--output").context("output_required")?);
            ensure!(!destination.exists(), "vacuum_output_already_exists");
            checkpoint(&db)?;
            db.execute(
                "VACUUM INTO ?1",
                [destination.to_str().context("output_path_invalid")?],
            )?;
            println!("{}", json!({"ok":true,"action":action}));
        }
        _ => anyhow::bail!("action_unsupported"),
    }
    Ok(())
}

fn fingerprint(db: &Connection, table: &str) -> anyhow::Result<Value> {
    let mut columns = db
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(!columns.is_empty(), "fingerprint_table_missing:{table}");
    let has_version = columns.iter().any(|(c, _)| c == "storage_version");
    let has_blob = columns.iter().any(|(c, _)| c == "storage_blob");
    columns.retain(|(c, _)| c != "storage_version" && c != "storage_blob");
    let quote = |c: &str| format!("\"{}\"", c.replace('"', "\"\""));
    let fields = columns
        .iter()
        .map(|(c, _)| quote(c))
        .collect::<Vec<_>>()
        .join(",");
    let mut primary = columns.iter().filter(|(_, p)| *p > 0).collect::<Vec<_>>();
    primary.sort_by_key(|(_, p)| *p);
    let order = if primary.is_empty() {
        "rowid".into()
    } else {
        primary
            .iter()
            .map(|(c, _)| quote(c))
            .collect::<Vec<_>>()
            .join(",")
    };
    let payload = task_event_storage_maintenance::DATASETS
        .iter()
        .find(|(t, _)| *t == table)
        .and_then(|(_, field)| columns.iter().position(|(c, _)| c == field));
    let version = if has_version { "storage_version" } else { "0" };
    let blob = if has_blob { "storage_blob" } else { "NULL" };
    let mut statement = db.prepare(&format!(
        "SELECT {fields},{version},{blob} FROM {table} ORDER BY {order}"
    ))?;
    let mut rows = statement.query([])?;
    let mut hash = Sha256::new();
    let mut count = 0_u64;
    let mut bytes = 0_u64;
    while let Some(row) = rows.next()? {
        hash.update(b"row\0");
        for n in 0..columns.len() {
            let decoded;
            let value = if Some(n) == payload {
                decoded = event_json_chunks::decode(
                    db,
                    row.get_ref(n)?.as_str()?,
                    row.get(columns.len())?,
                    row.get::<_, Option<Vec<u8>>>(columns.len() + 1)?.as_deref(),
                )?;
                ValueRef::Text(decoded.as_bytes())
            } else {
                row.get_ref(n)?
            };
            match value {
                ValueRef::Null => hash.update([0]),
                ValueRef::Integer(v) => {
                    hash.update([1]);
                    hash.update(v.to_le_bytes());
                }
                ValueRef::Real(v) => {
                    hash.update([2]);
                    hash.update(v.to_bits().to_le_bytes());
                }
                ValueRef::Text(v) | ValueRef::Blob(v) => {
                    hash.update([if matches!(value, ValueRef::Text(_)) {
                        3
                    } else {
                        4
                    }]);
                    hash.update((v.len() as u64).to_le_bytes());
                    hash.update(v);
                    bytes += v.len() as u64;
                }
            }
        }
        count += 1;
    }
    Ok(json!({"rows":count,"logical_bytes":bytes,"sha256":format!("{:x}",hash.finalize())}))
}

fn checkpoint(db: &Connection) -> anyhow::Result<()> {
    let (busy, log, written): (i64, i64, i64) =
        db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    ensure!(
        busy == 0 && log == written && log <= 0,
        "event_storage_checkpoint_busy"
    );
    Ok(())
}

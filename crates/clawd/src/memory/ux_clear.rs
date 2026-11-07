use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) fn purge_private_clear_state(
    db: &Connection,
    principal_id: &str,
    include_derived: bool,
    now_ts: i64,
) -> anyhow::Result<()> {
    anyhow::ensure!(!db.is_autocommit(), "memory_clear_requires_transaction");
    db.execute(
        "UPDATE memory_revisions SET previous_snapshot_json = '{\"schema_version\":1,\"purged\":true}',
            undo_expires_at_ts = NULL
         WHERE principal_id = ?1 AND (?2 = 1 OR object_kind = 'recent')",
        params![principal_id,include_derived],
    )?;
    db.execute(
        "UPDATE memory_privacy_purge_queue SET status = 'purged', completed_at_ts = ?3
         WHERE principal_id = ?1 AND status = 'grace' AND (?2 = 1 OR object_kind = 'recent')",
        params![principal_id, include_derived, now_ts],
    )?;
    // The clear removes every owned transcript row in both modes. Refresh only
    // source usage; never create a missing quota or reset policy/spent costs.
    db.execute(
        "UPDATE memory_principal_quotas SET used_rows = 0, used_bytes = 0, updated_at_ts = ?2
         WHERE principal_id = ?1",
        params![principal_id, now_ts],
    )?;
    purge_import_sessions(db, principal_id, include_derived)?;
    crate::memory::import_upload::purge_for_clear(db, principal_id, include_derived, now_ts)?;
    Ok(())
}

fn purge_import_sessions(
    db: &Connection,
    principal_id: &str,
    include_derived: bool,
) -> anyhow::Result<()> {
    // Each SELECT finishes before its UPDATE; SQLite does not define visibility
    // for a live SELECT cursor when the same connection changes its input table.
    // The existing owner/digest index supports bounded-memory keyset reads. A
    // moved row may be visited once more at its canonical tombstone digest;
    // normalization is idempotent, and every original unvisited key stays put.
    let mut after: Option<String> = None;
    while let Some((import_id, payload, previous_digest)) =
        next_import_session(db, principal_id, after.as_deref())?
    {
        after = Some(previous_digest.clone());
        let previous = serde_json::from_str::<Value>(&payload).ok();
        if !include_derived {
            let staged = match previous.as_ref() {
                Some(value) => crate::memory::import_upload::recent_payload(
                    db,
                    principal_id,
                    value,
                    &previous_digest,
                )?,
                None => None,
            };
            let recent = staged.unwrap_or_else(|| {
                previous
                    .as_ref()
                    .and_then(|value| value.get("items"))
                    .and_then(Value::as_array)
                    .is_some_and(|items| {
                        items
                            .iter()
                            .any(|item| item.get("kind").and_then(Value::as_str) == Some("recent"))
                    })
            });
            if !recent {
                continue;
            }
        }
        let original_digest = if previous
            .as_ref()
            .is_some_and(|value| value.get("purged").and_then(Value::as_bool) == Some(true))
        {
            previous
                .as_ref()
                .and_then(|value| value.get("original_payload_digest"))
                .and_then(Value::as_str)
                .filter(|value| is_digest(value))
                .map(str::to_string)
        } else {
            is_digest(&previous_digest).then_some(previous_digest)
        };
        // Bind the redacted payload to its own digest, retaining only the old
        // checksum for audit. This frees the unique original payload binding so
        // an explicit fresh upload gets a new ID; old confirmations stay invalid.
        let tombstone = json!({"schema_version":1,"purged":true,
            "import_id":&import_id,"original_payload_digest":original_digest})
        .to_string();
        let digest = format!("sha256:{:x}", Sha256::digest(tombstone.as_bytes()));
        db.execute(
            "UPDATE memory_import_sessions SET preview_json = ?1, payload_digest = ?2,
                status = CASE WHEN status = 'preview' THEN 'cancelled' ELSE status END
             WHERE import_id = ?3 AND principal_id = ?4",
            params![tombstone, digest, import_id, principal_id],
        )?;
    }
    Ok(())
}

fn next_import_session(
    db: &Connection,
    principal_id: &str,
    after: Option<&str>,
) -> rusqlite::Result<Option<(String, String, String)>> {
    let decode = |row: &rusqlite::Row<'_>| Ok((row.get(0)?, row.get(1)?, row.get(2)?));
    match after {
        Some(after) => db
            .query_row(
                "SELECT import_id, preview_json, payload_digest FROM memory_import_sessions
             WHERE principal_id = ?1 AND payload_digest > ?2 ORDER BY payload_digest LIMIT 1",
                params![principal_id, after],
                decode,
            )
            .optional(),
        None => db
            .query_row(
                "SELECT import_id, preview_json, payload_digest FROM memory_import_sessions
             WHERE principal_id = ?1 ORDER BY payload_digest LIMIT 1",
                [principal_id],
                decode,
            )
            .optional(),
    }
}

fn is_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
#[path = "ux_clear_cursor_tests.rs"]
mod cursor_tests;

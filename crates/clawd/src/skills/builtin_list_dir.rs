use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::Path;
use std::time::UNIX_EPOCH;

#[derive(Debug)]
pub(super) enum ListDirError {
    Io(io::Error),
    CursorOutOfRange { cursor: usize, total: usize },
}

impl From<io::Error> for ListDirError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(super) fn list_directory_page(
    requested_path: &str,
    path: &Path,
    cursor: usize,
    max_entries: usize,
) -> Result<Value, ListDirError> {
    let mut entries = Vec::new();
    for result in fs::read_dir(path)? {
        let entry = result?;
        let metadata = fs::symlink_metadata(entry.path())?;
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            "symlink"
        } else if file_type.is_dir() {
            "directory"
        } else if file_type.is_file() {
            "file"
        } else {
            "other"
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let modified_ts = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());
        entries.push(json!({
            "name": name,
            "path": entry.path().display().to_string(),
            "kind": kind,
            "is_symlink": file_type.is_symlink(),
            "size_bytes": file_type.is_file().then_some(metadata.len()),
            "modified_ts": modified_ts,
        }));
    }
    entries.sort_by(|left, right| {
        left.get("name")
            .and_then(Value::as_str)
            .cmp(&right.get("name").and_then(Value::as_str))
    });

    let total = entries.len();
    if cursor > total {
        return Err(ListDirError::CursorOutOfRange { cursor, total });
    }
    let end = cursor.saturating_add(max_entries).min(total);
    let page = entries[cursor..end].to_vec();
    let complete = end >= total;
    let names = page
        .iter()
        .filter_map(|entry| entry.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let snapshot_hash = directory_snapshot_hash(&entries);
    let canonical_path = path.canonicalize()?.display().to_string();
    let continuation = (!complete).then(|| {
        json!({
            "kind": "cursor",
            "capability": "list_dir",
            "args": {
                "path": requested_path,
                "cursor": end,
                "max_entries": max_entries,
            },
            "snapshot_hash": snapshot_hash,
        })
    });

    Ok(json!({
        "schema_version": 1,
        "source": "list_dir",
        "status": "ok",
        "requested_path": requested_path,
        "path": canonical_path,
        "canonical_path": canonical_path,
        "entries": page,
        "names": names,
        "count": end.saturating_sub(cursor),
        "total_count": total,
        "cursor": cursor,
        "next_cursor": (!complete).then_some(end),
        "snapshot_hash": snapshot_hash,
        "complete": complete,
        "continuation": continuation,
        "artifacts": [],
    }))
}

fn directory_snapshot_hash(entries: &[Value]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.to_string().as_bytes());
        hasher.update([b'\n']);
    }
    format!("{:x}", hasher.finalize())
}

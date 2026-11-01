use rustix::fs::FileType;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io;
use std::path::Path;

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
    workspace_root: Option<&Path>,
    host_filesystem: &crate::runtime::host_filesystem::HostFilesystemPolicy,
    cursor: usize,
    max_entries: usize,
) -> Result<Value, ListDirError> {
    let directory = host_filesystem.open_directory(workspace_root, path)?;
    let mut entries = Vec::new();
    for entry in directory.entries()? {
        let kind = match entry.file_type {
            FileType::Symlink => "symlink",
            FileType::Directory => "directory",
            FileType::RegularFile => "file",
            _ => "other",
        };
        let name = entry.name.to_string_lossy().into_owned();
        entries.push(json!({
            "name": name,
            "path": path.join(&entry.name).display().to_string(),
            "kind": kind,
            "is_symlink": entry.file_type == FileType::Symlink,
            "size_bytes": entry.size_bytes,
            "modified_ts": entry.modified_ts,
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
    let canonical_path = directory.canonical_path().display().to_string();
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

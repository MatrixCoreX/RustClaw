use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const DEFAULT_READ_BYTES: usize = 64 * 1024;
const MIN_READ_BYTES: usize = 256;
const MAX_READ_BYTES: usize = 1024 * 1024;

pub(super) fn read_file_page(
    requested_path: &str,
    path: &Path,
    workspace_root: Option<&Path>,
    start_byte: u64,
    requested_max_bytes: Option<usize>,
) -> io::Result<Value> {
    let mut file = match workspace_root {
        Some(root) => crate::secure_workspace_fs::open_workspace_file(root, path)?,
        None => File::open(path)?,
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "read_file_target_not_regular_file",
        ));
    }
    let total_size = metadata.len();
    if start_byte > total_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("start_byte {start_byte} exceeds file size {total_size}"),
        ));
    }

    let max_bytes = requested_max_bytes
        .unwrap_or(DEFAULT_READ_BYTES)
        .clamp(MIN_READ_BYTES, MAX_READ_BYTES);
    let sha256 = file_sha256(&mut file)?;
    file.seek(SeekFrom::Start(start_byte))?;
    let remaining = total_size.saturating_sub(start_byte);
    let read_budget = remaining.min(max_bytes as u64) as usize;
    let mut bytes = vec![0_u8; read_budget];
    file.read_exact(&mut bytes)?;

    let end_byte = start_byte.saturating_add(bytes.len() as u64);
    let complete = end_byte >= total_size;
    let (content, encoding, lossy) = match String::from_utf8(bytes.clone()) {
        Ok(content) => (content, "utf-8", false),
        Err(_) => (
            String::from_utf8_lossy(&bytes).into_owned(),
            "utf-8-lossy",
            true,
        ),
    };
    let canonical_path = path.canonicalize()?.display().to_string();
    let continuation = (!complete).then(|| {
        json!({
            "kind": "byte_range",
            "capability": "read_file",
            "args": {
                "path": requested_path,
                "start_byte": end_byte,
                "max_bytes": max_bytes,
            }
        })
    });

    Ok(json!({
        "schema_version": 1,
        "source": "read_file",
        "status": "ok",
        "requested_path": requested_path,
        "path": canonical_path,
        "canonical_path": canonical_path,
        "start_byte": start_byte,
        "end_byte": end_byte,
        "returned_bytes": bytes.len(),
        "size_bytes": total_size,
        "sha256": sha256,
        "encoding": encoding,
        "lossy": lossy,
        "content": content,
        "complete": complete,
        "continuation": continuation,
        "artifacts": [],
    }))
}

fn file_sha256(file: &mut File) -> io::Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

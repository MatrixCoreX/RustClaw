use std::io::{self, Write};

use serde_json::{json, Value};

use super::*;
use crate::memory::ux::MemoryImportPreview;

pub(super) const STORAGE: &str = "memory_import_upload_v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Descriptor {
    pub(super) schema_version: u8,
    pub(super) storage: String,
    pub(super) upload_id: String,
    pub(super) accepted_items: usize,
    pub(super) skipped_items: usize,
    pub(super) duplicate_items: usize,
    pub(super) max_item_bytes: u64,
    pub(super) has_recent: bool,
}

impl Descriptor {
    pub(super) fn decode(value: &Value) -> anyhow::Result<Self> {
        let value: Self =
            serde_json::from_value(value.clone()).map_err(|_| UploadError::Corrupt)?;
        anyhow::ensure!(
            value.schema_version == 2
                && value.storage == STORAGE
                && !value.upload_id.is_empty()
                && value.max_item_bytes <= i64::MAX as u64
                && (value.accepted_items == 0 || value.max_item_bytes > 0),
            UploadError::Corrupt
        );
        Ok(value)
    }
    pub(super) fn preview(&self, id: String, digest: String) -> MemoryImportPreview {
        MemoryImportPreview {
            schema_version: 1,
            import_id: id,
            payload_digest: digest,
            accepted_items: self.accepted_items,
            skipped_items: self.skipped_items,
            duplicate_items: self.duplicate_items,
            trust_tier: "imported_legacy".into(),
            scope_kind: "principal".into(),
        }
    }
}

/// Distinguish the versioned storage envelope, not human-visible prose.
pub(in crate::memory) fn is_staged(value: &Value) -> bool {
    value.get("schema_version").and_then(Value::as_u64) == Some(2)
        && value.get("storage").and_then(Value::as_str) == Some(STORAGE)
}

pub(super) fn each_item(
    db: &Connection,
    descriptor: &Descriptor,
    mut consume: impl FnMut(Value) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut after = -1i64;
    let mut count = 0usize;
    loop {
        // Do not allocate an oversized damaged DB value before checking its size.
        let row=db.query_row("SELECT item_index,CASE WHEN LENGTH(CAST(item_json AS BLOB))<=?3 THEN item_json END
            FROM memory_import_upload_items WHERE upload_id=?1 AND item_index>?2 ORDER BY item_index LIMIT 1",
            params![descriptor.upload_id,after,descriptor.max_item_bytes],
            |row|Ok((row.get::<_,i64>(0)?,row.get::<_,Option<String>>(1)?))).optional()?;
        let Some((index, json)) = row else { break };
        anyhow::ensure!(index >= 0 && index > after, UploadError::Corrupt);
        let json = json.ok_or(UploadError::Corrupt)?;
        let item = serde_json::from_str(&json).map_err(|_| UploadError::Corrupt)?;
        consume(item)?;
        count = count.checked_add(1).ok_or(UploadError::Corrupt)?;
        anyhow::ensure!(count <= descriptor.accepted_items, UploadError::Corrupt);
        after = index;
    }
    anyhow::ensure!(count == descriptor.accepted_items, UploadError::Corrupt);
    Ok(())
}

struct HashWriter<'a>(&'a mut Sha256);
impl Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct CountedHashWriter<'a>(&'a mut Sha256, &'a mut u64);
impl Write for CountedHashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        *self.1 += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Reproduce the existing logical schema1 payload's serde JSON key order and
/// item serialization without materializing its full items array.
pub(super) fn payload_digest(db: &Connection, descriptor: &Descriptor) -> anyhow::Result<String> {
    let metadata = json!({"schema_version":1,"items":null,
        "skipped_items":descriptor.skipped_items,"duplicate_items":descriptor.duplicate_items,
        "trust_tier":"imported_legacy","scope_kind":"principal"});
    let fields = metadata.as_object().ok_or(UploadError::Corrupt)?;
    let mut hasher = Sha256::new();
    let mut observed_max = 0;
    let mut observed_recent = false;
    hasher.update(b"{");
    for (index, (key, value)) in fields.iter().enumerate() {
        if index > 0 {
            hasher.update(b",");
        }
        serde_json::to_writer(HashWriter(&mut hasher), key)?;
        hasher.update(b":");
        if key == "items" {
            hasher.update(b"[");
            let mut first = true;
            each_item(db, descriptor, |item| {
                if !first {
                    hasher.update(b",");
                }
                first = false;
                let mut item_bytes = 0;
                serde_json::to_writer(CountedHashWriter(&mut hasher, &mut item_bytes), &item)?;
                observed_max = observed_max.max(item_bytes);
                observed_recent |= item["kind"].as_str() == Some("recent");
                Ok(())
            })?;
            hasher.update(b"]");
        } else {
            serde_json::to_writer(HashWriter(&mut hasher), value)?;
        }
    }
    hasher.update(b"}");
    anyhow::ensure!(
        observed_max == descriptor.max_item_bytes && observed_recent == descriptor.has_recent,
        UploadError::Corrupt
    );
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

use super::*;
use serde_json::json;

pub(super) fn item(content: &str) -> MemoryListItem {
    serde_json::from_value(json!({
        "id":"source_record", "revision":1, "kind":"fact", "scope_kind":"principal",
        "origin":"explicit", "status":"active", "content":content, "source":"user",
        "evidence_available":false, "trust_tier":"user_asserted", "updated_at_ts":1,
        "expires_at_ts":null, "supersedes_memory_id":null, "last_recalled_at_ts":null,
        "freshness":"fresh"
    }))
    .unwrap()
}
pub(super) fn export(items: Vec<MemoryListItem>) -> Vec<u8> {
    let checksum = digest(&serde_json::to_vec(&items).unwrap());
    serde_json::to_vec(&crate::memory::ux::MemoryExport {
        schema_version: 1,
        exported_at_ts: 100,
        scope_kind: "principal".into(),
        items,
        checksum,
    })
    .unwrap()
}
pub(super) fn rejected(bytes: &[u8], budget: u64, expected: UploadError) {
    let result = parse_export(bytes, budget, |_| Ok(()));
    crate::memory::import_upload::test_fixtures::error(result, expected);
}
pub(super) struct OneByte(pub(super) io::Cursor<Vec<u8>>);
impl Read for OneByte {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let count = out.len().min(1);
        self.0.read(&mut out[..count])
    }
}
pub(super) struct BrokenReader {
    pub(super) bytes: io::Cursor<Vec<u8>>,
    pub(super) fail_at: u64,
}
impl Read for BrokenReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.bytes.position() >= self.fail_at {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fixture_read_failure",
            ));
        }
        let count = out
            .len()
            .min((self.fail_at - self.bytes.position()) as usize);
        self.bytes.read(&mut out[..count])
    }
}

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) struct ResultPage<T> {
    pub(super) items: Vec<T>,
    pub(super) metadata: Value,
    pub(super) returned_count: usize,
    pub(super) total_count: usize,
    pub(super) has_more: bool,
    pub(super) snapshot_sha256: String,
}

pub(super) fn cursor_from_args(obj: &serde_json::Map<String, Value>) -> usize {
    obj.get("cursor")
        .or_else(|| obj.get("offset"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(usize::MAX as u64) as usize
}

pub(super) fn paginate<T>(
    values: &[T],
    cursor: usize,
    limit: usize,
    scan_truncated: bool,
) -> ResultPage<T>
where
    T: Clone + Serialize,
{
    let total_count = values.len();
    let page_start = cursor.min(total_count);
    let page_end = page_start.saturating_add(limit).min(total_count);
    let items = values[page_start..page_end].to_vec();
    let returned_count = items.len();
    let has_more = page_end < total_count || scan_truncated;
    let snapshot_sha256 = snapshot_sha256(values);
    let next_cursor = (page_end < total_count).then_some(page_end);
    let previous_cursor = (page_start > 0).then_some(page_start.saturating_sub(limit));
    let metadata = json!({
        "cursor": page_start,
        "limit": limit,
        "returned_count": returned_count,
        "total_count": total_count,
        "has_more": has_more,
        "next_cursor": next_cursor,
        "previous_cursor": previous_cursor,
        "scan_truncated": scan_truncated,
        "snapshot_sha256": snapshot_sha256,
    });
    ResultPage {
        items,
        metadata,
        returned_count,
        total_count,
        has_more,
        snapshot_sha256,
    }
}

fn snapshot_sha256<T: Serialize>(values: &[T]) -> String {
    let encoded = serde_json::to_vec(values).unwrap_or_default();
    format!("{:x}", Sha256::digest(encoded))
}

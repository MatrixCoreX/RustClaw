//! Streaming schema1 decoding. The consumer may persist only provisional staging:
//! callbacks run before the final checksum/EOF check and never authorize publication.
use std::io::{self, Read, Write};

use serde::de::DeserializeSeed;

use super::*;
use crate::memory::ux::MemoryListItem;

mod budget;
mod items;
mod root;

use budget::{BoundedReader, Budget};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedExport {
    pub(crate) schema_version: u32,
    pub(crate) exported_at_ts: i64,
    pub(crate) scope_kind: String,
    pub(crate) item_count: u64,
    pub(crate) checksum: String,
}

struct Context<'a> {
    budget: &'a Budget,
    consumer: &'a mut dyn FnMut(MemoryListItem) -> anyhow::Result<()>,
    consumer_error: Option<anyhow::Error>,
    fault: Option<UploadError>,
    item_hash: Sha256,
    count: u64,
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

/// The caller supplies an advertised host-derived per-value byte budget.
/// There is no item-count or model-context limit in this decoder. Record-level
/// domain filtering remains a separate shared importer responsibility.
pub(crate) fn parse_export<R: Read>(
    reader: R,
    value_budget_bytes: u64,
    mut consumer: impl FnMut(MemoryListItem) -> anyhow::Result<()>,
) -> anyhow::Result<ParsedExport> {
    anyhow::ensure!(
        value_budget_bytes > 0 && value_budget_bytes <= i64::MAX as u64,
        UploadError::InvalidMetadata
    );
    let budget = Budget::new(value_budget_bytes);
    let mut decoder = serde_json::Deserializer::from_reader(BoundedReader::new(reader, &budget));
    let mut context = Context {
        budget: &budget,
        consumer: &mut consumer,
        consumer_error: None,
        fault: None,
        item_hash: Sha256::new(),
        count: 0,
    };
    let result = root::ExportSeed(&mut context).deserialize(&mut decoder);
    let result = result.and_then(|header| {
        // Trailing whitespace need not fit one value budget, but a second JSON
        // document is still rejected. The durable reader must actually see EOF.
        budget.disable_value_limit();
        decoder.end()?;
        Ok(header)
    });
    if let Some(error) = context.consumer_error {
        return Err(error);
    }
    if let Some(error) = budget.take_io_error() {
        return Err(error.into());
    }
    if budget.exhausted() {
        return Err(UploadError::ValueTooLarge.into());
    }
    if let Some(error) = context.fault {
        return Err(error.into());
    }
    let header = result.map_err(|_| UploadError::InvalidJson)?;
    let checksum = format!("sha256:{:x}", context.item_hash.finalize());
    anyhow::ensure!(
        valid_digest(&header.checksum) && header.checksum == checksum,
        UploadError::ExportChecksumMismatch
    );
    Ok(ParsedExport {
        schema_version: header.schema_version,
        exported_at_ts: header.exported_at_ts,
        scope_kind: header.scope_kind,
        item_count: context.count,
        checksum,
    })
}

#[cfg(test)]
mod tests;

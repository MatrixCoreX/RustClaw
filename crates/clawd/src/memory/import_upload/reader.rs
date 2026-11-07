use std::io::{self, Read};

use super::*;

/// Reads one bounded durable chunk at a time. The parser's own per-item budget
/// can wrap this reader without buffering the complete export in memory.
pub(crate) struct ChunkReader<'a> {
    db: &'a Connection,
    actor: &'a AuthIdentity,
    lease: &'a ParseLease,
    position: u64,
    buffer: Vec<u8>,
    cursor: usize,
    hasher: Sha256,
    verified_eof: bool,
}

impl<'a> ChunkReader<'a> {
    pub(crate) fn new(
        db: &'a Connection,
        actor: &'a AuthIdentity,
        lease: &'a ParseLease,
    ) -> anyhow::Result<Self> {
        checked_read(db, actor, |db| {
            lease::require_lease(db, actor, lease)?;
            Ok(())
        })?;
        Ok(Self {
            db,
            actor,
            lease,
            position: 0,
            buffer: Vec::new(),
            cursor: 0,
            hasher: Sha256::new(),
            verified_eof: false,
        })
    }

    pub(crate) fn verify_complete(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.position == self.lease.total_bytes && self.verified_eof,
            UploadError::Incomplete
        );
        Ok(())
    }

    fn next_chunk(&mut self) -> anyhow::Result<()> {
        self.buffer = checked_read(self.db, self.actor, |db| {
            lease::require_lease(db, self.actor, self.lease)?;
            let (bytes, expected, count) = db
                .query_row(
                    "SELECT content,content_digest,byte_count FROM memory_import_upload_chunks
                 WHERE upload_id=?1 AND byte_offset=?2",
                    params![self.lease.upload_id, self.position],
                    |row| {
                        Ok((
                            row.get::<_, Vec<u8>>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, u64>(2)?,
                        ))
                    },
                )
                .optional()?
                .ok_or(UploadError::Corrupt)?;
            anyhow::ensure!(
                count > 0
                    && count <= self.lease.chunk_bytes
                    && count == bytes.len() as u64
                    && self
                        .position
                        .checked_add(count)
                        .is_some_and(|end| end <= self.lease.total_bytes)
                    && digest(&bytes) == expected,
                UploadError::Corrupt
            );
            Ok(bytes)
        })?;
        self.cursor = 0;
        Ok(())
    }
}

impl Read for ChunkReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.position == self.lease.total_bytes {
            checked_read(self.db, self.actor, |db| {
                lease::require_lease(db, self.actor, self.lease)?;
                anyhow::ensure!(
                    format!("sha256:{:x}", self.hasher.clone().finalize())
                        == self.lease.file_digest,
                    UploadError::FileDigestMismatch
                );
                Ok(())
            })
            .map_err(read_failure)?;
            self.verified_eof = true;
            return Ok(0);
        }
        if self.cursor == self.buffer.len() {
            self.next_chunk().map_err(read_failure)?;
        }
        let count = output.len().min(self.buffer.len() - self.cursor);
        output[..count].copy_from_slice(&self.buffer[self.cursor..self.cursor + count]);
        self.cursor += count;
        self.position += count as u64;
        self.hasher.update(&output[..count]);
        Ok(count)
    }
}

#[derive(Debug)]
struct ReadFailure(anyhow::Error);

impl std::fmt::Display for ReadFailure {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, out)
    }
}
impl std::error::Error for ReadFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}
fn read_failure(error: anyhow::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, ReadFailure(error))
}

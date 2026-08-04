use std::collections::BTreeMap;
use std::io::Write;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::{
    SkillProgressFrame, SkillProgressKind, SkillSdkResult, SKILL_PROGRESS_FRAME_RECORD_TYPE,
    SKILL_PROGRESS_FRAME_SCHEMA_VERSION,
};

/// Writes ordered, machine-only progress records without changing the final
/// one-line response contract.
pub struct SkillProgressEmitter<'a, W: Write> {
    writer: &'a mut W,
    request_id: String,
    sequence: u64,
    last_emitted_at: Option<Instant>,
}

impl<'a, W: Write> SkillProgressEmitter<'a, W> {
    pub fn new(writer: &'a mut W, request_id: impl Into<String>) -> Self {
        Self {
            writer,
            request_id: request_id.into(),
            sequence: 0,
            last_emitted_at: None,
        }
    }

    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn emit_progress(
        &mut self,
        detail_key: impl Into<String>,
        params: BTreeMap<String, Value>,
        current: Option<u64>,
        total: Option<u64>,
    ) -> SkillSdkResult<()> {
        self.emit(
            SkillProgressKind::Progress,
            detail_key,
            params,
            current,
            total,
        )
    }

    pub fn emit_progress_throttled(
        &mut self,
        detail_key: impl Into<String>,
        params: BTreeMap<String, Value>,
        current: Option<u64>,
        total: Option<u64>,
        minimum_interval: Duration,
    ) -> SkillSdkResult<bool> {
        if self
            .last_emitted_at
            .is_some_and(|last| last.elapsed() < minimum_interval)
        {
            return Ok(false);
        }
        self.emit_progress(detail_key, params, current, total)?;
        Ok(true)
    }

    fn emit(
        &mut self,
        kind: SkillProgressKind,
        detail_key: impl Into<String>,
        params: BTreeMap<String, Value>,
        current: Option<u64>,
        total: Option<u64>,
    ) -> SkillSdkResult<()> {
        self.sequence = self.sequence.saturating_add(1);
        let frame = SkillProgressFrame {
            schema_version: SKILL_PROGRESS_FRAME_SCHEMA_VERSION,
            record_type: SKILL_PROGRESS_FRAME_RECORD_TYPE.to_string(),
            request_id: self.request_id.clone(),
            sequence: self.sequence,
            kind,
            detail_key: detail_key.into(),
            params,
            current,
            total,
            reference: None,
        };
        let line = frame.to_line()?;
        writeln!(self.writer, "{line}")?;
        self.writer.flush()?;
        self.last_emitted_at = Some(Instant::now());
        Ok(())
    }
}

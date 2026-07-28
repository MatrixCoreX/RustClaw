use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{BackendProvenance, CancellationToken, CaseMode, Completeness};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPatternKind {
    Literal,
    Regex,
}

#[derive(Debug, Clone)]
pub struct RipgrepTextRequest {
    pub workspace_root: PathBuf,
    pub root: PathBuf,
    pub paths: Vec<PathBuf>,
    pub query: String,
    pub pattern_kind: TextPatternKind,
    pub case_mode: CaseMode,
    pub multiline: bool,
    pub max_matches: usize,
    pub max_output_bytes: usize,
    pub max_line_chars: usize,
    pub deadline: Option<std::time::Duration>,
    pub cancellation: Option<CancellationToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RipgrepTextMatch {
    pub path: String,
    pub line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub text: String,
    pub matched_text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RipgrepTextReport {
    pub matches: Vec<RipgrepTextMatch>,
    pub completeness: Completeness,
    pub backend: BackendProvenance,
    pub cancelled: bool,
    pub output_truncated: bool,
}

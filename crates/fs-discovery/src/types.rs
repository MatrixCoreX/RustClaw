use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_HARD_ENTRY_LIMIT: usize = 500_000;
pub const DEFAULT_MATCH_SNAPSHOT_LIMIT: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    Complete,
    PartialDeadline,
    PartialHardLimit,
    PartialPermission,
    StaleSnapshot,
}

impl Completeness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::PartialDeadline => "partial_deadline",
            Self::PartialHardLimit => "partial_hard_limit",
            Self::PartialPermission => "partial_permission",
            Self::StaleSnapshot => "stale_snapshot",
        }
    }

    pub fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Directory,
    Other,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "dir",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TargetKind {
    #[default]
    Any,
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MatchMode {
    Exact,
    StartsWith,
    EndsWith,
    #[default]
    Contains,
    Fuzzy,
    Glob,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CaseMode {
    Sensitive,
    Insensitive,
    #[default]
    Smart,
}

#[derive(Debug, Clone)]
pub struct DiscoverySelector {
    pub patterns: Vec<String>,
    pub globs: Vec<String>,
    pub extensions: Vec<String>,
    pub target_kind: TargetKind,
    pub match_mode: MatchMode,
    pub case_mode: CaseMode,
}

impl Default for DiscoverySelector {
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
            globs: Vec::new(),
            extensions: Vec::new(),
            target_kind: TargetKind::Any,
            match_mode: MatchMode::Contains,
            // Preserve the established locator behavior. Planner-facing
            // actions can opt into smart case explicitly.
            case_mode: CaseMode::Insensitive,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BackendPreference {
    #[default]
    Auto,
    Rust,
    Ripgrep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryBackend {
    Rust,
    Ripgrep,
}

impl DiscoveryBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Ripgrep => "ripgrep",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendProvenance {
    pub backend: DiscoveryBackend,
    pub version: Option<String>,
    pub fallback_reason: Option<String>,
    pub elapsed_ms: u64,
}

impl BackendProvenance {
    pub(crate) fn rust(elapsed_ms: u64, fallback_reason: Option<String>) -> Self {
        Self {
            backend: DiscoveryBackend::Rust,
            version: None,
            fallback_reason,
            elapsed_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RipgrepStatus {
    pub available: bool,
    pub executable: Option<String>,
    pub version: Option<String>,
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiscoveryPolicy {
    pub include_hidden: bool,
    pub respect_ignore: bool,
}

impl Default for DiscoveryPolicy {
    fn default() -> Self {
        Self {
            include_hidden: false,
            respect_ignore: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveryBudget {
    pub max_depth: Option<usize>,
    /// Number of traversal entries already consumed by a prior resumable shard.
    pub start_after_entries: usize,
    pub hard_entry_limit: usize,
    pub match_snapshot_limit: usize,
    pub deadline: Option<Duration>,
    pub cancellation: Option<CancellationToken>,
}

impl Default for DiscoveryBudget {
    fn default() -> Self {
        Self {
            max_depth: None,
            start_after_entries: 0,
            hard_entry_limit: DEFAULT_HARD_ENTRY_LIMIT,
            match_snapshot_limit: DEFAULT_MATCH_SNAPSHOT_LIMIT,
            deadline: None,
            cancellation: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveryRequest {
    pub workspace_root: PathBuf,
    pub root: PathBuf,
    pub selector: DiscoverySelector,
    pub policy: DiscoveryPolicy,
    pub budget: DiscoveryBudget,
    pub backend: BackendPreference,
}

impl DiscoveryRequest {
    pub fn new(workspace_root: impl Into<PathBuf>, root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            root: root.into(),
            selector: DiscoverySelector::default(),
            policy: DiscoveryPolicy::default(),
            budget: DiscoveryBudget::default(),
            backend: BackendPreference::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscoveryEntry {
    #[serde(skip)]
    pub path: PathBuf,
    pub relative_path: String,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryReport {
    pub workspace_root: PathBuf,
    pub root: PathBuf,
    pub entries: Vec<DiscoveryEntry>,
    pub completeness: Completeness,
    pub visited_files: usize,
    pub visited_directories: usize,
    pub skipped_ignored: usize,
    pub skipped_hidden: usize,
    pub skipped_symlinks: usize,
    pub permission_denied: usize,
    pub skipped_counts_complete: bool,
    pub cancelled: bool,
    pub traversal_start: usize,
    pub traversal_next: Option<usize>,
    pub backend: BackendProvenance,
}

impl DiscoveryReport {
    pub fn visited_entries(&self) -> usize {
        self.visited_files.saturating_add(self.visited_directories)
    }

    pub fn scan_truncated(&self) -> bool {
        !self.completeness.is_complete()
    }
}

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("workspace resolution failed: {0}")]
    WorkspaceResolution(String),
    #[error("path resolution failed: {0}")]
    PathResolution(String),
    #[error("path with '..' is not allowed")]
    ParentTraversal,
    #[error("path is outside workspace")]
    OutsideWorkspace,
    #[error("search root is not a file or directory")]
    UnsupportedRoot,
    #[error("ripgrep backend unavailable: {0}")]
    BackendUnavailable(String),
    #[error("ripgrep backend failed: {0}")]
    BackendFailed(String),
}

impl DiscoveryError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::WorkspaceResolution(_) => "workspace_resolution_failed",
            Self::PathResolution(_) => "path_resolution_failed",
            Self::ParentTraversal => "parent_traversal_rejected",
            Self::OutsideWorkspace => "outside_workspace",
            Self::UnsupportedRoot => "unsupported_root",
            Self::BackendUnavailable(_) => "backend_unavailable",
            Self::BackendFailed(_) => "backend_failed",
        }
    }
}

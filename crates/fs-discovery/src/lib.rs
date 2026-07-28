mod backend;
mod matcher;
mod ripgrep;
mod ripgrep_process;
mod root;
mod text_backend;
mod text_types;
mod types;
mod walker;

pub use backend::{discover, ripgrep_status};
pub use root::{relative_path, resolve_root};
pub use text_backend::ripgrep_text_search;
pub use text_types::{RipgrepTextMatch, RipgrepTextReport, RipgrepTextRequest, TextPatternKind};
pub use types::{
    BackendPreference, BackendProvenance, CancellationToken, CaseMode, Completeness,
    DiscoveryBackend, DiscoveryBudget, DiscoveryEntry, DiscoveryError, DiscoveryPolicy,
    DiscoveryReport, DiscoveryRequest, DiscoverySelector, EntryKind, MatchMode, RipgrepStatus,
    TargetKind, DEFAULT_HARD_ENTRY_LIMIT, DEFAULT_MATCH_SNAPSHOT_LIMIT,
};
pub use walker::normalize_name;

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;

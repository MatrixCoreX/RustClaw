use std::path::PathBuf;

use crate::AppState;
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeliveryMessageKind {
    Rule1BothRootsMiss,
    Rule2DirNotFound,
    Rule2FileNotFound,
    Rule3ScanTooMany,
    Rule3FileNotFound,
    FilenameNotUnique,
    DirectoryBothRootsMiss,
    DirectoryEntriesTooMany,
    DirectoryMultipleCandidates,
    DirectoryNoFilesInCurrentLevel,
    DirectoryNoSendableFilesInCurrentLevel,
    DirectoryHasChildDirsHint,
}

impl DeliveryMessageKind {
    fn i18n_key(self) -> &'static str {
        match self {
            Self::Rule1BothRootsMiss => "clawd.msg.delivery.rule1_both_roots_miss",
            Self::Rule2DirNotFound => "clawd.msg.delivery.rule2_dir_not_found",
            Self::Rule2FileNotFound => "clawd.msg.delivery.rule2_file_not_found",
            Self::Rule3ScanTooMany => "clawd.msg.delivery.rule3_scan_too_many",
            Self::Rule3FileNotFound => "clawd.msg.delivery.rule3_file_not_found",
            Self::FilenameNotUnique => "clawd.msg.delivery.filename_not_unique",
            Self::DirectoryBothRootsMiss => "clawd.msg.directory.not_found_dual_root",
            Self::DirectoryEntriesTooMany => "clawd.msg.directory.entries_too_many",
            Self::DirectoryMultipleCandidates => "clawd.msg.directory.multiple_candidates",
            Self::DirectoryNoFilesInCurrentLevel => "clawd.msg.directory.no_files_current_level",
            Self::DirectoryNoSendableFilesInCurrentLevel => {
                "clawd.msg.directory.no_sendable_files_current_level"
            }
            Self::DirectoryHasChildDirsHint => "clawd.msg.directory.child_dirs_hint",
        }
    }

    fn machine_default_payload(self) -> String {
        json!({
            "message_key": self.i18n_key(),
        })
        .to_string()
    }
}

pub(super) fn localize_delivery_message_for_request(
    state: &AppState,
    kind: DeliveryMessageKind,
    user_request: &str,
) -> String {
    let default_text =
        crate::i18n_t_with_default(state, kind.i18n_key(), &kind.machine_default_payload());
    crate::i18n_t_for_language_hint_with_default_vars(
        state,
        crate::language_policy::request_language_hint(user_request),
        kind.i18n_key(),
        &default_text,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FileDeliveryLocatorInput {
    ExplicitFilePath {
        file_path: String,
    },
    DirectoryAndFilename {
        directory_path: String,
        file_name: String,
    },
    FilenameOnly {
        file_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FileDeliveryTargetResolution {
    Resolved(PathBuf),
    Candidates(Vec<PathBuf>),
    UserMessage(DeliveryMessageKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DirectoryFileLookupResult {
    Found(PathBuf),
    Candidates(Vec<PathBuf>),
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FilenameScanResult {
    Found(PathBuf),
    Candidates(Vec<PathBuf>),
    NotFound,
    TooManyEntries,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DirectoryLookupInput {
    ExplicitPath { directory_path: String },
    NameHint { directory_hint: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DirectoryLookupResolution {
    Resolved(PathBuf),
    MultipleCandidates(Vec<PathBuf>),
    UserMessage(DeliveryMessageKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DirectoryEntriesListResult {
    FilePaths(Vec<PathBuf>),
    UserMessage(DeliveryMessageKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CurrentLevelDeliveryEntries {
    pub(super) files: Vec<PathBuf>,
    pub(super) has_child_dirs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CurrentLevelDeliveryEntriesResult {
    Ready(CurrentLevelDeliveryEntries),
    UserMessage(DeliveryMessageKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BatchDirectoryDeliveryResolution {
    FileTokens(String),
    UserMessage(String),
}

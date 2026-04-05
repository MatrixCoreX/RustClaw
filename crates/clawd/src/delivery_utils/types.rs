use std::path::PathBuf;

use crate::AppState;

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

    fn default_text(self) -> &'static str {
        match self {
            Self::Rule1BothRootsMiss => "File not found under system root and project root.",
            Self::Rule2DirNotFound => "Directory does not exist. Please provide a correct path.",
            Self::Rule2FileNotFound => "The file was not found in that directory.",
            Self::Rule3ScanTooMany => "Too many files. Please provide an exact path.",
            Self::Rule3FileNotFound => "File not found.",
            Self::FilenameNotUnique => {
                "Multiple files with the same name were found. Please provide an exact path."
            }
            Self::DirectoryBothRootsMiss => {
                "Directory not found under system root and project root."
            }
            Self::DirectoryEntriesTooMany => {
                "Too many files/directories in this directory. Please provide a more specific path or a smaller scope."
            }
            Self::DirectoryMultipleCandidates => {
                "Found multiple possible directories. Please confirm which one:"
            }
            Self::DirectoryNoFilesInCurrentLevel => {
                "No files were found in the current directory level."
            }
            Self::DirectoryNoSendableFilesInCurrentLevel => {
                "No sendable files were found in this directory's current level."
            }
            Self::DirectoryHasChildDirsHint => {
                "There are other subdirectories under this directory. If you want to continue sending, please provide a more precise path."
            }
        }
    }
}

pub(super) fn localize_delivery_message(state: &AppState, kind: DeliveryMessageKind) -> String {
    crate::i18n_t_with_default(state, kind.i18n_key(), kind.default_text())
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
    UserMessage(DeliveryMessageKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DirectoryFileLookupResult {
    Found(PathBuf),
    NotFound,
    Multiple,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FilenameScanResult {
    Found(PathBuf),
    NotFound,
    TooManyEntries,
    Multiple,
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

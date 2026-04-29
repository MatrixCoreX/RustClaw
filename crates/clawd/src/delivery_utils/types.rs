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

    fn default_en(self) -> &'static str {
        match self {
            Self::Rule1BothRootsMiss => "File not found at the provided path.",
            Self::Rule2DirNotFound => "Directory does not exist. Please provide a correct path.",
            Self::Rule2FileNotFound => "The file was not found in that directory.",
            Self::Rule3ScanTooMany => "Too many files. Please provide an exact path.",
            Self::Rule3FileNotFound => "File not found.",
            Self::FilenameNotUnique => {
                "Multiple files with the same name were found. Please provide an exact path."
            }
            Self::DirectoryBothRootsMiss => "Directory not found at the provided path.",
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

    fn default_zh(self) -> &'static str {
        match self {
            Self::Rule1BothRootsMiss => "未在提供的路径找到文件。",
            Self::Rule2DirNotFound => "目录不存在，请提供正确路径。",
            Self::Rule2FileNotFound => "该目录下没有找到这个文件。",
            Self::Rule3ScanTooMany => "匹配文件过多，请提供精确路径。",
            Self::Rule3FileNotFound => "未找到文件。",
            Self::FilenameNotUnique => "找到多个同名文件，请提供精确路径。",
            Self::DirectoryBothRootsMiss => "未在提供的路径找到目录。",
            Self::DirectoryEntriesTooMany => {
                "该目录下文件或子目录过多，请提供更具体路径或更小范围。"
            }
            Self::DirectoryMultipleCandidates => "找到多个可能的目录，请确认是哪一个：",
            Self::DirectoryNoFilesInCurrentLevel => "当前目录层级没有找到文件。",
            Self::DirectoryNoSendableFilesInCurrentLevel => "该目录当前层级没有找到可发送文件。",
            Self::DirectoryHasChildDirsHint => {
                "这个目录下还有其他子目录，如需继续发送，请提供更准确路径。"
            }
        }
    }
}

pub(super) fn localize_delivery_message(state: &AppState, kind: DeliveryMessageKind) -> String {
    crate::i18n_t_with_default(state, kind.i18n_key(), kind.default_en())
}

pub(super) fn localize_delivery_message_for_request(
    state: &AppState,
    kind: DeliveryMessageKind,
    user_request: &str,
) -> String {
    match crate::language_policy::request_language_hint(user_request) {
        "en" => crate::bilingual_t_with_default_vars(
            state,
            kind.i18n_key(),
            kind.default_zh(),
            kind.default_en(),
            true,
            &[],
        ),
        "zh-CN" => crate::bilingual_t_with_default_vars(
            state,
            kind.i18n_key(),
            kind.default_zh(),
            kind.default_en(),
            false,
            &[],
        ),
        _ => localize_delivery_message(state, kind),
    }
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

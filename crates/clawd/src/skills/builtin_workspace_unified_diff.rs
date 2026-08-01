use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use super::{patch_io_error, read_optional_regular_file, validate_relative_patch_path};

#[path = "builtin_workspace_unified_diff_parse.rs"]
mod parse;

const MAX_PATCH_FILES: usize = 512;
const MAX_TARGET_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_TARGET_BYTES: u64 = 128 * 1024 * 1024;

pub(super) struct PureRustPatch {
    files: Vec<PreparedFile>,
}

pub(super) struct PureRustStat {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

struct PreparedFile {
    path: String,
    target: PathBuf,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
}

pub(super) fn inspect(
    root: &Path,
    patch: &str,
) -> Result<(PureRustPatch, Vec<PureRustStat>), String> {
    let parsed = parse::parse_patch(patch)?;
    if parsed.files.len() > MAX_PATCH_FILES {
        return Err(diff_error(
            "patch_file_limit_exceeded",
            "workspace.patch.file_limit_exceeded",
            json!({"file_count": parsed.files.len(), "max_files": MAX_PATCH_FILES}),
        ));
    }
    let mut prepared = Vec::with_capacity(parsed.files.len());
    let mut stats = Vec::with_capacity(parsed.files.len());
    let mut total_target_bytes = 0_u64;
    for file in parsed.files {
        let target = validate_relative_patch_path(root, &file.path)?;
        let target_bytes = match fs::metadata(&target) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => {
                return Err(patch_io_error(
                    "patch_target_inspection_failed",
                    "workspace.patch.target_inspection_failed",
                    error,
                ));
            }
        };
        total_target_bytes = total_target_bytes.saturating_add(target_bytes);
        if target_bytes > MAX_TARGET_FILE_BYTES || total_target_bytes > MAX_TOTAL_TARGET_BYTES {
            return Err(diff_error(
                "patch_target_limit_exceeded",
                "workspace.patch.target_limit_exceeded",
                json!({
                    "path": file.path,
                    "target_bytes": target_bytes,
                    "max_target_file_bytes": MAX_TARGET_FILE_BYTES,
                    "total_target_bytes": total_target_bytes,
                    "max_total_target_bytes": MAX_TOTAL_TARGET_BYTES,
                }),
            ));
        }
        let before = read_optional_regular_file(&target)?;
        if file.old_missing && before.is_some() {
            return Err(context_error(&file.path, 0, "target_already_exists"));
        }
        if !file.old_missing && before.is_none() {
            return Err(context_error(&file.path, 0, "target_missing"));
        }
        let after = apply_file(&file, before.as_deref())?;
        if after
            .as_ref()
            .is_some_and(|bytes| bytes.len() as u64 > MAX_TARGET_FILE_BYTES)
        {
            return Err(diff_error(
                "patch_target_limit_exceeded",
                "workspace.patch.target_limit_exceeded",
                json!({"path": file.path, "max_target_file_bytes": MAX_TARGET_FILE_BYTES}),
            ));
        }
        stats.push(PureRustStat {
            path: file.path.clone(),
            additions: file.additions,
            deletions: file.deletions,
        });
        prepared.push(PreparedFile {
            path: file.path,
            target,
            before,
            after,
        });
    }
    Ok((PureRustPatch { files: prepared }, stats))
}

pub(super) fn apply(root: &Path, patch: &PureRustPatch) -> Result<(), String> {
    for file in &patch.files {
        validate_relative_patch_path(root, &file.path)?;
        let current = read_optional_regular_file(&file.target)?;
        if current != file.before {
            return Err(context_error(&file.path, 0, "pre_apply_state_changed"));
        }
    }
    for file in &patch.files {
        match &file.after {
            Some(bytes) => {
                if let Some(parent) = file.target.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        patch_io_error(
                            "patch_parent_create_failed",
                            "workspace.patch.parent_create_failed",
                            error,
                        )
                    })?;
                }
                super::super::builtin_workspace_mutation::atomic_write_file(&file.target, bytes)
                    .map_err(|error| {
                        patch_io_error("patch_apply_failed", "workspace.patch.apply_failed", error)
                    })?;
            }
            None => fs::remove_file(&file.target).map_err(|error| {
                patch_io_error("patch_apply_failed", "workspace.patch.apply_failed", error)
            })?,
        }
    }
    Ok(())
}

fn apply_file(file: &parse::ParsedFile, before: Option<&[u8]>) -> Result<Option<Vec<u8>>, String> {
    let source = match before {
        Some(bytes) if bytes.contains(&0) => {
            return Err(diff_error(
                "binary_file_unsupported",
                "workspace.patch.binary_file_unsupported",
                json!({"path": file.path}),
            ));
        }
        Some(bytes) => std::str::from_utf8(bytes).map_err(|_| {
            diff_error(
                "non_utf8_file_unsupported",
                "workspace.patch.non_utf8_file_unsupported",
                json!({"path": file.path}),
            )
        })?,
        None => "",
    };
    let source_lines = source.split_inclusive('\n').collect::<Vec<_>>();
    let mut output = String::new();
    let mut cursor = 0;
    let mut output_line = 0;
    for (hunk_index, hunk) in file.hunks.iter().enumerate() {
        let expected_index = if hunk.old_start == 0 {
            0
        } else {
            hunk.old_start - 1
        };
        if expected_index < cursor || expected_index > source_lines.len() {
            return Err(context_error(
                &file.path,
                hunk_index,
                "invalid_hunk_position",
            ));
        }
        for line in &source_lines[cursor..expected_index] {
            output.push_str(line);
            output_line += 1;
        }
        cursor = expected_index;
        let expected_output_line = if hunk.new_start == 0 {
            0
        } else {
            hunk.new_start - 1
        };
        if output_line != expected_output_line {
            return Err(context_error(
                &file.path,
                hunk_index,
                "invalid_new_hunk_position",
            ));
        }
        for line in &hunk.lines {
            match line.kind {
                parse::LineKind::Add => {
                    output.push_str(&line.text);
                    output_line += 1;
                }
                parse::LineKind::Context | parse::LineKind::Remove => {
                    let actual = source_lines.get(cursor).copied();
                    if actual != Some(line.text.as_str()) {
                        return Err(context_error(&file.path, hunk_index, "context_mismatch"));
                    }
                    if line.kind == parse::LineKind::Context {
                        output.push_str(&line.text);
                        output_line += 1;
                    }
                    cursor += 1;
                }
            }
        }
    }
    for line in &source_lines[cursor..] {
        output.push_str(line);
    }
    if file.new_missing {
        if !output.is_empty() {
            return Err(context_error(
                &file.path,
                file.hunks.len(),
                "deletion_not_empty",
            ));
        }
        Ok(None)
    } else {
        Ok(Some(output.into_bytes()))
    }
}

fn context_error(path: &str, hunk_index: usize, reason: &str) -> String {
    diff_error(
        "patch_context_mismatch",
        "workspace.patch.context_mismatch",
        json!({
            "engine": "pure_rust",
            "path": path,
            "hunk_index": hunk_index,
            "reason": reason,
        }),
    )
}

pub(super) fn diff_error(error_code: &str, message_key: &str, details: Value) -> String {
    super::patch_error(error_code, message_key, details)
}

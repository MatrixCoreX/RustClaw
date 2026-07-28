use std::path::{Component, Path};
use std::time::Instant;

use serde_json::Value;

use crate::ripgrep_process::{base_command, resolve_binary, run_bounded};
use crate::{
    relative_path, resolve_root, BackendProvenance, CaseMode, Completeness, DiscoveryBackend,
    DiscoveryBudget, DiscoveryError, RipgrepTextMatch, RipgrepTextReport, RipgrepTextRequest,
    TextPatternKind,
};

const MAX_PATH_ARGS: usize = 512;
const MAX_PATH_ARG_BYTES: usize = 128 * 1024;

pub fn ripgrep_text_search(
    request: &RipgrepTextRequest,
) -> Result<RipgrepTextReport, DiscoveryError> {
    let started = Instant::now();
    if request.query.is_empty() {
        return Err(DiscoveryError::BackendFailed("query_empty".to_string()));
    }
    let binary = resolve_binary().map_err(DiscoveryError::BackendUnavailable)?;
    let (workspace_root, root) = resolve_root(&request.workspace_root, &request.root)?;
    let paths = validated_relative_paths(&root, &request.paths)?;
    if paths.len() > MAX_PATH_ARGS
        || paths
            .iter()
            .map(|path| path.as_os_str().len())
            .sum::<usize>()
            > MAX_PATH_ARG_BYTES
    {
        return Err(DiscoveryError::BackendUnavailable(
            "ripgrep_path_batch_too_large".to_string(),
        ));
    }

    let mut command = base_command(binary, &root);
    command
        .arg("--json")
        .arg("--color=never")
        .arg("--no-messages")
        .arg("--with-filename")
        .arg("--line-number");
    if request.pattern_kind == TextPatternKind::Literal {
        command.arg("--fixed-strings");
    }
    if case_insensitive(request.case_mode, &request.query) {
        command.arg("--ignore-case");
    } else {
        command.arg("--case-sensitive");
    }
    if request.multiline {
        command.arg("--multiline").arg("--multiline-dotall");
    }
    command.arg("--").arg(&request.query);
    for path in &paths {
        command.arg(path);
    }
    let budget = DiscoveryBudget {
        max_depth: None,
        hard_entry_limit: usize::MAX,
        match_snapshot_limit: request.max_matches.max(1),
        deadline: request.deadline,
        cancellation: request.cancellation.clone(),
    };
    let captured = run_bounded(
        command,
        &budget,
        request.max_output_bytes.clamp(4 * 1024, 64 * 1024 * 1024),
        true,
    )
    .map_err(DiscoveryError::BackendFailed)?;
    let mut matches = parse_matches(
        &captured.stdout,
        &workspace_root,
        &root,
        request.max_line_chars,
    )?;
    let mut completeness = if captured.timed_out || captured.cancelled {
        Completeness::PartialDeadline
    } else if captured.output_truncated || matches.len() > request.max_matches.max(1) {
        Completeness::PartialHardLimit
    } else {
        Completeness::Complete
    };
    matches.truncate(request.max_matches.max(1));
    matches.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.start_byte.cmp(&right.start_byte))
    });
    matches.dedup_by(|left, right| {
        left.path == right.path
            && left.start_byte == right.start_byte
            && left.end_byte == right.end_byte
    });
    if captured.status.code() == Some(1) && matches.is_empty() {
        completeness = Completeness::Complete;
    }
    Ok(RipgrepTextReport {
        matches,
        completeness,
        backend: BackendProvenance {
            backend: DiscoveryBackend::Ripgrep,
            version: Some(binary.version.clone()),
            fallback_reason: None,
            elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        },
        cancelled: captured.cancelled,
        output_truncated: captured.output_truncated,
    })
}

fn validated_relative_paths(
    root: &Path,
    paths: &[std::path::PathBuf],
) -> Result<Vec<std::path::PathBuf>, DiscoveryError> {
    let mut relative = Vec::with_capacity(paths.len());
    for path in paths {
        let candidate = if path.is_absolute() {
            path.clone()
        } else {
            root.join(path)
        };
        if candidate
            .components()
            .any(|component| component == Component::ParentDir)
            || !candidate.starts_with(root)
            || !candidate.is_file()
        {
            return Err(DiscoveryError::OutsideWorkspace);
        }
        relative.push(
            candidate
                .strip_prefix(root)
                .map_err(|_| DiscoveryError::OutsideWorkspace)?
                .to_path_buf(),
        );
    }
    relative.sort();
    relative.dedup();
    Ok(relative)
}

fn parse_matches(
    bytes: &[u8],
    workspace_root: &Path,
    root: &Path,
    max_line_chars: usize,
) -> Result<Vec<RipgrepTextMatch>, DiscoveryError> {
    let mut matches = Vec::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: Value = serde_json::from_slice(line)
            .map_err(|_| DiscoveryError::BackendFailed("ripgrep_json_invalid".to_string()))?;
        if value.get("type").and_then(Value::as_str) != Some("match") {
            continue;
        }
        let data = &value["data"];
        let path = data
            .pointer("/path/text")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                DiscoveryError::BackendFailed("ripgrep_json_path_missing".to_string())
            })?;
        let absolute = data
            .get("absolute_offset")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let base_line = data.get("line_number").and_then(Value::as_u64).unwrap_or(1) as usize;
        let text = data
            .pointer("/lines/text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let absolute_path = root.join(path);
        if !absolute_path.starts_with(root) {
            return Err(DiscoveryError::OutsideWorkspace);
        }
        let relative_path = relative_path(workspace_root, &absolute_path);
        let Some(submatches) = data.get("submatches").and_then(Value::as_array) else {
            continue;
        };
        for submatch in submatches {
            let start = submatch.get("start").and_then(Value::as_u64).unwrap_or(0) as usize;
            let end = submatch
                .get("end")
                .and_then(Value::as_u64)
                .unwrap_or(start as u64) as usize;
            if start == end || end > text.len() {
                continue;
            }
            let prefix = &text.as_bytes()[..start];
            let matched = &text.as_bytes()[start..end];
            let matched_text = String::from_utf8_lossy(matched).to_string();
            let line = base_line + prefix.iter().filter(|byte| **byte == b'\n').count();
            let end_line = line + matched.iter().filter(|byte| **byte == b'\n').count();
            matches.push(RipgrepTextMatch {
                path: relative_path.clone(),
                line,
                end_line,
                start_byte: absolute.saturating_add(start),
                end_byte: absolute.saturating_add(end),
                text: truncate(text.trim(), max_line_chars.clamp(40, 2_000)),
                matched_text: truncate(&matched_text, max_line_chars.clamp(40, 2_000)),
            });
        }
    }
    Ok(matches)
}

fn case_insensitive(mode: CaseMode, pattern: &str) -> bool {
    match mode {
        CaseMode::Sensitive => false,
        CaseMode::Insensitive => true,
        CaseMode::Smart => !pattern.chars().any(char::is_uppercase),
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut out = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

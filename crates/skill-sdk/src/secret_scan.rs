use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{SkillSdkError, SkillSdkResult};

const MAX_SCAN_FILES: usize = 20_000;
const MAX_SCAN_BYTES: u64 = 256 * 1024 * 1024;

pub(crate) fn scan_package_source(root: &Path) -> SkillSdkResult<()> {
    let root = fs::canonicalize(root)?;
    let mut files = Vec::new();
    collect_files(&root, &mut files)?;
    if files.len() > MAX_SCAN_FILES {
        return Err(SkillSdkError::new(
            "package_secret_scan_limit",
            format!("files={} limit={MAX_SCAN_FILES}", files.len()),
        )
        .phase("security"));
    }
    let mut total = 0_u64;
    for path in files {
        let bytes = fs::read(&path)?;
        total = total.saturating_add(bytes.len() as u64);
        if total > MAX_SCAN_BYTES {
            return Err(SkillSdkError::new(
                "package_secret_scan_limit",
                format!("bytes={total} limit={MAX_SCAN_BYTES}"),
            )
            .phase("security"));
        }
        if contains_secret_signature(&bytes) {
            let relative = path.strip_prefix(&root).unwrap_or(&path);
            return Err(SkillSdkError::new(
                "package_secret_detected",
                format!("path={}", relative.display()),
            )
            .phase("security"));
        }
    }
    Ok(())
}

pub fn redact_diagnostics(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            if contains_secret_signature(line.as_bytes()) || sensitive_assignment(line) {
                "[redacted sensitive diagnostic]"
            } else {
                line
            }
        })
        .take(200)
        .collect::<Vec<_>>()
        .join("\n")
}

fn collect_files(current: &Path, files: &mut Vec<PathBuf>) -> SkillSdkResult<()> {
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if ignored_name(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(SkillSdkError::new(
                "package_secret_scan_symlink_forbidden",
                path.display().to_string(),
            )
            .phase("security"));
        }
        if metadata.is_dir() {
            collect_files(&path, files)?;
        } else if metadata.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn contains_secret_signature(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return false;
    }
    let text = String::from_utf8_lossy(bytes);
    let upper = text.to_ascii_uppercase();
    if upper.contains("-----BEGIN PRIVATE KEY-----")
        || upper.contains("-----BEGIN RSA PRIVATE KEY-----")
        || upper.contains("-----BEGIN OPENSSH PRIVATE KEY-----")
        || contains_slack_token(&text)
    {
        return true;
    }
    text.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(known_secret_token)
}

fn contains_slack_token(value: &str) -> bool {
    ["xoxb-", "xoxa-", "xoxp-", "xoxr-", "xoxs-"]
        .iter()
        .filter_map(|prefix| value.find(prefix).map(|index| (prefix, index)))
        .any(|(prefix, index)| {
            value[index + prefix.len()..]
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                })
                .count()
                >= 19
        })
}

fn known_secret_token(token: &str) -> bool {
    (token.starts_with("AKIA")
        && token.len() == 20
        && token
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()))
        || (token.starts_with("ghp_") && token.len() >= 36)
        || (token.starts_with("github_pat_") && token.len() >= 32)
        || (token.starts_with("xoxb-") && token.len() >= 24)
}

fn sensitive_assignment(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    let sensitive = ["secret", "token", "password", "api_key", "private_key"]
        .iter()
        .any(|needle| lower.contains(needle));
    sensitive && (line.contains('=') || line.contains(':'))
}

fn ignored_name(name: &OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(".git" | "target" | "node_modules" | "__pycache__" | ".venv")
    )
}

#[cfg(test)]
#[path = "secret_scan_tests.rs"]
mod tests;

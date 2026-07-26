use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::{normalize_repo_relative_path, resolve_revision, GitBasicError};

const MAX_COMMIT_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_PATH_COUNT: usize = 512;

pub(super) fn is_local_write_action(action: &str) -> bool {
    matches!(
        action,
        "stage" | "commit" | "create_branch" | "checkout_branch"
    )
}

pub(super) fn execute_local_write(
    root: &Path,
    args: &Map<String, Value>,
    action: &str,
) -> Result<(String, Value), GitBasicError> {
    ensure_only_action_keys(args, action)?;
    let (subcommand, output) = match action {
        "stage" => {
            let paths = required_paths(args)?;
            let mut command_args = vec!["add".to_string(), "--".to_string()];
            command_args.extend(paths.iter().cloned());
            ("add", run_git(root, &command_args)?)
        }
        "commit" => {
            let message = required_commit_message(args)?;
            let before = repository_state(root)?;
            if before.staged_paths.is_empty() {
                return Err(GitBasicError::new(
                    "git_stage_empty",
                    "git_basic.stage_empty",
                ));
            }
            (
                "commit",
                run_git(
                    root,
                    &[
                        "commit".to_string(),
                        "--no-verify".to_string(),
                        "--no-gpg-sign".to_string(),
                        "-m".to_string(),
                        message.to_string(),
                    ],
                )?,
            )
        }
        "create_branch" => {
            let branch_name = required_token(args, "branch_name", "git_branch_name_invalid")?;
            validate_branch_name(root, branch_name)?;
            let mut command_args = vec![
                "branch".to_string(),
                "--no-track".to_string(),
                branch_name.to_string(),
            ];
            if let Some(start_point) = optional_token(args, "start_point")? {
                command_args.push(resolve_revision(root, start_point)?);
            }
            ("branch", run_git(root, &command_args)?)
        }
        "checkout_branch" => {
            let branch_name = required_token(args, "branch_name", "git_branch_name_invalid")?;
            validate_branch_name(root, branch_name)?;
            let before = repository_state(root)?;
            if !before.clean {
                return Err(GitBasicError::new(
                    "git_checkout_dirty_worktree",
                    "git_basic.checkout_dirty_worktree",
                )
                .with_extra(json!({
                    "changed_paths": before.changed_paths,
                    "staged_paths": before.staged_paths,
                })));
            }
            verify_local_branch(root, branch_name)?;
            (
                "checkout",
                run_git(
                    root,
                    &[
                        "checkout".to_string(),
                        "--no-guess".to_string(),
                        "--no-recurse-submodules".to_string(),
                        branch_name.to_string(),
                    ],
                )?,
            )
        }
        _ => {
            return Err(GitBasicError::new(
                "unsupported_action",
                "git_basic.unsupported_action",
            ));
        }
    };

    let state = repository_state(root)?;
    let commit_hash = head_revision(root)?;
    let output_text = command_output_text(&output);
    let digest = Sha256::digest(output_text.as_bytes());
    let extra = json!({
        "schema_version": 1,
        "source_skill": "git_basic",
        "status": "ok",
        "action": action,
        "subcommand": subcommand,
        "effect": "mutate",
        "exit_code": output.status.code().unwrap_or(0),
        "branch": state.branch,
        "commit_hash": commit_hash,
        "staged_paths": state.staged_paths,
        "changed_paths": state.changed_paths,
        "worktree_state": if state.clean { "clean" } else { "dirty" },
        "clean": state.clean,
        "hooks_enabled": false,
        "signing_enabled": false,
        "remote_mutation": false,
        "output_bytes": output_text.len(),
        "output_sha256": format!("sha256:{digest:x}"),
        "provenance": {
            "source": "git_cli",
            "repository_root": root,
            "operation_class": "local_mutation",
            "observed_at": epoch_seconds(),
        },
    });
    Ok((output_text, extra))
}

fn ensure_only_action_keys(args: &Map<String, Value>, action: &str) -> Result<(), GitBasicError> {
    let action_fields: &[&str] = match action {
        "stage" => &["paths"],
        "commit" => &["message"],
        "create_branch" => &["branch_name", "start_point"],
        "checkout_branch" => &["branch_name"],
        _ => &[],
    };
    for key in args.keys() {
        if key != "action" && key != "repo" && !action_fields.contains(&key.as_str()) {
            return Err(
                GitBasicError::new("git_unexpected_arg", "git_basic.unexpected_arg")
                    .with_extra(json!({"arg": key, "action": action})),
            );
        }
    }
    Ok(())
}

fn run_git(root: &Path, args: &[String]) -> Result<Output, GitBasicError> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| GitBasicError::new("git_spawn_failed", error.to_string()))?;
    if output.status.success() {
        return Ok(output);
    }
    Err(
        GitBasicError::new("git_command_failed", command_output_text(&output)).with_extra(json!({
            "exit_code": output.status.code().unwrap_or(-1),
            "argv": args,
        })),
    )
}

fn required_paths(args: &Map<String, Value>) -> Result<Vec<String>, GitBasicError> {
    let paths = args
        .get("paths")
        .and_then(Value::as_array)
        .ok_or_else(|| GitBasicError::new("git_paths_missing", "git_basic.paths_required"))?;
    if paths.is_empty() || paths.len() > MAX_PATH_COUNT {
        return Err(GitBasicError::new(
            "git_paths_invalid",
            "git_basic.paths_invalid",
        ));
    }
    let mut unique = BTreeSet::new();
    for value in paths {
        let path = value
            .as_str()
            .ok_or_else(|| GitBasicError::new("git_path_invalid", "git_basic.path_invalid"))?;
        let path = normalize_repo_relative_path(path.trim())?;
        if path == "." {
            return Err(GitBasicError::new(
                "git_path_invalid",
                "git_basic.explicit_file_path_required",
            ));
        }
        unique.insert(path);
    }
    Ok(unique.into_iter().collect())
}

fn required_commit_message(args: &Map<String, Value>) -> Result<&str, GitBasicError> {
    let message = args
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            !value.is_empty() && value.len() <= MAX_COMMIT_MESSAGE_BYTES && !value.contains('\0')
        })
        .ok_or_else(|| {
            GitBasicError::new(
                "git_commit_message_invalid",
                "git_basic.commit_message_invalid",
            )
        })?;
    Ok(message)
}

fn required_token<'a>(
    args: &'a Map<String, Value>,
    key: &str,
    error_code: &'static str,
) -> Result<&'a str, GitBasicError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| GitBasicError::new(error_code, error_code))
}

fn optional_token<'a>(
    args: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, GitBasicError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
                Err(GitBasicError::new(
                    "git_start_point_invalid",
                    "git_basic.start_point_invalid",
                ))
            } else {
                Ok(Some(value))
            }
        }
        Some(_) => Err(GitBasicError::new(
            "git_start_point_invalid",
            "git_basic.start_point_invalid",
        )),
    }
}

fn validate_branch_name(root: &Path, branch_name: &str) -> Result<(), GitBasicError> {
    run_git(
        root,
        &[
            "check-ref-format".to_string(),
            "--branch".to_string(),
            branch_name.to_string(),
        ],
    )
    .map(|_| ())
    .map_err(|_| GitBasicError::new("git_branch_name_invalid", "git_basic.branch_name_invalid"))
}

fn verify_local_branch(root: &Path, branch_name: &str) -> Result<(), GitBasicError> {
    let reference = format!("refs/heads/{branch_name}");
    let output = Command::new("git")
        .current_dir(root)
        .args(["show-ref", "--verify", "--quiet", &reference])
        .output()
        .map_err(|error| GitBasicError::new("git_spawn_failed", error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GitBasicError::new(
            "git_branch_not_found",
            "git_basic.branch_not_found",
        ))
    }
}

#[derive(Debug)]
struct RepositoryState {
    branch: Option<String>,
    clean: bool,
    staged_paths: Vec<String>,
    changed_paths: Vec<String>,
}

fn repository_state(root: &Path) -> Result<RepositoryState, GitBasicError> {
    let status = run_git(
        root,
        &[
            "status".to_string(),
            "--porcelain=v1".to_string(),
            "-z".to_string(),
        ],
    )?;
    let changed_paths = parse_status_paths(&status.stdout);
    let staged = run_git(
        root,
        &[
            "diff".to_string(),
            "--cached".to_string(),
            "--name-only".to_string(),
            "-z".to_string(),
        ],
    )?;
    let staged_paths = parse_nul_paths(&staged.stdout);
    let branch = current_branch(root)?;
    Ok(RepositoryState {
        clean: changed_paths.is_empty(),
        changed_paths,
        staged_paths,
        branch,
    })
}

fn parse_status_paths(bytes: &[u8]) -> Vec<String> {
    let fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    let mut paths = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let field = String::from_utf8_lossy(fields[index]);
        let status = field.get(..2).unwrap_or_default();
        let path = field.get(3..).unwrap_or_default();
        if !path.is_empty() {
            paths.push(path.to_string());
        }
        index += if status.contains('R') || status.contains('C') {
            2
        } else {
            1
        };
    }
    paths.sort();
    paths.dedup();
    paths
}

fn parse_nul_paths(bytes: &[u8]) -> Vec<String> {
    let mut paths = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8_lossy(field).to_string())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn current_branch(root: &Path) -> Result<Option<String>, GitBasicError> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .output()
        .map_err(|error| GitBasicError::new("git_spawn_failed", error.to_string()))?;
    if output.status.success() {
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ))
    } else if output.status.code() == Some(1) {
        Ok(None)
    } else {
        Err(GitBasicError::new(
            "git_branch_query_failed",
            command_output_text(&output),
        ))
    }
}

fn head_revision(root: &Path) -> Result<Option<String>, GitBasicError> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .map_err(|error| GitBasicError::new("git_spawn_failed", error.to_string()))?;
    if output.status.success() {
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ))
    } else {
        Ok(None)
    }
}

fn command_output_text(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    text
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "local_write_tests.rs"]
mod tests;

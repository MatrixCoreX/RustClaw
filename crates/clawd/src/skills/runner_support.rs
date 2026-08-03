use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

pub(super) fn selected_provider_api_key_env_names(
    vendor: &str,
    provider_type: &str,
) -> Vec<&'static str> {
    let mut names = match vendor.trim().to_ascii_lowercase().as_str() {
        "openai" => vec!["OPENAI_API_KEY"],
        "google" | "gemini" => vec!["GOOGLE_API_KEY"],
        "anthropic" | "claude" => vec!["ANTHROPIC_API_KEY"],
        "grok" | "xai" => vec!["GROK_API_KEY"],
        "deepseek" => vec!["DEEPSEEK_API_KEY"],
        "qwen" => vec!["QWEN_API_KEY"],
        "minimax" => vec!["MINIMAX_API_KEY"],
        "mimo" | "xiaomi" => vec!["MIMO_API_KEY"],
        "custom" => Vec::new(),
        _ => Vec::new(),
    };
    if provider_type.trim().eq_ignore_ascii_case("openai_compat")
        && !names.contains(&"OPENAI_API_KEY")
    {
        names.push("OPENAI_API_KEY");
    }
    names
}

pub(super) fn local_clawd_base_url_from_internal_listen(internal_listen: Option<&str>) -> String {
    let address = internal_listen
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<SocketAddr>().ok())
        .filter(|value| value.ip().is_loopback());
    address
        .map(|value| format!("http://{value}"))
        .unwrap_or_else(|| claw_core::config::CLAWD_INTERNAL_BASE_URL.to_string())
}

pub(super) fn inherited_sandbox_backend(backend: &'static str) -> Option<&'static str> {
    (backend != "direct").then_some(backend)
}

pub(super) fn runner_additional_writable_paths(
    secret_store_directory: Option<&Path>,
    skill_storage_directory: Option<&Path>,
    artifact_output_directory: Option<&Path>,
) -> Vec<PathBuf> {
    secret_store_directory
        .into_iter()
        .chain(skill_storage_directory)
        .chain(artifact_output_directory)
        .map(Path::to_path_buf)
        .collect()
}

pub(super) fn invocation_artifact_output_directory(
    workspace_root: &Path,
    task_id: &str,
    skill_name: &str,
) -> PathBuf {
    fn component(value: &str, fallback: &str) -> String {
        let normalized: String = value
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                    ch
                } else {
                    '_'
                }
            })
            .take(96)
            .collect();
        if normalized.is_empty() {
            fallback.to_string()
        } else {
            normalized
        }
    }

    claw_core::workspace_state::workspace_artifacts_root(workspace_root)
        .join("skill-invocations")
        .join(component(task_id, "task"))
        .join(component(skill_name, "skill"))
        .join(uuid::Uuid::new_v4().to_string())
}

pub(super) fn cancelled_capture_projection(
    artifact_output_directory: &Path,
    task_id: &str,
    skill_name: &str,
) -> Option<Value> {
    let mut manifests = Vec::new();
    let mut pending = vec![(artifact_output_directory.to_path_buf(), 0_u8)];
    while let Some((directory, depth)) = pending.pop() {
        if depth > 6 || manifests.len() >= 32 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten().take(96) {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                pending.push((path, depth + 1));
            } else if file_type.is_file()
                && entry.file_name() == std::ffi::OsStr::new("manifest.jsonl")
            {
                let run_root = path
                    .parent()
                    .and_then(Path::parent)
                    .unwrap_or(artifact_output_directory)
                    .to_path_buf();
                manifests.push((run_root, path));
            }
        }
    }
    let mut receipts = Vec::new();
    let mut artifacts = Vec::new();
    for (run_root, manifest) in manifests {
        let Ok(content) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        for value in content
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .take(10)
        {
            let Some(receipt_id) = value.get("receipt_id").and_then(Value::as_str) else {
                continue;
            };
            receipts.push(json!({
                "receipt_id": receipt_id,
                "ordinal": value.get("ordinal").cloned().unwrap_or(Value::Null),
                "status": value.get("status").cloned().unwrap_or(Value::Null),
                "content_hash_sha256": value.get("content_hash_sha256").cloned().unwrap_or(Value::Null),
                "html_hash_sha256": value.get("html_hash_sha256").cloned().unwrap_or(Value::Null),
                "image_hash_sha256": value.get("image_hash_sha256").cloned().unwrap_or(Value::Null),
                "error_code": value.get("error_code").cloned().unwrap_or(Value::Null),
            }));
            for key in ["html_path", "text_path", "image_path"] {
                if let Some(relative) = value.get(key).and_then(Value::as_str) {
                    append_cancelled_capture_artifact(&mut artifacts, &run_root, relative, key);
                }
            }
            if let Some(paths) = value.get("image_paths").and_then(Value::as_array) {
                for path in paths.iter().filter_map(Value::as_str).take(16) {
                    append_cancelled_capture_artifact(
                        &mut artifacts,
                        &run_root,
                        path,
                        "image_path",
                    );
                }
            }
        }
        artifacts.push(json!({
            "kind": "capture_manifest",
            "path": manifest,
        }));
    }
    if receipts.is_empty() {
        return None;
    }
    Some(json!({
        "schema_version": 1,
        "source": skill_name,
        "data_only": true,
        "task_id": task_id,
        "status": "cancelled_partial",
        "hard_termination": true,
        "final_partial_generated": false,
        "completed_page_count": receipts.iter().filter(|receipt| receipt["status"] == "ok").count(),
        "receipts": receipts,
        "artifacts": artifacts,
        "secrets_included": false,
    }))
}

fn append_cancelled_capture_artifact(
    artifacts: &mut Vec<Value>,
    run_root: &Path,
    relative: &str,
    kind: &str,
) {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        })
    {
        return;
    }
    let path = run_root.join(relative_path);
    if path.is_file() {
        artifacts.push(json!({"kind": kind, "path": path}));
    }
}

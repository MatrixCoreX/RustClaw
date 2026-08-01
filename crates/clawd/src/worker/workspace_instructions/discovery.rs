use std::collections::HashSet;
use std::path::{Path, PathBuf};

use claw_core::config::WorkspaceInstructionsConfig;
use serde_json::Value;

use self::source_io::{read_instruction_source, unloaded_instruction_source, utf8_prefix_len};

mod source_io;

#[derive(Debug)]
pub(super) struct InstructionSource {
    pub(super) logical_path: String,
    pub(super) depth: usize,
    pub(super) precedence: usize,
    pub(super) source_bytes: u64,
    pub(super) loaded_bytes: usize,
    pub(super) injected_bytes: usize,
    pub(super) content_sha256: Option<String>,
    pub(super) digest_scope: &'static str,
    pub(super) status: &'static str,
    pub(super) file_budget_truncated: bool,
    pub(super) total_budget_truncated: bool,
    content: String,
}

pub(super) struct DiscoveryResult {
    pub(super) cwd_status: &'static str,
    pub(super) relative_cwd: String,
    pub(super) sources: Vec<InstructionSource>,
    pub(super) rendered_sources: String,
}

pub(super) fn enabled_for_payload(config: &WorkspaceInstructionsConfig, payload: &Value) -> bool {
    if payload
        .get("execution_profile")
        .and_then(Value::as_str)
        .map(str::trim)
        == Some("coding")
    {
        config.enabled_for_coding
    } else {
        config.enabled_for_non_coding
    }
}

pub(super) fn discover_workspace_instructions(
    workspace_root: &Path,
    config: &WorkspaceInstructionsConfig,
    payload: &Value,
) -> anyhow::Result<DiscoveryResult> {
    let root = workspace_root
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("workspace_instruction_root_unavailable:{error}"))?;
    let (cwd, cwd_status) = resolve_working_directory(&root, payload);
    let relative_cwd = cwd
        .strip_prefix(&root)
        .ok()
        .filter(|path| !path.as_os_str().is_empty())
        .map(normalized_relative_path)
        .unwrap_or_else(|| ".".to_string());
    let directories = instruction_directories(&root, &cwd);
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for (depth, directory) in directories.iter().enumerate() {
        for filename in &config.filenames {
            let candidate = directory.join(filename);
            let Ok(canonical) = candidate.canonicalize() else {
                continue;
            };
            if !canonical.starts_with(&root) || !canonical.is_file() {
                continue;
            }
            if !seen.insert(canonical.clone()) {
                continue;
            }
            let logical_path = canonical
                .strip_prefix(&root)
                .map(normalized_relative_path)
                .unwrap_or_else(|_| filename.clone());
            candidates.push((canonical, logical_path, depth));
        }
    }
    let first_selected = candidates.len().saturating_sub(config.max_files);
    let mut sources = Vec::with_capacity(candidates.len());
    for (precedence, (path, logical_path, depth)) in candidates.into_iter().enumerate() {
        if precedence < first_selected {
            sources.push(unloaded_instruction_source(
                &path,
                logical_path,
                depth,
                precedence,
                "omitted_file_limit",
            ));
        } else {
            let source = read_instruction_source(
                &path,
                logical_path.clone(),
                depth,
                precedence,
                config.max_file_bytes,
            )
            .unwrap_or_else(|_| {
                unloaded_instruction_source(&path, logical_path, depth, precedence, "unreadable")
            });
            sources.push(source);
        }
    }
    apply_total_budget(&mut sources, first_selected, config.max_total_bytes);
    let rendered_sources = render_sources(&sources);
    Ok(DiscoveryResult {
        cwd_status,
        relative_cwd,
        sources,
        rendered_sources,
    })
}

fn resolve_working_directory(root: &Path, payload: &Value) -> (PathBuf, &'static str) {
    let Some(raw) = payload
        .pointer("/workspace_context/current_working_directory")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return (root.to_path_buf(), "default_root");
    };
    let requested = Path::new(raw);
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let Ok(canonical) = candidate.canonicalize() else {
        return (root.to_path_buf(), "unavailable");
    };
    if !canonical.starts_with(root) {
        return (root.to_path_buf(), "outside_workspace");
    }
    if !canonical.is_dir() {
        return (root.to_path_buf(), "not_directory");
    }
    (canonical, "resolved")
}

fn instruction_directories(root: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut directories = vec![root.to_path_buf()];
    let Ok(relative) = cwd.strip_prefix(root) else {
        return directories;
    };
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        directories.push(current.clone());
    }
    directories
}

fn apply_total_budget(
    sources: &mut [InstructionSource],
    first_selected: usize,
    max_total_bytes: usize,
) {
    let mut remaining = max_total_bytes;
    for source in sources.iter_mut().skip(first_selected).rev() {
        if matches!(source.status, "invalid_utf8" | "unreadable") {
            continue;
        }
        let injected_bytes = utf8_prefix_len(
            source.content.as_bytes(),
            remaining.min(source.content.len()),
        )
        .unwrap_or(0);
        source.injected_bytes = injected_bytes;
        remaining = remaining.saturating_sub(injected_bytes);
        if injected_bytes == 0 && !source.content.is_empty() {
            source.status = "omitted_total_budget";
            source.total_budget_truncated = true;
        } else if injected_bytes < source.content.len() {
            source.status = "injected_total_truncated";
            source.total_budget_truncated = true;
        } else if source.file_budget_truncated {
            source.status = "injected_file_truncated";
        } else {
            source.status = "injected";
        }
    }
}

fn render_sources(sources: &[InstructionSource]) -> String {
    let mut rendered = String::new();
    for source in sources.iter().filter(|source| source.injected_bytes > 0) {
        if !rendered.is_empty() {
            rendered.push_str("\n\n");
        }
        rendered.push_str("--- WORKSPACE INSTRUCTION SOURCE BEGIN ---\n");
        rendered.push_str(&format!(
            "logical_path: {}\nprecedence: {}\ncontent_sha256: {}\ninjected_bytes: {}\ncontent:\n",
            source.logical_path,
            source.precedence,
            source.content_sha256.as_deref().unwrap_or(""),
            source.injected_bytes
        ));
        rendered.push_str(&source.content[..source.injected_bytes]);
        rendered.push_str("\n--- WORKSPACE INSTRUCTION SOURCE END ---");
    }
    rendered
}

fn normalized_relative_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

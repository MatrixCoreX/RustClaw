use super::*;

const WORKSPACE_UPDATE_CONFIG_SNAPSHOT_MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct WorkspaceUpdateConfigSnapshot {
    files: Vec<WorkspaceUpdateConfigSnapshotFile>,
}

#[derive(Debug)]
struct WorkspaceUpdateConfigSnapshotFile {
    relative_path: String,
    bytes: Option<Vec<u8>>,
    permissions: Option<std::fs::Permissions>,
}

fn workspace_update_conflict_paths(
    paths: &WorkspaceUpdateConflictPaths,
) -> impl Iterator<Item = &String> {
    paths.tracked.iter().chain(paths.untracked.iter())
}

fn workspace_update_runtime_config_path(path: &str) -> bool {
    Path::new(path)
        .components()
        .next()
        .is_some_and(|component| component.as_os_str() == "configs")
}

pub(super) fn workspace_update_has_non_config_conflicts(
    paths: &WorkspaceUpdateConflictPaths,
) -> bool {
    workspace_update_conflict_paths(paths).any(|path| !workspace_update_runtime_config_path(path))
}

pub(super) async fn snapshot_workspace_update_config_conflicts(
    workspace_root: &Path,
    paths: &WorkspaceUpdateConflictPaths,
) -> Result<WorkspaceUpdateConfigSnapshot, String> {
    let mut files = Vec::with_capacity(paths.len());
    let mut total_bytes = 0usize;
    for relative_path in workspace_update_conflict_paths(paths) {
        if !workspace_update_runtime_config_path(relative_path) {
            return Err("workspace_update_config_snapshot_scope_invalid".to_string());
        }
        let absolute_path = workspace_root.join(relative_path);
        let snapshot = match tokio::fs::symlink_metadata(&absolute_path).await {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err("workspace_update_config_snapshot_non_regular_file".to_string());
                }
                let bytes = tokio::fs::read(&absolute_path)
                    .await
                    .map_err(|_| "workspace_update_config_snapshot_read_failed".to_string())?;
                total_bytes = total_bytes.saturating_add(bytes.len());
                if total_bytes > WORKSPACE_UPDATE_CONFIG_SNAPSHOT_MAX_BYTES {
                    return Err("workspace_update_config_snapshot_size_exceeded".to_string());
                }
                WorkspaceUpdateConfigSnapshotFile {
                    relative_path: relative_path.clone(),
                    bytes: Some(bytes),
                    permissions: Some(metadata.permissions()),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                WorkspaceUpdateConfigSnapshotFile {
                    relative_path: relative_path.clone(),
                    bytes: None,
                    permissions: None,
                }
            }
            Err(_) => {
                return Err("workspace_update_config_snapshot_metadata_failed".to_string());
            }
        };
        files.push(snapshot);
    }
    Ok(WorkspaceUpdateConfigSnapshot { files })
}

pub(super) async fn prepare_workspace_update_config_paths_for_pull(
    workspace_root: &Path,
    paths: &WorkspaceUpdateConflictPaths,
) -> Result<(), String> {
    for batch in paths.tracked.chunks(WORKSPACE_UPDATE_PATH_BATCH_SIZE) {
        let mut args = vec![
            "restore".to_string(),
            "--source".to_string(),
            "HEAD".to_string(),
            "--staged".to_string(),
            "--worktree".to_string(),
            "--".to_string(),
        ];
        args.extend(batch.iter().cloned());
        let out = run_workspace_update_command_args("git", &args, workspace_root, 600).await?;
        if out.exit_code != Some(0) {
            tracing::error!(
                error_code = "workspace_update_config_restore_head_failed",
                detail = %workspace_update_output_detail(&out),
                "workspace_update_config_prepare_failed"
            );
            return Err("workspace_update_config_restore_head_failed".to_string());
        }
    }

    for batch in paths.untracked.chunks(WORKSPACE_UPDATE_PATH_BATCH_SIZE) {
        let mut args = vec!["clean".to_string(), "-fd".to_string(), "--".to_string()];
        args.extend(batch.iter().cloned());
        let out = run_workspace_update_command_args("git", &args, workspace_root, 600).await?;
        if out.exit_code != Some(0) {
            tracing::error!(
                error_code = "workspace_update_config_clean_failed",
                detail = %workspace_update_output_detail(&out),
                "workspace_update_config_prepare_failed"
            );
            return Err("workspace_update_config_clean_failed".to_string());
        }
    }

    Ok(())
}

pub(super) async fn restore_workspace_update_config_snapshot(
    workspace_root: &Path,
    snapshot: &WorkspaceUpdateConfigSnapshot,
) -> Result<(), String> {
    for file in &snapshot.files {
        let target = workspace_root.join(&file.relative_path);
        let Some(bytes) = file.bytes.as_ref() else {
            match tokio::fs::remove_file(&target).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {
                    return Err("workspace_update_config_restore_delete_failed".to_string());
                }
            }
            continue;
        };
        let parent = target
            .parent()
            .ok_or_else(|| "workspace_update_config_restore_parent_invalid".to_string())?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| "workspace_update_config_restore_parent_failed".to_string())?;
        let file_name = target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "workspace_update_config_restore_name_invalid".to_string())?;
        let process_id = std::process::id();
        let temporary = parent.join(format!(".rustclaw_update_{process_id}_{file_name}"));
        tokio::fs::write(&temporary, bytes)
            .await
            .map_err(|_| "workspace_update_config_restore_write_failed".to_string())?;
        if let Some(permissions) = file.permissions.clone() {
            if let Err(error) = tokio::fs::set_permissions(&temporary, permissions).await {
                let _ = tokio::fs::remove_file(&temporary).await;
                tracing::error!(
                    error_code = "workspace_update_config_restore_permissions_failed",
                    detail = %error,
                    "workspace_update_config_restore_failed"
                );
                return Err("workspace_update_config_restore_permissions_failed".to_string());
            }
        }
        if let Err(error) = tokio::fs::rename(&temporary, &target).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            tracing::error!(
                error_code = "workspace_update_config_restore_rename_failed",
                detail = %error,
                "workspace_update_config_restore_failed"
            );
            return Err("workspace_update_config_restore_rename_failed".to_string());
        }
    }
    Ok(())
}

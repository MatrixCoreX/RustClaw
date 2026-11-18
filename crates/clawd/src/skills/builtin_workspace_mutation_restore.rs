use super::*;
use crate::runtime::host_filesystem::HostDirectory;

struct RestoreFrame {
    directory: HostDirectory,
    children: std::vec::IntoIter<PathBuf>,
    mode: Option<HostAccessMode>,
}

pub(super) fn restore_snapshot(
    root: &Path,
    checkpoint_dir: &Path,
    manifest: &MutationCheckpointManifest,
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    let plan = plan_restore(root, checkpoint_dir, manifest)?;
    let mut files = BTreeMap::new();
    for file in plan.files {
        let bytes = filesystem
            .read_file_bounded(Some(checkpoint_dir), &file.backup, file.expected_bytes)
            .map_err(|error| match error {
                BoundedFileReadError::Io(error) => io_error("restore_read_failed", error),
                BoundedFileReadError::LimitExceeded {
                    observed_bytes_at_least,
                    ..
                } => backup_validation_error(
                    &file.entry.path,
                    "size_mismatch",
                    json!({
                        "expected_size_bytes":file.expected_bytes,
                        "observed_bytes_at_least":observed_bytes_at_least,
                    }),
                ),
            })?;
        if bytes.len() != file.expected_bytes {
            return Err(backup_validation_error(
                &file.entry.path,
                "size_mismatch",
                json!({
                    "expected_size_bytes":file.expected_bytes,"observed_bytes":bytes.len(),
                }),
            ));
        }
        if file.entry.sha256.as_deref() != Some(sha256_label(&bytes).as_str()) {
            return Err(backup_validation_error(
                &file.entry.path,
                "hash_mismatch",
                Value::Null,
            ));
        }
        files.insert(file.target, (bytes, file.entry.unix_mode));
    }

    let mut directories = BTreeMap::new();
    let mut children: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    let mut depth = 0;
    for (path, entry) in plan.directories {
        depth = depth.max(
            path.strip_prefix(&plan.target)
                .expect("validated subtree")
                .components()
                .count()
                + 1,
        );
        if path != plan.target {
            children
                .entry(path.parent().expect("validated parent").to_path_buf())
                .or_default()
                .push(path.clone());
        }
        directories.insert(path, entry.unix_mode);
    }
    for path in files.keys().filter(|path| *path != &plan.target) {
        children
            .entry(path.parent().expect("validated parent").to_path_buf())
            .or_default()
            .push(path.clone());
    }

    // Admit the parent and detect known descriptor exhaustion before deletion.
    // This is a headroom check, not a process-wide reservation against other threads.
    let parent = if files.contains_key(&plan.target) || directories.contains_key(&plan.target) {
        let parent = filesystem
            .open_directory(Some(root), plan.target.parent().expect("validated parent"))
            .map_err(|error| io_error("restore_parent_failed", error))?;
        parent
            .verify_descriptor_headroom(depth + 6)
            .map_err(|error| io_error("restore_descriptor_budget_unavailable", error))?;
        Some(parent)
    } else {
        None
    };
    remove_existing_target(root, &plan.target, filesystem)?;
    if let Some((bytes, mode)) = files.remove(&plan.target) {
        filesystem
            .restore_file_at(
                parent.as_ref().expect("admitted file parent"),
                plan.target.file_name().expect("validated filename"),
                &bytes,
                mode,
            )
            .map_err(|error| io_error("restore_write_failed", error))?;
    } else if let Some(mode) = directories.remove(&plan.target) {
        let directory = filesystem
            .create_restore_directory_at(
                parent.as_ref().expect("admitted directory parent"),
                plan.target.file_name().expect("validated filename"),
            )
            .map_err(|error| io_error("restore_create_failed", error))?;
        let mut stack = vec![RestoreFrame {
            directory,
            children: children
                .remove(&plan.target)
                .unwrap_or_default()
                .into_iter(),
            mode,
        }];
        while let Some(frame) = stack.last_mut() {
            match frame.children.next() {
                Some(path) => {
                    let name = path.file_name().expect("validated filename");
                    if let Some(mode) = directories.remove(&path) {
                        let directory = filesystem
                            .create_restore_directory_at(&frame.directory, name)
                            .map_err(|error| io_error("restore_create_failed", error))?;
                        stack.push(RestoreFrame {
                            directory,
                            children: children.remove(&path).unwrap_or_default().into_iter(),
                            mode,
                        });
                    } else if let Some((bytes, mode)) = files.remove(&path) {
                        filesystem
                            .restore_file_at(&frame.directory, name, &bytes, mode)
                            .map_err(|error| io_error("restore_write_failed", error))?;
                    } else {
                        return Err(restore_manifest_error(
                            "restore_child_missing",
                            &path.display().to_string(),
                        ));
                    }
                }
                None => {
                    let frame = stack.pop().expect("current frame");
                    // Apply read-only modes only after all descendants are published,
                    // on this exact created descriptor, never a reopened pathname.
                    filesystem
                        .finish_restore_directory(&frame.directory, frame.mode)
                        .map_err(|error| io_error("restore_permissions_failed", error))?;
                }
            }
        }
    }
    if !files.is_empty() || !directories.is_empty() || !children.is_empty() {
        return Err(restore_manifest_error(
            "restore_entries_remaining",
            &manifest.target_path,
        ));
    }
    for path in plan.missing_parents {
        match filesystem.remove_path(Some(root), &path, false) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(io_error("restore_parent_cleanup_failed", error)),
        }
    }
    let actual = capture_snapshot(root, &manifest.target_path, None, filesystem)?;
    if !snapshot_matches_recorded_state(&manifest.before, &actual, manifest.schema_version == 1)
        || !missing_parents_absent(root, manifest, filesystem)?
    {
        return Err(restore_manifest_error(
            "restored_state_mismatch",
            &manifest.target_path,
        ));
    }
    Ok(())
}

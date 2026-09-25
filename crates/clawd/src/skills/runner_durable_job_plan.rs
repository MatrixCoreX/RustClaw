#[derive(Debug, Clone)]
struct DurableRunnerJobPlan {
    job_id: String,
    job_dir: PathBuf,
    global_slot_root: PathBuf,
    skill_slot_root: PathBuf,
    poll_after_seconds: u64,
    retention_seconds: u64,
    retention_deadline_at: i64,
    queue_scoped: bool,
}

impl DurableRunnerJobPlan {
    fn new(
        workspace_root: &Path,
        skill_name: &str,
        retention_seconds: u64,
        dispatch_queue_key: Option<&str>,
    ) -> Self {
        let job_uuid = uuid::Uuid::new_v4().to_string();
        let state_root = claw_core::workspace_state::workspace_state_root(workspace_root);
        let slot_root = state_root.join("durable_skill_slots");
        let skill_component = safe_path_component(skill_name, "skill");
        let skill_slot_root = dispatch_queue_key.map_or_else(
            || slot_root.join("skills").join(&skill_component),
            |queue_key| {
                slot_root
                    .join("queues")
                    .join(&skill_component)
                    .join(stable_queue_path_component(queue_key))
            },
        );
        let retention_seconds = retention_seconds.max(1);
        let now_ts = crate::now_ts_u64().min(i64::MAX as u64) as i64;
        Self {
            job_id: format!("local_process:{job_uuid}"),
            job_dir: state_root.join("async_jobs").join(job_uuid),
            global_slot_root: slot_root.join("global"),
            skill_slot_root,
            poll_after_seconds: 5,
            retention_seconds,
            retention_deadline_at: now_ts
                .saturating_add(retention_seconds.min(i64::MAX as u64) as i64),
            queue_scoped: dispatch_queue_key.is_some(),
        }
    }

    fn create_directories(&self) -> Result<(), String> {
        for path in [
            self.job_dir.as_path(),
            self.global_slot_root.as_path(),
            self.skill_slot_root.as_path(),
        ] {
            std::fs::create_dir_all(path).map_err(|error| {
                format!(
                    "durable_skill_job_directory_create_failed: path={} error={error}",
                    path.display()
                )
            })?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.job_dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("durable_skill_job_permissions_failed: {error}"))?;
        }
        Ok(())
    }
}

struct DurableRunnerJobSetupGuard {
    job_dir: PathBuf,
    preserve: bool,
}

impl DurableRunnerJobSetupGuard {
    fn new(job_dir: &Path) -> Self {
        Self {
            job_dir: job_dir.to_path_buf(),
            preserve: false,
        }
    }

    fn preserve(&mut self) {
        self.preserve = true;
    }
}

impl Drop for DurableRunnerJobSetupGuard {
    fn drop(&mut self) {
        if !self.preserve {
            let _ = std::fs::remove_dir_all(&self.job_dir);
        }
    }
}

fn safe_path_component(value: &str, fallback: &str) -> String {
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

fn stable_queue_path_component(queue_key: &str) -> String {
    hex::encode(Sha256::digest(queue_key.as_bytes()))
}

pub(super) fn local_process_durable_background_requested(
    mapping: Option<&PlannerCapabilityMapping>,
) -> bool {
    mapping.is_some_and(|mapping| {
        matches!(
            mapping.execution_mode,
            Some(CapabilityExecutionMode::AsyncPreferred | CapabilityExecutionMode::AsyncRequired)
        ) && mapping.async_adapter_kind.as_deref() == Some("local_process_poll")
    })
}

fn skill_secret_token_ttl(durable_background: bool, retention_seconds: u64) -> Duration {
    // Durable runners redeem ordinary credential references before they can
    // outlive the parent dispatch. Internal endpoint tokens must remain valid
    // while a job waits for a concurrency slot, so bind their lifetime to the
    // renewable job-retention window instead of the foreground handoff window.
    Duration::from_secs(if durable_background {
        retention_seconds.max(300)
    } else {
        300
    })
}

fn sandbox_target_for_source(
    source: Option<&std::path::Path>,
    sources: &[std::path::PathBuf],
    targets: &[std::path::PathBuf],
) -> Option<std::path::PathBuf> {
    let source = source?;
    sources
        .iter()
        .position(|candidate| candidate == source)
        .and_then(|index| targets.get(index))
        .cloned()
}

fn map_storage_descriptor_to_sandbox(
    mut descriptor: Option<crate::skill_storage::SkillStorageDescriptor>,
    sandbox_directory: Option<&std::path::Path>,
) -> Result<Option<crate::skill_storage::SkillStorageDescriptor>, String> {
    let Some(storage_descriptor) = descriptor.as_mut() else {
        return Ok(None);
    };
    let Some(sandbox_directory) = sandbox_directory else {
        return Err("skill storage sandbox target unavailable".to_string());
    };
    if storage_descriptor.storage_kind == "directory" {
        storage_descriptor.directory_path = Some(sandbox_directory.display().to_string());
    } else {
        let file_name = std::path::Path::new(&storage_descriptor.database_path)
            .file_name()
            .ok_or_else(|| "skill_storage_database_filename_missing".to_string())?;
        storage_descriptor.database_path = sandbox_directory.join(file_name).display().to_string();
    }
    Ok(descriptor)
}

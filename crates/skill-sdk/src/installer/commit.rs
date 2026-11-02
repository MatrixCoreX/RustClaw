//! Publish immutable packages only after the execution boundary verifies them.
use std::fs::{self, File, OpenOptions};
use std::path::Path;

use fs2::FileExt;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{emit_phase, InstallControl, InstallOrigin, InstallOutcome, StagingGuard};
use crate::receipt::{InstallReceipt, InstallReceiptStore};
use crate::runtime::SkillRuntimeResolver;
use crate::{SkillSdkError, SkillSdkResult};

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_install(
    store: &InstallReceiptStore,
    staging: &Path,
    guard: &mut StagingGuard,
    receipt: InstallReceipt,
    phases: Vec<String>,
    origin: InstallOrigin,
    control: Option<&InstallControl>,
) -> SkillSdkResult<InstallOutcome> {
    emit_phase(control, "activate")?;
    store.write_receipt(staging, &receipt)?;
    let identity = receipt_install_identity(&receipt)?;
    let base = store.version_dir(&receipt.skill_name, &receipt.version, &identity)?;
    let versions = base
        .parent()
        .ok_or_else(|| SkillSdkError::new("install_parent_missing", base.display().to_string()))?;
    fs::create_dir_all(versions)?;
    // The process mutex does not serialize a separate skillctl against the host.
    // Hold this short, non-blocking lock only while publishing and activating.
    let _commit_lock = commit_lock(store, &receipt.skill_name)?;
    let resolver = SkillRuntimeResolver::new(store.root());
    let base_name = base.file_name().unwrap().to_string_lossy().to_string();
    let repair_prefix = format!("{base_name}-repair-");
    let mut candidates = vec![base_name.clone()];
    for entry in fs::read_dir(versions)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type()?.is_dir() && name.starts_with(&repair_prefix) {
            candidates.push(name);
        }
    }
    candidates.sort();
    // Prefer the current healthy repair over an older identical copy.
    if let Ok(current) = store.current_pointer(&receipt.skill_name) {
        if let Some(index) = candidates
            .iter()
            .position(|name| *name == current.install_dir)
        {
            candidates.swap(0, index);
        }
    }
    let mut selected = None;
    for name in candidates {
        let path = versions.join(&name);
        if !fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_dir()) {
            continue;
        }
        // A damaged receipt, manifest, artifact, or trusted interpreter is not
        // reusable. Never overwrite it: an in-flight lease may still refer to it.
        if let Ok((existing, launch)) = resolver.verified_install(&receipt.skill_name, &name) {
            if receipt_install_identity(&existing)? != identity {
                return Err(SkillSdkError::new(
                    "install_identity_collision",
                    path.display().to_string(),
                )
                .phase("activate"));
            }
            selected = Some((existing, launch.install_root));
            break;
        }
    }
    let reused = selected.is_some();
    let mut published_guard = None;
    let (receipt, install_root) = match selected {
        Some(verified) => verified,
        None => {
            let destination = if fs::symlink_metadata(&base).is_ok() {
                versions.join(format!("{repair_prefix}{}", Uuid::new_v4()))
            } else {
                base.clone()
            };
            fs::rename(staging, &destination).map_err(|error| {
                SkillSdkError::new(
                    "install_stage_commit_failed",
                    format!(
                        "source={} destination={} error={error}",
                        staging.display(),
                        destination.display(),
                    ),
                )
                .phase("activate")
            })?;
            guard.disarm();
            // This new directory has never been activated or leased. On failed
            // verification/cancellation, remove only our unpublished output.
            published_guard = Some(StagingGuard::new(destination.clone()));
            let name = destination.file_name().unwrap().to_string_lossy();
            let (verified, launch) = resolver.verified_install(&receipt.skill_name, &name)?;
            if verified.digest()? != receipt.digest()? {
                return Err(SkillSdkError::new(
                    "install_receipt_changed",
                    name.to_string(),
                ));
            }
            (verified, launch.install_root)
        }
    };
    let outcome = InstallOutcome {
        skill_name: receipt.skill_name.clone(),
        version: receipt.version.clone(),
        install_root,
        receipt_digest: receipt.digest()?,
        adapter: receipt.adapter,
        origin,
        reused,
        phases,
    };
    emit_phase(control, "activate")?;
    store.activate(&outcome.install_root, &receipt)?;
    if let Some(guard) = published_guard.as_mut() {
        guard.disarm();
    }
    // No fallible work after activation: success means the pointer was written.
    Ok(outcome)
}

fn commit_lock(store: &InstallReceiptStore, skill_name: &str) -> SkillSdkResult<File> {
    let path = store.skill_root(skill_name)?.join(".install.lock");
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    file.try_lock_exclusive().map_err(|error| {
        SkillSdkError::new("install_commit_busy", error.to_string())
            .retryable(error.kind() == std::io::ErrorKind::WouldBlock)
            .phase("activate")
    })?;
    Ok(file)
}

fn receipt_install_identity(receipt: &InstallReceipt) -> SkillSdkResult<String> {
    let identity = serde_json::json!({
        "manifest_digest": receipt.manifest_digest,
        "source_digest": receipt.source_digest,
        "lockfile_digests": receipt.lockfile_digests,
        "adapter": receipt.adapter,
        "adapter_version": receipt.adapter_version,
        "platform": receipt.platform,
        "artifacts": receipt.artifacts,
        "launch": receipt.launch,
        "sandbox_profile": receipt.sandbox_profile,
        "runtime_network": receipt.runtime_network,
    });
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&identity)?)))
}

#[cfg(test)]
#[path = "commit_tests.rs"]
mod tests;

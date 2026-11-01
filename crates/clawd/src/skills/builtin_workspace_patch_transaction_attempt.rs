use super::*;

const ATTEMPT_SCHEMA_VERSION: u32 = 1;
const MAX_ATTEMPT_BYTES: usize = 64 * 1024;

// Intent metadata, not a process-closure proof or a permission grant.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AttemptReceipt {
    schema_version: u32,
    transaction_id: String,
    task_id: String,
    shard_count: usize,
    pub(super) shard_index: usize,
    pub(super) checkpoint_id: String,
    patch_id: String,
    request_digest: String,
}

impl AttemptReceipt {
    fn shard_record(&self) -> PatchShardRecord {
        PatchShardRecord {
            shard_index: self.shard_index,
            checkpoint_id: self.checkpoint_id.clone(),
            patch_id: self.patch_id.clone(),
            state: "applied".into(),
            request_digest: Some(self.request_digest.clone()),
        }
    }
}

// Only successful durable creation returns an owned claim. Drop retains it;
// unknown errors, cancellation and process exit must not reopen execution.
pub(super) struct ClaimedAttempt {
    receipt: AttemptReceipt,
}

impl ClaimedAttempt {
    pub(super) fn reserve(
        root: &Path,
        manifest: &PatchTransactionManifest,
        shard_index: usize,
        request_digest: &str,
        patch_id: &str,
        filesystem: &HostFilesystemPolicy,
    ) -> Result<Self, String> {
        let receipt = AttemptReceipt {
            schema_version: ATTEMPT_SCHEMA_VERSION,
            transaction_id: manifest.transaction_id.clone(),
            task_id: manifest.task_id.clone(),
            shard_count: manifest.shard_count,
            shard_index,
            checkpoint_id: format!("patch_{}", uuid::Uuid::new_v4().simple()),
            patch_id: patch_id.into(),
            request_digest: request_digest.into(),
        };
        // Use the exact commit shape and encoder before claiming an attempt or
        // invoking the payload. A later publication error remains uncertain.
        encode_manifest(&manifest_with_applied_shard(
            manifest.clone(),
            receipt.shard_record(),
        ))
        .map_err(|error| {
            crate::skills::annotate_structured_skill_error_not_applied(
                &error,
                "pre_dispatch",
                Some(false),
                None,
            )
        })?;
        let path = receipt_path(root, &manifest.transaction_id, shard_index);
        filesystem
            .create_directory(Some(root), path.parent().expect("receipt parent"), true)
            .map_err(|error| storage_failure(&receipt, "transaction_attempt_unavailable", error))?;
        let bytes = serde_json::to_vec(&receipt)
            .map_err(|_| invalid(&receipt.transaction_id, shard_index))?;
        if bytes.len() > MAX_ATTEMPT_BYTES || !valid(&receipt) {
            return Err(invalid(&receipt.transaction_id, shard_index));
        }
        if let Err(error) = filesystem.atomic_create_file(Some(root), &path, &bytes) {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                let previous = lookup(
                    root,
                    &manifest.task_id,
                    &manifest.transaction_id,
                    shard_index,
                    filesystem,
                )?;
                return Err(operation_failure(
                    "transaction_attempt_unresolved",
                    &manifest.transaction_id,
                    [
                        ("shard_index", json!(shard_index)),
                        (
                            "checkpoint_id",
                            json!(previous.map(|receipt| receipt.checkpoint_id)),
                        ),
                        ("execution_started", json!(false)),
                    ],
                ));
            }
            return Err(storage_failure(
                &receipt,
                "transaction_attempt_unavailable",
                error,
            ));
        }
        if lookup(
            root,
            &receipt.task_id,
            &receipt.transaction_id,
            shard_index,
            filesystem,
        )?
        .as_ref()
            != Some(&receipt)
        {
            return Err(invalid(&receipt.transaction_id, shard_index));
        }
        Ok(Self { receipt })
    }

    pub(super) fn checkpoint_id(&self) -> &str {
        &self.receipt.checkpoint_id
    }

    pub(super) fn shard_record(&self) -> PatchShardRecord {
        self.receipt.shard_record()
    }

    pub(super) fn matches(
        &self,
        manifest: &PatchTransactionManifest,
        shard: &PatchShardRecord,
    ) -> bool {
        matches_shard(&self.receipt, manifest, shard)
    }

    pub(super) fn retire_rejected(
        self,
        root: &Path,
        filesystem: &HostFilesystemPolicy,
        refusal: &str,
    ) -> Result<(), String> {
        let rejected = crate::skills::parse_structured_skill_error(refusal)
            .and_then(|error| error.extra)
            .is_some_and(|extra| {
                extra["side_effect_applied"] == false && extra["failure_phase"] == "pre_dispatch"
            });
        if !rejected
            || lookup(
                root,
                &self.receipt.task_id,
                &self.receipt.transaction_id,
                self.receipt.shard_index,
                filesystem,
            )?
            .as_ref()
                != Some(&self.receipt)
        {
            return Err(invalid(
                &self.receipt.transaction_id,
                self.receipt.shard_index,
            ));
        }
        filesystem
            .remove_path(
                Some(root),
                &receipt_path(root, &self.receipt.transaction_id, self.receipt.shard_index),
                false,
            )
            .map_err(|error| {
                storage_failure(
                    &self.receipt,
                    "transaction_attempt_retirement_unconfirmed",
                    error,
                )
            })
    }
}

pub(super) fn lookup(
    root: &Path,
    task_id: &str,
    transaction_id: &str,
    shard_index: usize,
    filesystem: &HostFilesystemPolicy,
) -> Result<Option<AttemptReceipt>, String> {
    let (file, _) =
        match filesystem.open_file(Some(root), &receipt_path(root, transaction_id, shard_index)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(read_failure(transaction_id, shard_index, error)),
        };
    let mut bytes = Vec::new();
    file.take(MAX_ATTEMPT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| read_failure(transaction_id, shard_index, error))?;
    if bytes.len() > MAX_ATTEMPT_BYTES {
        return Err(invalid(transaction_id, shard_index));
    }
    let receipt: AttemptReceipt =
        serde_json::from_slice(&bytes).map_err(|_| invalid(transaction_id, shard_index))?;
    if receipt.task_id != task_id {
        return Err(operation_failure(
            "transaction_owner_mismatch",
            transaction_id,
            [],
        ));
    }
    if !valid(&receipt)
        || receipt.transaction_id != transaction_id
        || receipt.shard_index != shard_index
    {
        return Err(invalid(transaction_id, shard_index));
    }
    Ok(Some(receipt))
}

pub(super) fn verify(
    root: &Path,
    manifest: &PatchTransactionManifest,
    shard: &PatchShardRecord,
    filesystem: &HostFilesystemPolicy,
) -> Result<(), String> {
    let receipt = lookup(
        root,
        &manifest.task_id,
        &manifest.transaction_id,
        shard.shard_index,
        filesystem,
    )?;
    if !receipt
        .as_ref()
        .is_some_and(|receipt| matches_shard(receipt, manifest, shard))
    {
        return Err(operation_failure(
            "transaction_attempt_binding_missing",
            &manifest.transaction_id,
            [
                ("shard_index", json!(shard.shard_index)),
                ("checkpoint_id", json!(shard.checkpoint_id)),
            ],
        ));
    }
    Ok(())
}

pub(super) fn status(
    root: &Path,
    task_id: &str,
    transaction_id: &str,
    shard_index: usize,
    filesystem: &HostFilesystemPolicy,
) -> Result<Value, String> {
    let receipt = lookup(root, task_id, transaction_id, shard_index, filesystem)?
        .ok_or_else(|| invalid(transaction_id, shard_index))?;
    Ok(
        json!({"schema_version":1,"source":"workspace_patch_transaction","status":"ok",
        "action":"transaction_status","transaction_id":transaction_id,"state":"attempt_evidence",
        "execution_outcome":"unknown","complete":false,"continuation":null,"attempt":receipt}),
    )
}

fn receipt_path(root: &Path, transaction_id: &str, shard_index: usize) -> PathBuf {
    super::super::checkpoint_root(root)
        .join("patch-transactions")
        .join(format!("{transaction_id}.attempts"))
        .join(format!("{shard_index}.json"))
}

fn matches_shard(
    receipt: &AttemptReceipt,
    manifest: &PatchTransactionManifest,
    shard: &PatchShardRecord,
) -> bool {
    receipt.transaction_id == manifest.transaction_id
        && receipt.task_id == manifest.task_id
        && receipt.shard_count == manifest.shard_count
        && receipt.shard_index == shard.shard_index
        && receipt.checkpoint_id == shard.checkpoint_id
        && receipt.patch_id == shard.patch_id
        && Some(receipt.request_digest.as_str()) == shard.request_digest.as_deref()
}

fn valid(receipt: &AttemptReceipt) -> bool {
    let checkpoint = receipt
        .checkpoint_id
        .strip_prefix("patch_")
        .and_then(|text| uuid::Uuid::parse_str(text).ok().map(|id| (text, id)))
        .is_some_and(|(text, id)| !id.is_nil() && text == id.simple().to_string());
    let digest = |text: &str| {
        text.strip_prefix("sha256:").is_some_and(|text| {
            text.len() == 64
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    };
    receipt.schema_version == ATTEMPT_SCHEMA_VERSION
        && !receipt.task_id.is_empty()
        && receipt.shard_count > 0
        && receipt.shard_index < receipt.shard_count
        && checkpoint
        && digest(&receipt.patch_id)
        && digest(&receipt.request_digest)
}

fn invalid(transaction_id: &str, shard_index: usize) -> String {
    operation_failure(
        "transaction_attempt_invalid",
        transaction_id,
        [("shard_index", json!(shard_index))],
    )
}

fn read_failure(transaction_id: &str, shard_index: usize, error: std::io::Error) -> String {
    let code = if HostFilesystemPolicy::is_private_access_denied(&error) {
        "host_private_directory_access_denied"
    } else {
        "transaction_attempt_unavailable"
    };
    operation_failure(
        code,
        transaction_id,
        [
            ("shard_index", json!(shard_index)),
            ("io_kind", json!(format!("{:?}", error.kind()))),
        ],
    )
}

fn storage_failure(receipt: &AttemptReceipt, code: &str, error: std::io::Error) -> String {
    let code = if HostFilesystemPolicy::is_private_access_denied(&error) {
        "host_private_directory_access_denied"
    } else {
        code
    };
    operation_failure(
        code,
        &receipt.transaction_id,
        [
            ("shard_index", json!(receipt.shard_index)),
            ("checkpoint_id", json!(receipt.checkpoint_id)),
            ("io_kind", json!(format!("{:?}", error.kind()))),
        ],
    )
}

#[cfg(test)]
#[path = "builtin_workspace_patch_transaction_attempt_tests.rs"]
mod tests;

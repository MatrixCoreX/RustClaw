//! Durable logical parts reuse the existing dispatch lease and receipt state machine.
use super::*;
use claw_core::channel_delivery_tokens::{
    parse_legacy_delivery_line, strip_legacy_delivery_lines, LegacyDeliveryLocation,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;

#[derive(Serialize, Deserialize)]
struct Plan {
    schema_version: u8,
    identity_digest: String,
    file_digests: Vec<Option<String>>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn texts(state: &AppState, envelope: &ChannelDeliveryEnvelope) -> anyhow::Result<Vec<String>> {
    if !envelope.artifacts.is_empty() {
        anyhow::bail!("channel_delivery_typed_artifact_transport_required");
    }
    let full = envelope
        .text_segments
        .iter()
        .map(|v| v.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = claw_core::channel_chunk::chunk_text_for_channel(
        &strip_legacy_delivery_lines(&full),
        crate::channel_send::delivery_text_chunk_chars(state, envelope.channel),
    );
    let mut media_lines = full
        .lines()
        .filter(|line| parse_legacy_delivery_line(line).is_some())
        .collect::<Vec<_>>();
    if envelope.channel == ChannelKind::Wechat {
        // Preserve the adapter's small-artifact-first delivery contract while
        // keeping the plan independent of mutable file size or existence.
        media_lines.sort_by_key(|line| {
            match parse_legacy_delivery_line(line).map(|token| token.kind) {
                Some(
                    claw_core::channel_delivery_tokens::LegacyDeliveryKind::File
                    | claw_core::channel_delivery_tokens::LegacyDeliveryKind::Auto,
                ) => 0,
                Some(claw_core::channel_delivery_tokens::LegacyDeliveryKind::Image) => 1,
                Some(
                    claw_core::channel_delivery_tokens::LegacyDeliveryKind::Voice
                    | claw_core::channel_delivery_tokens::LegacyDeliveryKind::Music,
                ) => 2,
                _ => 3,
            }
        });
    }
    for line in media_lines {
        if !out.iter().any(|v| v == line) {
            out.push(line.to_string());
        }
    }
    if out.len() > 128 {
        anyhow::bail!("channel_delivery_too_many_parts");
    }
    Ok(out)
}

fn local_file(state: &AppState, text: &str) -> Option<std::path::PathBuf> {
    let token = parse_legacy_delivery_line(text)?;
    if token.location != LegacyDeliveryLocation::LocalFile {
        return None;
    }
    let path = std::path::PathBuf::from(token.reference);
    Some(if path.is_absolute() {
        path
    } else {
        state.skill_rt.workspace_root.join(path)
    })
}

fn file_digest(path: &std::path::Path) -> anyhow::Result<String> {
    if !std::fs::metadata(path)?.is_file() {
        anyhow::bail!("channel_delivery_artifact_not_regular");
    }
    let mut file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        anyhow::bail!("channel_delivery_artifact_not_regular");
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hasher.update(&buffer[..size]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn load_or_create_plan(
    state: &AppState,
    task: &ClaimedTask,
    envelope: &ChannelDeliveryEnvelope,
    texts: &[String],
) -> anyhow::Result<Plan> {
    let db = state.core.db.get()?;
    crate::repo::channel_delivery_receipt::ensure_channel_delivery_receipt_schema(&db)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS channel_delivery_part_plans (
        idempotency_key TEXT PRIMARY KEY, plan_json TEXT NOT NULL, task_id TEXT NOT NULL,
        part_keys_json TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS idx_channel_delivery_part_plans_task ON channel_delivery_part_plans(task_id);
        CREATE TRIGGER IF NOT EXISTS delete_channel_delivery_part_plan_receipts
        AFTER DELETE ON channel_delivery_part_plans BEGIN
          DELETE FROM channel_delivery_dispatch_claims WHERE idempotency_key IN (SELECT value FROM json_each(OLD.part_keys_json));
          DELETE FROM channel_delivery_receipts WHERE idempotency_key IN (SELECT value FROM json_each(OLD.part_keys_json));
        END;
        CREATE TRIGGER IF NOT EXISTS delete_channel_delivery_part_plan_on_receipt
        AFTER DELETE ON channel_delivery_receipts BEGIN
          DELETE FROM channel_delivery_part_plans WHERE idempotency_key=OLD.idempotency_key;
        END;
        CREATE TRIGGER IF NOT EXISTS delete_channel_delivery_part_plan_on_task
        AFTER DELETE ON tasks BEGIN
          DELETE FROM channel_delivery_part_plans WHERE task_id=OLD.task_id;
        END;",
    )?;
    let identity_digest = digest(serde_json::to_string(&serde_json::json!({
        "task_id": task.task_id, "user_key": task.user_key, "user_id": task.user_id, "chat_id": task.chat_id,
        "channel": envelope.channel, "adapter": envelope.adapter, "recipient": envelope.reply_target,
        "texts": texts,
    }))?.as_bytes());
    let existing: Option<String> = db
        .query_row(
            "SELECT plan_json FROM channel_delivery_part_plans WHERE idempotency_key=?1",
            params![envelope.idempotency_key],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(raw) = existing {
        let plan: Plan = serde_json::from_str(&raw)?;
        if plan.schema_version != 1
            || plan.identity_digest != identity_digest
            || plan.file_digests.len() != texts.len()
        {
            anyhow::bail!("channel_delivery_plan_identity_conflict");
        }
        return Ok(plan);
    }
    let plan = Plan {
        schema_version: 1,
        identity_digest,
        file_digests: texts
            .iter()
            .map(|text| local_file(state, text).and_then(|path| file_digest(&path).ok()))
            .collect(),
    };
    db.execute(
        "INSERT OR IGNORE INTO channel_delivery_part_plans VALUES (?1, ?2, ?3, ?4)",
        params![
            envelope.idempotency_key,
            serde_json::to_string(&plan)?,
            task.task_id,
            serde_json::to_string(
                &texts
                    .iter()
                    .enumerate()
                    .map(|(i, text)| part_envelope(envelope, &plan, i, text).idempotency_key)
                    .collect::<Vec<_>>()
            )?
        ],
    )?;
    // A simultaneous writer may have installed another immutable plan.
    let stored: String = db.query_row(
        "SELECT plan_json FROM channel_delivery_part_plans WHERE idempotency_key=?1",
        params![envelope.idempotency_key],
        |row| row.get(0),
    )?;
    let stored: Plan = serde_json::from_str(&stored)?;
    if stored.identity_digest != plan.identity_digest || stored.file_digests != plan.file_digests {
        anyhow::bail!("channel_delivery_plan_identity_conflict");
    }
    Ok(stored)
}

fn part_envelope(
    envelope: &ChannelDeliveryEnvelope,
    plan: &Plan,
    index: usize,
    text: &str,
) -> ChannelDeliveryEnvelope {
    let mut child = envelope.clone();
    let id = digest(
        format!(
            "{}:{}:{}",
            envelope.idempotency_key, plan.identity_digest, index
        )
        .as_bytes(),
    );
    child.delivery_id = format!("delivery-part:{id}");
    child.idempotency_key = format!("part:{id}");
    child.text_segments = vec![ChannelTextSegment {
        text: text.to_string(),
        format: ChannelTextFormat::Plain,
    }];
    child.notice = None;
    child
}

pub(super) async fn deliver(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    envelope: &ChannelDeliveryEnvelope,
) -> anyhow::Result<ChannelDeliveryServiceResult> {
    envelope.validate().map_err(|e| anyhow!(e.to_string()))?;
    // Transport-only notices keep their existing idempotency and formatting contract.
    if envelope.source == ChannelDeliverySource::ProactiveNotice || envelope.notice.is_some() {
        return super::deliver_task_envelope_once(state, task, payload, envelope, true).await;
    }
    let texts = texts(state, envelope)?;
    if texts.len() <= 1 {
        return super::deliver_task_envelope_once(state, task, payload, envelope, true).await;
    }
    let state_for_plan = state.clone();
    let task_for_plan = task.clone();
    let envelope_for_plan = envelope.clone();
    let texts_for_plan = texts.clone();
    let plan = tokio::task::spawn_blocking(move || {
        load_or_create_plan(
            &state_for_plan,
            &task_for_plan,
            &envelope_for_plan,
            &texts_for_plan,
        )
    })
    .await??;
    let lease = match crate::repo::claim_channel_delivery_dispatch(
        &state.core.db,
        envelope,
        DELIVERY_DISPATCH_LEASE_SECONDS,
    )? {
        ClaimChannelDeliveryDispatchOutcome::Acquired { lease_token } => lease_token,
        ClaimChannelDeliveryDispatchOutcome::ExistingReceipt(receipt) => {
            let mut result = result_from_existing_receipt(receipt);
            attach_failed_files(state, &texts, &mut result);
            return Ok(result);
        }
        other => {
            return Ok(ChannelDeliveryServiceResult {
                status: if matches!(other, ClaimChannelDeliveryDispatchOutcome::InProgress) {
                    ChannelDeliveryServiceStatus::InProgress
                } else {
                    ChannelDeliveryServiceStatus::QueryRequired
                },
                receipt: None,
                error_code: None,
                message_key: None,
                retryable: false,
                failed_parts: Vec::new(),
            })
        }
    };
    let results = send_with_dispatch_lease(state, &envelope.idempotency_key, &lease, async {
        let mut results = Vec::new();
        for (index, text) in texts.iter().enumerate() {
            let child = part_envelope(envelope, &plan, index, text);
            let previous = {
                let db = state
                    .core
                    .db
                    .get()
                    .map_err(|_| "channel_delivery_receipt_db_failed")?;
                crate::repo::channel_delivery_receipt::load_channel_delivery_receipt_from_db(
                    &db,
                    &child.idempotency_key,
                )
                .map_err(|_| "channel_delivery_receipt_load_failed")?
            };
            if let Some(receipt) = previous.as_ref().filter(|r| {
                !r.retryable
                    || matches!(
                        r.status,
                        ChannelDeliveryStatus::Accepted
                            | ChannelDeliveryStatus::Delivered
                            | ChannelDeliveryStatus::Read
                    )
            }) {
                results.push(result_from_existing_receipt(receipt.clone()));
                continue;
            }
            if let Some(path) = local_file(state, text) {
                let actual = tokio::task::spawn_blocking(move || file_digest(&path))
                    .await
                    .map_err(|_| "channel_delivery_artifact_check_failed")?;
                if actual.ok().as_ref() != plan.file_digests[index].as_ref()
                    || plan.file_digests[index].is_none()
                {
                    results.push(
                        reject_artifact(state, &child)
                            .map_err(|_| "channel_delivery_artifact_check_failed")?,
                    );
                    continue;
                }
            }
            results.push(
                super::deliver_task_envelope_once(state, task, payload, &child, false)
                    .await
                    .map_err(|_| "channel_delivery_part_dispatch_failed")?,
            );
        }
        Ok(results)
    })
    .await
    .map_err(|e| anyhow!(e))?;
    let mut result = aggregate(envelope, &results);
    attach_failed_files(state, &texts, &mut result);
    crate::repo::record_channel_delivery_receipt(&state.core.db, result.receipt.as_ref().unwrap())?;
    crate::repo::complete_channel_delivery_dispatch(
        &state.core.db,
        &envelope.idempotency_key,
        &lease,
    )?;
    Ok(result)
}

fn attach_failed_files(
    state: &AppState,
    texts: &[String],
    result: &mut ChannelDeliveryServiceResult,
) {
    let Some(receipt) = result.receipt.as_ref() else {
        return;
    };
    for part in receipt
        .parts
        .iter()
        .filter(|part| part.status == ChannelDeliveryStatus::Failed)
    {
        let Some(path) = texts
            .get(part.part_index as usize)
            .and_then(|text| local_file(state, text))
        else {
            continue;
        };
        let path = path.canonicalize().unwrap_or(path);
        if result
            .failed_parts
            .iter()
            .any(|failure| failure.local_path.as_ref() == Some(&path))
        {
            continue;
        }
        result
            .failed_parts
            .push(crate::channel_send::ChannelSendPartFailure {
                part_kind: "artifact".into(),
                part_index: part.part_index as usize,
                local_path: Some(path),
                error_code: part
                    .error_code
                    .clone()
                    .unwrap_or_else(|| "channel.delivery.failed".into()),
                message_key: result
                    .message_key
                    .clone()
                    .unwrap_or_else(|| "channel.error.delivery_failed".into()),
                actual_bytes: None,
                max_bytes: None,
            });
    }
}

fn reject_artifact(
    state: &AppState,
    envelope: &ChannelDeliveryEnvelope,
) -> anyhow::Result<ChannelDeliveryServiceResult> {
    match crate::repo::claim_channel_delivery_dispatch(
        &state.core.db,
        envelope,
        DELIVERY_DISPATCH_LEASE_SECONDS,
    )? {
        ClaimChannelDeliveryDispatchOutcome::Acquired { lease_token } => {
            let mut receipt =
                accepted_delivery_receipt(envelope, Default::default(), crate::now_ts_u64());
            receipt.status = ChannelDeliveryStatus::Failed;
            receipt.error_code = Some("channel_delivery_artifact_changed_or_expired".into());
            receipt.message_key = Some("channel.media.preflight.unreadable".into());
            receipt.diagnostic_id = Some(format!("delivery:{}", Uuid::new_v4().simple()));
            crate::repo::record_channel_delivery_receipt(&state.core.db, &receipt)?;
            crate::repo::complete_channel_delivery_dispatch(
                &state.core.db,
                &envelope.idempotency_key,
                &lease_token,
            )?;
            Ok(result_from_existing_receipt(receipt))
        }
        ClaimChannelDeliveryDispatchOutcome::ExistingReceipt(receipt) => {
            Ok(result_from_existing_receipt(receipt))
        }
        other => Ok(ChannelDeliveryServiceResult {
            status: if matches!(other, ClaimChannelDeliveryDispatchOutcome::InProgress) {
                ChannelDeliveryServiceStatus::InProgress
            } else {
                ChannelDeliveryServiceStatus::QueryRequired
            },
            receipt: None,
            error_code: None,
            message_key: None,
            retryable: false,
            failed_parts: Vec::new(),
        }),
    }
}

fn aggregate(
    envelope: &ChannelDeliveryEnvelope,
    results: &[ChannelDeliveryServiceResult],
) -> ChannelDeliveryServiceResult {
    let all_accepted = results.iter().all(ChannelDeliveryServiceResult::accepted);
    let any_accepted = results.iter().any(ChannelDeliveryServiceResult::accepted);
    let retryable = results.iter().any(|r| r.retryable);
    let failed = results.iter().find(|r| !r.accepted());
    let receipt = ChannelDeliveryReceipt {
        schema_version: CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION,
        delivery_id: envelope.delivery_id.clone(),
        idempotency_key: envelope.idempotency_key.clone(),
        channel: envelope.channel,
        adapter: envelope.adapter.clone(),
        status: if all_accepted {
            ChannelDeliveryStatus::Accepted
        } else if any_accepted {
            ChannelDeliveryStatus::Partial
        } else {
            ChannelDeliveryStatus::Failed
        },
        // Only the aggregate parent owns the provider-message webhook index.
        provider_message_ids: results
            .iter()
            .filter_map(|v| v.receipt.as_ref())
            .flat_map(|v| {
                v.parts
                    .iter()
                    .filter_map(|part| part.provider_message_id.clone())
            })
            .collect(),
        parts: results
            .iter()
            .enumerate()
            .map(|(i, result)| ChannelDeliveryPartReceipt {
                part_index: i as u32,
                status: if result.accepted() {
                    result
                        .receipt
                        .as_ref()
                        .map(|v| v.status)
                        .unwrap_or(ChannelDeliveryStatus::Accepted)
                } else {
                    ChannelDeliveryStatus::Failed
                },
                provider_message_id: result.receipt.as_ref().and_then(|v| {
                    v.parts
                        .iter()
                        .find_map(|part| part.provider_message_id.clone())
                }),
                error_code: result.error_code.clone(),
            })
            .collect(),
        error_code: failed.map(|r| {
            r.error_code
                .clone()
                .unwrap_or_else(|| "channel_delivery_part_query_required".into())
        }),
        message_key: failed.map(|r| {
            r.message_key
                .clone()
                .unwrap_or_else(|| "channel.error.delivery_failed".into())
        }),
        diagnostic_id: failed.map(|r| {
            r.receipt
                .as_ref()
                .and_then(|v| v.diagnostic_id.clone())
                .unwrap_or_else(|| format!("delivery:{}", Uuid::new_v4().simple()))
        }),
        provider_error_code: None,
        retryable,
        updated_at_ts: crate::now_ts_u64(),
    };
    let mut result = result_from_existing_receipt(receipt);
    result.failed_parts = results
        .iter()
        .flat_map(|v| v.failed_parts.clone())
        .collect();
    result
}

#[cfg(test)]
#[path = "parts_tests.rs"]
mod tests;

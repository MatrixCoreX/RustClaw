//! Model-written, text-only explanations of terminal attachment delivery failures.
//! The original delivery receipt remains authoritative; this is a separate send.

use std::path::Path;

use anyhow::{anyhow, Context};
use claw_core::channel_delivery::{
    ChannelDeliveryEnvelope, ChannelDeliverySource, ChannelDeliveryStatus,
};
use claw_core::channel_provider_error::{ChannelProviderError, ChannelProviderFailureClass};
use claw_core::wechat_reply_media::{
    extract_wechat_outbound_media, WechatOutboundKind, WechatOutboundSource,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{ChannelDeliveryServiceResult, ChannelDeliveryServiceStatus};
use crate::{AppState, ClaimedTask};

const PROMPT: &str = "prompts/layers/overlays/channel_delivery_failure_notice.md";

// Decode only existing, language-neutral preflight protocol records, never provider prose.
pub(super) fn decode_local_media_error(raw: &str) -> Option<ChannelProviderError> {
    let record = raw
        .strip_prefix("channel_media_preflight_failed:")
        .or_else(|| raw.strip_prefix("whatsapp_cloud_media_preflight_failed:"))?;
    let mut fields = record.split(':');
    let reason = fields.next()?;
    if !matches!(
        reason,
        "channel_media_too_large"
            | "channel_media_empty"
            | "channel_media_unreadable"
            | "channel_media_not_regular_file"
    ) {
        return None;
    }
    fields.next()?.parse::<u64>().ok()?;
    let limit = fields.next()?;
    if limit != "none" {
        limit.parse::<u64>().ok()?;
    }
    if fields.next().is_some() {
        return None;
    }
    Some(ChannelProviderError::from_machine_failure(
        "local_media_preflight",
        "send_media",
        ChannelProviderFailureClass::PayloadRejected,
        None,
        Some(reason),
        None,
        raw,
    ))
}

#[derive(Debug, Serialize)]
struct FileLocation {
    kind: &'static str,
    filename: String,
    path: String,
    directory: String,
    size_bytes: Option<u64>,
    exists: bool,
}

fn file_locations(envelope: &ChannelDeliveryEnvelope, workspace: &Path) -> Vec<FileLocation> {
    let text = envelope
        .text_segments
        .iter()
        .map(|part| part.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    extract_wechat_outbound_media(&text, workspace)
        .into_iter()
        .filter_map(|media| {
            let WechatOutboundSource::LocalPath(path) = media.source else {
                return None;
            };
            // Use the same machine delivery references as the sender; never search other tasks.
            let path = path.canonicalize().unwrap_or(path);
            let metadata = path.metadata().ok().filter(|metadata| metadata.is_file());
            Some(FileLocation {
                kind: match media.kind {
                    WechatOutboundKind::Image => "image",
                    WechatOutboundKind::Video => "video",
                    WechatOutboundKind::Audio => "audio",
                    WechatOutboundKind::File => "file",
                },
                filename: path.file_name()?.to_string_lossy().into_owned(),
                directory: path.parent()?.to_string_lossy().into_owned(),
                path: path.to_string_lossy().into_owned(),
                size_bytes: metadata.as_ref().map(|metadata| metadata.len()),
                exists: metadata.is_some(),
            })
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelNotice {
    text: String,
}

fn notice_text(raw: &str, files: &[FileLocation], workspace: &Path) -> anyhow::Result<String> {
    let notice = crate::prompt_utils::parse_llm_json_raw_or_any::<ModelNotice>(raw.trim())
        .ok_or_else(|| anyhow!("delivery_failure_notice_model_response_invalid"))?;
    let text = notice.text.trim();
    if text.is_empty() || !extract_wechat_outbound_media(text, workspace).is_empty() {
        return Err(anyhow!("delivery_failure_notice_not_plain_text"));
    }
    // Literal path validation is a delivery contract, not natural-language matching.
    if files.iter().any(|file| !text.contains(&file.path)) {
        return Err(anyhow!("delivery_failure_notice_file_path_missing"));
    }
    Ok(text.to_string())
}

fn should_notify(
    envelope: &ChannelDeliveryEnvelope,
    result: &ChannelDeliveryServiceResult,
) -> bool {
    envelope.source != ChannelDeliverySource::ProactiveNotice
        && result.status == ChannelDeliveryServiceStatus::Failed
        && !result.retryable
        && result.receipt.as_ref().is_some_and(|receipt| {
            matches!(
                receipt.status,
                ChannelDeliveryStatus::Failed | ChannelDeliveryStatus::Partial
            )
        })
}

pub(super) async fn deliver(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    original: &ChannelDeliveryEnvelope,
    result: &ChannelDeliveryServiceResult,
) -> anyhow::Result<()> {
    if !should_notify(original, result) {
        return Ok(());
    }
    let files = file_locations(original, &state.skill_rt.workspace_root);
    if files.is_empty() {
        return Ok(());
    }
    let mut notice = original.clone();
    notice.delivery_id.push_str(":failure-notice");
    notice.idempotency_key.push_str(":failure-notice");
    // Keep the original reply authorization/context. This is a delivery follow-up,
    // not an unsolicited proactive message (and scheduled replies need fresh context).
    notice.artifacts.clear();
    notice.previews.clear();
    notice.notice = None;

    let existing = crate::repo::channel_delivery_receipt::load_channel_delivery_receipt_from_db(
        &*state.core.db.get()?,
        &notice.idempotency_key,
    )?;
    if let Some(receipt) = existing {
        // Never replay an accepted/partial notice, or an explicitly permanent rejection.
        if receipt.status != ChannelDeliveryStatus::Failed || !receipt.retryable {
            return Ok(());
        }
    }
    let receipt = result.receipt.as_ref().expect("checked by should_notify");
    let request = crate::language_policy::task_original_user_text(task).unwrap_or_default();
    let language = if original.locale != "und" {
        original.locale.clone()
    } else {
        crate::language_policy::task_response_language_hint(state, task, &request)
    };
    let context = json!({
        "language": language,
        "channel": original.channel,
        "adapter": original.adapter,
        "delivery_status": receipt.status,
        "error_code": receipt.error_code,
        "message_key": receipt.message_key,
        "provider_error_code": receipt.provider_error_code,
        "accepted_part_count": receipt.provider_message_ids.len(),
        "individual_file_delivery_status": "not_individually_confirmed",
        "files": files,
    });
    let (template, source) =
        crate::bootstrap::load_required_prompt_template_for_state(state, PROMPT)
            .map_err(|_| anyhow!("delivery_failure_notice_prompt_unavailable"))?;
    let prompt =
        crate::render_prompt_template(&template, &[("__CONTEXT_JSON__", &context.to_string())]);
    crate::log_prompt_render(
        state,
        &task.task_id,
        "channel_delivery_failure_notice",
        &source,
        None,
    );
    let raw =
        crate::llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &source)
            .await
            .map_err(|_| anyhow!("delivery_failure_notice_model_unavailable"))?;
    let text = notice_text(&raw, &files, &state.skill_rt.workspace_root)?;
    notice.text_segments = vec![claw_core::channel_delivery::ChannelTextSegment {
        text,
        format: claw_core::channel_delivery::ChannelTextFormat::Plain,
    }];
    // The inner dispatch keeps its own receipt/lease and does not recursively create notices.
    let notice_result = super::deliver_task_envelope_once(state, task, payload, &notice)
        .await
        .context("delivery_failure_notice_dispatch_failed")?;
    if matches!(
        notice_result.status,
        ChannelDeliveryServiceStatus::InProgress | ChannelDeliveryServiceStatus::QueryRequired
    ) || notice_result.retryable
    {
        return Err(anyhow!("delivery_failure_notice_pending"));
    }
    // The caller returns the original failure, never the successful explanation's receipt.
    Ok(())
}

#[cfg(test)]
#[path = "failure_notice_tests.rs"]
mod tests;

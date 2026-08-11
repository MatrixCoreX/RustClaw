use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use claw_core::channel_notice::{ChannelNotice, ChannelNoticeSeverity};
use serde_json::{json, Value};

const RUNTIME_DELIVERY_OWNER: &str = "runtime";
const MINIMUM_NOTICE_INTERVAL_SECONDS: u64 = 15 * 60;

#[derive(Debug, Clone)]
pub(crate) struct RuntimeProgressNotice {
    pub(crate) interval: Duration,
    pub(crate) notice: ChannelNotice,
    pub(crate) sequence: u64,
}

fn public_param(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("null".to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) if value.len() <= 256 => Some(value.clone()),
        Value::Array(values) if values.len() <= 16 => values
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
            .map(|values| values.join(",")),
        _ => None,
    }
}

pub(crate) fn runtime_progress_notice(
    frame: &skill_sdk::SkillProgressFrame,
) -> Option<RuntimeProgressNotice> {
    if !matches!(frame.kind, skill_sdk::SkillProgressKind::Heartbeat)
        || frame
            .params
            .get("notification_delivery")
            .and_then(Value::as_str)
            != Some(RUNTIME_DELIVERY_OWNER)
    {
        return None;
    }
    let requested_interval = frame
        .params
        .get("notification_interval_seconds")
        .and_then(Value::as_u64)?;
    if requested_interval < MINIMUM_NOTICE_INTERVAL_SECONDS {
        return None;
    }
    let message_key = frame.params.get("message_key")?.as_str()?.trim();
    if !message_key.starts_with("channel.notice.") {
        return None;
    }

    let params = frame
        .params
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "notification_delivery" | "notification_interval_seconds" | "message_key"
            )
        })
        .filter_map(|(name, value)| public_param(value).map(|value| (name.clone(), value)))
        .collect::<BTreeMap<_, _>>();
    let mut notice = ChannelNotice::status(
        frame.detail_key.clone(),
        message_key.to_string(),
        ChannelNoticeSeverity::Info,
    );
    notice.params = params;
    notice.validate().ok()?;
    Some(RuntimeProgressNotice {
        interval: Duration::from_secs(requested_interval),
        notice,
        sequence: frame.sequence,
    })
}

pub(crate) fn due_runtime_progress_notice(
    frame: &skill_sdk::SkillProgressFrame,
    last_delivered_at: Option<Instant>,
    now: Instant,
) -> Option<RuntimeProgressNotice> {
    let notice = runtime_progress_notice(frame)?;
    if last_delivered_at.is_some_and(|last| now.duration_since(last) < notice.interval) {
        return None;
    }
    Some(notice)
}

fn delivery_payload(task: &crate::ClaimedTask) -> Value {
    let mut payload =
        serde_json::from_str::<Value>(&task.payload_json).unwrap_or_else(|_| json!({}));
    if let Value::Object(fields) = &mut payload {
        fields.insert("channel".to_string(), Value::String(task.channel.clone()));
        if let Some(value) = task.external_user_id.as_ref() {
            fields
                .entry("external_user_id".to_string())
                .or_insert_with(|| Value::String(value.clone()));
        }
        if let Some(value) = task.external_chat_id.as_ref() {
            fields
                .entry("external_chat_id".to_string())
                .or_insert_with(|| Value::String(value.clone()));
        }
    }
    payload
}

pub(crate) async fn deliver_runtime_progress_notice(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    progress: &RuntimeProgressNotice,
) -> Result<(), String> {
    if task.channel.trim().eq_ignore_ascii_case("ui") {
        return Ok(());
    }
    let now = crate::now_ts_u64();
    let recent_delivery = state
        .core
        .db
        .get()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT MAX(updated_at_ts) FROM channel_delivery_receipts
             WHERE delivery_id LIKE ?1",
            rusqlite::params![format!("delivery:{}:progress-%", task.task_id)],
            |row| row.get::<_, Option<u64>>(0),
        )
        .map_err(|error| error.to_string())?;
    if recent_delivery.is_some_and(|last| now.saturating_sub(last) < progress.interval.as_secs()) {
        return Ok(());
    }
    let payload = delivery_payload(task);
    let suffix = format!("progress-{}", progress.sequence);
    let envelope = crate::delivery_service::build_proactive_notice_envelope(
        state,
        task,
        &payload,
        &suffix,
        progress.notice.clone(),
    )
    .map_err(|error| error.to_string())?;
    let result = crate::delivery_service::deliver_task_envelope(state, task, &payload, &envelope)
        .await
        .map_err(|error| error.to_string())?;
    if result.accepted() {
        Ok(())
    } else {
        Err(result
            .error_code
            .unwrap_or_else(|| "channel_progress_delivery_not_accepted".to_string()))
    }
}

pub(crate) async fn project_durable_progress_lines(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    skill_name: &str,
    skill_version: &str,
    lines: &str,
) {
    for frame in validated_progress_frames(lines, &task.task_id) {
        let payload = json!({
            "schema_version": 1,
            "source": "skill_progress",
            "data_only": true,
            "render_owner": "ui_cli_channel_projection",
            "skill_name": skill_name,
            "skill_version": skill_version,
            "frame": &frame,
        });
        if let Err(error) = crate::task_event_transport::publish_claimed_event(
            state,
            task,
            "skill_progress",
            payload,
        ) {
            tracing::warn!(
                skill = skill_name,
                error = %error,
                "durable_skill_progress_event_publish_failed"
            );
        }
        if let Some(progress) = runtime_progress_notice(&frame) {
            if let Err(error) = deliver_runtime_progress_notice(state, task, &progress).await {
                tracing::warn!(
                    skill = skill_name,
                    sequence = progress.sequence,
                    error = %error,
                    "durable_skill_progress_channel_delivery_failed"
                );
            }
        }
    }
}

pub(crate) fn validated_progress_frames(
    lines: &str,
    request_id: &str,
) -> Vec<skill_sdk::SkillProgressFrame> {
    lines
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let record = serde_json::from_str::<Value>(line).ok()?;
            (record.get("record_type").and_then(Value::as_str)
                == Some(skill_sdk::SKILL_PROGRESS_FRAME_RECORD_TYPE))
            .then_some(record)
        })
        .filter_map(|record| {
            let encoded = serde_json::to_vec(&record).ok()?;
            skill_sdk::validate_progress_frame_line(&encoded, request_id).ok()
        })
        .collect()
}

#[cfg(test)]
#[path = "progress_delivery_tests.rs"]
mod tests;

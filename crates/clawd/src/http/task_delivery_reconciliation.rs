use crate::{
    repo::channel_delivery_receipt::reconciliation::{self, Decision},
    AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use claw_core::types::{ApiResponse, TaskQueryResponse};
use serde_json::{json, Value};
use uuid::Uuid;

pub(crate) fn attach_status(
    state: &AppState,
    task: &mut TaskQueryResponse,
    channel: &str,
    may_reconcile: bool,
) {
    let mut delivery = if channel == "ui" {
        json!({"schema_version":1,"status":"not_required","can_reconcile":false})
    } else {
        state
            .core
            .db
            .get()
            .ok()
            .and_then(|db| {
                reconciliation::inspect(&db, &task.task_id.to_string(), crate::now_ts_u64() as i64)
                    .ok()
            })
            .unwrap_or_else(
                || json!({"schema_version":1,"status":"unavailable","can_reconcile":false}),
            )
    };
    let terminal = matches!(
        task.status,
        claw_core::types::TaskStatus::Succeeded
            | claw_core::types::TaskStatus::Failed
            | claw_core::types::TaskStatus::Canceled
            | claw_core::types::TaskStatus::Timeout
    );
    if !may_reconcile || !terminal {
        delivery["can_reconcile"] = json!(false);
    }
    let result = task.result_json.get_or_insert_with(|| json!({}));
    if !result.is_object() {
        return;
    }
    if result.get("task_journal").is_none() {
        result["task_journal"] = json!({"summary":{}});
    }
    if result["task_journal"].is_object() {
        if result["task_journal"].get("summary").is_none() {
            result["task_journal"]["summary"] = json!({});
        }
        if result["task_journal"]["summary"].is_object() {
            result["task_journal"]["summary"]["delivery"] = delivery;
        }
    }
}

#[cfg(test)]
#[path = "task_delivery_reconciliation_tests.rs"]
mod tests;

pub(crate) async fn reconcile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
    Json(decision): Json<Decision>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let record = match crate::repo::get_task_delivery_record(&state, &task_id.to_string()) {
        Ok(Some(record)) => record,
        Ok(None) => return crate::api_err(StatusCode::NOT_FOUND, "task_not_found"),
        Err(_) => {
            return crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_delivery_lookup_failed",
            )
        }
    };
    let Ok(payload) = serde_json::from_str(&record.task.payload_json) else {
        return crate::api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "task_delivery_payload_invalid",
        );
    };
    if !super::task_delivery::authorized_delivery_request(&state, &headers, &record, &payload) {
        return crate::api_err(StatusCode::UNAUTHORIZED, "task_delivery_unauthorized");
    }
    let Ok(identity) = super::task_delivery::current_delivery_identity(&state, &record, &payload)
    else {
        return crate::api_err(StatusCode::UNAUTHORIZED, "task_delivery_unauthorized");
    };
    let result = state
        .core
        .db
        .get()
        .map_err(anyhow::Error::from)
        .and_then(|mut db| {
            reconciliation::reconcile(
                &mut db,
                &task_id.to_string(),
                &identity,
                &decision,
                crate::now_ts_u64() as i64,
            )
        });
    match result {
        Ok(value) => crate::api_ok(value),
        Err(error) => {
            let code = error.to_string();
            match code.as_str() {
                "task_delivery_unauthorized" => {
                    crate::api_err(StatusCode::UNAUTHORIZED, "task_delivery_unauthorized")
                }
                "delivery_snapshot_changed"
                | "delivery_dispatch_active"
                | "delivery_decision_conflict" => crate::api_err(StatusCode::CONFLICT, &code),
                "delivery_snapshot_invalid"
                | "delivery_duplicate_risk_confirmation_required"
                | "delivery_no_eligible_parts"
                | "delivery_dispatch_missing" => crate::api_err(StatusCode::BAD_REQUEST, &code),
                _ => crate::api_err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "task_delivery_reconciliation_failed",
                ),
            }
        }
    }
}

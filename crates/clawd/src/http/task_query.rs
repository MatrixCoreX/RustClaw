use crate::*;

pub(crate) async fn get_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<Uuid>,
) -> (StatusCode, Json<ApiResponse<TaskQueryResponse>>) {
    let read_result = get_task_query_record(&state, task_id);

    match read_result {
        Ok(Some((mut task, task_user_key, channel))) => {
            let provided_key = auth_key_from_headers(&headers).map(str::to_string);
            match check_task_view_access(
                &state,
                task_user_key.as_deref(),
                &channel,
                provided_key.as_deref(),
            ) {
                Ok(()) => {}
                Err(TaskViewerAccessError::AuthLookup(err)) => {
                    error!("Resolve task viewer failed: {}", err);
                    return api_err::<TaskQueryResponse>(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Auth lookup failed",
                    );
                }
                Err(TaskViewerAccessError::TaskOwnerMismatch) => {
                    return api_err::<TaskQueryResponse>(
                        StatusCode::UNAUTHORIZED,
                        "task_owner_mismatch",
                    );
                }
                Err(TaskViewerAccessError::InvalidUserKey) => {
                    return api_err::<TaskQueryResponse>(
                        StatusCode::UNAUTHORIZED,
                        "auth_key_invalid",
                    );
                }
            }
            let may_reconcile = repo::get_task_delivery_record(&state, &task_id.to_string())
                .ok()
                .flatten()
                .is_some_and(|record| {
                    serde_json::from_str(&record.task.payload_json)
                        .ok()
                        .is_some_and(|payload| {
                            super::task_delivery::authorized_delivery_request(
                                &state, &headers, &record, &payload,
                            )
                        })
                });
            super::task_delivery_reconciliation::attach_status(
                &state,
                &mut task,
                &channel,
                may_reconcile,
            );
            api_ok(crate::visible_text::sanitize_task_query_response_for_delivery(task))
        }
        Ok(None) => api_err::<TaskQueryResponse>(StatusCode::NOT_FOUND, "Task not found"),
        Err(err) => {
            error!("Read task failed: {}", err);
            api_err::<TaskQueryResponse>(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
        }
    }
}

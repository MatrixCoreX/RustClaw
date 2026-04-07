pub(crate) mod audit;
pub(crate) mod auth;
pub(crate) mod submit;
pub(crate) mod tasks;

pub(crate) use audit::{insert_audit_log, insert_audit_log_raw};
#[allow(unused_imports)]
pub(crate) use auth::{
    attach_pending_channel_bind_session_install_flow, bind_channel_identity, create_auth_key,
    create_pending_channel_bind_session, delete_auth_key_by_id, ensure_bootstrap_admin_key,
    ensure_key_auth_schema, exchange_credential_status_for_user_key,
    finalize_pending_channel_bind_session, get_auth_key_value_by_id,
    get_pending_channel_bind_session_by_id, get_pending_channel_bind_session_by_token,
    list_auth_keys, mark_pending_channel_bind_session_detected, rotate_auth_key_by_user_key,
    mark_pending_channel_bind_session_expired, mark_pending_channel_bind_session_failed,
    normalize_user_key, resolve_auth_identity_by_key,
    resolve_channel_binding_identity, seed_channel_bindings, update_auth_key_by_id,
    upsert_exchange_credential_for_user_key, upsert_webd_login_account,
    verify_webd_password_login, PendingChannelBindSession,
};
pub(crate) use submit::{
    build_conversation_chat_id, build_submit_task_payload, check_submit_task_access,
    check_submit_task_limits, find_recent_failed_resume_context, insert_submitted_task,
    is_user_allowed, maybe_find_submit_task_dedup, resolve_submit_task_context,
    stable_i64_from_key, submit_task_audit_detail, task_count_by_status, task_kind_name,
    SubmitTaskAccessError, SubmitTaskContextError, SubmitTaskLimitError,
};
pub(crate) use tasks::{
    cancel_one_task_for_user_chat, cancel_tasks_for_user_chat, check_task_view_access,
    claim_next_task, get_task_query_record, is_task_still_running, list_active_tasks_internal,
    touch_running_task, update_task_failure, update_task_failure_with_result,
    update_task_progress_result, update_task_success, update_task_timeout, TaskViewerAccessError,
};

pub(crate) mod approval_scope;
pub(crate) mod audit;
pub(crate) mod auth;
pub(crate) mod child_patch;
pub(crate) mod child_task_control;
pub(crate) mod child_tasks;
pub(crate) mod submit;
pub(crate) mod task_admin;
pub(crate) mod task_approval;
pub(crate) mod task_goal;
pub(crate) mod task_mutation_ledger;
pub(crate) mod task_resume_execution;
pub(crate) mod tasks;

pub(crate) use approval_scope::{
    list_approval_scope_grants, match_approval_scope_grant, revoke_approval_scope_grant,
};
pub(crate) use audit::{insert_audit_log, insert_audit_log_raw};
pub(crate) use auth::{
    attach_pending_channel_bind_session_install_flow, bind_channel_identity, create_auth_key,
    create_pending_channel_bind_session, delete_auth_key_by_id, ensure_bootstrap_admin_key,
    ensure_key_auth_schema, exchange_credential_status_for_user_key, factory_reset_auth_state,
    finalize_pending_channel_bind_session, get_auth_key_value_by_id,
    get_pending_channel_bind_session_by_id, get_pending_channel_bind_session_by_token,
    has_channel_binding_for_user_key, list_auth_keys, mark_pending_channel_bind_session_detected,
    mark_pending_channel_bind_session_expired, mark_pending_channel_bind_session_failed,
    normalize_user_key, reset_channel_binding_state_for_user_key, resolve_auth_identity_by_key,
    resolve_channel_binding_identity, seed_channel_bindings, update_auth_key_by_id,
    upsert_exchange_credential_for_user_key, upsert_webd_login_account, verify_webd_password_login,
    FactoryResetDbResult, PendingChannelBindSession,
};
pub(crate) use child_task_control::retry_child_task_with_revised_goal;
pub(crate) use submit::{
    build_conversation_chat_id, build_submit_task_payload, check_submit_task_access,
    check_submit_task_limits, insert_submitted_task, is_user_allowed, maybe_find_submit_task_dedup,
    resolve_submit_task_context, stable_i64_from_key, submit_task_audit_detail,
    task_count_by_status, task_kind_name, SubmitTaskAccessError, SubmitTaskContextError,
    SubmitTaskLimitError,
};
pub(crate) use task_admin::{
    cancel_one_task_for_user_chat, cancel_task_by_id, cancel_tasks_for_user_chat,
    get_task_admin_target, pause_task_by_id, resume_task_with_input, TaskAdminTarget,
    TaskResumeControlInput,
};
pub(crate) use task_approval::{
    consume_task_approval_grant, decide_task_approval_request_for_actor, TaskApprovalConsumeOutcome,
};
pub(crate) use task_goal::{update_task_goal_payload, TaskGoalControlOperation};
pub(crate) use task_mutation_ledger::{
    begin_task_mutation, complete_task_mutation, mark_task_mutation_uncertain,
    BeginTaskMutationOutcome, TaskMutationLease, TaskMutationRecord,
};
pub(crate) use task_resume_execution::record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal;
pub(crate) use task_resume_execution::{
    claim_dispatched_paused_checkpoint_resume_execution_internal,
    claim_handoff_paused_checkpoint_resume_execution_internal,
    claim_recorded_paused_checkpoint_resume_dispatch_result_internal,
    list_dispatched_paused_checkpoint_resume_executions_internal,
    list_handoff_paused_checkpoint_resume_executions_internal,
    list_planned_paused_checkpoint_resume_executions_internal,
    list_recorded_paused_checkpoint_resume_dispatch_results_internal,
    record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal,
    record_claimed_handoff_paused_checkpoint_resume_dispatch_internal,
    record_planned_paused_checkpoint_resume_handoff_internal,
    renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal,
    ClaimedDispatchedPausedCheckpointResumeExecution,
    ClaimedHandoffPausedCheckpointResumeExecution, ClaimedPausedCheckpointResumeDispatchResult,
};
pub(crate) use tasks::{
    check_task_view_access, claim_due_paused_checkpoint_task_internal, claim_next_task,
    claim_ready_paused_checkpoint_resume_executor_internal, get_task_query_record,
    is_task_still_running, is_task_still_running_or_pending_ask_success_projection,
    list_active_tasks_internal, list_due_paused_checkpoint_tasks_internal,
    list_ready_paused_checkpoint_resume_executors_internal,
    record_paused_checkpoint_resume_execution_plan_internal,
    record_paused_checkpoint_resume_executor_state_internal,
    record_paused_checkpoint_resume_work_item_internal, touch_running_task,
    update_task_checkpointed_result, update_task_failure, update_task_failure_with_result,
    update_task_progress_result, update_task_success, update_task_timeout,
    worker_task_lease_expires_at, ClaimedPausedCheckpointResumeExecutor, DuePausedCheckpointTask,
    TaskViewerAccessError,
};

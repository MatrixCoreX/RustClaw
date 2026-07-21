use axum::http::StatusCode;
use claw_core::config::{ToolApprovalPolicy, ToolSandboxMode};
use claw_core::types::AuthIdentity;
use serde_json::{json, Value};

use crate::{AppState, ClaimedTask};

pub(crate) const CLIENT_ORIGIN_HEADER: &str = "x-rustclaw-client";
pub(crate) const EXECUTION_MODE_HEADER: &str = "x-rustclaw-execution-mode";
pub(crate) const POLICY_PAYLOAD_FIELD: &str = "_rustclaw_execution_policy";
const CLAWCLI_ORIGIN: &str = "clawcli";
const YOLO_MODE: &str = "yolo";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskExecutionMode {
    Configured,
    Yolo,
}

impl TaskExecutionMode {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Yolo => YOLO_MODE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TaskExecutionPolicy {
    pub(crate) mode: TaskExecutionMode,
    pub(crate) approval_policy: ToolApprovalPolicy,
    pub(crate) sandbox_mode: ToolSandboxMode,
    pub(crate) derivation: &'static str,
    pub(crate) actor_role: Option<&'static str>,
}

impl TaskExecutionPolicy {
    pub(crate) fn approval_required(
        self,
        risk_requires_approval: bool,
        planner_requested_approval: bool,
        mutates_or_external: bool,
    ) -> bool {
        crate::ToolsPolicy::approval_required_for_policy(
            self.approval_policy,
            risk_requires_approval,
            planner_requested_approval,
            mutates_or_external,
        )
    }

    pub(crate) fn sandbox_denial(
        self,
        requirements: crate::runtime::policy::SandboxRequirements<'_>,
    ) -> Option<&'static str> {
        crate::ToolsPolicy::sandbox_denial_for_mode(self.sandbox_mode, requirements)
    }

    pub(crate) fn to_machine_json(self) -> Value {
        json!({
            "schema_version": 1,
            "mode": self.mode.as_token(),
            "derivation": self.derivation,
            "actor_role": self.actor_role,
            "approval_policy": self.approval_policy.as_token(),
            "sandbox_mode": self.sandbox_mode.as_token(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubmissionPolicyError {
    UnsupportedExecutionMode,
    AdminRequired,
    PayloadObjectRequired,
}

impl SubmissionPolicyError {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::UnsupportedExecutionMode => "execution_mode_unsupported",
            Self::AdminRequired => "yolo_mode_admin_required",
            Self::PayloadObjectRequired => "yolo_mode_payload_object_required",
        }
    }

    pub(crate) fn status_code(self) -> StatusCode {
        match self {
            Self::UnsupportedExecutionMode => StatusCode::BAD_REQUEST,
            Self::AdminRequired => StatusCode::FORBIDDEN,
            Self::PayloadObjectRequired => StatusCode::BAD_REQUEST,
        }
    }
}

pub(crate) fn stamp_authenticated_submission_policy(
    payload: &mut Value,
    identity: Option<&AuthIdentity>,
    client_origin: Option<&str>,
    requested_execution_mode: Option<&str>,
) -> Result<(), SubmissionPolicyError> {
    if let Some(object) = payload.as_object_mut() {
        object.remove(POLICY_PAYLOAD_FIELD);
    }
    let requested_mode = requested_execution_mode
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    if requested_mode
        .as_deref()
        .is_some_and(|value| value != YOLO_MODE)
    {
        return Err(SubmissionPolicyError::UnsupportedExecutionMode);
    }
    let admin = identity.is_some_and(|identity| identity.role.eq_ignore_ascii_case("admin"));
    if requested_mode.as_deref() == Some(YOLO_MODE) && !admin {
        return Err(SubmissionPolicyError::AdminRequired);
    }
    let clawcli = client_origin
        .map(str::trim)
        .is_some_and(|origin| origin.eq_ignore_ascii_case(CLAWCLI_ORIGIN));
    if !admin || (clawcli && requested_mode.as_deref() != Some(YOLO_MODE)) {
        return Ok(());
    }
    if !payload.is_object() {
        return Err(SubmissionPolicyError::PayloadObjectRequired);
    }
    let derivation = if clawcli {
        "clawcli_explicit_admin"
    } else {
        "admin_channel_default"
    };
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            POLICY_PAYLOAD_FIELD.to_string(),
            json!({
                "schema_version": 1,
                "mode": YOLO_MODE,
                "authority": "authenticated_admin",
                "actor_role": "admin",
                "derivation": derivation,
                "approval_policy": ToolApprovalPolicy::Never.as_token(),
                "sandbox_mode": ToolSandboxMode::DangerFull.as_token(),
            }),
        );
    }
    Ok(())
}

pub(crate) fn effective_policy_for_task(
    state: &AppState,
    task: &ClaimedTask,
) -> TaskExecutionPolicy {
    let configured = || configured_policy(state);
    let payload = match serde_json::from_str::<Value>(&task.payload_json) {
        Ok(payload) => payload,
        Err(_) => return configured(),
    };
    let Some(policy) = payload.get(POLICY_PAYLOAD_FIELD) else {
        return configured();
    };
    if !valid_yolo_stamp(policy) || !task_has_current_admin_identity(state, task) {
        return configured();
    }
    TaskExecutionPolicy {
        mode: TaskExecutionMode::Yolo,
        approval_policy: ToolApprovalPolicy::Never,
        sandbox_mode: ToolSandboxMode::DangerFull,
        derivation: match policy.get("derivation").and_then(Value::as_str) {
            Some("clawcli_explicit_admin") => "clawcli_explicit_admin",
            Some("admin_channel_default") => "admin_channel_default",
            _ => "authenticated_admin_stamp",
        },
        actor_role: Some("admin"),
    }
}

pub(crate) fn execution_policy_authorization_error(
    state: &AppState,
    task: &ClaimedTask,
) -> Option<&'static str> {
    let payload = serde_json::from_str::<Value>(&task.payload_json).ok()?;
    let policy = payload.get(POLICY_PAYLOAD_FIELD)?;
    if !valid_yolo_stamp(policy) {
        return Some("task_execution_policy_invalid");
    }
    if !task_has_current_admin_identity(state, task) {
        return Some("yolo_mode_admin_authority_expired");
    }
    None
}

pub(crate) fn configured_policy(state: &AppState) -> TaskExecutionPolicy {
    TaskExecutionPolicy {
        mode: TaskExecutionMode::Configured,
        approval_policy: state.skill_rt.tools_policy.approval_policy,
        sandbox_mode: state.skill_rt.tools_policy.sandbox_mode,
        derivation: "configured_tools_policy",
        actor_role: None,
    }
}

pub(crate) fn inheritable_policy_stamp(state: &AppState, task: &ClaimedTask) -> Option<Value> {
    let policy = effective_policy_for_task(state, task);
    if policy.mode != TaskExecutionMode::Yolo {
        return None;
    }
    Some(json!({
        "schema_version": 1,
        "mode": YOLO_MODE,
        "authority": "authenticated_admin",
        "actor_role": "admin",
        "derivation": "authenticated_parent_task",
        "approval_policy": ToolApprovalPolicy::Never.as_token(),
        "sandbox_mode": ToolSandboxMode::DangerFull.as_token(),
    }))
}

pub(crate) fn stamped_execution_mode(payload: &Value) -> &'static str {
    if payload
        .get(POLICY_PAYLOAD_FIELD)
        .and_then(|policy| policy.get("mode"))
        .and_then(Value::as_str)
        == Some(YOLO_MODE)
    {
        YOLO_MODE
    } else {
        "configured"
    }
}

fn task_has_current_admin_identity(state: &AppState, task: &ClaimedTask) -> bool {
    task.user_key
        .as_deref()
        .and_then(|key| {
            crate::resolve_auth_identity_by_key(state, key)
                .ok()
                .flatten()
        })
        .is_some_and(|identity| identity.role.eq_ignore_ascii_case("admin"))
}

fn valid_yolo_stamp(policy: &Value) -> bool {
    policy.get("schema_version").and_then(Value::as_u64) == Some(1)
        && policy.get("mode").and_then(Value::as_str) == Some(YOLO_MODE)
        && policy.get("authority").and_then(Value::as_str) == Some("authenticated_admin")
        && policy.get("actor_role").and_then(Value::as_str) == Some("admin")
        && policy.get("approval_policy").and_then(Value::as_str)
            == Some(ToolApprovalPolicy::Never.as_token())
        && policy.get("sandbox_mode").and_then(Value::as_str)
            == Some(ToolSandboxMode::DangerFull.as_token())
}

#[cfg(test)]
#[path = "task_execution_policy_tests.rs"]
mod tests;

use axum::http::StatusCode;
use claw_core::config::{ToolApprovalPolicy, ToolSandboxMode};
use claw_core::types::AuthIdentity;
use serde_json::{json, Value};

use crate::{AppState, ClaimedTask};

pub(crate) const CLIENT_ORIGIN_HEADER: &str = "x-rustclaw-client";
pub(crate) const EXECUTION_MODE_HEADER: &str = "x-rustclaw-execution-mode";
pub(crate) const POLICY_PAYLOAD_FIELD: &str = "_rustclaw_execution_policy";
const CLAWCLI_ORIGIN: &str = "clawcli";
const SAFE_MODE: &str = "safe";
const ASK_MODE: &str = "ask";
const YOLO_MODE: &str = "yolo";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskExecutionMode {
    Configured,
    Safe,
    Ask,
    Yolo,
}

impl TaskExecutionMode {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Safe => SAFE_MODE,
            Self::Ask => ASK_MODE,
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
    pub(crate) fn has_unrestricted_admin_authority(self) -> bool {
        self.mode == TaskExecutionMode::Yolo
            && self.actor_role == Some("admin")
            && self.approval_policy == ToolApprovalPolicy::Never
            && self.sandbox_mode == ToolSandboxMode::DangerFull
    }

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
        let unrestricted_admin = self.has_unrestricted_admin_authority();
        json!({
            "schema_version": 1,
            "mode": self.mode.as_token(),
            "derivation": self.derivation,
            "actor_role": self.actor_role,
            "approval_policy": self.approval_policy.as_token(),
            "sandbox_mode": self.sandbox_mode.as_token(),
            "authority_scope": if unrestricted_admin { "unrestricted_admin" } else { "configured" },
            "host_scope": if unrestricted_admin { "system" } else { "workspace" },
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
        .is_some_and(|value| !matches!(value, SAFE_MODE | ASK_MODE | YOLO_MODE))
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
    if matches!(requested_mode.as_deref(), Some(SAFE_MODE | ASK_MODE)) {
        if !payload.is_object() {
            return Err(SubmissionPolicyError::PayloadObjectRequired);
        }
        let mode = requested_mode.as_deref().unwrap_or(ASK_MODE);
        let configured = configured_policy_values(mode);
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                POLICY_PAYLOAD_FIELD.to_string(),
                json!({
                    "schema_version": 1,
                    "mode": mode,
                    "authority": "server_validated_client_preference",
                    "actor_role": identity.map(|identity| identity.role.as_str()),
                    "derivation": "explicit_restrictive_client_mode",
                    "approval_policy": configured.0.as_token(),
                    "sandbox_mode": configured.1.as_token(),
                }),
            );
        }
        return Ok(());
    }
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
    match policy.get("mode").and_then(Value::as_str) {
        Some(SAFE_MODE) if valid_restrictive_stamp(policy, SAFE_MODE) => {
            let configured = configured();
            TaskExecutionPolicy {
                mode: TaskExecutionMode::Safe,
                approval_policy: stricter_approval(
                    configured.approval_policy,
                    ToolApprovalPolicy::Always,
                ),
                sandbox_mode: ToolSandboxMode::ReadOnly,
                derivation: "explicit_restrictive_client_mode",
                actor_role: Some("authenticated_client"),
            }
        }
        Some(ASK_MODE) if valid_restrictive_stamp(policy, ASK_MODE) => {
            let configured = configured();
            TaskExecutionPolicy {
                mode: TaskExecutionMode::Ask,
                approval_policy: stricter_approval(
                    configured.approval_policy,
                    ToolApprovalPolicy::OnRequest,
                ),
                sandbox_mode: configured.sandbox_mode,
                derivation: "explicit_restrictive_client_mode",
                actor_role: Some("authenticated_client"),
            }
        }
        Some(YOLO_MODE)
            if valid_yolo_stamp(policy) && task_has_current_admin_identity(state, task) =>
        {
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
        _ => configured(),
    }
}

pub(crate) fn execution_policy_authorization_error(
    state: &AppState,
    task: &ClaimedTask,
) -> Option<&'static str> {
    let payload = serde_json::from_str::<Value>(&task.payload_json).ok()?;
    let policy = payload.get(POLICY_PAYLOAD_FIELD)?;
    if matches!(
        policy.get("mode").and_then(Value::as_str),
        Some(SAFE_MODE | ASK_MODE)
    ) {
        return (!valid_restrictive_stamp(policy, policy.get("mode").and_then(Value::as_str)?))
            .then_some("task_execution_policy_invalid");
    }
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

pub(crate) fn task_has_unrestricted_admin_authority(state: &AppState, task: &ClaimedTask) -> bool {
    effective_policy_for_task(state, task).has_unrestricted_admin_authority()
}

pub(crate) fn inheritable_policy_stamp(state: &AppState, task: &ClaimedTask) -> Option<Value> {
    let policy = effective_policy_for_task(state, task);
    if policy.mode == TaskExecutionMode::Configured {
        return None;
    }
    if matches!(
        policy.mode,
        TaskExecutionMode::Safe | TaskExecutionMode::Ask
    ) {
        return Some(json!({
            "schema_version": 1,
            "mode": policy.mode.as_token(),
            "authority": "server_validated_client_preference",
            "actor_role": policy.actor_role,
            "derivation": "inherited_restrictive_parent_task",
            "approval_policy": policy.approval_policy.as_token(),
            "sandbox_mode": policy.sandbox_mode.as_token(),
        }));
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
    match payload
        .get(POLICY_PAYLOAD_FIELD)
        .and_then(|policy| policy.get("mode"))
        .and_then(Value::as_str)
    {
        Some(SAFE_MODE) => SAFE_MODE,
        Some(ASK_MODE) => ASK_MODE,
        Some(YOLO_MODE) => YOLO_MODE,
        _ => "configured",
    }
}

fn configured_policy_values(mode: &str) -> (ToolApprovalPolicy, ToolSandboxMode) {
    match mode {
        SAFE_MODE => (ToolApprovalPolicy::Always, ToolSandboxMode::ReadOnly),
        _ => (
            ToolApprovalPolicy::OnRequest,
            ToolSandboxMode::WorkspaceWrite,
        ),
    }
}

fn stricter_approval(
    configured: ToolApprovalPolicy,
    requested: ToolApprovalPolicy,
) -> ToolApprovalPolicy {
    fn rank(policy: ToolApprovalPolicy) -> u8 {
        match policy {
            ToolApprovalPolicy::Never => 0,
            ToolApprovalPolicy::OnRisk => 1,
            ToolApprovalPolicy::OnRequest => 2,
            ToolApprovalPolicy::Always => 3,
        }
    }
    if rank(configured) >= rank(requested) {
        configured
    } else {
        requested
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

fn valid_restrictive_stamp(policy: &Value, mode: &str) -> bool {
    policy.get("schema_version").and_then(Value::as_u64) == Some(1)
        && matches!(mode, SAFE_MODE | ASK_MODE)
        && policy.get("mode").and_then(Value::as_str) == Some(mode)
        && policy.get("authority").and_then(Value::as_str)
            == Some("server_validated_client_preference")
        && policy
            .get("derivation")
            .and_then(Value::as_str)
            .is_some_and(|value| {
                matches!(
                    value,
                    "explicit_restrictive_client_mode" | "inherited_restrictive_parent_task"
                )
            })
}

#[cfg(test)]
#[path = "task_execution_policy_tests.rs"]
mod tests;

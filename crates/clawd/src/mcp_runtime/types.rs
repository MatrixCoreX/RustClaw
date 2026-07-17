use serde::Serialize;
use serde_json::Value;

use claw_core::config::{McpToolEffectConfig, McpToolPolicyConfig, McpToolRiskConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum McpLifecycleState {
    Disabled,
    Starting,
    Ready,
    Degraded,
    Stopped,
}

impl McpLifecycleState {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct McpLifecycleSnapshot {
    pub(crate) server_id: String,
    pub(crate) state: McpLifecycleState,
    pub(crate) transport: String,
    pub(crate) tool_count: usize,
    pub(crate) last_error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct McpProbeOutcome {
    pub(crate) server_id: String,
    pub(crate) status: String,
    pub(crate) latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct McpToolPolicy {
    pub(crate) effect: String,
    pub(crate) risk_level: String,
    pub(crate) idempotent: bool,
    pub(crate) isolation_profile: Option<String>,
    pub(crate) network_access: bool,
    pub(crate) filesystem_write: bool,
    pub(crate) external_publish: bool,
    pub(crate) credential_access: bool,
    pub(crate) subprocess: bool,
    pub(crate) package_install: bool,
    pub(crate) privilege_escalation: bool,
}

impl From<&McpToolPolicyConfig> for McpToolPolicy {
    fn from(value: &McpToolPolicyConfig) -> Self {
        Self {
            effect: value.effect.as_token().to_string(),
            risk_level: value.risk_level.as_token().to_string(),
            idempotent: value.idempotent,
            isolation_profile: value
                .isolation_profile
                .map(|profile| profile.as_token().to_string()),
            network_access: value.network_access,
            filesystem_write: value.filesystem_write,
            external_publish: value.external_publish,
            credential_access: value.credential_access,
            subprocess: value.subprocess,
            package_install: value.package_install,
            privilege_escalation: value.privilege_escalation,
        }
    }
}

impl Default for McpToolPolicy {
    fn default() -> Self {
        Self::from(&McpToolPolicyConfig {
            effect: McpToolEffectConfig::Mutate,
            risk_level: McpToolRiskConfig::Unknown,
            ..McpToolPolicyConfig::default()
        })
    }
}

impl McpToolPolicy {
    pub(crate) fn permission_policy_json(&self) -> Value {
        serde_json::json!({
            "source": "mcp_config",
            "effect": self.effect,
            "risk_level": self.risk_level,
            "idempotent": self.idempotent,
            "isolation_profile": self.isolation_profile,
            "network_access": self.network_access,
            "filesystem_write": self.filesystem_write,
            "external_publish": self.external_publish,
            "credential_access": self.credential_access,
            "subprocess": self.subprocess,
            "package_install": self.package_install,
            "privilege_escalation": self.privilege_escalation,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct McpToolDescriptor {
    pub(crate) capability: String,
    pub(crate) server_id: String,
    pub(crate) tool_name: String,
    pub(crate) description: Option<String>,
    pub(crate) input_schema: Value,
    pub(crate) required_args: Vec<String>,
    pub(crate) optional_args: Vec<String>,
    pub(crate) policy: McpToolPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct McpCallOutcome {
    pub(crate) capability: String,
    pub(crate) server_id: String,
    pub(crate) tool_name: String,
    pub(crate) status: String,
    pub(crate) structured_content: Option<Value>,
    pub(crate) content: Value,
    pub(crate) protocol_meta: Option<Value>,
    pub(crate) output_bytes: usize,
    pub(crate) truncated: bool,
    pub(crate) error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}")]
pub(crate) struct McpRuntimeError {
    code: &'static str,
    context: Option<String>,
}

impl McpRuntimeError {
    pub(crate) fn new(code: &'static str) -> Self {
        Self {
            code,
            context: None,
        }
    }

    pub(crate) fn with_context(code: &'static str, context: impl Into<String>) -> Self {
        Self {
            code,
            context: Some(context.into()),
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        self.code
    }

    pub(crate) fn context(&self) -> Option<&str> {
        self.context.as_deref()
    }
}

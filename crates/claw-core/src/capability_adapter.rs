use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::skill_registry::{Capability, PlannerCapabilityKind, SkillKind, SkillRegistryEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAdapterKind {
    Workflow,
    ExternalApiAdapter,
    LocalToolAdapter,
    PureSkill,
    ShellTool,
    HttpTool,
    McpTool,
}

impl CapabilityAdapterKind {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Workflow => "workflow",
            Self::ExternalApiAdapter => "external_api_adapter",
            Self::LocalToolAdapter => "local_tool_adapter",
            Self::PureSkill => "pure_skill",
            Self::ShellTool => "shell_tool",
            Self::HttpTool => "http_tool",
            Self::McpTool => "mcp_tool",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        match token.trim() {
            "workflow" => Some(Self::Workflow),
            "external_api_adapter" => Some(Self::ExternalApiAdapter),
            "local_tool_adapter" => Some(Self::LocalToolAdapter),
            "pure_skill" => Some(Self::PureSkill),
            "shell_tool" => Some(Self::ShellTool),
            "http_tool" => Some(Self::HttpTool),
            "mcp_tool" => Some(Self::McpTool),
            _ => None,
        }
    }

    pub fn for_skill_registry_entry(entry: &SkillRegistryEntry) -> Self {
        if entry.planner_kind == Some(PlannerCapabilityKind::Workflow) {
            return Self::Workflow;
        }
        if skill_uses_external_api(entry) {
            return Self::ExternalApiAdapter;
        }
        if entry.planner_kind == Some(PlannerCapabilityKind::Tool)
            || entry.kind == SkillKind::Builtin
            || entry.runtime_skill.is_some()
            || entry.runtime_action.is_some()
        {
            return Self::LocalToolAdapter;
        }
        Self::PureSkill
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityAdapterInvocation {
    pub adapter_kind: CapabilityAdapterKind,
    pub capability_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "JsonValue::is_null")]
    pub args: JsonValue,
    #[serde(default, skip_serializing_if = "JsonValue::is_null")]
    pub context: JsonValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_ref: Option<String>,
}

pub fn skill_uses_external_api(entry: &SkillRegistryEntry) -> bool {
    entry.resolved_capabilities.iter().any(|capability| {
        matches!(
            capability,
            Capability::Llm | Capability::Net | Capability::Secrets(_)
        )
    }) || entry
        .external_endpoint
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
}

pub fn skill_background_job_capable(entry: &SkillRegistryEntry) -> bool {
    entry.planner_capabilities.iter().any(|capability| {
        let name = capability.name.trim();
        let action = capability
            .action
            .as_deref()
            .map(str::trim)
            .unwrap_or_default();
        action == "poll"
            || name.ends_with(".poll")
            || capability
                .optional
                .iter()
                .chain(capability.required.iter())
                .any(|field| {
                    matches!(
                        field.trim(),
                        "async_start"
                            | "wait_for_completion"
                            | "poll_after_seconds"
                            | "expires_at"
                            | "expires_in_seconds"
                    )
                })
    })
}

#[cfg(test)]
#[path = "capability_adapter_tests.rs"]
mod tests;

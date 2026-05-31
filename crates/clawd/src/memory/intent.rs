use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub(crate) struct MemoryIntentOut {
    #[serde(default)]
    pub(crate) memory_actions: Vec<MemoryAction>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub(crate) struct MemoryAction {
    pub(crate) action: MemoryActionOp,
    pub(crate) kind: MemoryActionKind,
    pub(crate) scope: MemoryScope,
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) normalized_value: Option<String>,
    pub(crate) confidence: f32,
    pub(crate) ttl_policy: MemoryTtlPolicy,
    pub(crate) expires_at_ts: Option<i64>,
    pub(crate) source: MemoryActionSource,
    pub(crate) reason: String,
    pub(crate) risk: MemoryActionRisk,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryActionOp {
    Upsert,
    Delete,
    Expire,
    Noop,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryActionKind {
    Preference,
    ProfileFact,
    ProjectFact,
    Rule,
    TransientEvent,
    SafetySignal,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryScope {
    Chat,
    User,
    Project,
}

impl MemoryScope {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryTtlPolicy {
    Session,
    ShortTerm,
    LongTerm,
    ExplicitUntil,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub(crate) struct MemoryActionSource {
    pub(crate) source_kind: MemorySourceKind,
    pub(crate) source_ref: Option<String>,
    pub(crate) source_text: String,
    #[serde(default)]
    pub(crate) memory_ids: Vec<i64>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemorySourceKind {
    CurrentUserMessage,
    RouteSemanticExtract,
    LlmMemoryExtract,
    RuleShortcut,
    SystemEvent,
}

impl MemorySourceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::CurrentUserMessage => "current_user_message",
            Self::RouteSemanticExtract => "route_semantic_extract",
            Self::LlmMemoryExtract => "llm_memory_extract",
            Self::RuleShortcut => "rule_shortcut",
            Self::SystemEvent => "system_event",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct MemoryActionRisk {
    pub(crate) sensitive: bool,
    pub(crate) injection_like: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MemoryIntentValidation {
    pub(crate) accepted: Vec<MemoryAction>,
    pub(crate) rejected: Vec<RejectedMemoryAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RejectedMemoryAction {
    pub(crate) index: usize,
    pub(crate) reason: String,
}

pub(crate) fn parse_memory_intent_schema(
    raw: &str,
) -> Result<MemoryIntentOut, crate::prompt_utils::SchemaValidationError> {
    crate::prompt_utils::validate_against_schema::<MemoryIntentOut>(
        raw,
        crate::prompt_utils::PromptSchemaId::MemoryIntent,
    )
    .map(|validated| validated.value)
}

pub(crate) fn validate_memory_intent_actions(
    intent: MemoryIntentOut,
    min_confidence: f32,
) -> MemoryIntentValidation {
    let min_confidence = min_confidence.clamp(0.0, 1.0);
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for (index, action) in intent.memory_actions.into_iter().enumerate() {
        if let Some(reason) = reject_memory_action_reason(&action, min_confidence) {
            rejected.push(RejectedMemoryAction { index, reason });
        } else {
            accepted.push(action);
        }
    }
    MemoryIntentValidation { accepted, rejected }
}

fn reject_memory_action_reason(action: &MemoryAction, min_confidence: f32) -> Option<String> {
    if action.action == MemoryActionOp::Noop {
        return None;
    }
    if action.confidence < min_confidence {
        return Some("confidence_below_threshold".to_string());
    }
    if action.risk.injection_like
        && action.kind != MemoryActionKind::SafetySignal
        && action.action != MemoryActionOp::Noop
    {
        return Some("injection_like_requires_safety_signal_or_noop".to_string());
    }
    if action.kind == MemoryActionKind::ProjectFact && action.scope != MemoryScope::Project {
        return Some("project_fact_requires_project_scope".to_string());
    }
    if action.kind == MemoryActionKind::TransientEvent
        && matches!(
            action.ttl_policy,
            MemoryTtlPolicy::LongTerm | MemoryTtlPolicy::ExplicitUntil
        )
    {
        return Some("transient_event_cannot_be_long_term".to_string());
    }
    if matches!(
        action.ttl_policy,
        MemoryTtlPolicy::LongTerm | MemoryTtlPolicy::ExplicitUntil
    ) && action.source.source_text.trim().is_empty()
        && action.source.memory_ids.is_empty()
    {
        return Some("durable_memory_requires_source_evidence".to_string());
    }
    if action.action != MemoryActionOp::Delete
        && action.ttl_policy == MemoryTtlPolicy::ExplicitUntil
        && action.expires_at_ts.filter(|ts| *ts > 0).is_none()
    {
        return Some("explicit_until_requires_expires_at_ts".to_string());
    }
    if matches!(
        action.action,
        MemoryActionOp::Delete | MemoryActionOp::Expire
    ) && action.key.trim().is_empty()
        && action
            .source
            .source_ref
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return Some("delete_or_expire_requires_key_or_source_ref".to_string());
    }
    if action.action == MemoryActionOp::Upsert
        && action.kind != MemoryActionKind::SafetySignal
        && action.value.trim().is_empty()
        && action
            .normalized_value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return Some("upsert_requires_value".to_string());
    }
    None
}

#[cfg(test)]
#[path = "intent_tests.rs"]
mod tests;

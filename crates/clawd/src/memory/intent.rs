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
mod tests {
    use std::collections::HashSet;

    use serde_json::Value;

    use super::{
        parse_memory_intent_schema, validate_memory_intent_actions, MemoryAction, MemoryActionKind,
        MemoryActionOp, MemoryActionRisk, MemoryActionSource, MemoryIntentOut, MemoryScope,
        MemorySourceKind, MemoryTtlPolicy,
    };

    fn source() -> MemoryActionSource {
        MemoryActionSource {
            source_kind: MemorySourceKind::CurrentUserMessage,
            source_ref: Some("task:1".to_string()),
            source_text: "以后默认用中文回复".to_string(),
            memory_ids: vec![42],
        }
    }

    fn action() -> MemoryAction {
        MemoryAction {
            action: MemoryActionOp::Upsert,
            kind: MemoryActionKind::Preference,
            scope: MemoryScope::User,
            key: "response_language".to_string(),
            value: "中文".to_string(),
            normalized_value: Some("zh-CN".to_string()),
            confidence: 0.92,
            ttl_policy: MemoryTtlPolicy::LongTerm,
            expires_at_ts: None,
            source: source(),
            reason: "explicit durable response language preference".to_string(),
            risk: MemoryActionRisk {
                sensitive: false,
                injection_like: false,
            },
        }
    }

    #[test]
    fn memory_intent_schema_drift() {
        const SCHEMA_RAW: &str =
            include_str!("../../../../prompts/schemas/memory_intent.schema.json");
        let schema: Value =
            serde_json::from_str(SCHEMA_RAW).expect("memory_intent schema must be valid JSON");
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema.properties must be an object");
        assert!(
            properties.contains_key("memory_actions"),
            "schema missing parser field `memory_actions`"
        );

        let action_props = properties
            .get("memory_actions")
            .and_then(|value| value.get("items"))
            .and_then(|value| value.get("properties"))
            .and_then(Value::as_object)
            .expect("memory_actions.items.properties must be an object");
        for field in [
            "action",
            "kind",
            "scope",
            "key",
            "value",
            "normalized_value",
            "confidence",
            "ttl_policy",
            "expires_at_ts",
            "source",
            "reason",
            "risk",
        ] {
            assert!(
                action_props.contains_key(field),
                "schema missing MemoryAction field `{field}`"
            );
        }

        let action_enum = action_props
            .get("action")
            .and_then(|value| value.get("enum"))
            .and_then(Value::as_array)
            .expect("action enum must exist")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<HashSet<_>>();
        assert_eq!(
            action_enum,
            HashSet::from([
                "upsert".to_string(),
                "delete".to_string(),
                "expire".to_string(),
                "noop".to_string(),
            ])
        );

        let kind_enum = action_props
            .get("kind")
            .and_then(|value| value.get("enum"))
            .and_then(Value::as_array)
            .expect("kind enum must exist")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<HashSet<_>>();
        assert_eq!(
            kind_enum,
            HashSet::from([
                "preference".to_string(),
                "profile_fact".to_string(),
                "project_fact".to_string(),
                "rule".to_string(),
                "transient_event".to_string(),
                "safety_signal".to_string(),
            ])
        );
    }

    #[test]
    fn memory_intent_schema_accepts_valid_probe() {
        let raw = serde_json::json!({
            "memory_actions": [
                {
                    "action": "upsert",
                    "kind": "preference",
                    "scope": "user",
                    "key": "response_language",
                    "value": "中文",
                    "normalized_value": "zh-CN",
                    "confidence": 0.93,
                    "ttl_policy": "long_term",
                    "expires_at_ts": null,
                    "source": {
                        "source_kind": "current_user_message",
                        "source_ref": "task:1",
                        "source_text": "以后默认用中文回复",
                        "memory_ids": []
                    },
                    "reason": "explicit durable response language preference",
                    "risk": {
                        "sensitive": false,
                        "injection_like": false
                    }
                }
            ]
        })
        .to_string();

        let parsed = parse_memory_intent_schema(&raw).expect("valid memory intent");
        assert_eq!(parsed.memory_actions.len(), 1);
        assert_eq!(parsed.memory_actions[0].action, MemoryActionOp::Upsert);
        assert_eq!(parsed.memory_actions[0].kind, MemoryActionKind::Preference);
    }

    #[test]
    fn memory_intent_schema_rejects_bad_enum() {
        let raw = serde_json::json!({
            "memory_actions": [
                {
                    "action": "store",
                    "kind": "preference",
                    "scope": "user",
                    "key": "response_language",
                    "value": "中文",
                    "normalized_value": "zh-CN",
                    "confidence": 0.93,
                    "ttl_policy": "long_term",
                    "expires_at_ts": null,
                    "source": {
                        "source_kind": "current_user_message",
                        "source_ref": null,
                        "source_text": "以后默认用中文回复",
                        "memory_ids": []
                    },
                    "reason": "bad enum probe",
                    "risk": {
                        "sensitive": false,
                        "injection_like": false
                    }
                }
            ]
        })
        .to_string();

        let err = parse_memory_intent_schema(&raw).expect_err("bad enum must fail");
        assert!(err.to_string().contains("expected one of"));
    }

    #[test]
    fn memory_intent_validation_accepts_valid_preference() {
        let validation = validate_memory_intent_actions(
            MemoryIntentOut {
                memory_actions: vec![action()],
            },
            0.72,
        );
        assert_eq!(validation.accepted.len(), 1);
        assert!(validation.rejected.is_empty());
    }

    #[test]
    fn memory_intent_validation_rejects_low_confidence_long_term_fact() {
        let mut action = action();
        action.kind = MemoryActionKind::ProfileFact;
        action.key = "profile.language".to_string();
        action.confidence = 0.5;
        let validation = validate_memory_intent_actions(
            MemoryIntentOut {
                memory_actions: vec![action],
            },
            0.72,
        );
        assert!(validation.accepted.is_empty());
        assert_eq!(validation.rejected[0].reason, "confidence_below_threshold");
    }

    #[test]
    fn memory_intent_validation_rejects_project_fact_outside_project_scope() {
        let mut action = action();
        action.kind = MemoryActionKind::ProjectFact;
        action.scope = MemoryScope::User;
        action.key = "project.test_command".to_string();
        action.value = "cargo check -p clawd".to_string();
        let validation = validate_memory_intent_actions(
            MemoryIntentOut {
                memory_actions: vec![action],
            },
            0.72,
        );
        assert!(validation.accepted.is_empty());
        assert_eq!(
            validation.rejected[0].reason,
            "project_fact_requires_project_scope"
        );
    }

    #[test]
    fn memory_intent_validation_rejects_injection_like_non_safety_action() {
        let mut action = action();
        action.risk.injection_like = true;
        let validation = validate_memory_intent_actions(
            MemoryIntentOut {
                memory_actions: vec![action],
            },
            0.72,
        );
        assert!(validation.accepted.is_empty());
        assert_eq!(
            validation.rejected[0].reason,
            "injection_like_requires_safety_signal_or_noop"
        );
    }

    #[test]
    fn memory_intent_validation_delete_requires_key_or_source_ref() {
        let mut action = action();
        action.action = MemoryActionOp::Delete;
        action.key.clear();
        action.source.source_ref = None;
        let validation = validate_memory_intent_actions(
            MemoryIntentOut {
                memory_actions: vec![action],
            },
            0.72,
        );
        assert!(validation.accepted.is_empty());
        assert_eq!(
            validation.rejected[0].reason,
            "delete_or_expire_requires_key_or_source_ref"
        );
    }

    #[test]
    fn memory_intent_validation_explicit_until_requires_expiry() {
        let mut action = action();
        action.ttl_policy = MemoryTtlPolicy::ExplicitUntil;
        action.expires_at_ts = None;
        let validation = validate_memory_intent_actions(
            MemoryIntentOut {
                memory_actions: vec![action],
            },
            0.72,
        );
        assert!(validation.accepted.is_empty());
        assert_eq!(
            validation.rejected[0].reason,
            "explicit_until_requires_expires_at_ts"
        );
    }

    #[test]
    fn memory_intent_validation_delete_ignores_ttl_expiry_shape() {
        let mut action = action();
        action.action = MemoryActionOp::Delete;
        action.key = "response_language".to_string();
        action.value.clear();
        action.normalized_value = None;
        action.ttl_policy = MemoryTtlPolicy::ExplicitUntil;
        action.expires_at_ts = None;
        let validation = validate_memory_intent_actions(
            MemoryIntentOut {
                memory_actions: vec![action],
            },
            0.72,
        );
        assert_eq!(validation.accepted.len(), 1);
        assert!(validation.rejected.is_empty());
    }
}

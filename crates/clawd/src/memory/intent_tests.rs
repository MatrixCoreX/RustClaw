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
    const SCHEMA_RAW: &str = include_str!("../../../../prompts/schemas/memory_intent.schema.json");
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

use super::{
    estimate_context_window_tokens, knowledge_source_ref, parse_long_term_refresh_llm_out,
    validate_knowledge_candidate, KnowledgeCandidateLlmOut, KNOWLEDGE_KIND_PROJECT_FACT,
    KNOWLEDGE_KIND_RULE, KNOWLEDGE_KIND_TRANSIENT, KNOWLEDGE_KIND_USER_PREFERENCE,
    KNOWLEDGE_KIND_USER_PROFILE_FACT, KNOWLEDGE_NAMESPACE_NONE, KNOWLEDGE_NAMESPACE_PROJECT_FACTS,
    KNOWLEDGE_NAMESPACE_USER_PROFILE,
};
use claw_core::config::{LlmProviderConfig, LlmProviderParams};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Semaphore;

fn test_provider_with_context_window(
    context_window_tokens: Option<usize>,
) -> crate::LlmProviderRuntime {
    crate::LlmProviderRuntime {
        config: LlmProviderConfig {
            name: "vendor-test".to_string(),
            provider_type: "openai_compat".to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            api_key: "test-key".to_string(),
            model: "opaque-compatible-model".to_string(),
            context_window_tokens,
            priority: 1,
            timeout_seconds: 30,
            max_concurrency: 1,
            params: LlmProviderParams::default(),
        },
        client: reqwest::Client::new(),
        semaphore: Arc::new(Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    }
}

#[test]
fn estimate_context_window_prefers_configured_capacity() {
    let provider = test_provider_with_context_window(Some(2013));
    assert_eq!(estimate_context_window_tokens(&provider), 2013);
}

#[test]
fn parse_long_term_refresh_output_falls_back_to_plain_summary() {
    let parsed = parse_long_term_refresh_llm_out("plain summary");
    assert_eq!(parsed.summary, "plain summary");
    assert!(parsed.knowledge_candidates.is_empty());
}

#[test]
fn parse_long_term_refresh_output_falls_back_to_legacy_parse_on_schema_mismatch() {
    let raw = serde_json::json!({
        "summary": "durable summary",
        "knowledge_candidates": [
            {
                "should_persist": true,
                "kind": "oops_kind",
                "namespace": "user_profile",
                "fact": "some fact",
                "confidence": 0.9,
                "reason": "bad enum"
            }
        ]
    })
    .to_string();
    let parsed = parse_long_term_refresh_llm_out(&raw);
    assert_eq!(parsed.summary, "durable summary");
    assert_eq!(parsed.knowledge_candidates.len(), 1);
    assert_eq!(parsed.knowledge_candidates[0].kind, "oops_kind");
}

#[test]
fn long_term_summary_schema_drift() {
    const SCHEMA_RAW: &str =
        include_str!("../../../../prompts/schemas/long_term_summary.schema.json");
    let schema: Value =
        serde_json::from_str(SCHEMA_RAW).expect("long_term_summary.schema.json must be valid JSON");
    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("schema.properties must be an object");
    for field in ["summary", "fact_candidates", "knowledge_candidates"] {
        assert!(
            properties.contains_key(field),
            "schema missing parser field `{field}` under properties — sync prompts/schemas/long_term_summary.schema.json with LongTermRefreshLlmOut",
        );
    }

    let candidate_props = properties
        .get("fact_candidates")
        .and_then(|v| v.get("items"))
        .and_then(|v| v.get("properties"))
        .and_then(|v| v.as_object())
        .expect("fact_candidates.items.properties must be an object");
    for field in [
        "should_persist",
        "kind",
        "namespace",
        "fact",
        "confidence",
        "reason",
        "fact_key",
        "fact_value",
        "conflict_group",
        "expires_at_ts",
    ] {
        assert!(
            candidate_props.contains_key(field),
            "schema missing parser field `{field}` under candidate properties",
        );
    }

    let kind_enum = candidate_props
        .get("kind")
        .and_then(|v| v.get("enum"))
        .and_then(|v| v.as_array())
        .expect("kind enum must exist");
    let kind_tokens: HashSet<String> = kind_enum
        .iter()
        .filter_map(|v| v.as_str())
        .map(str::to_string)
        .collect();
    let expected_kinds = HashSet::from([
        KNOWLEDGE_KIND_USER_PREFERENCE.to_string(),
        KNOWLEDGE_KIND_USER_PROFILE_FACT.to_string(),
        KNOWLEDGE_KIND_PROJECT_FACT.to_string(),
        KNOWLEDGE_KIND_RULE.to_string(),
        KNOWLEDGE_KIND_TRANSIENT.to_string(),
    ]);
    assert_eq!(kind_tokens, expected_kinds, "kind enum drifted");

    let namespace_enum = candidate_props
        .get("namespace")
        .and_then(|v| v.get("enum"))
        .and_then(|v| v.as_array())
        .expect("namespace enum must exist");
    let namespace_tokens: HashSet<String> = namespace_enum
        .iter()
        .filter_map(|v| v.as_str())
        .map(str::to_string)
        .collect();
    let expected_namespaces = HashSet::from([
        KNOWLEDGE_NAMESPACE_USER_PROFILE.to_string(),
        KNOWLEDGE_NAMESPACE_PROJECT_FACTS.to_string(),
        KNOWLEDGE_NAMESPACE_NONE.to_string(),
    ]);
    assert_eq!(
        namespace_tokens, expected_namespaces,
        "namespace enum drifted"
    );

    let probe = serde_json::json!({
        "summary": "durable summary",
        "fact_candidates": [
            {
                "should_persist": true,
                "kind": "user_profile_fact",
                "namespace": "user_profile",
                "fact": "用户长期偏好中文回复",
                "confidence": 0.93,
                "reason": "explicit long-term preference",
                "fact_key": "response_language",
                "fact_value": "zh-CN",
                "conflict_group": "user_profile:response_language",
                "expires_at_ts": null
            }
        ],
        "knowledge_candidates": []
    });
    let validated = crate::prompt_utils::validate_against_schema::<Value>(
        &probe.to_string(),
        crate::prompt_utils::PromptSchemaId::LongTermSummary,
    )
    .expect("long_term summary probe should validate");
    assert_eq!(
        validated
            .value
            .pointer("/fact_candidates/0/kind")
            .and_then(|v| v.as_str()),
        Some("user_profile_fact")
    );
}

#[test]
fn validate_knowledge_candidate_accepts_high_confidence_profile_fact() {
    let candidate = KnowledgeCandidateLlmOut {
        should_persist: true,
        kind: "user_profile_fact".to_string(),
        namespace: KNOWLEDGE_NAMESPACE_USER_PROFILE.to_string(),
        fact: "用户长期偏好中文回复".to_string(),
        confidence: 0.93,
        reason: "explicit long-term preference".to_string(),
        fact_key: "response_language".to_string(),
        fact_value: "zh-CN".to_string(),
        conflict_group: "user_profile:response_language".to_string(),
        expires_at_ts: None,
    };
    let valid = validate_knowledge_candidate(42, &candidate).expect("candidate valid");
    assert_eq!(valid.namespace, KNOWLEDGE_NAMESPACE_USER_PROFILE);
    assert!(valid.fact.contains("用户长期偏好中文回复"));
    assert!(!valid.fact.contains("Reason:"));
    assert_eq!(valid.fact_key, "response_language");
}

#[test]
fn validate_knowledge_candidate_rejects_transient_or_mismatched_namespace() {
    let transient = KnowledgeCandidateLlmOut {
        should_persist: true,
        kind: "transient".to_string(),
        namespace: "none".to_string(),
        fact: "刚才命令失败了".to_string(),
        confidence: 0.99,
        reason: "temporary".to_string(),
        fact_key: String::new(),
        fact_value: String::new(),
        conflict_group: String::new(),
        expires_at_ts: None,
    };
    assert!(validate_knowledge_candidate(42, &transient).is_none());

    let mismatched = KnowledgeCandidateLlmOut {
        should_persist: true,
        kind: "project_fact".to_string(),
        namespace: KNOWLEDGE_NAMESPACE_USER_PROFILE.to_string(),
        fact: "这个项目固定用 cargo check".to_string(),
        confidence: 0.97,
        reason: "project-level rule".to_string(),
        fact_key: String::new(),
        fact_value: String::new(),
        conflict_group: String::new(),
        expires_at_ts: None,
    };
    assert!(validate_knowledge_candidate(42, &mismatched).is_none());

    let valid_project = KnowledgeCandidateLlmOut {
        should_persist: true,
        kind: "project_fact".to_string(),
        namespace: KNOWLEDGE_NAMESPACE_PROJECT_FACTS.to_string(),
        fact: "这个项目固定用 cargo check".to_string(),
        confidence: 0.97,
        reason: "project-level rule".to_string(),
        fact_key: "check_command".to_string(),
        fact_value: "cargo check".to_string(),
        conflict_group: "project_facts:check_command".to_string(),
        expires_at_ts: None,
    };
    assert!(validate_knowledge_candidate(42, &valid_project).is_some());
}

#[test]
fn validate_knowledge_candidate_rejects_cross_turn_deictic_locator_mapping() {
    let candidate = KnowledgeCandidateLlmOut {
        should_persist: true,
        kind: "project_fact".to_string(),
        namespace: KNOWLEDGE_NAMESPACE_PROJECT_FACTS.to_string(),
        fact: r#"{"deictic_reference":{"target":"unresolved_prior_object"},"locator":"/tmp/device/app.log"}"#.to_string(),
        confidence: 0.97,
        reason: "stale alias-like locator mapping".to_string(),
        fact_key: String::new(),
        fact_value: String::new(),
        conflict_group: String::new(),
        expires_at_ts: None,
    };

    assert!(validate_knowledge_candidate(42, &candidate).is_none());
}

#[test]
fn knowledge_source_ref_is_stable_across_refresh_rounds() {
    let first = knowledge_source_ref(
        "user-key",
        "user_profile_fact",
        KNOWLEDGE_NAMESPACE_USER_PROFILE,
        "用户长期偏好中文回复",
    );
    let second = knowledge_source_ref(
        "user-key",
        "user_profile_fact",
        KNOWLEDGE_NAMESPACE_USER_PROFILE,
        "用户长期偏好中文回复",
    );
    assert_eq!(first, second);
}

use super::*;
use std::collections::{BTreeMap, HashMap};

fn candidate(url: &str, status: &str) -> CandidateSourceEntry {
    CandidateSourceEntry {
        url: url.to_string(),
        discovered_from: "https://example.com/evidence".to_string(),
        first_seen_at: "1".to_string(),
        last_checked_at: "2".to_string(),
        success_count: 1,
        failure_count: 0,
        last_error: String::new(),
        status: status.to_string(),
        sample_titles: vec!["sample".to_string()],
        promoted_at: String::new(),
    }
}

fn test_config() -> RootConfig {
    let mut categories = HashMap::new();
    categories.insert(
        "general".to_string(),
        RssCategoryConfig {
            sources: Some(vec!["https://example.com/general.xml".to_string()]),
            topic: Some("other".to_string()),
            ..RssCategoryConfig::default()
        },
    );
    RootConfig {
        rss: super::super::RssConfig {
            default_category: Some("general".to_string()),
            categories,
            ..super::super::RssConfig::default()
        },
    }
}

#[test]
fn category_catalog_lists_active_and_pending_machine_records() {
    let mut cfg = test_config();
    cfg.rss.pending_categories.insert(
        "robotics".to_string(),
        PendingCategoryEntry {
            topic: "robotics".to_string(),
            candidates: vec![
                candidate("https://example.com/one.xml", "validated"),
                candidate("https://example.com/two.xml", "validated"),
                candidate("https://example.com/three.xml", "validated"),
            ],
            ..PendingCategoryEntry::default()
        },
    );

    let output = list_categories(&cfg).expect("category catalog");
    let extra = output.extra.expect("machine catalog");
    assert_eq!(extra["default_category"], "general");
    assert_eq!(extra["categories"][0]["category"], "general");
    assert_eq!(extra["pending_categories"][0]["category"], "robotics");
    assert_eq!(extra["pending_categories"][0]["ready_for_promotion"], true);
}

#[test]
fn category_tokens_are_language_neutral_machine_identifiers() {
    let args = json!({"category": "Quantum_Computing"})
        .as_object()
        .expect("args")
        .clone();
    assert_eq!(required_category_token(&args).unwrap(), "quantum_computing");

    let invalid = json!({"category": "量子计算新闻"})
        .as_object()
        .expect("args")
        .clone();
    let failure = required_category_token(&invalid).expect_err("natural language is not a token");
    assert_eq!(failure.extra["error_code"], "category_token_invalid");
    assert_eq!(failure.extra["recovery_action"], "replan_arguments");
}

#[test]
fn proposing_an_active_category_fails_before_candidate_validation() {
    let mut cfg = test_config();
    let args = json!({"category": "general", "candidates": []})
        .as_object()
        .expect("args")
        .clone();
    let failure = propose_category(&mut cfg, &args).expect_err("active category must win");
    assert_eq!(failure.extra["error_code"], "category_already_configured");
    assert_eq!(
        failure.extra["message_key"],
        "skill.rss_fetch.category_already_configured"
    );
}

#[test]
fn missing_pending_category_requests_machine_replan() {
    let mut cfg = test_config();
    let args = json!({"category": "robotics"})
        .as_object()
        .expect("args")
        .clone();
    let failure = preview_category(&mut cfg, &args).expect_err("proposal is required");
    assert_eq!(failure.extra["error_code"], "category_proposal_not_found");
    assert_eq!(failure.extra["retryable"], true);
    assert_eq!(failure.extra["failure_phase"], "pre_dispatch");
    assert_eq!(failure.extra["side_effect_applied"], false);
    assert_eq!(failure.extra["recovery_action"], "replan_arguments");
}

#[test]
fn invalid_proposal_metadata_does_not_remove_existing_pending_state() {
    let mut cfg = test_config();
    let original = PendingCategoryEntry {
        topic: "robotics".to_string(),
        candidates: vec![candidate("https://example.com/one.xml", "validated")],
        ..PendingCategoryEntry::default()
    };
    cfg.rss
        .pending_categories
        .insert("robotics".to_string(), original.clone());
    let args = json!({
        "category": "robotics",
        "topic_token": "this is natural-language metadata",
        "candidates": [{
            "url": "https://example.com/two.xml",
            "discovered_from": "https://example.com/evidence"
        }]
    })
    .as_object()
    .expect("args")
    .clone();

    let failure = propose_category(&mut cfg, &args).expect_err("invalid metadata must fail");

    assert_eq!(failure.extra["error_code"], "topic_token_invalid");
    assert_eq!(cfg.rss.pending_categories["robotics"], original);
}

#[test]
fn legacy_machine_state_json_defaults_pending_categories() {
    let state: super::super::RssMachineState = serde_json::from_value(json!({
        "source_states": {},
        "candidates": {},
        "deprecated": []
    }))
    .expect("legacy state remains readable");
    assert!(state.pending_categories.is_empty());

    let mut pending_categories = BTreeMap::new();
    pending_categories.insert("robotics".to_string(), PendingCategoryEntry::default());
    let state = super::super::RssMachineState {
        pending_categories,
        ..super::super::RssMachineState::default()
    };
    assert!(!state.is_empty());
}

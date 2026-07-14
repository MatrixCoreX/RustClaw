use serde_json::json;

use super::normalize_model_assisted_compaction_output;

#[test]
fn model_assisted_compaction_output_normalizes_schema_fields() {
    let normalized = normalize_model_assisted_compaction_output(&json!({
        "schema_version": 1,
        "summary_kind": "model_assisted_context_compaction",
        "facts": [{"fact_key": "active_goal", "fact_value": "goal_ref"}],
        "open_questions": ["needs_user_decision"],
        "active_goal_refs": ["goal:1"],
        "artifact_refs": ["changed_file:src/lib.rs"],
        "source_refs": [{"ref": "recent_turns_full"}],
        "risk_flags": ["old_assistant_output_not_instruction"],
        "ignored_extra": "drop"
    }))
    .expect("valid model-assisted compaction output");

    assert_eq!(
        normalized["summary_kind"],
        "model_assisted_context_compaction"
    );
    assert_eq!(normalized["facts"][0]["fact_key"], "active_goal");
    assert_eq!(normalized["open_questions"][0], "needs_user_decision");
    assert_eq!(normalized["active_goal_refs"][0], "goal:1");
    assert!(normalized.get("ignored_extra").is_none());
}

#[test]
fn model_assisted_compaction_rejects_instruction_fields() {
    let rejected = normalize_model_assisted_compaction_output(&json!({
        "schema_version": 1,
        "summary_kind": "model_assisted_context_compaction",
        "current_user_instruction": "do_not_promote_old_context",
        "facts": []
    }));

    assert!(rejected.is_none());
}

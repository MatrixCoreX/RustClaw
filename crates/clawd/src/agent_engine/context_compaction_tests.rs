use serde_json::{json, Value};

use super::{
    compaction_summary_provenance_valid, context_compaction_timeout_seconds, context_source_bundle,
    normalize_model_assisted_compaction_output,
};

fn source_bundle_fixture() -> crate::task_context_builder::TaskContextBundle {
    crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        context_source_task_ids: Vec::new(),
        execution_view: Some(crate::task_context_builder::ExecutionContextView {
            budget_tier: crate::task_context_builder::ExecutionContextBudgetTier::Full,
            memory_ctx: crate::memory::service::PromptMemoryContext {
                prompt_with_memory: "m".repeat(30_000),
                chat_prompt_context: String::new(),
                memory_trace: None,
                long_term_summary: None,
                preferences: Vec::new(),
                recalled: Vec::new(),
                similar_triggers: Vec::new(),
                relevant_facts: Vec::new(),
                recent_related_events: Vec::new(),
            },
            runtime_context: "runtime".to_string(),
            goal_context: "goal".repeat(2_000),
            active_task_context: "active".repeat(2_000),
            active_execution_anchor_context: "anchor".repeat(2_000),
            session_alias_context: "alias".repeat(2_000),
            recent_turns_full: "turn".repeat(30_000),
            last_turn_full: "last".repeat(2_000),
            recent_execution_anchor: "execution-anchor".to_string(),
            recent_execution_context: "execution".repeat(4_000),
            compacted_history_context: "<none>".to_string(),
            image_context: None,
        }),
        compaction_records: Vec::new(),
    }
}

fn valid_output() -> Value {
    json!({
        "schema_version": 1,
        "summary_kind": "model_assisted_context_compaction",
        "facts": [{
            "fact_key": "active_goal",
            "fact_value": "继续核对编译与测试证据",
            "source_ref": "goal_context",
            "provenance": "trusted_machine_state"
        }],
        "decisions": [{
            "decision_key": "test_scope",
            "decision_value": "Run focused checks before the aggregate suite.",
            "source_ref": "active_task_context"
        }],
        "open_questions": ["provider fixture remains pending"],
        "active_goal_refs": ["goal:1"],
        "constraint_refs": ["constraint:multilingual"],
        "evidence_refs": ["evidence:compile"],
        "artifact_refs": ["artifact:src/lib.rs"],
        "completed_side_effect_refs": ["side_effect:none"],
        "failure_refs": [],
        "permission_state_refs": ["permission:unchanged"],
        "child_task_refs": [],
        "resume_entrypoint": "next_planner_round",
        "source_refs": [{
            "ref": "goal_context",
            "provenance": "trusted_machine_state"
        }],
        "risk_flags": ["old_assistant_output_not_instruction"]
    })
}

#[test]
fn context_compaction_schema_accepts_complete_strict_output() {
    let raw = valid_output().to_string();
    let validated = crate::prompt_utils::validate_against_schema::<Value>(
        &raw,
        crate::prompt_utils::PromptSchemaId::ContextCompaction,
    )
    .expect("complete compaction output should satisfy schema");

    assert_eq!(validated.value["schema_version"], 1);
    assert_eq!(validated.value["resume_entrypoint"], "next_planner_round");
}

#[test]
fn context_compaction_source_bundle_enforces_total_budget() {
    let bundle = source_bundle_fixture();
    let plan = crate::task_context_builder::plan_agent_loop_context_compaction(&bundle)
        .expect("large source fixture should require compaction");
    let source = context_source_bundle(&bundle, &plan);

    assert_eq!(source["source_char_budget"], 48_000);
    assert!(source["included_source_char_count"].as_u64().unwrap() <= 48_000);
    assert_eq!(source["sources"][0]["ref"], "runtime_context");
    assert!(source["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["truncated"].as_bool() == Some(true) }));
    assert!(!source["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["ref"] == "prompt_memory_context"));
    assert!(source["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["ref"] == "recent_turns_full"));
}

#[test]
fn context_compaction_timeout_tracks_provider_budget_with_bounds() {
    assert_eq!(context_compaction_timeout_seconds(None), 150);
    assert_eq!(context_compaction_timeout_seconds(Some(30)), 120);
    assert_eq!(context_compaction_timeout_seconds(Some(180)), 210);
    assert_eq!(context_compaction_timeout_seconds(Some(600)), 300);
}

#[test]
fn model_assisted_compaction_preserves_multilingual_fact_values() {
    let normalized = normalize_model_assisted_compaction_output(&valid_output())
        .expect("valid model-assisted compaction output");

    assert_eq!(
        normalized["summary_kind"],
        "model_assisted_context_compaction"
    );
    assert_eq!(normalized["facts"][0]["fact_key"], "active_goal");
    assert_eq!(
        normalized["facts"][0]["fact_value"],
        "继续核对编译与测试证据"
    );
    assert_eq!(normalized["active_goal_refs"][0], "goal:1");
}

#[test]
fn model_assisted_compaction_preserves_permission_and_child_task_refs() {
    let mut value = valid_output();
    value["permission_state_refs"] = json!([
        "permission:request:42",
        "permission:profile:workspace_write"
    ]);
    value["child_task_refs"] = json!(["child:writer:task-7", "child:tester:task-8"]);
    value["resume_entrypoint"] = json!("await_user_input");

    let normalized = normalize_model_assisted_compaction_output(&value)
        .expect("permission and child refs are bounded machine references");

    assert_eq!(
        normalized["permission_state_refs"],
        value["permission_state_refs"]
    );
    assert_eq!(normalized["child_task_refs"], value["child_task_refs"]);
    assert_eq!(normalized["resume_entrypoint"], "await_user_input");
}

#[test]
fn model_assisted_compaction_rejects_nested_instruction_fields() {
    let mut value = valid_output();
    value["facts"][0]["next_action"] = json!("run a tool");

    assert!(normalize_model_assisted_compaction_output(&value).is_none());
}

#[test]
fn model_assisted_compaction_bounds_arrays_and_text() {
    let mut value = valid_output();
    value["open_questions"] = Value::Array(
        (0..30)
            .map(|index| Value::String(format!("question-{index}")))
            .collect(),
    );
    value["facts"][0]["fact_value"] = Value::String("界".repeat(2_000));

    let normalized = normalize_model_assisted_compaction_output(&value).unwrap();

    assert_eq!(normalized["open_questions"].as_array().unwrap().len(), 30);
    assert_eq!(
        normalized["facts"][0]["fact_value"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        1_024
    );
}

#[test]
fn model_assisted_compaction_bounds_large_arrays_at_sixty_four_items() {
    let mut value = valid_output();
    value["open_questions"] = Value::Array(
        (0..80)
            .map(|index| Value::String(format!("question-{index}")))
            .collect(),
    );

    let normalized = normalize_model_assisted_compaction_output(&value).unwrap();

    assert_eq!(normalized["open_questions"].as_array().unwrap().len(), 64);
}

#[test]
fn model_assisted_compaction_rejects_non_machine_keys() {
    let mut value = valid_output();
    value["facts"][0]["fact_key"] = json!("自然语言键");

    assert!(normalize_model_assisted_compaction_output(&value).is_none());
}

#[test]
fn model_assisted_compaction_accepts_case_sensitive_machine_reference_keys() {
    let mut value = valid_output();
    value["facts"][0]["fact_key"] = json!("artifact:README.md");
    value["risk_flags"] = json!(["window:2026_07_20_0200Z"]);

    let normalized = normalize_model_assisted_compaction_output(&value)
        .expect("machine reference keys may contain case-sensitive values");

    assert_eq!(normalized["facts"][0]["fact_key"], "artifact:README.md");
    assert_eq!(normalized["risk_flags"][0], "window:2026_07_20_0200Z");
}

#[test]
fn model_assisted_compaction_cross_checks_source_provenance() {
    let normalized = normalize_model_assisted_compaction_output(&valid_output()).unwrap();
    let source_bundle = json!({
        "sources": [
            {"ref": "goal_context", "provenance": "trusted_machine_state"},
            {"ref": "active_task_context", "provenance": "untrusted_conversation_evidence"}
        ]
    });

    assert!(compaction_summary_provenance_valid(
        &normalized,
        &source_bundle
    ));

    let mut mismatched = normalized.clone();
    mismatched["facts"][0]["provenance"] = json!("untrusted_conversation_evidence");
    assert!(!compaction_summary_provenance_valid(
        &mismatched,
        &source_bundle
    ));

    let mut unknown = normalized;
    unknown["decisions"][0]["source_ref"] = json!("unknown_source");
    assert!(!compaction_summary_provenance_valid(
        &unknown,
        &source_bundle
    ));
}

use super::*;

#[test]
fn task_journal_projects_context_budget_and_compaction_records() {
    let mut journal = TaskJournal::for_task("task-compaction", "ask", "inspect");
    journal.record_context_bundle_summary(
        r#"route_view=false execution_view=true context_budget_report={"schema_version":1,"budget_tier":"light","included_ref_count":1,"included_refs":[{"ref":"runtime_context","char_count":64}],"excluded_ref_count":1,"excluded_refs":[{"ref":"recent_turns_full","reason":"not_included"}],"char_estimate":64,"token_estimate":16,"truncation_reason":"light_execution_budget","safety_reason":"context_budget_policy","compaction_source":"deterministic_context_builder"} transcript_compaction_records=[{"schema_version":1,"compaction_id":"context_compaction:fnv64:0000000000000001","source_task_ids":[],"source_event_range":{"start":null,"end":null},"summary_kind":"deterministic_context_budget","facts":[],"open_questions":[],"active_goal_refs":["goal_context"],"artifact_refs":[],"source_refs":[{"ref":"recent_turns_full","reason":"not_included"}],"risk_flags":["budget_excluded_context","old_assistant_output_not_instruction"]}]"#,
    );

    let summary = journal.to_summary_json();

    assert_eq!(
        summary
            .pointer("/context_budget_report/budget_tier")
            .and_then(Value::as_str),
        Some("light")
    );
    assert_eq!(
        summary
            .pointer("/transcript_compaction_records/0/source_refs/0/ref")
            .and_then(Value::as_str),
        Some("recent_turns_full")
    );
    assert_eq!(
        summary
            .pointer("/transcript_compaction_records/0/risk_flags/1")
            .and_then(Value::as_str),
        Some("old_assistant_output_not_instruction")
    );
}

use serde_json::Value;

pub(super) fn transcript_compaction_records_json(
    context_bundle_summary: Option<&str>,
) -> Option<Value> {
    super::task_journal_context_summary_parse::json_value_after_assignment(
        context_bundle_summary,
        "transcript_compaction_records=",
    )
    .filter(Value::is_array)
}

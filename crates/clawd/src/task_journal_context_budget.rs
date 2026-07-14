use serde_json::Value;

pub(super) fn context_budget_report_json(context_bundle_summary: Option<&str>) -> Option<Value> {
    super::task_journal_context_summary_parse::json_value_after_assignment(
        context_bundle_summary,
        "context_budget_report=",
    )
    .filter(Value::is_object)
}

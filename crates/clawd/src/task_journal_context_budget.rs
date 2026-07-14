use serde_json::Value;

pub(super) fn context_budget_report_json(context_bundle_summary: Option<&str>) -> Option<Value> {
    let summary = context_bundle_summary?.trim();
    let (_, report) = summary.split_once("context_budget_report=")?;
    serde_json::from_str::<Value>(report.trim())
        .ok()
        .filter(Value::is_object)
}

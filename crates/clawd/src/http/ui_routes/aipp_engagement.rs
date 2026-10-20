use super::bounded_aipp_optional_text;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) fn aipp_engagement(record: &Value) -> Option<Value> {
    let engagement = record.get("engagement")?;
    if engagement.get("schema_version").and_then(Value::as_u64) != Some(1)
        || engagement.get("platform").and_then(Value::as_str)
            != record.get("platform").and_then(Value::as_str)
    {
        return None;
    }
    let mut metrics = BTreeMap::new();
    for name in [
        "views",
        "reads",
        "likes",
        "comments",
        "favorites",
        "shares",
        "reposts",
        "coins",
        "danmaku",
    ] {
        let Some(metric) = engagement.pointer(&format!("/metrics/{name}")) else {
            continue;
        };
        let Some(display) = metric
            .get("display")
            .and_then(Value::as_str)
            .filter(|value| {
                !value.is_empty()
                    && value.chars().count() <= 32
                    && !value.chars().any(char::is_control)
            })
        else {
            continue;
        };
        metrics.insert(name.to_string(), json!({
            "display": display,
            "value": metric.get("value").and_then(Value::as_u64)
                .filter(|value| *value <= 9_007_199_254_740_991),
            "source": bounded_aipp_optional_text(metric.get("source"), 128)
                .filter(|value| value.chars().all(|c| c.is_ascii_alphanumeric() || "_:.-".contains(c))),
        }));
    }
    Some(json!({
        "schema_version": 1,
        "platform": engagement.get("platform").and_then(Value::as_str),
        "captured_at": bounded_aipp_optional_text(engagement.get("captured_at"), 64),
        "metrics": metrics,
    }))
}

#[cfg(test)]
#[path = "aipp_engagement_tests.rs"]
mod tests;

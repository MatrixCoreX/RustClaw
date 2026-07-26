use serde_json::{json, Map, Value};

use crate::AppState;

pub(super) fn enrich_runtime_owned_skill_extra(
    state: &AppState,
    skill_name: &str,
    extra: Option<Value>,
) -> Option<Value> {
    if skill_name != "health_check" {
        return extra;
    }

    let database_available = state.core.db.try_get().and_then(|connection| {
        connection
            .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .ok()
    }) == Some(1);
    Some(enrich_health_check_extra(extra, database_available))
}

fn enrich_health_check_extra(extra: Option<Value>, database_available: bool) -> Value {
    let mut fields = match extra {
        Some(Value::Object(fields)) => fields,
        _ => Map::new(),
    };
    let process_visible = fields
        .get("clawd_process_count")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0);
    let port_visible = fields
        .get("clawd_health_port_open")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let clawd_visible = process_visible || port_visible;
    let overall_status = if clawd_visible && database_available {
        "healthy"
    } else {
        "degraded"
    };

    fields.insert("db_available".to_string(), json!(database_available));
    fields.insert("clawd_visible".to_string(), json!(clawd_visible));
    fields.insert("overall_status".to_string(), json!(overall_status));
    fields.insert(
        "runtime_probe".to_string(),
        json!({
            "database": {
                "available": database_available,
                "source": "runtime_pool_select_1",
            },
            "clawd": {
                "visible": clawd_visible,
                "process_visible": process_visible,
                "health_port_open": port_visible,
            },
        }),
    );
    Value::Object(fields)
}

#[cfg(test)]
#[path = "result_enrichment_tests.rs"]
mod tests;

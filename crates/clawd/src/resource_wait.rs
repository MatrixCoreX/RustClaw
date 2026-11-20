//! Bounded, data-only resource diagnostics for task presentation.
use serde_json::{json, Value};

pub(crate) fn from_error(raw: &str) -> Option<Value> {
    if !crate::skills::is_retryable_resource_admission_error(raw) {
        return None;
    }
    let error = crate::skills::parse_structured_skill_error(raw)?;
    project(error.extra.as_ref()?)
}

fn project(extra: &Value) -> Option<Value> {
    let reason = extra
        .get("wait_reason")
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        })
        .unwrap_or("resource_pressure");
    let mut result = json!({"schema_version":1,"reason_code":reason});
    if let Some(volumes) = extra
        .pointer("/storage_pressure/volumes")
        .and_then(Value::as_array)
    {
        let safe = volumes.iter().take(8).filter_map(|volume| {
            let role = volume.get("role")?.as_str()?;
            if !matches!(role, "system" | "workspace" | "temporary" | "database" | "audit_database") {
                return None;
            }
            let status = volume.get("state")?.as_str()?;
            if !matches!(status, "critical" | "unavailable") {
                return None;
            }
            let available = volume.get("available_bytes").and_then(Value::as_u64);
            let reserve = volume.get("critical_reserve_bytes").and_then(Value::as_u64);
            Some(json!({"role":role,"state":status,"available_bytes":available,
                "critical_reserve_bytes":reserve,
                "shortfall_bytes":reserve.zip(available).map(|(r,a)|r.saturating_add(1).saturating_sub(a))}))
        }).collect::<Vec<_>>();
        result["volumes"] = json!(safe);
    }
    if let Some(grant) = extra.get("resource_grant") {
        result["memory"] = json!({
            "available_bytes":grant.pointer("/host/memory_available_mb").and_then(Value::as_u64).and_then(|v|v.checked_mul(1024*1024)),
            "required_bytes":grant.pointer("/request/memory_mb").and_then(Value::as_u64).and_then(|v|v.checked_mul(1024*1024)),
            "safety_reserve_bytes":grant.get("host_safety_reserve_mb").and_then(Value::as_u64).and_then(|v|v.checked_mul(1024*1024)),
        });
        result["cpu"] = json!({
            "available_cores":grant.pointer("/host/cpu_cores").and_then(Value::as_u64),
            "required_cores":grant.pointer("/request/cpu_cores").and_then(Value::as_u64),
        });
    }
    Some(result)
}

#[cfg(test)]
#[path = "resource_wait_tests.rs"]
mod tests;

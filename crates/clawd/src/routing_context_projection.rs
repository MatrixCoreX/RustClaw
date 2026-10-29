use super::*;

pub(crate) fn bounded_prior_context_value(value: &Value) -> (Option<Value>, bool) {
    let safe = strip_prior_task_locators(redacted_context_value(value));
    let mut truncated = false;
    let projected = project(&safe, 0, &mut 2048, &mut truncated, ProjectionKind::Preview);
    (projected, truncated)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProjectionKind {
    Preview,
    ArgumentEvidence,
}

fn project(
    value: &Value,
    depth: usize,
    budget: &mut usize,
    truncated: &mut bool,
    kind: ProjectionKind,
) -> Option<Value> {
    if depth > 5 || *budget < 8 {
        *truncated = true;
        return None;
    }
    let projected = match value {
        Value::Object(fields) => {
            *truncated |= fields.len() > 32;
            Value::Object(
                fields
                    .iter()
                    .take(32)
                    .filter_map(|(key, value)| {
                        if key.len() > 64
                            || (kind == ProjectionKind::Preview && is_prior_task_locator_key(key))
                        {
                            *truncated = true;
                            return None;
                        }
                        *budget = budget.saturating_sub(key.len() + 4);
                        project(value, depth + 1, budget, truncated, kind)
                            .map(|value| (key.clone(), value))
                    })
                    .collect(),
            )
        }
        Value::Array(items) => {
            *truncated |= items.len() > 16;
            Value::Array(
                items
                    .iter()
                    .take(16)
                    .filter_map(|value| project(value, depth + 1, budget, truncated, kind))
                    .collect(),
            )
        }
        Value::String(text) if text.len() > 512 || is_private_artifact_locator(text) => {
            *truncated = true;
            return None;
        }
        other => {
            let size = other.to_string().len();
            if size > *budget {
                *truncated = true;
                return None;
            }
            *budget -= size;
            other.clone()
        }
    };
    match &projected {
        Value::Null if kind == ProjectionKind::Preview => None,
        Value::Object(fields) if fields.is_empty() && kind == ProjectionKind::Preview => None,
        Value::Array(items) if items.is_empty() && kind == ProjectionKind::Preview => None,
        _ => Some(projected),
    }
}

pub(crate) fn bounded_execution_argument_value(
    value: &Value,
) -> (Option<Value>, bool, Vec<String>) {
    let Ok(safe) = crate::task_context_privacy::redact(value, 0) else {
        return (None, true, Vec::new());
    };
    let kind = ProjectionKind::ArgumentEvidence;
    let mut budget = 2048;
    let mut truncated = false;
    let mut complete_fields = Vec::new();
    let Some(fields) = safe.as_object() else {
        return (
            project(&safe, 0, &mut budget, &mut truncated, kind),
            truncated,
            complete_fields,
        );
    };
    let mut output = serde_json::Map::new();
    truncated |= fields.len() > 32;
    for (key, value) in fields.iter().take(32) {
        if key.len() > 64 {
            truncated = true;
            continue;
        }
        budget = budget.saturating_sub(key.len() + 4);
        let mut field_truncated = false;
        if let Some(value) = project(value, 1, &mut budget, &mut field_truncated, kind) {
            output.insert(key.clone(), value);
            if !field_truncated {
                complete_fields.push(key.clone());
            }
        }
        truncated |= field_truncated;
    }
    (Some(Value::Object(output)), truncated, complete_fields)
}

#[cfg(test)]
#[path = "routing_context_projection_tests.rs"]
mod tests;

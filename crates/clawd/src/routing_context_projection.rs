use super::*;

pub(crate) fn bounded_prior_context_value(value: &Value) -> (Option<Value>, bool) {
    fn project(
        value: &Value,
        depth: usize,
        budget: &mut usize,
        truncated: &mut bool,
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
                            if key.len() > 64 || is_prior_task_locator_key(key) {
                                *truncated = true;
                                return None;
                            }
                            *budget = budget.saturating_sub(key.len() + 4);
                            project(value, depth + 1, budget, truncated)
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
                        .filter_map(|value| project(value, depth + 1, budget, truncated))
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
            Value::Null => None,
            Value::Object(fields) if fields.is_empty() => None,
            Value::Array(items) if items.is_empty() => None,
            _ => Some(projected),
        }
    }
    let safe = strip_prior_task_locators(redacted_context_value(value));
    let mut truncated = false;
    let projected = project(&safe, 0, &mut 2048, &mut truncated);
    (projected, truncated)
}

#[cfg(test)]
#[path = "routing_context_projection_tests.rs"]
mod tests;

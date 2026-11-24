//! Lossless references for repeated values inside one host capability result.
use serde_json::{json, Value};

pub(crate) const REFERENCE_CONTRACT: &str = "same_result_json_pointer_v1";

pub(crate) fn project_capability_evidence(value: Value) -> (Value, bool) {
    project(value, None)
}

pub(crate) fn project_capability_evidence_from_source(
    value: Value,
    source: &Value,
) -> (Value, bool) {
    project(value, Some(source))
}

fn has_reserved_keys(value: &Value) -> bool {
    value.pointer("/data/output_reference").is_some()
        || value.pointer("/data/output/field_references").is_some()
        || value.pointer("/data/output/extra_reference").is_some()
}

fn project(mut value: Value, source: Option<&Value>) -> (Value, bool) {
    // Producer-owned keys never become host reference instructions.
    if has_reserved_keys(&value) || source.is_some_and(has_reserved_keys) {
        return (value, false);
    }
    let source_agrees = |left: &str, right: &str| {
        source.is_none_or(|source| {
            source
                .pointer(left)
                .is_some_and(|value| Some(value) == source.pointer(right))
        })
    };
    let equal_output = value.pointer("/data/output").is_some_and(|output| {
        !output.is_null()
            && output.to_string().len() >= 512
            && Some(output) == value.pointer("/data/extra")
            && source_agrees("/data/output", "/data/extra")
    });
    if equal_output {
        let data = value
            .get_mut("data")
            .and_then(Value::as_object_mut)
            .unwrap();
        data.remove("output");
        data.insert("output_reference".into(), json!("/data/extra"));
        return (value, true);
    }
    let mut referenced = false;
    let equal_extra = value.pointer("/data/extra").is_some_and(|extra| {
        !extra.is_null()
            && Some(extra) == value.pointer("/data/output/extra")
            && source_agrees("/data/output/extra", "/data/extra")
    });
    if equal_extra {
        if let Some(output) = value
            .pointer_mut("/data/output")
            .and_then(Value::as_object_mut)
        {
            output.remove("extra");
            output.insert("extra_reference".into(), json!("data.extra"));
            referenced = true;
        }
    }
    let shared = value
        .pointer("/data/output")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, field)| {
            (field.to_string().len()
                > json!(format!(
                    "/data/extra/{}",
                    key.replace('~', "~0").replace('/', "~1")
                ))
                .to_string()
                .len()
                    + 32
                && value
                    .pointer("/data/extra")
                    .and_then(|extra| extra.get(key))
                    == Some(field)
                && source_agrees(
                    &format!("/data/output/{}", key.replace('~', "~0").replace('/', "~1")),
                    &format!("/data/extra/{}", key.replace('~', "~0").replace('/', "~1")),
                ))
            .then(|| {
                (
                    key.clone(),
                    format!("/data/extra/{}", key.replace('~', "~0").replace('/', "~1")),
                )
            })
        })
        .collect::<Vec<_>>();
    // Charge the reference-map overhead too; tiny repeated facts stay inline.
    let removed_bytes = shared
        .iter()
        .map(|(key, _)| {
            json!(key).to_string().len() + value["data"]["output"][key].to_string().len() + 2
        })
        .sum::<usize>();
    let reference_map = shared
        .iter()
        .map(|(key, pointer)| (key.clone(), json!(pointer)))
        .collect::<serde_json::Map<_, _>>();
    let added_bytes = json!("field_references").to_string().len()
        + Value::Object(reference_map).to_string().len()
        + 2;
    if !shared.is_empty() && removed_bytes > added_bytes + 128 {
        if let Some(output) = value
            .pointer_mut("/data/output")
            .and_then(Value::as_object_mut)
        {
            let mut references = serde_json::Map::new();
            for (key, pointer) in shared {
                output.remove(&key);
                references.insert(key, json!(pointer));
            }
            output.insert("field_references".into(), references.into());
            referenced = true;
        }
    }
    (value, referenced)
}

#[cfg(test)]
#[path = "prompt_projection_capability_tests.rs"]
mod tests;

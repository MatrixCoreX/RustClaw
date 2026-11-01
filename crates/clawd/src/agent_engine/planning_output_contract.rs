use serde_json::Value;

use crate::pipeline_types::OutputSelectionContract;
use crate::{IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape};

pub(super) fn parse_planner_output_contract(raw: &str) -> Option<IntentOutputContract> {
    let value = crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw)?;
    let contract = value.get("output_contract")?.as_object()?;

    let response_shape = match machine_token(contract.get("response_shape"))? {
        "free" => OutputResponseShape::Free,
        "one_sentence" => OutputResponseShape::OneSentence,
        "strict" => OutputResponseShape::Strict,
        "scalar" => OutputResponseShape::Scalar,
        "file_token" => OutputResponseShape::FileToken,
        _ => return None,
    };
    let locator_kind = match machine_token(contract.get("locator_kind"))? {
        "none" => OutputLocatorKind::None,
        "path" => OutputLocatorKind::Path,
        "current_workspace" => OutputLocatorKind::CurrentWorkspace,
        "url" => OutputLocatorKind::Url,
        "filename" => OutputLocatorKind::Filename,
        _ => return None,
    };
    let delivery_intent = match machine_token(contract.get("delivery_intent"))? {
        "none" => OutputDeliveryIntent::None,
        "file_single" => OutputDeliveryIntent::FileSingle,
        "directory_lookup" => OutputDeliveryIntent::DirectoryLookup,
        "directory_batch_files" => OutputDeliveryIntent::DirectoryBatchFiles,
        _ => return None,
    };
    let requires_content_evidence = contract
        .get("requires_content_evidence")
        .and_then(Value::as_bool)?;
    let delivery_required = contract.get("delivery_required").and_then(Value::as_bool)?;
    let exact_sentence_count = contract
        .get("exact_sentence_count")
        .filter(|value| !value.is_null())
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| (1..=20).contains(value));
    let structured_field_selector =
        optional_machine_selector(contract.get("structured_field_selector"))?;
    let output_field_labels = parse_output_field_labels(contract.get("output_field_labels"))?;
    // A source projection and a semantic presentation are different contracts.
    // Never infer an alias mapping or waive source evidence from output labels.
    if !output_field_labels.is_empty()
        && (structured_field_selector.is_some() || response_shape != OutputResponseShape::Strict)
    {
        return None;
    }

    Some(IntentOutputContract {
        response_shape,
        exact_sentence_count,
        requires_content_evidence,
        delivery_required,
        locator_kind,
        delivery_intent,
        selection: OutputSelectionContract {
            structured_field_selector,
            output_field_labels,
            ..OutputSelectionContract::default()
        },
        ..IntentOutputContract::default()
    })
}

fn machine_token(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn parse_output_field_labels(value: Option<&Value>) -> Option<Vec<String>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Some(Vec::new());
    };
    let labels = value
        .as_array()?
        .iter()
        .map(|item| {
            let label = item.as_str()?;
            (!label.is_empty() && !label.chars().any(char::is_control)).then(|| label.to_string())
        })
        .collect::<Option<Vec<_>>>()?;
    (labels
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        == labels.len())
    .then_some(labels)
}

fn optional_machine_selector(value: Option<&Value>) -> Option<Option<String>> {
    let Some(value) = value else {
        return Some(None);
    };
    if value.is_null() {
        return Some(None);
    }
    let selector = value.as_str()?.trim();
    if selector.is_empty()
        || selector.len() > 256
        || !selector.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(ch, '_' | '.' | ',' | '*' | '[' | ']' | '-' | ' ')
        })
    {
        return None;
    }
    Some(Some(selector.to_string()))
}

#[cfg(test)]
#[path = "planning_output_contract_tests.rs"]
mod tests;

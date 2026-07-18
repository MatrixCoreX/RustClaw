use serde_json::Value;

use crate::pipeline_types::OutputSelectionContract;
use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind,
};

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
    let result_kind_token = machine_token(contract.get("result_kind"))?;
    let semantic_kind = OutputSemanticKind::ALL
        .iter()
        .copied()
        .find(|kind| kind.as_str() == result_kind_token)?;
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

    Some(IntentOutputContract {
        response_shape,
        exact_sentence_count,
        requires_content_evidence,
        delivery_required,
        locator_kind,
        delivery_intent,
        semantic_kind,
        selection: OutputSelectionContract {
            structured_field_selector,
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

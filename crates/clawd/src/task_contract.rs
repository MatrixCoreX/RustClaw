use std::collections::BTreeSet;

use crate::{OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskTargetObject {
    Path,
    Directory,
    Web,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskOperation {
    Inspect,
    List,
    Count,
    Read,
    Run,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskDeliveryShape {
    OneSentence,
    List,
    Raw,
    File,
    Summary,
}

pub(crate) fn target_object_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> TaskTargetObject {
    target_object_for_locator_kind(output_contract.locator_kind)
}

fn target_object_for_locator_kind(locator_kind: OutputLocatorKind) -> TaskTargetObject {
    match locator_kind {
        OutputLocatorKind::Path | OutputLocatorKind::Filename => TaskTargetObject::Path,
        OutputLocatorKind::CurrentWorkspace => TaskTargetObject::Directory,
        OutputLocatorKind::Url => TaskTargetObject::Web,
        OutputLocatorKind::None => TaskTargetObject::Unknown,
    }
}

pub(crate) fn operation_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> TaskOperation {
    if output_contract.requests_exact_list() {
        return TaskOperation::List;
    }
    if output_contract.requests_exact_count() {
        return TaskOperation::Count;
    }
    if output_contract.requests_exact_command_output() {
        return TaskOperation::Run;
    }
    operation_for_unclassified_output_contract(output_contract)
}

fn operation_for_unclassified_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> TaskOperation {
    if output_contract.delivery_required {
        TaskOperation::Read
    } else if output_contract.requires_content_evidence
        || !matches!(output_contract.locator_kind, OutputLocatorKind::None)
        || !output_contract.locator_hint.trim().is_empty()
    {
        TaskOperation::Inspect
    } else {
        TaskOperation::Unknown
    }
}

pub(crate) fn delivery_shape_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> TaskDeliveryShape {
    if output_contract.requests_exact_list() {
        return TaskDeliveryShape::List;
    }
    delivery_shape_for_response_shape(output_contract.response_shape)
}

fn delivery_shape_for_response_shape(response_shape: OutputResponseShape) -> TaskDeliveryShape {
    match response_shape {
        OutputResponseShape::OneSentence => TaskDeliveryShape::OneSentence,
        OutputResponseShape::Strict | OutputResponseShape::Scalar => TaskDeliveryShape::Raw,
        OutputResponseShape::FileToken => TaskDeliveryShape::File,
        OutputResponseShape::Free => TaskDeliveryShape::Summary,
    }
}

pub(crate) fn required_evidence_fields_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> Vec<String> {
    if output_contract.requests_exact_structured_fields() {
        if let Some(fields) = output_contract
            .selection
            .structured_field_selector
            .as_deref()
            .and_then(crate::machine_kv_projection::exact_machine_field_selector)
        {
            return fields;
        }
    }
    let fallback = fallback_required_evidence_fields_for_output_contract(output_contract);
    match crate::evidence_policy::required_evidence_for_output_contract(output_contract) {
        Some(fields) if !fields.is_empty() => fields,
        Some(_) if !fallback.is_empty() => fallback,
        Some(fields) => fields,
        None => fallback,
    }
}

pub(crate) fn fallback_required_evidence_fields_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> Vec<String> {
    let mut fields = BTreeSet::new();
    if output_contract.delivery_required
        || matches!(
            output_contract.delivery_intent,
            OutputDeliveryIntent::FileSingle
                | OutputDeliveryIntent::DirectoryLookup
                | OutputDeliveryIntent::DirectoryBatchFiles
        )
    {
        fields.insert("path");
    }
    if output_contract.requests_exact_name_list() {
        fields.insert("candidates");
    }
    if output_contract.requests_exact_command_output() {
        fields.insert("command_output");
    }
    fields.into_iter().map(str::to_string).collect()
}

#[cfg(test)]
#[path = "task_contract_tests.rs"]
mod tests;

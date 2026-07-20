use std::collections::BTreeSet;

use crate::{OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, OutputSemanticKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskTargetObject {
    Path,
    Directory,
    ConfigKey,
    Service,
    Process,
    Db,
    System,
    Web,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskOperation {
    Inspect,
    List,
    Count,
    Read,
    Write,
    Modify,
    Run,
    Validate,
    Summarize,
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
    let semantic_kind = output_contract.semantic_kind;
    if let Some(target) = matrix_contract_for_output_contract(output_contract)
        .and_then(|contract| task_target_object_from_token(&contract.target_object))
    {
        return target;
    }
    match semantic_kind {
        OutputSemanticKind::ServiceStatus => return TaskTargetObject::Service,
        OutputSemanticKind::FilesystemMutationResult => return TaskTargetObject::Path,
        OutputSemanticKind::CommandOutputSummary => return TaskTargetObject::System,
        _ => {}
    }
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
    let semantic_kind = output_contract.semantic_kind;
    if let Some(operation) = matrix_contract_for_output_contract(output_contract)
        .and_then(|contract| task_operation_from_token(&contract.operation))
    {
        return operation;
    }
    match semantic_kind {
        OutputSemanticKind::RawCommandOutput => TaskOperation::Run,
        OutputSemanticKind::FileNames
        | OutputSemanticKind::DirectoryNames
        | OutputSemanticKind::DirectoryEntryGroups
        | OutputSemanticKind::FilePaths => TaskOperation::List,
        OutputSemanticKind::ScalarCount => TaskOperation::Count,
        OutputSemanticKind::CommandOutputSummary
        | OutputSemanticKind::ContentExcerptSummary
        | OutputSemanticKind::ContentExcerptWithSummary => TaskOperation::Summarize,
        OutputSemanticKind::GeneratedFileDelivery
        | OutputSemanticKind::GeneratedFilePathReport
        | OutputSemanticKind::FilesystemMutationResult => TaskOperation::Write,
        OutputSemanticKind::ServiceStatus
        | OutputSemanticKind::ScalarPathOnly
        | OutputSemanticKind::ExistenceWithPath => TaskOperation::Inspect,
        OutputSemanticKind::QuantityComparison => TaskOperation::Validate,
        OutputSemanticKind::ExecutionFailedStep => TaskOperation::Validate,
        OutputSemanticKind::None => operation_for_unclassified_output_contract(output_contract),
        _ => operation_for_unclassified_output_contract(output_contract),
    }
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
    matrix_contract_for_output_contract(output_contract)
        .and_then(|contract| task_delivery_shape_from_token(&contract.delivery_shape))
        .unwrap_or_else(|| delivery_shape_for_response_shape(output_contract.response_shape))
}

fn delivery_shape_for_response_shape(response_shape: OutputResponseShape) -> TaskDeliveryShape {
    match response_shape {
        OutputResponseShape::OneSentence => TaskDeliveryShape::OneSentence,
        OutputResponseShape::Strict | OutputResponseShape::Scalar => TaskDeliveryShape::Raw,
        OutputResponseShape::FileToken => TaskDeliveryShape::File,
        OutputResponseShape::Free => TaskDeliveryShape::Summary,
    }
}

fn matrix_contract_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> Option<&'static crate::contract_matrix::MatrixContract> {
    if output_contract.semantic_kind == OutputSemanticKind::None {
        return None;
    }
    crate::contract_matrix::bundled_contract_matrix()
        .and_then(|matrix| matrix.semantic_contract(output_contract.semantic_kind))
}

fn task_target_object_from_token(value: &str) -> Option<TaskTargetObject> {
    match value.trim() {
        "path" => Some(TaskTargetObject::Path),
        "directory" => Some(TaskTargetObject::Directory),
        "config_key" => Some(TaskTargetObject::ConfigKey),
        "service" => Some(TaskTargetObject::Service),
        "process" => Some(TaskTargetObject::Process),
        "db" => Some(TaskTargetObject::Db),
        "system" => Some(TaskTargetObject::System),
        "web" => Some(TaskTargetObject::Web),
        "unknown" => Some(TaskTargetObject::Unknown),
        _ => None,
    }
}

fn task_operation_from_token(value: &str) -> Option<TaskOperation> {
    match value.trim() {
        "inspect" => Some(TaskOperation::Inspect),
        "list" => Some(TaskOperation::List),
        "count" => Some(TaskOperation::Count),
        "read" => Some(TaskOperation::Read),
        "write" => Some(TaskOperation::Write),
        "modify" => Some(TaskOperation::Modify),
        "run" => Some(TaskOperation::Run),
        "validate" => Some(TaskOperation::Validate),
        "summarize" => Some(TaskOperation::Summarize),
        "unknown" => Some(TaskOperation::Unknown),
        _ => None,
    }
}

fn task_delivery_shape_from_token(value: &str) -> Option<TaskDeliveryShape> {
    match value.trim() {
        "one_sentence" => Some(TaskDeliveryShape::OneSentence),
        "list" => Some(TaskDeliveryShape::List),
        "raw" => Some(TaskDeliveryShape::Raw),
        "file" => Some(TaskDeliveryShape::File),
        "summary" => Some(TaskDeliveryShape::Summary),
        _ => None,
    }
}

pub(crate) fn required_evidence_fields_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> Vec<String> {
    if let Some(fields) = output_contract
        .selection
        .structured_field_selector
        .as_deref()
        .and_then(crate::machine_kv_projection::exact_machine_field_selector)
    {
        return fields;
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
    match output_contract.semantic_kind {
        OutputSemanticKind::RawCommandOutput | OutputSemanticKind::CommandOutputSummary => {
            fields.insert("command_output");
        }
        OutputSemanticKind::ScalarCount => {
            fields.insert("count");
        }
        OutputSemanticKind::ScalarPathOnly => {
            fields.insert("field_value");
        }
        OutputSemanticKind::ExistenceWithPath => {
            fields.insert("exists");
            fields.insert("kind");
            fields.insert("path");
        }
        OutputSemanticKind::ContentExcerptSummary
        | OutputSemanticKind::ContentExcerptWithSummary => {
            fields.insert("content_excerpt");
        }
        OutputSemanticKind::FileNames
        | OutputSemanticKind::DirectoryNames
        | OutputSemanticKind::DirectoryEntryGroups
        | OutputSemanticKind::FilePaths => {
            fields.insert("candidates");
        }
        OutputSemanticKind::ServiceStatus => {
            fields.insert("field_value");
        }
        OutputSemanticKind::ExecutionFailedStep => {
            fields.insert("command_output");
        }
        OutputSemanticKind::ConfigValidation => {
            fields.insert("valid");
        }
        OutputSemanticKind::ConfigMutation => {
            fields.insert("field_value");
            fields.insert("path");
            fields.insert("valid");
        }
        OutputSemanticKind::ConfigRiskAssessment => {
            fields.insert("candidates");
            fields.insert("count");
        }
        OutputSemanticKind::GeneratedFileDelivery
        | OutputSemanticKind::GeneratedFilePathReport
        | OutputSemanticKind::FilesystemMutationResult => {
            fields.insert("path");
        }
        OutputSemanticKind::QuantityComparison => {
            fields.insert("field_value");
            fields.insert("size_bytes");
            if matches!(
                output_contract.locator_kind,
                OutputLocatorKind::Path
                    | OutputLocatorKind::Filename
                    | OutputLocatorKind::CurrentWorkspace
            ) {
                fields.insert("exists");
                fields.insert("kind");
            }
        }
        _ => {}
    }
    fields.into_iter().map(str::to_string).collect()
}

#[cfg(test)]
#[path = "task_contract_tests.rs"]
mod tests;

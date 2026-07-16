use std::collections::BTreeSet;

use serde_json::json;

use crate::{
    OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, OutputSemanticKind, RouteResult,
};

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

impl TaskTargetObject {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Directory => "directory",
            Self::ConfigKey => "config_key",
            Self::Service => "service",
            Self::Process => "process",
            Self::Db => "db",
            Self::System => "system",
            Self::Web => "web",
            Self::Unknown => "unknown",
        }
    }
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

impl TaskOperation {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::List => "list",
            Self::Count => "count",
            Self::Read => "read",
            Self::Write => "write",
            Self::Modify => "modify",
            Self::Run => "run",
            Self::Validate => "validate",
            Self::Summarize => "summarize",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskDeliveryShape {
    OneSentence,
    List,
    Raw,
    File,
    Summary,
}

impl TaskDeliveryShape {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::OneSentence => "one_sentence",
            Self::List => "list",
            Self::Raw => "raw",
            Self::File => "file",
            Self::Summary => "summary",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskFailurePolicy {
    NoRetry,
    RetryWithAlternatives,
}

impl TaskFailurePolicy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoRetry => "no_retry",
            Self::RetryWithAlternatives => "retry_with_alternatives",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskTargetRole {
    Primary,
    Delivery,
}

impl TaskTargetRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Delivery => "delivery",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetContract {
    pub(crate) role: TaskTargetRole,
    pub(crate) kind: TaskTargetObject,
    pub(crate) locator: String,
}

impl TargetContract {
    fn compact_json(&self) -> serde_json::Value {
        json!({
            "role": self.role.as_str(),
            "kind": self.kind.as_str(),
            "locator": self.locator,
        })
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidencePolicyContract {
    pub(crate) targets: Vec<TargetContract>,
    pub(crate) target_object: TaskTargetObject,
    pub(crate) structured_field_selector: Option<String>,
    pub(crate) operation: TaskOperation,
    pub(crate) evidence_required: bool,
    pub(crate) required_evidence_fields: Vec<String>,
    pub(crate) delivery_shape: TaskDeliveryShape,
    pub(crate) missing_parameters: Vec<String>,
    pub(crate) failure_policy: TaskFailurePolicy,
}

#[cfg(test)]
impl EvidencePolicyContract {
    pub(crate) fn from_output_contract(output_contract: &crate::IntentOutputContract) -> Self {
        let required_evidence_fields =
            required_evidence_fields_for_output_contract(output_contract);
        let evidence_required = output_contract.requires_content_evidence
            || output_contract.delivery_required
            || !required_evidence_fields.is_empty();
        Self {
            targets: targets_for_output_contract(output_contract),
            target_object: target_object_for_output_contract(output_contract),
            structured_field_selector: output_contract
                .self_extension
                .structured_field_selector
                .clone(),
            operation: operation_for_output_contract(output_contract),
            evidence_required,
            required_evidence_fields,
            delivery_shape: delivery_shape_for_output_contract(output_contract),
            failure_policy: if evidence_required {
                TaskFailurePolicy::RetryWithAlternatives
            } else {
                TaskFailurePolicy::NoRetry
            },
            missing_parameters: Vec::new(),
        }
    }
}

#[cfg(test)]
pub(crate) fn evidence_policy_context_prompt_line_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> String {
    let missing_parameters = Vec::new();
    let required_evidence_fields = required_evidence_fields_for_output_contract(output_contract);
    let evidence_required = output_contract.requires_content_evidence
        || output_contract.delivery_required
        || !required_evidence_fields.is_empty();
    compact_prompt_line_with_fields(
        "evidence_policy_context",
        &targets_for_output_contract(output_contract),
        target_object_for_output_contract(output_contract),
        output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
        operation_for_output_contract(output_contract),
        evidence_required,
        &required_evidence_fields,
        delivery_shape_for_output_contract(output_contract),
        &missing_parameters,
        if evidence_required {
            TaskFailurePolicy::RetryWithAlternatives
        } else {
            TaskFailurePolicy::NoRetry
        },
    )
}

fn compact_prompt_line_with_fields(
    label: &str,
    targets: &[TargetContract],
    target_object: TaskTargetObject,
    structured_field_selector: Option<&str>,
    operation: TaskOperation,
    evidence_required: bool,
    required_evidence_fields: &[String],
    delivery_shape: TaskDeliveryShape,
    missing_parameters: &[String],
    failure_policy: TaskFailurePolicy,
) -> String {
    let required_evidence = if required_evidence_fields.is_empty() {
        "none".to_string()
    } else {
        required_evidence_fields.join(",")
    };
    let missing_parameters = if missing_parameters.is_empty() {
        "none".to_string()
    } else {
        missing_parameters.join(",")
    };
    let targets = if targets.is_empty() {
        "none".to_string()
    } else {
        let values = targets
            .iter()
            .map(TargetContract::compact_json)
            .collect::<Vec<_>>();
        serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_string())
    };
    format!(
        "- {label} targets={} target_object={} structured_field_selector={} operation={} evidence_required={} required_evidence_fields={} delivery_shape={} missing_parameters={} failure_policy={}",
        targets,
        target_object.as_str(),
        structured_field_selector.unwrap_or("none"),
        operation.as_str(),
        evidence_required,
        required_evidence,
        delivery_shape.as_str(),
        missing_parameters,
        failure_policy.as_str(),
    )
}

#[cfg(test)]
fn targets_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> Vec<TargetContract> {
    let mut targets = Vec::new();
    let primary_kind = target_object_for_output_contract(output_contract);
    let primary_locators = target_locators_for_output_contract(output_contract);
    if primary_locators.is_empty() && primary_kind != TaskTargetObject::Unknown {
        targets.push(TargetContract {
            role: TaskTargetRole::Primary,
            kind: primary_kind,
            locator: String::new(),
        });
    } else {
        for locator in primary_locators {
            targets.push(TargetContract {
                role: TaskTargetRole::Primary,
                kind: primary_kind,
                locator,
            });
        }
    }
    if output_contract.delivery_required
        && !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
    {
        let locator = output_contract.locator_hint.trim();
        if !locator.is_empty() {
            targets.push(TargetContract {
                role: TaskTargetRole::Delivery,
                kind: TaskTargetObject::Path,
                locator: locator.to_string(),
            });
        }
    }
    targets
}

#[cfg(test)]
pub(crate) fn target_locators_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> Vec<String> {
    split_locator_targets(&primary_locator_for_output_contract(output_contract))
}

fn split_locator_targets(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Vec::new();
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
        if let Some(items) = value.as_array() {
            return dedup_locators(
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(normalize_target_locator),
            );
        }
    }

    dedup_locators(
        raw.split(['|', '\n', ';', ',', '、'])
            .map(normalize_target_locator),
    )
}

fn normalize_target_locator(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '[' | ']'))
        .trim()
        .to_string()
}

fn dedup_locators<I>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let key = value.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(value.to_string());
        }
    }
    out
}

#[cfg(test)]
fn primary_locator_for_output_contract(output_contract: &crate::IntentOutputContract) -> String {
    let locator = output_contract.locator_hint.trim();
    if !locator.is_empty() {
        return locator.to_string();
    }
    match output_contract.locator_kind {
        OutputLocatorKind::CurrentWorkspace => ".".to_string(),
        _ => String::new(),
    }
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
        OutputSemanticKind::StructuredKeys => return TaskTargetObject::ConfigKey,
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

pub(crate) fn operation_for_route(route: &RouteResult) -> TaskOperation {
    operation_for_output_contract(&route.effective_output_contract())
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
        | OutputSemanticKind::ContentExcerptWithSummary
        | OutputSemanticKind::DirectoryPurposeSummary
        | OutputSemanticKind::WorkspaceProjectSummary
        | OutputSemanticKind::ExistenceWithPathSummary
        | OutputSemanticKind::RecentArtifactsJudgment
        | OutputSemanticKind::ExcerptKindJudgment => TaskOperation::Summarize,
        OutputSemanticKind::GeneratedFileDelivery
        | OutputSemanticKind::GeneratedFilePathReport
        | OutputSemanticKind::FilesystemMutationResult => TaskOperation::Write,
        OutputSemanticKind::ServiceStatus
        | OutputSemanticKind::HiddenEntriesCheck
        | OutputSemanticKind::ContentPresenceCheck
        | OutputSemanticKind::DocumentHeading
        | OutputSemanticKind::ScalarPathOnly
        | OutputSemanticKind::FileBasename
        | OutputSemanticKind::ExistenceWithPath
        | OutputSemanticKind::StructuredKeys => TaskOperation::Inspect,
        OutputSemanticKind::QuantityComparison | OutputSemanticKind::RecentScalarEqualityCheck => {
            TaskOperation::Validate
        }
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

pub(crate) fn delivery_shape_for_route(route: &RouteResult) -> TaskDeliveryShape {
    delivery_shape_for_output_contract(&route.effective_output_contract())
}

fn delivery_shape_for_output_contract(
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

pub(crate) fn required_evidence_fields_for_route(route: &RouteResult) -> Vec<String> {
    let output_contract = route.effective_output_contract();
    required_evidence_fields_for_output_contract(&output_contract)
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
        OutputSemanticKind::ScalarPathOnly
        | OutputSemanticKind::FileBasename
        | OutputSemanticKind::GitCommitSubject
        | OutputSemanticKind::GitRepositoryState => {
            fields.insert("field_value");
        }
        OutputSemanticKind::ExistenceWithPath | OutputSemanticKind::ExistenceWithPathSummary => {
            fields.insert("exists");
            fields.insert("kind");
            fields.insert("path");
        }
        OutputSemanticKind::ContentExcerptSummary
        | OutputSemanticKind::ContentExcerptWithSummary
        | OutputSemanticKind::ArchiveRead
        | OutputSemanticKind::DocumentHeading
        | OutputSemanticKind::ExcerptKindJudgment => {
            fields.insert("content_excerpt");
        }
        OutputSemanticKind::WorkspaceProjectSummary => {
            fields.insert("candidates");
            fields.insert("content_excerpt");
        }
        OutputSemanticKind::ContentPresenceCheck => {
            fields.insert("content_match");
            fields.insert("content_excerpt");
            fields.insert("field_value");
        }
        OutputSemanticKind::DirectoryPurposeSummary => {
            fields.insert("candidates");
        }
        OutputSemanticKind::RecentArtifactsJudgment => {
            fields.insert("candidates");
            fields.insert("content_excerpt");
        }
        OutputSemanticKind::FileNames
        | OutputSemanticKind::DirectoryNames
        | OutputSemanticKind::DirectoryEntryGroups
        | OutputSemanticKind::FilePaths
        | OutputSemanticKind::HiddenEntriesCheck
        | OutputSemanticKind::SqliteTableListing
        | OutputSemanticKind::SqliteTableNamesOnly
        | OutputSemanticKind::ArchiveList => {
            fields.insert("candidates");
        }
        OutputSemanticKind::ServiceStatus
        | OutputSemanticKind::StructuredKeys
        | OutputSemanticKind::SqliteDatabaseKindJudgment
        | OutputSemanticKind::SqliteSchemaVersion
        | OutputSemanticKind::RecentScalarEqualityCheck => {
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
        | OutputSemanticKind::FilesystemMutationResult
        | OutputSemanticKind::ArchivePack
        | OutputSemanticKind::ArchiveUnpack => {
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
    if output_contract.semantic_kind_is(OutputSemanticKind::ExistenceWithPathSummary) {
        fields.insert("content_excerpt");
    }
    if output_contract.semantic_kind_is(OutputSemanticKind::DocumentHeading) {
        fields.insert("path");
    }
    fields.into_iter().map(str::to_string).collect()
}

#[cfg(test)]
#[path = "task_contract_tests.rs"]
mod tests;

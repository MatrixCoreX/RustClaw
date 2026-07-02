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
    Clarify,
}

impl TaskFailurePolicy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoRetry => "no_retry",
            Self::RetryWithAlternatives => "retry_with_alternatives",
            Self::Clarify => "clarify",
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
    pub(crate) fn from_route_result(route: &RouteResult) -> Self {
        let missing_parameters = missing_parameters_for_route(route);
        let evidence_required = evidence_required_for_route(route);
        Self {
            targets: targets_for_route(route),
            target_object: target_object_for_route(route),
            structured_field_selector: route
                .output_contract
                .self_extension
                .structured_field_selector
                .clone(),
            operation: operation_for_route(route),
            evidence_required,
            required_evidence_fields: required_evidence_fields_for_route(route),
            delivery_shape: delivery_shape_for_route(route),
            failure_policy: failure_policy_for_route(route, evidence_required, &missing_parameters),
            missing_parameters,
        }
    }
}

pub(crate) fn evidence_policy_context_prompt_line_for_route(route: &RouteResult) -> String {
    let missing_parameters = missing_parameters_for_route(route);
    let required_evidence_fields = required_evidence_fields_for_route(route);
    let evidence_required = route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !required_evidence_fields.is_empty();
    compact_prompt_line_with_fields(
        "evidence_policy_context",
        &targets_for_route(route),
        target_object_for_route(route),
        route
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
        operation_for_route(route),
        evidence_required,
        &required_evidence_fields,
        delivery_shape_for_route(route),
        &missing_parameters,
        failure_policy_for_route(route, evidence_required, &missing_parameters),
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

fn targets_for_route(route: &RouteResult) -> Vec<TargetContract> {
    let mut targets = Vec::new();
    let primary_kind = target_object_for_route(route);
    let primary_locators = target_locators_for_route(route);
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
    if route.output_contract.delivery_required
        && !matches!(
            route.output_contract.delivery_intent,
            OutputDeliveryIntent::None
        )
    {
        let locator = route.output_contract.locator_hint.trim();
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

pub(crate) fn target_locators_for_route(route: &RouteResult) -> Vec<String> {
    split_locator_targets(&primary_locator_for_route(route))
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

fn primary_locator_for_route(route: &RouteResult) -> String {
    let locator = route.output_contract.locator_hint.trim();
    if !locator.is_empty() {
        return locator.to_string();
    }
    match route.output_contract.locator_kind {
        OutputLocatorKind::CurrentWorkspace => ".".to_string(),
        _ => String::new(),
    }
}

fn target_object_for_capability_ref(route: &RouteResult) -> Option<TaskTargetObject> {
    let capability = crate::machine_capability_ref::route_first_capability_ref(route)?;
    Some(match capability.namespace() {
        "config" | "config_basic" | "config_edit" => TaskTargetObject::ConfigKey,
        "service" | "service_control" => TaskTargetObject::Service,
        "docker" | "process" => TaskTargetObject::Process,
        "db" | "database" | "sqlite" => TaskTargetObject::Db,
        "filesystem" | "fs_basic" | "file" | "archive" | "photo" => TaskTargetObject::Path,
        "git" => TaskTargetObject::Directory,
        "package" | "package_manager" | "module" | "system" | "system_basic" => {
            TaskTargetObject::System
        }
        "browser" | "http" | "image_vision" | "rss" | "stock" | "crypto" | "weather" | "web"
        | "x" => TaskTargetObject::Web,
        _ => TaskTargetObject::Unknown,
    })
}

pub(crate) fn target_object_for_route(route: &RouteResult) -> TaskTargetObject {
    if let Some(target) = target_object_for_capability_ref(route) {
        return target;
    }
    let semantic_kind = route.effective_output_contract_semantic_kind();
    if semantic_kind.is_normalizer_schema_capability_bridge() {
        return target_object_for_locator_kind(route.output_contract.locator_kind);
    }
    match semantic_kind {
        OutputSemanticKind::ServiceStatus => return TaskTargetObject::Service,
        OutputSemanticKind::FilesystemMutationResult => return TaskTargetObject::Path,
        OutputSemanticKind::StructuredKeys => return TaskTargetObject::ConfigKey,
        OutputSemanticKind::CommandOutputSummary => return TaskTargetObject::System,
        _ => {}
    }
    target_object_for_locator_kind(route.output_contract.locator_kind)
}

fn target_object_for_locator_kind(locator_kind: OutputLocatorKind) -> TaskTargetObject {
    match locator_kind {
        OutputLocatorKind::Path | OutputLocatorKind::Filename => TaskTargetObject::Path,
        OutputLocatorKind::CurrentWorkspace => TaskTargetObject::Directory,
        OutputLocatorKind::Url => TaskTargetObject::Web,
        OutputLocatorKind::None => TaskTargetObject::Unknown,
    }
}

fn operation_for_capability_ref(route: &RouteResult) -> Option<TaskOperation> {
    let capability = crate::machine_capability_ref::route_first_capability_ref(route)?;
    let action = capability.action();
    Some(if action_has_any_segment(action, &["count"]) {
        TaskOperation::Count
    } else if action_has_any_segment(action, &["list", "find", "search", "candidates"]) {
        TaskOperation::List
    } else if action_has_any_segment(
        action,
        &[
            "apply",
            "delete",
            "install",
            "kill",
            "publish",
            "remove",
            "restart",
            "start",
            "stop",
            "uninstall",
            "unpack",
        ],
    ) {
        TaskOperation::Modify
    } else if action_has_any_segment(
        action,
        &["append", "create", "generate", "make", "pack", "write"],
    ) {
        TaskOperation::Write
    } else if action_has_any_segment(action, &["check", "compare", "guard", "validate", "verify"]) {
        TaskOperation::Validate
    } else if action_has_any_segment(
        action,
        &[
            "analyze",
            "current",
            "describe",
            "positions",
            "quote",
            "summary",
            "summarize",
        ],
    ) {
        TaskOperation::Summarize
    } else if capability.namespace() == "system"
        && action_has_any_segment(action, &["run", "cmd", "command"])
    {
        TaskOperation::Run
    } else {
        TaskOperation::Inspect
    })
}

pub(crate) fn operation_for_route(route: &RouteResult) -> TaskOperation {
    if let Some(operation) = operation_for_capability_ref(route) {
        return operation;
    }
    let semantic_kind = route.effective_output_contract_semantic_kind();
    if semantic_kind.is_normalizer_schema_capability_bridge() {
        return operation_for_unclassified_route(route);
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
        OutputSemanticKind::None => operation_for_unclassified_route(route),
        _ => operation_for_unclassified_route(route),
    }
}

fn operation_for_unclassified_route(route: &RouteResult) -> TaskOperation {
    if route.output_contract.delivery_required {
        TaskOperation::Read
    } else if route.output_contract.requires_content_evidence
        || !matches!(route.output_contract.locator_kind, OutputLocatorKind::None)
        || !route.output_contract.locator_hint.trim().is_empty()
    {
        TaskOperation::Inspect
    } else {
        TaskOperation::Unknown
    }
}

fn delivery_shape_for_capability_ref(route: &RouteResult) -> Option<TaskDeliveryShape> {
    let capability = crate::machine_capability_ref::route_first_capability_ref(route)?;
    if action_has_any_segment(
        capability.action(),
        &["candidates", "find", "list", "search"],
    ) {
        return Some(TaskDeliveryShape::List);
    }
    None
}

pub(crate) fn delivery_shape_for_route(route: &RouteResult) -> TaskDeliveryShape {
    if let Some(shape) = delivery_shape_for_capability_ref(route) {
        return shape;
    }
    if route
        .effective_output_contract_semantic_kind()
        .is_normalizer_schema_capability_bridge()
    {
        return delivery_shape_for_response_shape(route.output_contract.response_shape);
    }
    if matches!(
        route.effective_output_contract_semantic_kind(),
        OutputSemanticKind::FileNames
            | OutputSemanticKind::DirectoryNames
            | OutputSemanticKind::DirectoryEntryGroups
            | OutputSemanticKind::FilePaths
    ) {
        return TaskDeliveryShape::List;
    }
    delivery_shape_for_response_shape(route.output_contract.response_shape)
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
    if let Some(fields) = required_evidence_fields_for_capability_ref(route) {
        return fields;
    }
    let output_contract = route.effective_output_contract();
    if output_contract
        .semantic_kind
        .is_normalizer_schema_capability_bridge()
    {
        return Vec::new();
    }
    required_evidence_fields_for_output_contract(&output_contract)
}

pub(crate) fn evidence_required_for_route(route: &RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !required_evidence_fields_for_route(route).is_empty()
}

fn required_evidence_fields_for_capability_ref(route: &RouteResult) -> Option<Vec<String>> {
    let capability = crate::machine_capability_ref::route_first_capability_ref(route)?;
    let namespace = capability.namespace();
    let action = capability.action();
    let fields =
        if namespace == "config" || namespace == "config_basic" || namespace == "config_edit" {
            if action_has_any_segment(action, &["guard", "risk"]) {
                &["candidates", "count"][..]
            } else if action_has_any_segment(action, &["apply", "change", "mutate", "set", "write"])
            {
                &["field_value", "path", "valid"][..]
            } else if action_has_any_segment(action, &["validate", "verify"]) {
                &["valid"][..]
            } else {
                &["field_value"][..]
            }
        } else if namespace == "archive" {
            if action_has_any_segment(action, &["list"]) {
                &["candidates"][..]
            } else if action_has_any_segment(action, &["read"]) {
                &["content_excerpt"][..]
            } else if action_has_any_segment(action, &["pack", "unpack"]) {
                &["path"][..]
            } else {
                &["field_value"][..]
            }
        } else if action_has_any_segment(action, &["candidates", "find", "list", "search"]) {
            &["candidates"][..]
        } else if action_has_any_segment(
            action,
            &[
                "analyze",
                "describe",
                "extract",
                "quote",
                "read",
                "summary",
                "summarize",
            ],
        ) {
            &["content_excerpt"][..]
        } else if namespace == "web"
            || namespace == "browser"
            || namespace == "rss"
            || namespace == "weather"
            || namespace == "image_vision"
        {
            &["content_excerpt"][..]
        } else {
            &["field_value"][..]
        };
    Some(fields.iter().map(|field| (*field).to_string()).collect())
}

fn action_has_any_segment(action: &str, needles: &[&str]) -> bool {
    action
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')))
        .any(|segment| {
            let segment = segment.trim();
            !segment.is_empty()
                && needles.iter().any(|needle| {
                    segment == *needle
                        || segment.starts_with(&format!("{needle}_"))
                        || segment.ends_with(&format!("_{needle}"))
                        || segment.contains(&format!("_{needle}_"))
                })
        })
}

pub(crate) fn required_evidence_fields_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> Vec<String> {
    crate::evidence_policy::required_evidence_for_output_contract(output_contract)
        .unwrap_or_else(|| fallback_required_evidence_fields_for_output_contract(output_contract))
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

pub(crate) fn missing_parameters_for_route(route: &RouteResult) -> Vec<String> {
    if !route.needs_clarify {
        return Vec::new();
    }
    let mut missing = BTreeSet::new();
    if route.output_contract.locator_hint.trim().is_empty()
        && !matches!(route.output_contract.locator_kind, OutputLocatorKind::None)
    {
        missing.insert("locator");
    }
    if route.output_contract.delivery_required
        && route.output_contract.locator_hint.trim().is_empty()
    {
        missing.insert("delivery_target");
    }
    if missing.is_empty() {
        missing.insert("required_detail");
    }
    missing.into_iter().map(str::to_string).collect()
}

fn failure_policy_for_route(
    route: &RouteResult,
    evidence_required: bool,
    missing_parameters: &[String],
) -> TaskFailurePolicy {
    if route.needs_clarify || !missing_parameters.is_empty() {
        TaskFailurePolicy::Clarify
    } else if evidence_required {
        TaskFailurePolicy::RetryWithAlternatives
    } else {
        TaskFailurePolicy::NoRetry
    }
}

#[cfg(test)]
#[path = "task_contract_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "task_contract_service_capability_tests.rs"]
mod service_capability_tests;

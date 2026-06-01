use std::collections::BTreeSet;

use serde_json::json;

use crate::{
    OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, OutputSemanticKind, RouteResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskIntentKind {
    Clarify,
    DirectAnswer,
    PlannerExecute,
}

impl TaskIntentKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Clarify => "clarify",
            Self::DirectAnswer => "direct_answer",
            Self::PlannerExecute => "planner_execute",
        }
    }
}

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
#[allow(dead_code)]
pub(crate) enum TaskOperation {
    Inspect,
    List,
    Count,
    Read,
    Write,
    Modify,
    Run,
    Configure,
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
            Self::Configure => "configure",
            Self::Validate => "validate",
            Self::Summarize => "summarize",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum TaskDeliveryShape {
    OneSentence,
    List,
    Table,
    Raw,
    File,
    Summary,
}

impl TaskDeliveryShape {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::OneSentence => "one_sentence",
            Self::List => "list",
            Self::Table => "table",
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
#[allow(dead_code)]
pub(crate) enum TaskTargetRole {
    Primary,
    Delivery,
    Context,
}

impl TaskTargetRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Delivery => "delivery",
            Self::Context => "context",
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskContract {
    pub(crate) intent_kind: TaskIntentKind,
    pub(crate) targets: Vec<TargetContract>,
    pub(crate) target_object: TaskTargetObject,
    pub(crate) operation: TaskOperation,
    pub(crate) evidence_required: bool,
    pub(crate) required_evidence_fields: Vec<String>,
    pub(crate) delivery_shape: TaskDeliveryShape,
    pub(crate) missing_parameters: Vec<String>,
    pub(crate) failure_policy: TaskFailurePolicy,
}

impl TaskContract {
    pub(crate) fn from_route_result(route: &RouteResult) -> Self {
        let missing_parameters = missing_parameters_for_route(route);
        let evidence_required = route.output_contract.requires_content_evidence
            || route.output_contract.delivery_required
            || !required_evidence_fields_for_route(route).is_empty();
        Self {
            intent_kind: if route.needs_clarify || route.is_clarify_gate() {
                TaskIntentKind::Clarify
            } else if route.is_execute_gate() {
                TaskIntentKind::PlannerExecute
            } else {
                TaskIntentKind::DirectAnswer
            },
            targets: targets_for_route(route),
            target_object: target_object_for_route(route),
            operation: operation_for_route(route),
            evidence_required,
            required_evidence_fields: required_evidence_fields_for_route(route),
            delivery_shape: delivery_shape_for_route(route),
            failure_policy: failure_policy_for_route(route, evidence_required, &missing_parameters),
            missing_parameters,
        }
    }

    pub(crate) fn compact_prompt_line(&self) -> String {
        let required_evidence = if self.required_evidence_fields.is_empty() {
            "none".to_string()
        } else {
            self.required_evidence_fields.join(",")
        };
        let missing_parameters = if self.missing_parameters.is_empty() {
            "none".to_string()
        } else {
            self.missing_parameters.join(",")
        };
        let targets = if self.targets.is_empty() {
            "none".to_string()
        } else {
            let values = self
                .targets
                .iter()
                .map(TargetContract::compact_json)
                .collect::<Vec<_>>();
            serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_string())
        };
        format!(
            "- task_contract intent_kind={} targets={} target_object={} operation={} evidence_required={} required_evidence_fields={} delivery_shape={} missing_parameters={} failure_policy={}",
            self.intent_kind.as_str(),
            targets,
            self.target_object.as_str(),
            self.operation.as_str(),
            self.evidence_required,
            required_evidence,
            self.delivery_shape.as_str(),
            missing_parameters,
            self.failure_policy.as_str(),
        )
    }
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

fn target_object_for_route(route: &RouteResult) -> TaskTargetObject {
    match route.output_contract.semantic_kind {
        OutputSemanticKind::ServiceStatus => return TaskTargetObject::Service,
        OutputSemanticKind::DockerPs
        | OutputSemanticKind::DockerLogs
        | OutputSemanticKind::DockerImages
        | OutputSemanticKind::DockerContainerLifecycle => return TaskTargetObject::Process,
        OutputSemanticKind::SqliteTableListing
        | OutputSemanticKind::SqliteTableNamesOnly
        | OutputSemanticKind::SqliteDatabaseKindJudgment
        | OutputSemanticKind::SqliteSchemaVersion => return TaskTargetObject::Db,
        OutputSemanticKind::StructuredKeys
        | OutputSemanticKind::ConfigValidation
        | OutputSemanticKind::ConfigMutation
        | OutputSemanticKind::ConfigRiskAssessment => {
            return TaskTargetObject::ConfigKey;
        }
        OutputSemanticKind::RssNewsFetch
        | OutputSemanticKind::WebPageSummary
        | OutputSemanticKind::WebSearchSummary
        | OutputSemanticKind::WeatherQuery
        | OutputSemanticKind::MarketQuote
        | OutputSemanticKind::ImageUnderstanding => return TaskTargetObject::Web,
        OutputSemanticKind::PublishingPreview => return TaskTargetObject::Web,
        OutputSemanticKind::PackageManagerDetection => return TaskTargetObject::System,
        _ => {}
    }
    match route.output_contract.locator_kind {
        OutputLocatorKind::Path | OutputLocatorKind::Filename => TaskTargetObject::Path,
        OutputLocatorKind::CurrentWorkspace => TaskTargetObject::Directory,
        OutputLocatorKind::Url => TaskTargetObject::Web,
        OutputLocatorKind::None => TaskTargetObject::Unknown,
    }
}

fn operation_for_route(route: &RouteResult) -> TaskOperation {
    match route.output_contract.semantic_kind {
        OutputSemanticKind::RawCommandOutput => TaskOperation::Run,
        OutputSemanticKind::FileNames
        | OutputSemanticKind::DirectoryNames
        | OutputSemanticKind::DirectoryEntryGroups
        | OutputSemanticKind::FilePaths
        | OutputSemanticKind::SqliteTableListing
        | OutputSemanticKind::SqliteTableNamesOnly
        | OutputSemanticKind::ArchiveList
        | OutputSemanticKind::DockerPs
        | OutputSemanticKind::DockerImages
        | OutputSemanticKind::DockerLogs => TaskOperation::List,
        OutputSemanticKind::ScalarCount => TaskOperation::Count,
        OutputSemanticKind::ContentExcerptSummary
        | OutputSemanticKind::ContentExcerptWithSummary
        | OutputSemanticKind::WebPageSummary
        | OutputSemanticKind::WebSearchSummary
        | OutputSemanticKind::WeatherQuery
        | OutputSemanticKind::MarketQuote
        | OutputSemanticKind::ImageUnderstanding
        | OutputSemanticKind::DirectoryPurposeSummary
        | OutputSemanticKind::WorkspaceProjectSummary
        | OutputSemanticKind::ExistenceWithPathSummary
        | OutputSemanticKind::RecentArtifactsJudgment
        | OutputSemanticKind::ExcerptKindJudgment => TaskOperation::Summarize,
        OutputSemanticKind::GeneratedFileDelivery | OutputSemanticKind::ArchivePack => {
            TaskOperation::Write
        }
        OutputSemanticKind::ArchiveUnpack
        | OutputSemanticKind::ConfigMutation
        | OutputSemanticKind::DockerContainerLifecycle => TaskOperation::Modify,
        OutputSemanticKind::ServiceStatus
        | OutputSemanticKind::PackageManagerDetection
        | OutputSemanticKind::PublishingPreview
        | OutputSemanticKind::HiddenEntriesCheck
        | OutputSemanticKind::ContentPresenceCheck
        | OutputSemanticKind::ScalarPathOnly
        | OutputSemanticKind::ExistenceWithPath
        | OutputSemanticKind::GitCommitSubject
        | OutputSemanticKind::GitRepositoryState
        | OutputSemanticKind::StructuredKeys
        | OutputSemanticKind::ConfigValidation
        | OutputSemanticKind::ConfigRiskAssessment
        | OutputSemanticKind::SqliteDatabaseKindJudgment
        | OutputSemanticKind::SqliteSchemaVersion
        | OutputSemanticKind::RssNewsFetch
        | OutputSemanticKind::ArchiveRead => TaskOperation::Inspect,
        OutputSemanticKind::QuantityComparison | OutputSemanticKind::RecentScalarEqualityCheck => {
            TaskOperation::Validate
        }
        OutputSemanticKind::ExecutionFailedStep => TaskOperation::Validate,
        OutputSemanticKind::None => {
            if route.output_contract.delivery_required {
                TaskOperation::Read
            } else if route.is_execute_gate() {
                TaskOperation::Inspect
            } else {
                TaskOperation::Unknown
            }
        }
    }
}

fn delivery_shape_for_route(route: &RouteResult) -> TaskDeliveryShape {
    if matches!(
        route.output_contract.semantic_kind,
        OutputSemanticKind::FileNames
            | OutputSemanticKind::DirectoryNames
            | OutputSemanticKind::DirectoryEntryGroups
            | OutputSemanticKind::FilePaths
            | OutputSemanticKind::SqliteTableListing
            | OutputSemanticKind::SqliteTableNamesOnly
            | OutputSemanticKind::ArchiveList
            | OutputSemanticKind::DockerPs
            | OutputSemanticKind::DockerImages
    ) {
        return TaskDeliveryShape::List;
    }
    match route.output_contract.response_shape {
        OutputResponseShape::OneSentence => TaskDeliveryShape::OneSentence,
        OutputResponseShape::Strict | OutputResponseShape::Scalar => TaskDeliveryShape::Raw,
        OutputResponseShape::FileToken => TaskDeliveryShape::File,
        OutputResponseShape::Free => TaskDeliveryShape::Summary,
    }
}

fn required_evidence_fields_for_route(route: &RouteResult) -> Vec<String> {
    required_evidence_fields_for_output_contract(&route.output_contract)
}

pub(crate) fn required_evidence_fields_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> Vec<String> {
    crate::contract_matrix::required_evidence_for_output_contract(output_contract)
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
        OutputSemanticKind::RawCommandOutput => {
            fields.insert("command_output");
        }
        OutputSemanticKind::ScalarCount => {
            fields.insert("count");
        }
        OutputSemanticKind::ScalarPathOnly
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
        | OutputSemanticKind::WebPageSummary
        | OutputSemanticKind::WeatherQuery
        | OutputSemanticKind::MarketQuote
        | OutputSemanticKind::ImageUnderstanding
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
        }
        OutputSemanticKind::DirectoryPurposeSummary => {
            fields.insert("candidates");
        }
        OutputSemanticKind::WebSearchSummary => {
            fields.insert("candidates");
        }
        OutputSemanticKind::RecentArtifactsJudgment => {
            fields.insert("candidates");
            fields.insert("field_value");
        }
        OutputSemanticKind::FileNames
        | OutputSemanticKind::DirectoryNames
        | OutputSemanticKind::DirectoryEntryGroups
        | OutputSemanticKind::FilePaths
        | OutputSemanticKind::HiddenEntriesCheck
        | OutputSemanticKind::SqliteTableListing
        | OutputSemanticKind::SqliteTableNamesOnly
        | OutputSemanticKind::ArchiveList
        | OutputSemanticKind::DockerPs
        | OutputSemanticKind::DockerImages
        | OutputSemanticKind::DockerLogs => {
            fields.insert("candidates");
        }
        OutputSemanticKind::ServiceStatus
        | OutputSemanticKind::RssNewsFetch
        | OutputSemanticKind::PublishingPreview
        | OutputSemanticKind::PackageManagerDetection
        | OutputSemanticKind::StructuredKeys
        | OutputSemanticKind::SqliteDatabaseKindJudgment
        | OutputSemanticKind::SqliteSchemaVersion
        | OutputSemanticKind::RecentScalarEqualityCheck
        | OutputSemanticKind::ExecutionFailedStep
        | OutputSemanticKind::DockerContainerLifecycle => {
            fields.insert("field_value");
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
    if output_contract.semantic_kind == OutputSemanticKind::ExistenceWithPathSummary {
        fields.insert("content_excerpt");
    }
    fields.into_iter().map(str::to_string).collect()
}

fn missing_parameters_for_route(route: &RouteResult) -> Vec<String> {
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
    } else if route.is_execute_gate() || evidence_required {
        TaskFailurePolicy::RetryWithAlternatives
    } else {
        TaskFailurePolicy::NoRetry
    }
}

#[cfg(test)]
#[path = "task_contract_tests.rs"]
mod tests;

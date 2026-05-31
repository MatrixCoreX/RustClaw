use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    IntentOutputContract, OutputLocatorKind, OutputResponseShape, OutputSemanticKind, RouteResult,
};

#[cfg(test)]
use anyhow::{Context, Result};
#[cfg(test)]
use claw_core::skill_registry::{SkillKind, SkillsRegistry};
#[cfg(test)]
use std::path::Path;

#[cfg(test)]
pub(crate) const CONTRACT_MATRIX_REL_PATH: &str = "configs/task_contract_matrix.toml";

static BUNDLED_CONTRACT_MATRIX: OnceLock<Result<ContractMatrix, String>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct ContractMatrix {
    pub(crate) schema_version: u32,
    pub(crate) matrix_version: String,
    pub(crate) failure_attribution: Vec<String>,
    pub(crate) policy: MatrixPolicy,
    pub(crate) trace_policy: MatrixTracePolicy,
    pub(crate) generic_profiles: Vec<GenericProfile>,
    pub(crate) contracts: BTreeMap<String, MatrixContract>,
}

impl Default for ContractMatrix {
    fn default() -> Self {
        Self {
            schema_version: 1,
            matrix_version: String::new(),
            failure_attribution: Vec::new(),
            policy: MatrixPolicy::default(),
            trace_policy: MatrixTracePolicy::default(),
            generic_profiles: Vec::new(),
            contracts: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FailureAttribution {
    ModelError,
    SchemaError,
    CodeGap,
    ContractGap,
    ToolGap,
    PermissionDenied,
    BudgetExhausted,
    PromptBudgetError,
    DeliveryError,
    ProviderError,
}

impl FailureAttribution {
    pub(crate) const ALL: [Self; 10] = [
        Self::ModelError,
        Self::SchemaError,
        Self::CodeGap,
        Self::ContractGap,
        Self::ToolGap,
        Self::PermissionDenied,
        Self::BudgetExhausted,
        Self::PromptBudgetError,
        Self::DeliveryError,
        Self::ProviderError,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ModelError => "model_error",
            Self::SchemaError => "schema_error",
            Self::CodeGap => "code_gap",
            Self::ContractGap => "contract_gap",
            Self::ToolGap => "tool_gap",
            Self::PermissionDenied => "permission_denied",
            Self::BudgetExhausted => "budget_exhausted",
            Self::PromptBudgetError => "prompt_budget_error",
            Self::DeliveryError => "delivery_error",
            Self::ProviderError => "provider_error",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match normalize_action_token(value).as_str() {
            "model_error" => Some(Self::ModelError),
            "schema_error" => Some(Self::SchemaError),
            "code_gap" => Some(Self::CodeGap),
            "contract_gap" => Some(Self::ContractGap),
            "tool_gap" => Some(Self::ToolGap),
            "permission_denied" => Some(Self::PermissionDenied),
            "budget_exhausted" => Some(Self::BudgetExhausted),
            "prompt_budget_error" => Some(Self::PromptBudgetError),
            "delivery_error" => Some(Self::DeliveryError),
            "provider_error" => Some(Self::ProviderError),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct MatrixPolicy {
    pub(crate) unknown_semantic: String,
    pub(crate) unknown_action: String,
    pub(crate) evidence_missing: String,
}

impl Default for MatrixPolicy {
    fn default() -> Self {
        Self {
            unknown_semantic: "reject".to_string(),
            unknown_action: "reject".to_string(),
            evidence_missing: "retry_then_fail".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct MatrixTracePolicy {
    pub(crate) evidence_storage: String,
    pub(crate) provider_evidence_view: String,
    pub(crate) raw_excerpt_policy: String,
    pub(crate) max_items: usize,
    pub(crate) max_excerpt_chars: usize,
}

impl Default for MatrixTracePolicy {
    fn default() -> Self {
        Self {
            evidence_storage: "redacted_excerpt_hash".to_string(),
            provider_evidence_view: "provider_safe_redacted".to_string(),
            raw_excerpt_policy: "no_full_raw_excerpt".to_string(),
            max_items: 24,
            max_excerpt_chars: 240,
        }
    }
}

impl MatrixTracePolicy {
    fn stable_key(&self) -> String {
        format!(
            "storage={}|provider={}|raw={}|max_items={}|max_excerpt_chars={}",
            normalize_action_token(&self.evidence_storage),
            normalize_action_token(&self.provider_evidence_view),
            normalize_action_token(&self.raw_excerpt_policy),
            self.max_items,
            self.max_excerpt_chars,
        )
    }

    fn to_trace_json(&self) -> Value {
        json!({
            "evidence_storage": normalize_action_token(&self.evidence_storage),
            "provider_evidence_view": normalize_action_token(&self.provider_evidence_view),
            "raw_excerpt_policy": normalize_action_token(&self.raw_excerpt_policy),
            "max_items": self.max_items,
            "max_excerpt_chars": self.max_excerpt_chars,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct EvidenceExpression {
    pub(crate) all_of: Vec<String>,
    pub(crate) one_of: Vec<String>,
    pub(crate) any_of: Vec<String>,
    pub(crate) negative_evidence: Vec<String>,
}

impl EvidenceExpression {
    fn effective(&self, required_evidence: &[String]) -> Self {
        let mut effective = Self {
            all_of: normalized_tokens(&self.all_of),
            one_of: normalized_tokens(&self.one_of),
            any_of: normalized_tokens(&self.any_of),
            negative_evidence: normalized_tokens(&self.negative_evidence),
        };
        if effective.is_empty() {
            effective.all_of = normalized_tokens(required_evidence);
        }
        effective
    }

    fn is_empty(&self) -> bool {
        self.all_of.is_empty()
            && self.one_of.is_empty()
            && self.any_of.is_empty()
            && self.negative_evidence.is_empty()
    }

    fn stable_key(&self, required_evidence: &[String]) -> String {
        let effective = self.effective(required_evidence);
        format!(
            "all_of={}|one_of={}|any_of={}|negative={}",
            effective.all_of.join(","),
            effective.one_of.join(","),
            effective.any_of.join(","),
            effective.negative_evidence.join(","),
        )
    }

    pub(crate) fn to_trace_json(&self, required_evidence: &[String]) -> Value {
        let effective = self.effective(required_evidence);
        json!({
            "all_of": effective.all_of,
            "one_of": effective.one_of,
            "any_of": effective.any_of,
            "negative_evidence": effective.negative_evidence,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EvidenceToken {
    Candidates,
    CommandOutput,
    ContentExcerpt,
    ContentMatch,
    Count,
    Exists,
    ExistsFalse,
    ExistsTrue,
    FieldValue,
    Kind,
    Path,
    SizeBytes,
    Valid,
}

impl EvidenceToken {
    #[cfg(test)]
    pub(crate) const ALL: &'static [Self] = &[
        Self::Candidates,
        Self::CommandOutput,
        Self::ContentExcerpt,
        Self::ContentMatch,
        Self::Count,
        Self::Exists,
        Self::ExistsFalse,
        Self::ExistsTrue,
        Self::FieldValue,
        Self::Kind,
        Self::Path,
        Self::SizeBytes,
        Self::Valid,
    ];

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match normalize_action_token(raw).as_str() {
            "candidates" => Some(Self::Candidates),
            "command_output" => Some(Self::CommandOutput),
            "content_excerpt" => Some(Self::ContentExcerpt),
            "content_match" => Some(Self::ContentMatch),
            "count" => Some(Self::Count),
            "exists" => Some(Self::Exists),
            "exists_false" => Some(Self::ExistsFalse),
            "exists_true" => Some(Self::ExistsTrue),
            "field_value" => Some(Self::FieldValue),
            "kind" => Some(Self::Kind),
            "path" => Some(Self::Path),
            "size_bytes" => Some(Self::SizeBytes),
            "valid" => Some(Self::Valid),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Candidates => "candidates",
            Self::CommandOutput => "command_output",
            Self::ContentExcerpt => "content_excerpt",
            Self::ContentMatch => "content_match",
            Self::Count => "count",
            Self::Exists => "exists",
            Self::ExistsFalse => "exists_false",
            Self::ExistsTrue => "exists_true",
            Self::FieldValue => "field_value",
            Self::Kind => "kind",
            Self::Path => "path",
            Self::SizeBytes => "size_bytes",
            Self::Valid => "valid",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub(crate) struct MatrixContract {
    pub(crate) semantic_kind: String,
    pub(crate) operation: String,
    pub(crate) target_object: String,
    pub(crate) delivery_shape: String,
    pub(crate) policy_mode: String,
    pub(crate) evidence_scope: String,
    pub(crate) freshness: String,
    pub(crate) artifact_kind: String,
    pub(crate) channel_visibility: String,
    pub(crate) allowed_actions: Vec<String>,
    pub(crate) preferred_actions: Vec<String>,
    pub(crate) forbidden_actions: Vec<String>,
    pub(crate) required_evidence: Vec<String>,
    pub(crate) final_answer_shape: String,
    pub(crate) none_passthrough: bool,
    pub(crate) failure_policy: String,
    pub(crate) locator_kinds: Vec<String>,
    pub(crate) observation_sources: Vec<String>,
    pub(crate) observation_extractors: Vec<ObservationExtractor>,
    pub(crate) evidence_expression: EvidenceExpression,
}

impl MatrixContract {
    pub(crate) fn normalized_required_evidence(&self) -> Vec<String> {
        normalized_tokens(&self.required_evidence)
    }

    fn evidence_expression(&self) -> EvidenceExpression {
        self.evidence_expression
            .effective(&self.normalized_required_evidence())
    }

    fn observation_sources(&self) -> Vec<String> {
        let configured = normalized_tokens(&self.observation_sources);
        if configured.is_empty() {
            normalized_tokens(&self.allowed_actions)
        } else {
            configured
        }
    }

    fn observation_extractors(&self) -> Vec<ObservationExtractor> {
        observation_extractors_for_sources(self.observation_sources(), &self.observation_extractors)
    }

    fn policy_mode(&self) -> String {
        normalized_contract_field(&self.policy_mode, "enforce")
    }

    fn evidence_scope(&self) -> String {
        normalized_contract_field(&self.evidence_scope, "current_task")
    }

    fn freshness(&self) -> String {
        normalized_contract_field(&self.freshness, "current_task")
    }

    fn artifact_kind(&self) -> String {
        if !self.artifact_kind.trim().is_empty() {
            return normalized_contract_field(&self.artifact_kind, "text");
        }
        if normalize_action_token(&self.final_answer_shape) == "delivery_token_or_path" {
            "file".to_string()
        } else {
            "text".to_string()
        }
    }

    fn channel_visibility(&self) -> String {
        normalized_contract_field(&self.channel_visibility, "user_visible")
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub(crate) struct GenericProfile {
    pub(crate) name: String,
    pub(crate) semantic_kind: String,
    pub(crate) requires_content_evidence: Option<bool>,
    pub(crate) delivery_required: Option<bool>,
    pub(crate) response_shapes: Vec<String>,
    pub(crate) locator_kinds: Vec<String>,
    pub(crate) policy_mode: String,
    pub(crate) evidence_scope: String,
    pub(crate) freshness: String,
    pub(crate) artifact_kind: String,
    pub(crate) channel_visibility: String,
    pub(crate) allowed_actions: Vec<String>,
    pub(crate) preferred_actions: Vec<String>,
    pub(crate) forbidden_actions: Vec<String>,
    pub(crate) required_evidence: Vec<String>,
    pub(crate) final_answer_shape: String,
    pub(crate) failure_policy: String,
    pub(crate) observation_sources: Vec<String>,
    pub(crate) observation_extractors: Vec<ObservationExtractor>,
    pub(crate) evidence_expression: EvidenceExpression,
}

impl GenericProfile {
    fn matches(&self, output: &IntentOutputContract) -> bool {
        let semantic_kind = self.semantic_kind.trim();
        if !semantic_kind.is_empty() && semantic_kind != output.semantic_kind.as_str() {
            return false;
        }
        if let Some(expected) = self.requires_content_evidence {
            if expected != output.requires_content_evidence {
                return false;
            }
        }
        if let Some(expected) = self.delivery_required {
            if expected != output.delivery_required {
                return false;
            }
        }
        if !self.response_shapes.is_empty()
            && !contains_token(&self.response_shapes, output.response_shape.as_str())
        {
            return false;
        }
        if !self.locator_kinds.is_empty()
            && !contains_token(&self.locator_kinds, output.locator_kind.as_str())
        {
            return false;
        }
        true
    }

    pub(crate) fn normalized_required_evidence(&self) -> Vec<String> {
        normalized_tokens(&self.required_evidence)
    }

    fn evidence_expression(&self) -> EvidenceExpression {
        self.evidence_expression
            .effective(&self.normalized_required_evidence())
    }

    fn observation_sources(&self) -> Vec<String> {
        let configured = normalized_tokens(&self.observation_sources);
        if configured.is_empty() {
            normalized_tokens(&self.allowed_actions)
        } else {
            configured
        }
    }

    fn observation_extractors(&self) -> Vec<ObservationExtractor> {
        observation_extractors_for_sources(self.observation_sources(), &self.observation_extractors)
    }

    fn policy_mode(&self) -> String {
        normalized_contract_field(&self.policy_mode, "enforce")
    }

    fn evidence_scope(&self) -> String {
        normalized_contract_field(&self.evidence_scope, "current_task")
    }

    fn freshness(&self) -> String {
        normalized_contract_field(&self.freshness, "current_task")
    }

    fn artifact_kind(&self) -> String {
        if !self.artifact_kind.trim().is_empty() {
            return normalized_contract_field(&self.artifact_kind, "text");
        }
        if self.delivery_required.unwrap_or(false)
            || normalize_action_token(&self.final_answer_shape) == "delivery_token_or_path"
        {
            "file".to_string()
        } else {
            "text".to_string()
        }
    }

    fn channel_visibility(&self) -> String {
        normalized_contract_field(&self.channel_visibility, "user_visible")
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub(crate) struct ObservationExtractor {
    pub(crate) source: String,
    pub(crate) extractor_kind: String,
}

impl ObservationExtractor {
    fn normalized(source: &str, extractor_kind: &str) -> Option<Self> {
        let source = normalize_action_token(source);
        if source.is_empty() {
            return None;
        }
        Some(Self {
            source,
            extractor_kind: normalized_extractor_kind(extractor_kind),
        })
    }

    fn from_source(source: &str) -> Option<Self> {
        Self::normalized(
            source,
            default_extractor_kind_for_observation_source(source),
        )
    }

    fn to_trace_json(&self) -> Value {
        let registry = crate::task_journal::evidence_extractor_registry_trace(
            &self.source,
            &self.extractor_kind,
        );
        json!({
            "source": self.source,
            "extractor_kind": self.extractor_kind,
            "registry": registry,
        })
    }

    fn stable_key(&self) -> String {
        format!("{}={}", self.source, self.extractor_kind)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FinalAnswerShape {
    ArchiveMemberExcerpt,
    ArchiveMemberList,
    ComparisonVerdict,
    ContainerList,
    CreatedArchivePath,
    DatabaseKindJudgment,
    DeliveryTokenOrPath,
    ExistenceSummaryWithPath,
    ExistenceVerdictWithPath,
    ExcerptPlusSummary,
    FailedStepWithEvidence,
    Free,
    GitStateSummary,
    GroupedNameList,
    ImageList,
    JudgmentWithExcerptBasis,
    KeyListOrKeySummary,
    LifecycleResult,
    ListOrEmptyStatement,
    LogExcerptOrSummary,
    ManagerNameWithBasis,
    NameList,
    PathList,
    PresenceVerdictWithMatch,
    ProjectSummaryGroundedInFiles,
    RawOutputOrShortSummary,
    RecentArtifactJudgment,
    RiskAssessment,
    Scalar,
    ScalarEqualityVerdict,
    SchemaVersion,
    SingleCommitSubject,
    SinglePath,
    StatusWithSource,
    SummaryGroundedInExcerpt,
    SummaryGroundedInListing,
    SummaryWithEvidence,
    TableListing,
    TableNameList,
    UnpackDestinationSummary,
    ValidationVerdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FinalAnswerShapeClass {
    DeliveryArtifact,
    Freeform,
    GroundedSummary,
    ScalarValue,
    SinglePath,
    StrictList,
    Table,
    Verdict,
}

impl FinalAnswerShapeClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DeliveryArtifact => "delivery_artifact",
            Self::Freeform => "freeform",
            Self::GroundedSummary => "grounded_summary",
            Self::ScalarValue => "scalar_value",
            Self::SinglePath => "single_path",
            Self::StrictList => "strict_list",
            Self::Table => "table",
            Self::Verdict => "verdict",
        }
    }

    pub(crate) fn coarse_response_shape(self) -> OutputResponseShape {
        match self {
            Self::DeliveryArtifact => OutputResponseShape::FileToken,
            Self::ScalarValue | Self::SinglePath => OutputResponseShape::Scalar,
            Self::StrictList | Self::Table => OutputResponseShape::Strict,
            Self::Verdict => OutputResponseShape::OneSentence,
            Self::Freeform | Self::GroundedSummary => OutputResponseShape::Free,
        }
    }

    pub(crate) fn allows_model_language(self) -> bool {
        matches!(self, Self::Freeform | Self::GroundedSummary | Self::Verdict)
    }
}

impl FinalAnswerShape {
    #[cfg(test)]
    pub(crate) const ALL: &'static [Self] = &[
        Self::ArchiveMemberExcerpt,
        Self::ArchiveMemberList,
        Self::ComparisonVerdict,
        Self::ContainerList,
        Self::CreatedArchivePath,
        Self::DatabaseKindJudgment,
        Self::DeliveryTokenOrPath,
        Self::ExistenceSummaryWithPath,
        Self::ExistenceVerdictWithPath,
        Self::ExcerptPlusSummary,
        Self::FailedStepWithEvidence,
        Self::Free,
        Self::GitStateSummary,
        Self::GroupedNameList,
        Self::ImageList,
        Self::JudgmentWithExcerptBasis,
        Self::KeyListOrKeySummary,
        Self::LifecycleResult,
        Self::ListOrEmptyStatement,
        Self::LogExcerptOrSummary,
        Self::ManagerNameWithBasis,
        Self::NameList,
        Self::PathList,
        Self::PresenceVerdictWithMatch,
        Self::ProjectSummaryGroundedInFiles,
        Self::RawOutputOrShortSummary,
        Self::RecentArtifactJudgment,
        Self::RiskAssessment,
        Self::Scalar,
        Self::ScalarEqualityVerdict,
        Self::SchemaVersion,
        Self::SingleCommitSubject,
        Self::SinglePath,
        Self::StatusWithSource,
        Self::SummaryGroundedInExcerpt,
        Self::SummaryGroundedInListing,
        Self::SummaryWithEvidence,
        Self::TableListing,
        Self::TableNameList,
        Self::UnpackDestinationSummary,
        Self::ValidationVerdict,
    ];

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "archive_member_excerpt" => Some(Self::ArchiveMemberExcerpt),
            "archive_member_list" => Some(Self::ArchiveMemberList),
            "comparison_verdict" => Some(Self::ComparisonVerdict),
            "container_list" => Some(Self::ContainerList),
            "created_archive_path" => Some(Self::CreatedArchivePath),
            "database_kind_judgment" => Some(Self::DatabaseKindJudgment),
            "delivery_token_or_path" => Some(Self::DeliveryTokenOrPath),
            "existence_summary_with_path" => Some(Self::ExistenceSummaryWithPath),
            "existence_verdict_with_path" => Some(Self::ExistenceVerdictWithPath),
            "excerpt_plus_summary" => Some(Self::ExcerptPlusSummary),
            "failed_step_with_evidence" => Some(Self::FailedStepWithEvidence),
            "free" => Some(Self::Free),
            "git_state_summary" => Some(Self::GitStateSummary),
            "grouped_name_list" => Some(Self::GroupedNameList),
            "image_list" => Some(Self::ImageList),
            "judgment_with_excerpt_basis" => Some(Self::JudgmentWithExcerptBasis),
            "key_list_or_key_summary" => Some(Self::KeyListOrKeySummary),
            "lifecycle_result" => Some(Self::LifecycleResult),
            "list_or_empty_statement" => Some(Self::ListOrEmptyStatement),
            "log_excerpt_or_summary" => Some(Self::LogExcerptOrSummary),
            "manager_name_with_basis" => Some(Self::ManagerNameWithBasis),
            "name_list" => Some(Self::NameList),
            "path_list" => Some(Self::PathList),
            "presence_verdict_with_match" => Some(Self::PresenceVerdictWithMatch),
            "project_summary_grounded_in_files" => Some(Self::ProjectSummaryGroundedInFiles),
            "raw_output_or_short_summary" => Some(Self::RawOutputOrShortSummary),
            "recent_artifact_judgment" => Some(Self::RecentArtifactJudgment),
            "risk_assessment" => Some(Self::RiskAssessment),
            "scalar" => Some(Self::Scalar),
            "scalar_equality_verdict" => Some(Self::ScalarEqualityVerdict),
            "schema_version" => Some(Self::SchemaVersion),
            "single_commit_subject" => Some(Self::SingleCommitSubject),
            "single_path" => Some(Self::SinglePath),
            "status_with_source" => Some(Self::StatusWithSource),
            "summary_grounded_in_excerpt" => Some(Self::SummaryGroundedInExcerpt),
            "summary_grounded_in_listing" => Some(Self::SummaryGroundedInListing),
            "summary_with_evidence" => Some(Self::SummaryWithEvidence),
            "table_listing" => Some(Self::TableListing),
            "table_name_list" => Some(Self::TableNameList),
            "unpack_destination_summary" => Some(Self::UnpackDestinationSummary),
            "validation_verdict" => Some(Self::ValidationVerdict),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ArchiveMemberExcerpt => "archive_member_excerpt",
            Self::ArchiveMemberList => "archive_member_list",
            Self::ComparisonVerdict => "comparison_verdict",
            Self::ContainerList => "container_list",
            Self::CreatedArchivePath => "created_archive_path",
            Self::DatabaseKindJudgment => "database_kind_judgment",
            Self::DeliveryTokenOrPath => "delivery_token_or_path",
            Self::ExistenceSummaryWithPath => "existence_summary_with_path",
            Self::ExistenceVerdictWithPath => "existence_verdict_with_path",
            Self::ExcerptPlusSummary => "excerpt_plus_summary",
            Self::FailedStepWithEvidence => "failed_step_with_evidence",
            Self::Free => "free",
            Self::GitStateSummary => "git_state_summary",
            Self::GroupedNameList => "grouped_name_list",
            Self::ImageList => "image_list",
            Self::JudgmentWithExcerptBasis => "judgment_with_excerpt_basis",
            Self::KeyListOrKeySummary => "key_list_or_key_summary",
            Self::LifecycleResult => "lifecycle_result",
            Self::ListOrEmptyStatement => "list_or_empty_statement",
            Self::LogExcerptOrSummary => "log_excerpt_or_summary",
            Self::ManagerNameWithBasis => "manager_name_with_basis",
            Self::NameList => "name_list",
            Self::PathList => "path_list",
            Self::PresenceVerdictWithMatch => "presence_verdict_with_match",
            Self::ProjectSummaryGroundedInFiles => "project_summary_grounded_in_files",
            Self::RawOutputOrShortSummary => "raw_output_or_short_summary",
            Self::RecentArtifactJudgment => "recent_artifact_judgment",
            Self::RiskAssessment => "risk_assessment",
            Self::Scalar => "scalar",
            Self::ScalarEqualityVerdict => "scalar_equality_verdict",
            Self::SchemaVersion => "schema_version",
            Self::SingleCommitSubject => "single_commit_subject",
            Self::SinglePath => "single_path",
            Self::StatusWithSource => "status_with_source",
            Self::SummaryGroundedInExcerpt => "summary_grounded_in_excerpt",
            Self::SummaryGroundedInListing => "summary_grounded_in_listing",
            Self::SummaryWithEvidence => "summary_with_evidence",
            Self::TableListing => "table_listing",
            Self::TableNameList => "table_name_list",
            Self::UnpackDestinationSummary => "unpack_destination_summary",
            Self::ValidationVerdict => "validation_verdict",
        }
    }

    pub(crate) fn class(self) -> FinalAnswerShapeClass {
        match self {
            Self::DeliveryTokenOrPath => FinalAnswerShapeClass::DeliveryArtifact,
            Self::CreatedArchivePath | Self::SinglePath => FinalAnswerShapeClass::SinglePath,
            Self::Scalar | Self::SchemaVersion | Self::SingleCommitSubject => {
                FinalAnswerShapeClass::ScalarValue
            }
            Self::ArchiveMemberList
            | Self::ContainerList
            | Self::GroupedNameList
            | Self::ImageList
            | Self::KeyListOrKeySummary
            | Self::ListOrEmptyStatement
            | Self::NameList
            | Self::PathList
            | Self::TableNameList => FinalAnswerShapeClass::StrictList,
            Self::TableListing => FinalAnswerShapeClass::Table,
            Self::ComparisonVerdict
            | Self::DatabaseKindJudgment
            | Self::ExistenceVerdictWithPath
            | Self::JudgmentWithExcerptBasis
            | Self::LifecycleResult
            | Self::PresenceVerdictWithMatch
            | Self::RecentArtifactJudgment
            | Self::RiskAssessment
            | Self::ScalarEqualityVerdict
            | Self::StatusWithSource
            | Self::ValidationVerdict => FinalAnswerShapeClass::Verdict,
            Self::ArchiveMemberExcerpt
            | Self::ExistenceSummaryWithPath
            | Self::ExcerptPlusSummary
            | Self::FailedStepWithEvidence
            | Self::GitStateSummary
            | Self::LogExcerptOrSummary
            | Self::ManagerNameWithBasis
            | Self::ProjectSummaryGroundedInFiles
            | Self::RawOutputOrShortSummary
            | Self::SummaryGroundedInExcerpt
            | Self::SummaryGroundedInListing
            | Self::SummaryWithEvidence
            | Self::UnpackDestinationSummary => FinalAnswerShapeClass::GroundedSummary,
            Self::Free => FinalAnswerShapeClass::Freeform,
        }
    }

    pub(crate) fn coarse_response_shape(self) -> OutputResponseShape {
        self.class().coarse_response_shape()
    }

    pub(crate) fn allows_model_language(self) -> bool {
        self.class().allows_model_language()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionPolicyDecision {
    Allowed,
    RejectedForbidden,
    RejectedNotAllowed,
    RejectedNoActionsAllowed,
}

impl ActionPolicyDecision {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::RejectedForbidden => "rejected_forbidden",
            Self::RejectedNotAllowed => "rejected_not_allowed",
            Self::RejectedNoActionsAllowed => "rejected_no_actions_allowed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArgPolicyDecision {
    Allowed,
    DeferredTemplateArg,
    MissingTargetBinding,
}

impl ArgPolicyDecision {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::DeferredTemplateArg => "deferred_template_arg",
            Self::MissingTargetBinding => "missing_target_binding",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ActionRef {
    pub(crate) skill: String,
    pub(crate) action: Option<String>,
}

impl ActionRef {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let (skill, action) = match raw.split_once('.') {
            Some((skill, action)) => (skill, Some(action)),
            None => (raw, None),
        };
        let skill = normalize_action_token(skill).replace('-', "_");
        if skill.is_empty() {
            return None;
        }
        let action = action
            .map(normalize_action_token)
            .filter(|value| !value.is_empty());
        Some(Self { skill, action })
    }

    pub(crate) fn from_skill_args(skill: &str, args: &Value) -> Option<Self> {
        let mut action_ref = Self::parse(skill)?;
        if let Some(action) = args
            .get("action")
            .and_then(Value::as_str)
            .map(normalize_action_token)
            .filter(|value| !value.is_empty())
        {
            action_ref.action = Some(action);
        }
        Some(action_ref)
    }

    pub(crate) fn as_key(&self) -> String {
        match &self.action {
            Some(action) => format!("{}.{}", self.skill, action),
            None => self.skill.clone(),
        }
    }
}

pub(crate) enum MatchedContract<'a> {
    Semantic(&'a MatrixContract),
    Generic(&'a GenericProfile),
}

impl<'a> MatchedContract<'a> {
    pub(crate) fn required_evidence(&self) -> Vec<String> {
        match self {
            Self::Semantic(contract) => contract.normalized_required_evidence(),
            Self::Generic(profile) => profile.normalized_required_evidence(),
        }
    }

    pub(crate) fn final_answer_shape(&self) -> &str {
        match self {
            Self::Semantic(contract) => contract.final_answer_shape.as_str(),
            Self::Generic(profile) => profile.final_answer_shape.as_str(),
        }
    }

    pub(crate) fn final_answer_shape_kind(&self) -> Option<FinalAnswerShape> {
        FinalAnswerShape::parse(self.final_answer_shape())
    }

    pub(crate) fn evidence_expression(&self) -> EvidenceExpression {
        match self {
            Self::Semantic(contract) => contract.evidence_expression(),
            Self::Generic(profile) => profile.evidence_expression(),
        }
    }

    fn observation_sources(&self) -> Vec<String> {
        match self {
            Self::Semantic(contract) => contract.observation_sources(),
            Self::Generic(profile) => profile.observation_sources(),
        }
    }

    fn observation_extractors(&self) -> Vec<ObservationExtractor> {
        match self {
            Self::Semantic(contract) => contract.observation_extractors(),
            Self::Generic(profile) => profile.observation_extractors(),
        }
    }

    fn observation_extractor_for_source(&self, source: &str) -> Option<ObservationExtractor> {
        let source = normalize_action_token(source);
        self.observation_extractors()
            .into_iter()
            .find(|extractor| extractor.source == source)
            .or_else(|| ObservationExtractor::from_source(&source))
    }

    fn policy_mode(&self) -> String {
        match self {
            Self::Semantic(contract) => contract.policy_mode(),
            Self::Generic(profile) => profile.policy_mode(),
        }
    }

    fn evidence_scope(&self) -> String {
        match self {
            Self::Semantic(contract) => contract.evidence_scope(),
            Self::Generic(profile) => profile.evidence_scope(),
        }
    }

    fn freshness(&self) -> String {
        match self {
            Self::Semantic(contract) => contract.freshness(),
            Self::Generic(profile) => profile.freshness(),
        }
    }

    fn artifact_kind(&self) -> String {
        match self {
            Self::Semantic(contract) => contract.artifact_kind(),
            Self::Generic(profile) => profile.artifact_kind(),
        }
    }

    fn channel_visibility(&self) -> String {
        match self {
            Self::Semantic(contract) => contract.channel_visibility(),
            Self::Generic(profile) => profile.channel_visibility(),
        }
    }

    fn match_name(&self) -> &str {
        match self {
            Self::Semantic(contract) => contract.semantic_kind.as_str(),
            Self::Generic(profile) => profile.name.as_str(),
        }
    }

    fn allowed_actions(&self) -> &[String] {
        match self {
            Self::Semantic(contract) => contract.allowed_actions.as_slice(),
            Self::Generic(profile) => profile.allowed_actions.as_slice(),
        }
    }

    fn forbidden_actions(&self) -> &[String] {
        match self {
            Self::Semantic(contract) => contract.forbidden_actions.as_slice(),
            Self::Generic(profile) => profile.forbidden_actions.as_slice(),
        }
    }

    fn preferred_actions(&self) -> &[String] {
        match self {
            Self::Semantic(contract) => contract.preferred_actions.as_slice(),
            Self::Generic(profile) => profile.preferred_actions.as_slice(),
        }
    }

    fn action_policy(&self, action: &ActionRef) -> ActionPolicyDecision {
        if action_matches_any(action, self.forbidden_actions()) {
            return ActionPolicyDecision::RejectedForbidden;
        }
        let allowed_actions = self.allowed_actions();
        if allowed_actions.is_empty() {
            if matches!(
                self,
                Self::Semantic(MatrixContract {
                    none_passthrough: true,
                    ..
                })
            ) {
                return ActionPolicyDecision::Allowed;
            }
            return ActionPolicyDecision::RejectedNoActionsAllowed;
        }
        if action_matches_any(action, allowed_actions) {
            ActionPolicyDecision::Allowed
        } else {
            ActionPolicyDecision::RejectedNotAllowed
        }
    }
}

impl ContractMatrix {
    #[cfg(test)]
    pub(crate) fn load_from_path(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read task contract matrix: {}", path.display()))?;
        let matrix: Self = toml::from_str(&raw)
            .with_context(|| format!("parse task contract matrix: {}", path.display()))?;
        Ok(matrix)
    }

    #[cfg(test)]
    pub(crate) fn load_from_workspace(workspace_root: &Path) -> Result<Self> {
        Self::load_from_path(&workspace_root.join(CONTRACT_MATRIX_REL_PATH))
    }

    pub(crate) fn semantic_contract(
        &self,
        semantic_kind: OutputSemanticKind,
    ) -> Option<&MatrixContract> {
        self.contracts.get(semantic_kind.as_str())
    }

    pub(crate) fn match_output_contract(
        &self,
        output: &IntentOutputContract,
    ) -> Option<MatchedContract<'_>> {
        if output.semantic_kind != OutputSemanticKind::None {
            return self
                .semantic_contract(output.semantic_kind)
                .map(MatchedContract::Semantic);
        }
        for profile in &self.generic_profiles {
            if profile.matches(output) {
                return Some(MatchedContract::Generic(profile));
            }
        }
        self.semantic_contract(OutputSemanticKind::None)
            .map(MatchedContract::Semantic)
    }

    pub(crate) fn validate_shape(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.schema_version == 0 {
            errors.push("schema_version must be positive".to_string());
        }
        if self.matrix_version.trim().is_empty() {
            errors.push("matrix_version must not be empty".to_string());
        }
        let mut configured_attributions = BTreeSet::new();
        for raw in &self.failure_attribution {
            match FailureAttribution::parse(raw) {
                Some(kind) => {
                    configured_attributions.insert(kind);
                }
                None => {
                    errors.push(format!("invalid failure attribution `{raw}`"));
                }
            }
        }
        for expected in FailureAttribution::ALL {
            if !configured_attributions.contains(&expected) {
                errors.push(format!(
                    "missing failure attribution `{}`",
                    expected.as_str()
                ));
            }
        }
        if normalize_action_token(&self.trace_policy.evidence_storage) != "redacted_excerpt_hash" {
            errors.push("trace_policy.evidence_storage must be redacted_excerpt_hash".to_string());
        }
        if normalize_action_token(&self.trace_policy.provider_evidence_view)
            != "provider_safe_redacted"
        {
            errors.push(
                "trace_policy.provider_evidence_view must be provider_safe_redacted".to_string(),
            );
        }
        if normalize_action_token(&self.trace_policy.raw_excerpt_policy) != "no_full_raw_excerpt" {
            errors.push("trace_policy.raw_excerpt_policy must be no_full_raw_excerpt".to_string());
        }
        if self.trace_policy.max_items == 0 {
            errors.push("trace_policy.max_items must be positive".to_string());
        }
        if self.trace_policy.max_excerpt_chars == 0 {
            errors.push("trace_policy.max_excerpt_chars must be positive".to_string());
        }
        for kind in OutputSemanticKind::ALL {
            let key = kind.as_str();
            match self.contracts.get(key) {
                Some(contract) => {
                    let declared = contract.semantic_kind.trim();
                    if declared != key {
                        errors.push(format!(
                            "contract `{key}` declares semantic_kind `{declared}`"
                        ));
                    }
                    if contract.final_answer_shape.trim().is_empty() {
                        errors.push(format!("contract `{key}` missing final_answer_shape"));
                    } else if FinalAnswerShape::parse(&contract.final_answer_shape).is_none() {
                        errors.push(format!(
                            "contract `{key}` has unknown final_answer_shape `{}`",
                            contract.final_answer_shape
                        ));
                    }
                    let required = contract.normalized_required_evidence();
                    for field in &required {
                        if EvidenceToken::parse(field).is_none() {
                            errors.push(format!(
                                "contract `{key}` has unknown required_evidence `{field}`"
                            ));
                        }
                    }
                    if !required.is_empty() && contract.evidence_expression().is_empty() {
                        errors.push(format!("contract `{key}` missing evidence_expression"));
                    }
                    for token in evidence_expression_tokens(&contract.evidence_expression) {
                        if EvidenceToken::parse(&token).is_none() {
                            errors.push(format!(
                                "contract `{key}` has unknown evidence_expression token `{token}`"
                            ));
                        }
                    }
                    if !required.is_empty() && contract.observation_sources().is_empty() {
                        errors.push(format!("contract `{key}` missing observation_sources"));
                    }
                    validate_contract_runtime_fields(
                        &mut errors,
                        &format!("contract `{key}`"),
                        &contract.policy_mode,
                        &contract.evidence_scope,
                        &contract.freshness,
                        &contract.artifact_kind,
                        &contract.channel_visibility,
                    );
                    validate_artifact_shape_contract(
                        &mut errors,
                        &format!("contract `{key}`"),
                        Some(&contract.delivery_shape),
                        &contract.final_answer_shape,
                        &contract.artifact_kind,
                        &contract.channel_visibility,
                    );
                    validate_observation_extractors(
                        &mut errors,
                        &format!("contract `{key}`"),
                        &contract.observation_sources(),
                        &contract.observation_extractors,
                    );
                }
                None => errors.push(format!("missing contract for semantic `{key}`")),
            }
        }
        for key in self.contracts.keys() {
            if !OutputSemanticKind::ALL
                .iter()
                .any(|kind| kind.as_str() == key.as_str())
            {
                errors.push(format!("unknown semantic contract `{key}`"));
            }
        }
        let mut generic_names = BTreeSet::new();
        for profile in &self.generic_profiles {
            if profile.name.trim().is_empty() {
                errors.push("generic profile missing name".to_string());
            } else if !generic_names.insert(profile.name.trim().to_string()) {
                errors.push(format!("duplicate generic profile `{}`", profile.name));
            }
            if profile.final_answer_shape.trim().is_empty() {
                errors.push(format!(
                    "generic profile `{}` missing final_answer_shape",
                    profile.name
                ));
            } else if FinalAnswerShape::parse(&profile.final_answer_shape).is_none() {
                errors.push(format!(
                    "generic profile `{}` has unknown final_answer_shape `{}`",
                    profile.name, profile.final_answer_shape
                ));
            }
            let required = profile.normalized_required_evidence();
            for field in &required {
                if EvidenceToken::parse(field).is_none() {
                    errors.push(format!(
                        "generic profile `{}` has unknown required_evidence `{field}`",
                        profile.name
                    ));
                }
            }
            if !required.is_empty() && profile.evidence_expression().is_empty() {
                errors.push(format!(
                    "generic profile `{}` missing evidence_expression",
                    profile.name
                ));
            }
            for token in evidence_expression_tokens(&profile.evidence_expression) {
                if EvidenceToken::parse(&token).is_none() {
                    errors.push(format!(
                        "generic profile `{}` has unknown evidence_expression token `{token}`",
                        profile.name
                    ));
                }
            }
            if !required.is_empty() && profile.observation_sources().is_empty() {
                errors.push(format!(
                    "generic profile `{}` missing observation_sources",
                    profile.name
                ));
            }
            validate_contract_runtime_fields(
                &mut errors,
                &format!("generic profile `{}`", profile.name),
                &profile.policy_mode,
                &profile.evidence_scope,
                &profile.freshness,
                &profile.artifact_kind,
                &profile.channel_visibility,
            );
            validate_artifact_shape_contract(
                &mut errors,
                &format!("generic profile `{}`", profile.name),
                None,
                &profile.final_answer_shape,
                &profile.artifact_kind,
                &profile.channel_visibility,
            );
            validate_observation_extractors(
                &mut errors,
                &format!("generic profile `{}`", profile.name),
                &profile.observation_sources(),
                &profile.observation_extractors,
            );
        }
        errors
    }

    pub(crate) fn matrix_version_hash(&self) -> String {
        let mut input = format!(
            "{}|{}|{}|{}|{}",
            self.schema_version,
            self.matrix_version,
            self.contracts.len(),
            self.generic_profiles.len(),
            self.trace_policy.stable_key()
        );
        for (key, contract) in &self.contracts {
            input.push('|');
            input.push_str(key);
            input.push(':');
            input.push_str(&contract.normalized_required_evidence().join(","));
            input.push(':');
            input.push_str(&contract.final_answer_shape);
            input.push(':');
            input.push_str(&normalized_tokens(&contract.allowed_actions).join(","));
            input.push(':');
            input.push_str(&normalized_tokens(&contract.preferred_actions).join(","));
            input.push(':');
            input.push_str(&normalized_tokens(&contract.forbidden_actions).join(","));
            input.push(':');
            input.push_str(
                &contract
                    .evidence_expression
                    .stable_key(&contract.normalized_required_evidence()),
            );
            input.push(':');
            input.push_str(&contract.policy_mode());
            input.push(':');
            input.push_str(&contract.evidence_scope());
            input.push(':');
            input.push_str(&contract.freshness());
            input.push(':');
            input.push_str(&contract.artifact_kind());
            input.push(':');
            input.push_str(&contract.channel_visibility());
            input.push(':');
            input.push_str(&observation_extractors_stable_key(
                &contract.observation_extractors(),
            ));
        }
        for profile in &self.generic_profiles {
            input.push('|');
            input.push_str("generic:");
            input.push_str(&profile.name);
            input.push(':');
            input.push_str(&profile.normalized_required_evidence().join(","));
            input.push(':');
            input.push_str(&profile.final_answer_shape);
            input.push(':');
            input.push_str(&normalized_tokens(&profile.allowed_actions).join(","));
            input.push(':');
            input.push_str(&normalized_tokens(&profile.preferred_actions).join(","));
            input.push(':');
            input.push_str(&normalized_tokens(&profile.forbidden_actions).join(","));
            input.push(':');
            input.push_str(
                &profile
                    .evidence_expression
                    .stable_key(&profile.normalized_required_evidence()),
            );
            input.push(':');
            input.push_str(&profile.policy_mode());
            input.push(':');
            input.push_str(&profile.evidence_scope());
            input.push(':');
            input.push_str(&profile.freshness());
            input.push(':');
            input.push_str(&profile.artifact_kind());
            input.push(':');
            input.push_str(&profile.channel_visibility());
            input.push(':');
            input.push_str(&observation_extractors_stable_key(
                &profile.observation_extractors(),
            ));
        }
        fnv1a_hex(&input)
    }

    #[cfg(test)]
    pub(crate) fn all_action_tokens(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for contract in self.contracts.values() {
            collect_action_tokens(&mut out, &contract.allowed_actions);
            collect_action_tokens(&mut out, &contract.preferred_actions);
            collect_action_tokens(&mut out, &contract.forbidden_actions);
        }
        for profile in &self.generic_profiles {
            collect_action_tokens(&mut out, &profile.allowed_actions);
            collect_action_tokens(&mut out, &profile.preferred_actions);
            collect_action_tokens(&mut out, &profile.forbidden_actions);
        }
        out
    }

    #[cfg(test)]
    pub(crate) fn unknown_matrix_skills(&self, registry: &SkillsRegistry) -> Vec<String> {
        let mut unknown = BTreeSet::new();
        for token in self.all_action_tokens() {
            let Some(action_ref) = ActionRef::parse(&token) else {
                continue;
            };
            if registry.resolve_canonical(&action_ref.skill).is_none() {
                unknown.insert(action_ref.skill);
            }
        }
        unknown.into_iter().collect()
    }

    #[cfg(test)]
    pub(crate) fn unknown_matrix_action_refs(&self, registry: &SkillsRegistry) -> Vec<String> {
        let known_refs = available_action_refs_from_registry(registry);
        let mut unknown = BTreeSet::new();
        for token in self.all_action_tokens() {
            let Some(action_ref) = ActionRef::parse(&token) else {
                continue;
            };
            if action_ref.action.is_some() && !known_refs.contains(&action_ref.as_key()) {
                unknown.insert(action_ref.as_key());
            }
        }
        unknown.into_iter().collect()
    }

    #[cfg(test)]
    pub(crate) fn external_observation_admission_errors(
        &self,
        registry: &SkillsRegistry,
    ) -> Vec<String> {
        let mut errors = BTreeSet::new();
        for (key, contract) in &self.contracts {
            collect_external_observation_admission_errors(
                &mut errors,
                &format!("contract `{key}`"),
                &contract.observation_sources(),
                &contract.observation_extractors(),
                registry,
            );
        }
        for profile in &self.generic_profiles {
            collect_external_observation_admission_errors(
                &mut errors,
                &format!("generic profile `{}`", profile.name),
                &profile.observation_sources(),
                &profile.observation_extractors(),
                registry,
            );
        }
        errors.into_iter().collect()
    }

    #[cfg(test)]
    pub(crate) fn backing_tool_refs_in_main_contracts(&self) -> Vec<String> {
        const BACKING_TOOL_NAMES: &[&str] = &["fs_search", "read_file", "write_file", "list_dir"];
        const SYSTEM_BASIC_LEGACY_ACTIONS: &[&str] = &[
            "inventory_dir",
            "count_inventory",
            "read_range",
            "compare_paths",
            "find_path",
            "extract_field",
            "extract_fields",
            "structured_keys",
            "validate_structured",
            "path_batch_facts",
        ];
        let mut refs = BTreeSet::new();
        for token in self.all_action_tokens() {
            let Some(action_ref) = ActionRef::parse(&token) else {
                continue;
            };
            let is_legacy_system_basic = action_ref.skill == "system_basic"
                && action_ref
                    .action
                    .as_deref()
                    .is_some_and(|action| SYSTEM_BASIC_LEGACY_ACTIONS.contains(&action));
            if is_legacy_system_basic || BACKING_TOOL_NAMES.contains(&action_ref.skill.as_str()) {
                refs.insert(action_ref.as_key());
            }
        }
        refs.into_iter().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContractActionPolicy {
    pub(crate) decision: ActionPolicyDecision,
    pub(crate) action_key: String,
    pub(crate) contract_match: String,
    pub(crate) required_evidence: Vec<String>,
    pub(crate) preferred_actions: Vec<String>,
    pub(crate) final_answer_shape_kind: FinalAnswerShape,
    pub(crate) final_answer_shape: String,
    pub(crate) evidence_expression: EvidenceExpression,
    pub(crate) policy_mode: String,
    pub(crate) evidence_scope: String,
    pub(crate) freshness: String,
    pub(crate) artifact_kind: String,
    pub(crate) channel_visibility: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContractArgPolicy {
    pub(crate) decision: ArgPolicyDecision,
    pub(crate) action_key: String,
    pub(crate) contract_match: String,
    pub(crate) required_evidence: Vec<String>,
    pub(crate) missing_target_args: Vec<String>,
    pub(crate) deferred_target_args: Vec<String>,
    pub(crate) expected_target_args: Vec<String>,
    pub(crate) final_answer_shape: String,
    pub(crate) policy_mode: String,
    pub(crate) evidence_scope: String,
    pub(crate) freshness: String,
    pub(crate) artifact_kind: String,
    pub(crate) channel_visibility: String,
}

impl ContractArgPolicy {
    pub(crate) fn is_allowed(&self) -> bool {
        self.decision == ArgPolicyDecision::Allowed
    }
}

impl ContractActionPolicy {
    pub(crate) fn is_allowed(&self) -> bool {
        self.decision == ActionPolicyDecision::Allowed
    }

    pub(crate) fn action_matches_preferred(&self) -> bool {
        action_matches_policy_tokens(&self.action_key, &self.preferred_actions)
    }
}

fn parse_contract_matrix_source(source: &str) -> Result<ContractMatrix, String> {
    let matrix: ContractMatrix =
        toml::from_str(source).map_err(|err| format!("contract matrix parse failed: {err}"))?;
    let shape_errors = matrix.validate_shape();
    if !shape_errors.is_empty() {
        return Err(format!(
            "contract matrix shape invalid: {}",
            shape_errors.join("; ")
        ));
    }
    Ok(matrix)
}

pub(crate) fn bundled_contract_matrix_result() -> Result<&'static ContractMatrix, &'static str> {
    match BUNDLED_CONTRACT_MATRIX.get_or_init(|| {
        parse_contract_matrix_source(include_str!("../../../configs/task_contract_matrix.toml"))
    }) {
        Ok(matrix) => Ok(matrix),
        Err(err) => Err(err.as_str()),
    }
}

pub(crate) fn bundled_contract_matrix() -> Option<&'static ContractMatrix> {
    bundled_contract_matrix_result().ok()
}

pub(crate) fn compact_prompt_line_for_route(route: &RouteResult) -> Option<String> {
    compact_prompt_line_for_output_contract(&route.output_contract)
}

pub(crate) fn compact_prompt_line_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<String> {
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let required_evidence = matched.required_evidence();
    let required_evidence = if required_evidence.is_empty() {
        "none".to_string()
    } else {
        required_evidence.join(",")
    };
    let allowed_actions = normalized_tokens(matched.allowed_actions());
    let allowed_actions = if allowed_actions.is_empty() {
        "none".to_string()
    } else {
        allowed_actions.join(",")
    };
    let forbidden_actions = normalized_tokens(matched.forbidden_actions());
    let forbidden_actions = if forbidden_actions.is_empty() {
        "none".to_string()
    } else {
        forbidden_actions.join(",")
    };

    Some(format!(
        "- contract_matrix version={} hash={} match={} required_evidence={} final_answer_shape={} allowed_actions={} forbidden_actions={}",
        matrix.matrix_version,
        matrix.matrix_version_hash(),
        matched.match_name(),
        required_evidence,
        matched
            .final_answer_shape_kind()
            .map(FinalAnswerShape::as_str)
            .unwrap_or_else(|| matched.final_answer_shape()),
        allowed_actions,
        forbidden_actions,
    ))
}

pub(crate) fn required_evidence_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<Vec<String>> {
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let mut fields = matched
        .required_evidence()
        .into_iter()
        .collect::<BTreeSet<_>>();
    if output_contract.delivery_required
        || matches!(
            output_contract.delivery_intent,
            crate::OutputDeliveryIntent::FileSingle
                | crate::OutputDeliveryIntent::DirectoryLookup
                | crate::OutputDeliveryIntent::DirectoryBatchFiles
        )
    {
        fields.insert("path".to_string());
    }
    if output_contract.semantic_kind == OutputSemanticKind::QuantityComparison
        && matches!(
            output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        fields.insert("exists".to_string());
        fields.insert("kind".to_string());
    }
    Some(fields.into_iter().collect())
}

pub(crate) fn final_answer_shape_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<FinalAnswerShape> {
    if let Some(shape) = final_answer_shape_override_for_output_contract(output_contract) {
        return Some(shape);
    }
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    matched.final_answer_shape_kind()
}

fn final_answer_shape_override_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<FinalAnswerShape> {
    if output_contract.semantic_kind == OutputSemanticKind::HiddenEntriesCheck
        && output_contract.response_shape == OutputResponseShape::Scalar
    {
        return Some(FinalAnswerShape::Scalar);
    }
    if output_contract.semantic_kind == OutputSemanticKind::StructuredKeys
        && output_contract.response_shape != OutputResponseShape::Strict
    {
        return Some(FinalAnswerShape::ValidationVerdict);
    }
    None
}

pub(crate) fn trace_snapshot_for_route(route: &RouteResult) -> Option<Value> {
    trace_snapshot_for_output_contract(&route.output_contract)
}

pub(crate) fn runtime_contract_snapshot_for_route(route: &RouteResult) -> Option<Value> {
    runtime_contract_snapshot_for_output_contract(&route.output_contract)
}

pub(crate) fn runtime_contract_snapshot_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<Value> {
    let matrix = bundled_contract_matrix()?;
    let contract_snapshot = trace_snapshot_for_output_contract(output_contract)?;
    let compact_line = compact_prompt_line_for_output_contract(output_contract);
    Some(json!({
        "schema_version": 1,
        "matrix": {
            "version": matrix.matrix_version,
            "hash": matrix.matrix_version_hash(),
            "source": "bundled:configs/task_contract_matrix.toml",
        },
        "registry": {
            "hash": bundled_registry_hash(),
            "source": "bundled:configs/skills_registry.toml",
        },
        "prompt_layer": {
            "hash": bundled_prompt_layer_manifest_hash(),
            "source": "bundled:prompts/layers/manifest.toml",
        },
        "compact_contract_block": compact_line.as_ref().map(|line| {
            json!({
                "hash": fnv1a_hex(line),
                "bytes": line.len(),
                "present": true,
            })
        }),
        "contract": contract_snapshot,
    }))
}

pub(crate) fn trace_snapshot_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<Value> {
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let final_answer_shape_kind = final_answer_shape_override_for_output_contract(output_contract)
        .or_else(|| matched.final_answer_shape_kind());
    let observation_extractors = matched.observation_extractors();
    Some(json!({
        "contract_matrix_version": matrix.matrix_version,
        "contract_matrix_hash": matrix.matrix_version_hash(),
        "schema_version": matrix.schema_version,
        "trace_policy": matrix.trace_policy.to_trace_json(),
        "semantic_kind": output_contract.semantic_kind.as_str(),
        "response_shape": output_contract.response_shape.as_str(),
        "locator_kind": output_contract.locator_kind.as_str(),
        "delivery_intent": output_contract.delivery_intent.as_str(),
        "requires_content_evidence": output_contract.requires_content_evidence,
        "delivery_required": output_contract.delivery_required,
        "contract_match": matched.match_name(),
        "policy_mode": matched.policy_mode(),
        "evidence_scope": matched.evidence_scope(),
        "freshness": matched.freshness(),
        "artifact_kind": matched.artifact_kind(),
        "channel_visibility": matched.channel_visibility(),
        "required_evidence": required_evidence_for_output_contract(output_contract)
            .unwrap_or_else(|| matched.required_evidence()),
        "evidence_expression": matched
            .evidence_expression()
            .to_trace_json(&matched.required_evidence()),
        "observation_sources": matched.observation_sources(),
        "observation_extractors": observation_extractors_trace_json(&observation_extractors),
        "final_answer_shape": final_answer_shape_kind
            .map(FinalAnswerShape::as_str)
            .unwrap_or_else(|| matched.final_answer_shape()),
        "final_answer_shape_class": final_answer_shape_kind.map(|shape| shape.class().as_str()),
        "coarse_response_shape": final_answer_shape_kind
            .map(|shape| shape.coarse_response_shape().as_str()),
        "allows_model_language": final_answer_shape_kind.map(FinalAnswerShape::allows_model_language),
        "preferred_actions": normalized_tokens(matched.preferred_actions()),
        "allowed_actions": normalized_tokens(matched.allowed_actions()),
        "forbidden_actions": normalized_tokens(matched.forbidden_actions()),
    }))
}

pub(crate) fn action_trace_for_output_contract(
    output_contract: &IntentOutputContract,
    action_ref: &str,
) -> Option<Value> {
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let action = ActionRef::parse(action_ref)?;
    let action_key = action.as_key();
    let observation_extractor = matched.observation_extractor_for_source(&action_key);
    let final_answer_shape_kind = final_answer_shape_override_for_output_contract(output_contract)
        .or_else(|| matched.final_answer_shape_kind());
    Some(json!({
        "schema_version": 1,
        "action_ref": action_key,
        "contract_match": matched.match_name(),
        "decision": matched.action_policy(&action).as_str(),
        "policy_mode": matched.policy_mode(),
        "observation_extractor": observation_extractor.as_ref().map(ObservationExtractor::to_trace_json),
        "required_evidence": required_evidence_for_output_contract(output_contract)
            .unwrap_or_else(|| matched.required_evidence()),
        "evidence_expression": matched
            .evidence_expression()
            .to_trace_json(&matched.required_evidence()),
        "final_answer_shape": final_answer_shape_kind
            .map(FinalAnswerShape::as_str)
            .unwrap_or_else(|| matched.final_answer_shape()),
        "final_answer_shape_class": final_answer_shape_kind.map(|shape| shape.class().as_str()),
        "coarse_response_shape": final_answer_shape_kind
            .map(|shape| shape.coarse_response_shape().as_str()),
        "allows_model_language": final_answer_shape_kind.map(FinalAnswerShape::allows_model_language),
        "preferred_actions": normalized_tokens(matched.preferred_actions()),
        "allowed_actions": normalized_tokens(matched.allowed_actions()),
        "forbidden_actions": normalized_tokens(matched.forbidden_actions()),
    }))
}

pub(crate) fn contract_trace_action_key_for_output_contract(
    output_contract: &IntentOutputContract,
    action_ref: &str,
) -> Option<String> {
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let action = ActionRef::parse(action_ref)?;
    if matched.action_policy(&action) != ActionPolicyDecision::Allowed {
        return Some(action.as_key());
    }
    for raw in matched.allowed_actions() {
        let Some(policy_ref) = ActionRef::parse(raw) else {
            continue;
        };
        if action_matches_any(&action, std::slice::from_ref(raw)) {
            return Some(policy_ref.as_key());
        }
    }
    Some(action.as_key())
}

pub(crate) fn preferred_action_refs_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Vec<ActionRef> {
    bundled_contract_matrix()
        .and_then(|matrix| matrix.match_output_contract(output_contract))
        .map(|matched| {
            matched
                .preferred_actions()
                .iter()
                .filter_map(|action| ActionRef::parse(action))
                .collect()
        })
        .unwrap_or_default()
}

fn contract_policy_action_ref(normalized_skill: &str, args: &Value) -> Option<ActionRef> {
    runtime_equivalent_virtual_action_ref(normalized_skill, args).or_else(|| {
        let canonical =
            crate::virtual_tools::canonicalize_legacy_tool_call(normalized_skill, args.clone())?;
        ActionRef::from_skill_args(&canonical.tool, &canonical.args)
    })
}

fn policy_action_ref_for_match(
    matched: &MatchedContract<'_>,
    normalized_skill: &str,
    args: &Value,
) -> Option<ActionRef> {
    let action = ActionRef::from_skill_args(normalized_skill, args)?;
    if matched.action_policy(&action) == ActionPolicyDecision::Allowed {
        return Some(action);
    }
    contract_policy_action_ref(normalized_skill, args)
        .filter(|canonical| matched.action_policy(canonical) == ActionPolicyDecision::Allowed)
        .or(Some(action))
}

fn runtime_equivalent_virtual_action_ref(
    normalized_skill: &str,
    args: &Value,
) -> Option<ActionRef> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_action_token)?;
    match (
        normalize_action_token(normalized_skill).as_str(),
        action.as_str(),
    ) {
        ("config_edit", "guard_config") => ActionRef::parse("config_basic.guard_rustclaw_config"),
        _ => None,
    }
}

pub(crate) fn action_policy_for_output_contract(
    output_contract: Option<&IntentOutputContract>,
    normalized_skill: &str,
    args: &Value,
) -> Option<ContractActionPolicy> {
    let output_contract = output_contract?;
    if output_contract.semantic_kind == OutputSemanticKind::None
        && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
    {
        return None;
    }
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let policy_action = policy_action_ref_for_match(&matched, normalized_skill, args)?;
    let final_answer_shape_kind = matched.final_answer_shape_kind()?;
    Some(ContractActionPolicy {
        decision: matched.action_policy(&policy_action),
        action_key: policy_action.as_key(),
        contract_match: matched.match_name().to_string(),
        required_evidence: matched.required_evidence(),
        preferred_actions: normalized_tokens(matched.preferred_actions()),
        final_answer_shape_kind,
        final_answer_shape: final_answer_shape_kind.as_str().to_string(),
        evidence_expression: matched.evidence_expression(),
        policy_mode: matched.policy_mode(),
        evidence_scope: matched.evidence_scope(),
        freshness: matched.freshness(),
        artifact_kind: matched.artifact_kind(),
        channel_visibility: matched.channel_visibility(),
    })
}

pub(crate) fn arg_policy_decision(
    output_contract: Option<&IntentOutputContract>,
    normalized_skill: &str,
    resolved_args: &Value,
) -> Option<ContractArgPolicy> {
    let output_contract = output_contract?;
    if output_contract.semantic_kind == OutputSemanticKind::None
        && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
    {
        return None;
    }
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let action = policy_action_ref_for_match(&matched, normalized_skill, resolved_args)?;
    let final_answer_shape_kind = matched.final_answer_shape_kind()?;
    let target_groups = contract_target_arg_groups(output_contract, &action);
    let expected_target_args = target_groups
        .iter()
        .flat_map(|group| group.iter().copied())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut missing_target_args = Vec::new();
    let mut deferred_target_args = Vec::new();
    for group in &target_groups {
        if arg_group_has_concrete_value(resolved_args, group) {
            continue;
        }
        let group_label = group.join("|");
        if arg_group_has_unresolved_template(resolved_args, group) {
            deferred_target_args.push(group_label);
        } else {
            missing_target_args.push(group_label);
        }
    }
    let decision = if !deferred_target_args.is_empty() {
        ArgPolicyDecision::DeferredTemplateArg
    } else if !missing_target_args.is_empty() {
        ArgPolicyDecision::MissingTargetBinding
    } else {
        ArgPolicyDecision::Allowed
    };
    Some(ContractArgPolicy {
        decision,
        action_key: action.as_key(),
        contract_match: matched.match_name().to_string(),
        required_evidence: matched.required_evidence(),
        missing_target_args,
        deferred_target_args,
        expected_target_args,
        final_answer_shape: final_answer_shape_kind.as_str().to_string(),
        policy_mode: matched.policy_mode(),
        evidence_scope: matched.evidence_scope(),
        freshness: matched.freshness(),
        artifact_kind: matched.artifact_kind(),
        channel_visibility: matched.channel_visibility(),
    })
}

pub(crate) fn action_matches_policy_tokens(action_key: &str, policies: &[String]) -> bool {
    let Some(action) = ActionRef::parse(action_key) else {
        return false;
    };
    action_matches_any(&action, policies)
}

fn contract_target_arg_groups(
    output_contract: &IntentOutputContract,
    action: &ActionRef,
) -> Vec<Vec<&'static str>> {
    if !output_contract.requires_content_evidence && !output_contract.delivery_required {
        return Vec::new();
    }
    if !matches!(
        output_contract.locator_kind,
        OutputLocatorKind::Path | OutputLocatorKind::Filename
    ) && !output_contract.delivery_required
    {
        return Vec::new();
    }
    match (action.skill.as_str(), action.action.as_deref()) {
        ("fs_basic", Some("compare_paths")) => vec![vec!["left_path"], vec!["right_path"]],
        ("fs_basic", Some("stat_paths")) => vec![vec!["path", "paths"]],
        ("fs_basic", Some("count_entries")) => vec![vec!["path"]],
        ("fs_basic", Some("list_dir" | "read_text_range")) => vec![vec!["path"]],
        ("fs_basic", Some("grep_text")) => vec![vec!["root", "path"]],
        ("fs_basic", Some("write_text" | "append_text" | "make_dir" | "remove_path")) => {
            vec![vec!["path"]]
        }
        ("doc_parse", _) => vec![vec!["path", "file_path", "requested_path"]],
        ("archive_basic", Some("list" | "read")) => vec![vec!["archive", "archive_path", "path"]],
        ("archive_basic", Some("pack")) => vec![vec!["source", "source_path", "path"]],
        ("archive_basic", Some("unpack")) => {
            vec![
                vec!["archive", "archive_path", "path"],
                vec!["dest", "dest_path"],
            ]
        }
        (
            "config_basic",
            Some("read_field" | "read_fields" | "list_keys" | "validate" | "guard_rustclaw_config"),
        ) => vec![vec!["path"]],
        (
            "config_edit",
            Some(
                "plan_config_change"
                | "apply_config_change"
                | "validate_config"
                | "guard_config"
                | "read_back"
                | "restart_if_requested",
            ),
        ) => vec![vec!["path"]],
        ("db_basic", _) => vec![vec!["db_path", "path"]],
        _ => Vec::new(),
    }
}

fn arg_group_has_concrete_value(args: &Value, group: &[&str]) -> bool {
    group
        .iter()
        .any(|name| args.get(*name).is_some_and(arg_value_is_concrete))
}

fn arg_group_has_unresolved_template(args: &Value, group: &[&str]) -> bool {
    group.iter().any(|name| {
        args.get(*name)
            .is_some_and(arg_value_has_unresolved_template)
    })
}

fn arg_value_is_concrete(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            !trimmed.is_empty() && !string_has_unresolved_template(trimmed)
        }
        Value::Array(values) => values.iter().any(arg_value_is_concrete),
        Value::Object(map) => map.values().any(arg_value_is_concrete),
        Value::Null => false,
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn arg_value_has_unresolved_template(value: &Value) -> bool {
    match value {
        Value::String(text) => string_has_unresolved_template(text),
        Value::Array(values) => values.iter().any(arg_value_has_unresolved_template),
        Value::Object(map) => map.values().any(arg_value_has_unresolved_template),
        _ => false,
    }
}

fn string_has_unresolved_template(value: &str) -> bool {
    value.contains("{{") && value.contains("}}")
}

#[cfg(test)]
pub(crate) fn available_action_refs_from_registry(registry: &SkillsRegistry) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for name in registry.all_names() {
        out.insert(name.clone());
        if let Some(manifest) = registry.manifest(&name) {
            if let Some(action) = manifest.runtime_action.as_deref() {
                if let Some(action_ref) = ActionRef::parse(&format!("{name}.{action}")) {
                    out.insert(action_ref.as_key());
                }
            }
            for capability in manifest.planner_capabilities {
                if let Some(action) = capability.action.as_deref() {
                    if let Some(action_ref) = ActionRef::parse(&format!("{name}.{action}")) {
                        out.insert(action_ref.as_key());
                    }
                }
            }
            collect_input_schema_action_refs(&mut out, &name, manifest.input_schema.as_ref());
        }
    }
    out
}

#[cfg(test)]
fn collect_input_schema_action_refs(
    out: &mut BTreeSet<String>,
    skill: &str,
    schema: Option<&Value>,
) {
    let Some(schema) = schema else {
        return;
    };
    let Some(action_schema) = schema
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get("action"))
    else {
        return;
    };
    let Some(action_enum) = action_schema.get("enum").and_then(Value::as_array) else {
        return;
    };
    for action in action_enum.iter().filter_map(Value::as_str) {
        if let Some(action_ref) = ActionRef::parse(&format!("{skill}.{action}")) {
            out.insert(action_ref.as_key());
        }
    }
}

#[cfg(test)]
fn collect_action_tokens(out: &mut BTreeSet<String>, values: &[String]) {
    for value in values {
        if let Some(action_ref) = ActionRef::parse(value) {
            out.insert(action_ref.as_key());
        }
    }
}

#[cfg(test)]
fn collect_external_observation_admission_errors(
    errors: &mut BTreeSet<String>,
    context: &str,
    observation_sources: &[String],
    observation_extractors: &[ObservationExtractor],
    registry: &SkillsRegistry,
) {
    for token in observation_sources {
        let Some(action_ref) = ActionRef::parse(token) else {
            continue;
        };
        let Some(entry) = registry.get(&action_ref.skill) else {
            continue;
        };
        let requires_admission = entry.matrix_admission.is_some()
            || entry.kind == SkillKind::External
            || entry
                .external_bundle_dir
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
        if !requires_admission {
            continue;
        }
        if !registry.matrix_admission_eligible(&action_ref.skill, action_ref.action.as_deref()) {
            errors.insert(format!(
                "{context} observation_source `{}` requires matrix_admission.eligible=true for strict evidence use",
                action_ref.as_key()
            ));
        }
        let uses_text_legacy = observation_extractors.iter().any(|extractor| {
            extractor.extractor_kind == "text_legacy"
                && (extractor.source == action_ref.as_key() || extractor.source == action_ref.skill)
        });
        if uses_text_legacy {
            let admission_allows_text_legacy = entry
                .matrix_admission
                .as_ref()
                .and_then(|admission| admission.extractor_kind.as_deref())
                .is_some_and(|kind| normalize_action_token(kind) == "text_legacy");
            if !admission_allows_text_legacy {
                errors.insert(format!(
                    "{context} observation_source `{}` uses text_legacy extractor without matrix_admission.extractor_kind=text_legacy",
                    action_ref.as_key()
                ));
            }
        }
    }
}

fn action_matches_any(action: &ActionRef, policies: &[String]) -> bool {
    policies.iter().any(|policy| {
        let Some(policy_ref) = ActionRef::parse(policy) else {
            return false;
        };
        if action.skill != policy_ref.skill {
            return false;
        }
        match &policy_ref.action {
            Some(policy_action) => action
                .action
                .as_deref()
                .is_some_and(|action| action == policy_action),
            None => true,
        }
    })
}

fn contains_token(values: &[String], needle: &str) -> bool {
    values
        .iter()
        .any(|value| normalize_action_token(value) == normalize_action_token(needle))
}

fn normalized_tokens(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| normalize_action_token(value))
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn observation_extractors_for_sources(
    sources: Vec<String>,
    configured_extractors: &[ObservationExtractor],
) -> Vec<ObservationExtractor> {
    let mut extractors = BTreeMap::new();
    for source in sources {
        if let Some(extractor) = ObservationExtractor::from_source(&source) {
            extractors.insert(extractor.stable_key(), extractor);
        }
    }
    for configured in configured_extractors {
        if let Some(extractor) =
            ObservationExtractor::normalized(&configured.source, &configured.extractor_kind)
        {
            extractors.insert(extractor.stable_key(), extractor);
        }
    }
    extractors.into_values().collect()
}

fn observation_extractors_trace_json(extractors: &[ObservationExtractor]) -> Value {
    json!(extractors
        .iter()
        .map(ObservationExtractor::to_trace_json)
        .collect::<Vec<_>>())
}

fn observation_extractors_stable_key(extractors: &[ObservationExtractor]) -> String {
    extractors
        .iter()
        .map(ObservationExtractor::stable_key)
        .collect::<Vec<_>>()
        .join(",")
}

fn default_extractor_kind_for_observation_source(source: &str) -> &'static str {
    match normalize_action_token(source).as_str() {
        "run_cmd" | "http_basic" => "text_legacy",
        _ => "structured_json",
    }
}

fn normalized_extractor_kind(value: &str) -> String {
    normalized_contract_field(value, "structured_json")
}

fn extractor_kind_is_valid(value: &str) -> bool {
    matches!(value, "structured_json" | "text_legacy")
}

fn normalized_contract_field(value: &str, default: &str) -> String {
    let normalized = normalize_action_token(value);
    if normalized.is_empty() {
        default.to_string()
    } else {
        normalized
    }
}

fn validate_contract_field(
    errors: &mut Vec<String>,
    context: &str,
    field: &str,
    raw: &str,
    default: &str,
    allowed: &[&str],
) {
    let value = normalized_contract_field(raw, default);
    if !allowed.contains(&value.as_str()) {
        errors.push(format!("{context} has invalid {field} `{raw}`"));
    }
}

fn validate_contract_runtime_fields(
    errors: &mut Vec<String>,
    context: &str,
    policy_mode: &str,
    evidence_scope: &str,
    freshness: &str,
    artifact_kind: &str,
    channel_visibility: &str,
) {
    validate_contract_field(
        errors,
        context,
        "policy_mode",
        policy_mode,
        "enforce",
        &["observe", "enforce"],
    );
    validate_contract_field(
        errors,
        context,
        "evidence_scope",
        evidence_scope,
        "current_task",
        &[
            "current_step",
            "current_task",
            "active_task",
            "conversation",
            "long_term_memory",
        ],
    );
    validate_contract_field(
        errors,
        context,
        "freshness",
        freshness,
        "current_task",
        &[
            "realtime",
            "current_task",
            "active_task",
            "conversation",
            "long_term_memory",
        ],
    );
    validate_contract_field(
        errors,
        context,
        "artifact_kind",
        artifact_kind,
        "text",
        &["text", "file", "image", "audio", "url"],
    );
    validate_contract_field(
        errors,
        context,
        "channel_visibility",
        channel_visibility,
        "user_visible",
        &["user_visible", "trace_only"],
    );
}

fn validate_artifact_shape_contract(
    errors: &mut Vec<String>,
    context: &str,
    delivery_shape: Option<&str>,
    final_answer_shape: &str,
    artifact_kind: &str,
    channel_visibility: &str,
) {
    let normalized_artifact = normalized_contract_field(artifact_kind, "text");
    let normalized_visibility = normalized_contract_field(channel_visibility, "user_visible");
    let shape = FinalAnswerShape::parse(final_answer_shape);
    if shape.is_some_and(|shape| shape.class() == FinalAnswerShapeClass::DeliveryArtifact) {
        if normalized_artifact == "text" {
            errors.push(format!(
                "{context} delivery artifact final_answer_shape must declare non-text artifact_kind"
            ));
        }
        if normalized_visibility != "user_visible" {
            errors.push(format!(
                "{context} delivery artifact final_answer_shape must be user_visible"
            ));
        }
    }
    if delivery_shape.is_some_and(|value| normalize_action_token(value) == "file")
        && normalized_artifact != "file"
    {
        errors.push(format!(
            "{context} delivery_shape=file must declare artifact_kind=file"
        ));
    }
}

fn validate_observation_extractors(
    errors: &mut Vec<String>,
    context: &str,
    observation_sources: &[String],
    configured_extractors: &[ObservationExtractor],
) {
    let known_sources = observation_sources
        .iter()
        .map(|source| normalize_action_token(source))
        .collect::<BTreeSet<_>>();
    let mut seen_extractors = BTreeSet::new();
    for extractor in configured_extractors {
        let source = normalize_action_token(&extractor.source);
        if source.is_empty() {
            errors.push(format!(
                "{context} has observation_extractor without source"
            ));
            continue;
        }
        if !known_sources.contains(&source) {
            errors.push(format!(
                "{context} observation_extractor source `{}` is not in observation_sources",
                extractor.source
            ));
        }
        let extractor_kind = normalized_extractor_kind(&extractor.extractor_kind);
        if !extractor_kind_is_valid(&extractor_kind) {
            errors.push(format!(
                "{context} observation_extractor source `{}` has invalid extractor_kind `{}`",
                extractor.source, extractor.extractor_kind
            ));
            continue;
        }
        let extractor_key = format!("{source}={extractor_kind}");
        if !seen_extractors.insert(extractor_key) {
            errors.push(format!(
                "{context} has duplicate observation_extractor source `{}` extractor_kind `{}`",
                extractor.source, extractor.extractor_kind
            ));
        }
        if !crate::task_journal::evidence_extractor_registry_contains(&source, &extractor_kind) {
            errors.push(format!(
                "{context} observation_extractor source `{}` with extractor_kind `{}` is not declared in the evidence extractor registry",
                extractor.source, extractor.extractor_kind
            ));
        }
    }
}

fn evidence_expression_tokens(expression: &EvidenceExpression) -> Vec<String> {
    let mut tokens = BTreeSet::new();
    for value in expression
        .all_of
        .iter()
        .chain(expression.one_of.iter())
        .chain(expression.any_of.iter())
        .chain(expression.negative_evidence.iter())
    {
        let normalized = normalize_action_token(value);
        if !normalized.is_empty() {
            tokens.insert(normalized);
        }
    }
    tokens.into_iter().collect()
}

fn normalize_action_token(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(crate) fn fnv1a_hex(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn bundled_registry_hash() -> String {
    fnv1a_hex(include_str!("../../../configs/skills_registry.toml"))
}

fn bundled_prompt_layer_manifest_hash() -> String {
    fnv1a_hex(include_str!("../../../prompts/layers/manifest.toml"))
}

#[cfg(test)]
#[path = "contract_matrix_tests.rs"]
mod tests;

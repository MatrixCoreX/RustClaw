use crate::{IntentOutputContract, OutputResponseShape, OutputSemanticKind};
#[cfg(test)]
use anyhow::{Context, Result};
#[cfg(test)]
use claw_core::skill_registry::SkillsRegistry;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::path::Path;
use std::sync::OnceLock;

#[path = "contract_matrix_runtime.rs"]
mod runtime;
use runtime::{
    action_matches_any, contains_token, default_extractor_kind_for_observation_source,
    evidence_expression_tokens, normalize_action_token, normalized_contract_field,
    normalized_extractor_kind, normalized_tokens, observation_extractors_for_sources,
    observation_extractors_stable_key, validate_artifact_shape_contract,
    validate_contract_runtime_fields, validate_observation_extractors,
};
pub(crate) use runtime::{
    action_matches_policy_tokens, action_policy_for_output_contract,
    action_trace_for_output_contract, bundled_contract_matrix,
    compact_prompt_line_for_output_contract, final_answer_shape_for_output_contract, fnv1a_hex,
    required_evidence_for_output_contract, runtime_contract_snapshot_for_output_contract,
    trace_snapshot_for_output_contract,
};
#[cfg(test)]
pub(crate) use runtime::{
    available_action_refs_from_registry, bundled_contract_matrix_result,
    parse_contract_matrix_source,
};
#[cfg(test)]
use runtime::{collect_action_tokens, collect_external_observation_admission_errors};
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
    Count,
    DirectoryStructure,
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
        Self::Count,
        Self::DirectoryStructure,
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
            "count" => Some(Self::Count),
            "directory_structure" => Some(Self::DirectoryStructure),
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
            Self::Count => "count",
            Self::DirectoryStructure => "directory_structure",
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
    pub(crate) evidence_profile: String,
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

    fn evidence_profile(&self) -> String {
        normalized_contract_field(&self.evidence_profile, "generic")
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
    pub(crate) evidence_profile: String,
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

    fn evidence_profile(&self) -> String {
        normalized_contract_field(&self.evidence_profile, "generic")
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
    ComparisonVerdict,
    DeliveryTokenOrPath,
    ExistenceVerdictWithPath,
    ExcerptPlusSummary,
    FailedStepWithEvidence,
    Free,
    GroupedNameList,
    LifecycleResult,
    ListOrEmptyStatement,
    NameList,
    PathList,
    RawOutputOrShortSummary,
    RecentArtifactJudgment,
    RiskAssessment,
    Scalar,
    ScalarEqualityVerdict,
    SinglePath,
    StatusWithSource,
    SummaryGroundedInExcerpt,
    SummaryGroundedInListing,
    SummaryWithEvidence,
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
            Self::Verdict => "verdict",
        }
    }

    pub(crate) fn coarse_response_shape(self) -> OutputResponseShape {
        match self {
            Self::DeliveryArtifact => OutputResponseShape::FileToken,
            Self::ScalarValue | Self::SinglePath => OutputResponseShape::Scalar,
            Self::StrictList => OutputResponseShape::Strict,
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
        Self::ComparisonVerdict,
        Self::DeliveryTokenOrPath,
        Self::ExistenceVerdictWithPath,
        Self::ExcerptPlusSummary,
        Self::FailedStepWithEvidence,
        Self::Free,
        Self::GroupedNameList,
        Self::LifecycleResult,
        Self::ListOrEmptyStatement,
        Self::NameList,
        Self::PathList,
        Self::RawOutputOrShortSummary,
        Self::RecentArtifactJudgment,
        Self::RiskAssessment,
        Self::Scalar,
        Self::ScalarEqualityVerdict,
        Self::SinglePath,
        Self::StatusWithSource,
        Self::SummaryGroundedInExcerpt,
        Self::SummaryGroundedInListing,
        Self::SummaryWithEvidence,
        Self::ValidationVerdict,
    ];

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "comparison_verdict" => Some(Self::ComparisonVerdict),
            "delivery_token_or_path" => Some(Self::DeliveryTokenOrPath),
            "existence_verdict_with_path" => Some(Self::ExistenceVerdictWithPath),
            "excerpt_plus_summary" => Some(Self::ExcerptPlusSummary),
            "failed_step_with_evidence" => Some(Self::FailedStepWithEvidence),
            "free" => Some(Self::Free),
            "grouped_name_list" => Some(Self::GroupedNameList),
            "lifecycle_result" => Some(Self::LifecycleResult),
            "list_or_empty_statement" => Some(Self::ListOrEmptyStatement),
            "name_list" => Some(Self::NameList),
            "path_list" => Some(Self::PathList),
            "raw_output_or_short_summary" => Some(Self::RawOutputOrShortSummary),
            "recent_artifact_judgment" => Some(Self::RecentArtifactJudgment),
            "risk_assessment" => Some(Self::RiskAssessment),
            "scalar" => Some(Self::Scalar),
            "scalar_equality_verdict" => Some(Self::ScalarEqualityVerdict),
            "single_path" => Some(Self::SinglePath),
            "status_with_source" => Some(Self::StatusWithSource),
            "summary_grounded_in_excerpt" => Some(Self::SummaryGroundedInExcerpt),
            "summary_grounded_in_listing" => Some(Self::SummaryGroundedInListing),
            "summary_with_evidence" => Some(Self::SummaryWithEvidence),
            "validation_verdict" => Some(Self::ValidationVerdict),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ComparisonVerdict => "comparison_verdict",
            Self::DeliveryTokenOrPath => "delivery_token_or_path",
            Self::ExistenceVerdictWithPath => "existence_verdict_with_path",
            Self::ExcerptPlusSummary => "excerpt_plus_summary",
            Self::FailedStepWithEvidence => "failed_step_with_evidence",
            Self::Free => "free",
            Self::GroupedNameList => "grouped_name_list",
            Self::LifecycleResult => "lifecycle_result",
            Self::ListOrEmptyStatement => "list_or_empty_statement",
            Self::NameList => "name_list",
            Self::PathList => "path_list",
            Self::RawOutputOrShortSummary => "raw_output_or_short_summary",
            Self::RecentArtifactJudgment => "recent_artifact_judgment",
            Self::RiskAssessment => "risk_assessment",
            Self::Scalar => "scalar",
            Self::ScalarEqualityVerdict => "scalar_equality_verdict",
            Self::SinglePath => "single_path",
            Self::StatusWithSource => "status_with_source",
            Self::SummaryGroundedInExcerpt => "summary_grounded_in_excerpt",
            Self::SummaryGroundedInListing => "summary_grounded_in_listing",
            Self::SummaryWithEvidence => "summary_with_evidence",
            Self::ValidationVerdict => "validation_verdict",
        }
    }

    pub(crate) fn class(self) -> FinalAnswerShapeClass {
        match self {
            Self::DeliveryTokenOrPath => FinalAnswerShapeClass::DeliveryArtifact,
            Self::SinglePath => FinalAnswerShapeClass::SinglePath,
            Self::Scalar => FinalAnswerShapeClass::ScalarValue,
            Self::GroupedNameList
            | Self::ListOrEmptyStatement
            | Self::NameList
            | Self::PathList => FinalAnswerShapeClass::StrictList,
            Self::ComparisonVerdict
            | Self::ExistenceVerdictWithPath
            | Self::LifecycleResult
            | Self::RecentArtifactJudgment
            | Self::RiskAssessment
            | Self::ScalarEqualityVerdict
            | Self::StatusWithSource
            | Self::ValidationVerdict => FinalAnswerShapeClass::Verdict,
            Self::ExcerptPlusSummary
            | Self::FailedStepWithEvidence
            | Self::RawOutputOrShortSummary
            | Self::SummaryGroundedInExcerpt
            | Self::SummaryGroundedInListing
            | Self::SummaryWithEvidence => FinalAnswerShapeClass::GroundedSummary,
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

    pub(crate) fn evidence_profile(&self) -> String {
        match self {
            Self::Semantic(contract) => contract.evidence_profile(),
            Self::Generic(profile) => profile.evidence_profile(),
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
                        &contract.evidence_profile,
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
                &profile.evidence_profile,
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
            input.push_str(&contract.evidence_profile());
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
            input.push_str(&profile.evidence_profile());
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

#[cfg(test)]
#[path = "contract_matrix_tests.rs"]
mod tests;

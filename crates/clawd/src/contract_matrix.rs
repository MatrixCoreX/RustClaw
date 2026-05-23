use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::{IntentOutputContract, OutputSemanticKind, RouteResult};

#[cfg(test)]
use anyhow::{Context, Result};
#[cfg(test)]
use claw_core::skill_registry::SkillsRegistry;
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
    pub(crate) allowed_actions: Vec<String>,
    pub(crate) preferred_actions: Vec<String>,
    pub(crate) forbidden_actions: Vec<String>,
    pub(crate) required_evidence: Vec<String>,
    pub(crate) final_answer_shape: String,
    pub(crate) none_passthrough: bool,
    pub(crate) failure_policy: String,
    pub(crate) locator_kinds: Vec<String>,
    pub(crate) observation_sources: Vec<String>,
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
    pub(crate) allowed_actions: Vec<String>,
    pub(crate) preferred_actions: Vec<String>,
    pub(crate) forbidden_actions: Vec<String>,
    pub(crate) required_evidence: Vec<String>,
    pub(crate) final_answer_shape: String,
    pub(crate) failure_policy: String,
    pub(crate) observation_sources: Vec<String>,
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
        const EXPECTED_ATTRIBUTIONS: &[&str] = &[
            "model_error",
            "schema_error",
            "code_gap",
            "contract_gap",
            "tool_gap",
            "permission_denied",
            "budget_exhausted",
            "prompt_budget_error",
            "delivery_error",
            "provider_error",
        ];
        for expected in EXPECTED_ATTRIBUTIONS {
            if !self
                .failure_attribution
                .iter()
                .any(|value| normalize_action_token(value) == *expected)
            {
                errors.push(format!("missing failure attribution `{expected}`"));
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
    pub(crate) fn backing_tool_refs_in_main_contracts(&self) -> Vec<String> {
        const BACKING_TOOL_NAMES: &[&str] = &[
            "system_basic",
            "fs_search",
            "read_file",
            "write_file",
            "list_dir",
        ];
        let mut refs = BTreeSet::new();
        for token in self.all_action_tokens() {
            let Some(action_ref) = ActionRef::parse(&token) else {
                continue;
            };
            if BACKING_TOOL_NAMES.contains(&action_ref.skill.as_str()) {
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
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    matched.final_answer_shape_kind()
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
        "required_evidence": required_evidence_for_output_contract(output_contract)
            .unwrap_or_else(|| matched.required_evidence()),
        "evidence_expression": matched
            .evidence_expression()
            .to_trace_json(&matched.required_evidence()),
        "observation_sources": matched.observation_sources(),
        "final_answer_shape": matched
            .final_answer_shape_kind()
            .map(FinalAnswerShape::as_str)
            .unwrap_or_else(|| matched.final_answer_shape()),
        "preferred_actions": normalized_tokens(matched.preferred_actions()),
        "allowed_actions": normalized_tokens(matched.allowed_actions()),
        "forbidden_actions": normalized_tokens(matched.forbidden_actions()),
    }))
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
    let action = ActionRef::from_skill_args(normalized_skill, args)?;
    let final_answer_shape_kind = matched.final_answer_shape_kind()?;
    Some(ContractActionPolicy {
        decision: matched.action_policy(&action),
        action_key: action.as_key(),
        contract_match: matched.match_name().to_string(),
        required_evidence: matched.required_evidence(),
        preferred_actions: normalized_tokens(matched.preferred_actions()),
        final_answer_shape_kind,
        final_answer_shape: final_answer_shape_kind.as_str().to_string(),
        evidence_expression: matched.evidence_expression(),
    })
}

pub(crate) fn action_matches_policy_tokens(action_key: &str, policies: &[String]) -> bool {
    let Some(action) = ActionRef::parse(action_key) else {
        return false;
    };
    action_matches_any(&action, policies)
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

fn fnv1a_hex(input: &str) -> String {
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
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::*;
    use crate::task_contract::fallback_required_evidence_fields_for_output_contract;
    use crate::{OutputLocatorKind, OutputResponseShape};

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn load_workspace_matrix() -> ContractMatrix {
        ContractMatrix::load_from_workspace(&workspace_root()).expect("load contract matrix")
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum GeneratedContractMatch {
        Semantic(OutputSemanticKind),
        Generic(String),
    }

    #[derive(Debug, Clone)]
    struct GeneratedMatrixCase {
        id: String,
        matched: GeneratedContractMatch,
        action: Option<ActionRef>,
        expected_decision: Option<ActionPolicyDecision>,
        expected_required_evidence: Vec<String>,
        expected_final_answer_shape: String,
    }

    fn generated_allowed_action(matched: &MatchedContract<'_>) -> Option<ActionRef> {
        for raw in matched.allowed_actions() {
            let action = ActionRef::parse(raw)?;
            if matched.action_policy(&action) == ActionPolicyDecision::Allowed {
                return Some(action);
            }
        }
        if matches!(
            matched,
            MatchedContract::Semantic(MatrixContract {
                none_passthrough: true,
                ..
            })
        ) {
            return ActionRef::parse("respond");
        }
        None
    }

    fn generated_negative_action(
        matched: &MatchedContract<'_>,
    ) -> Option<(ActionRef, ActionPolicyDecision)> {
        for raw in matched.forbidden_actions() {
            let action = ActionRef::parse(raw)?;
            let decision = matched.action_policy(&action);
            if decision != ActionPolicyDecision::Allowed {
                return Some((action, decision));
            }
        }

        let probes = [
            "run_cmd",
            "fs_basic.list_dir",
            "fs_basic.read_text_range",
            "fs_basic.write_text",
            "archive_basic.pack",
            "config_basic.validate",
            "docker_basic",
            "package_manager.detect",
            "db_basic",
            "health_check",
            "respond",
        ];
        for probe in probes {
            let action = ActionRef::parse(probe).expect("probe action parses");
            let decision = matched.action_policy(&action);
            if decision != ActionPolicyDecision::Allowed {
                return Some((action, decision));
            }
        }
        None
    }

    fn push_generated_case(
        cases: &mut Vec<GeneratedMatrixCase>,
        id: String,
        matched: GeneratedContractMatch,
        contract: &MatchedContract<'_>,
        action: Option<ActionRef>,
        expected_decision: Option<ActionPolicyDecision>,
    ) {
        cases.push(GeneratedMatrixCase {
            id,
            matched,
            action,
            expected_decision,
            expected_required_evidence: contract.required_evidence(),
            expected_final_answer_shape: contract.final_answer_shape().to_string(),
        });
    }

    fn generated_contract_cases(
        matrix: &ContractMatrix,
        minimum_count: usize,
    ) -> Vec<GeneratedMatrixCase> {
        let mut cases = Vec::new();

        for kind in OutputSemanticKind::ALL {
            let contract = matrix
                .semantic_contract(*kind)
                .expect("semantic contract exists");
            let matched = MatchedContract::Semantic(contract);
            let case_match = GeneratedContractMatch::Semantic(*kind);
            let prefix = kind.as_str();

            push_generated_case(
                &mut cases,
                format!("{prefix}::evidence_shape"),
                case_match.clone(),
                &matched,
                None,
                None,
            );

            if let Some(action) = generated_allowed_action(&matched) {
                let decision = matched.action_policy(&action);
                push_generated_case(
                    &mut cases,
                    format!("{prefix}::allowed::{}", action.as_key()),
                    case_match.clone(),
                    &matched,
                    Some(action),
                    Some(decision),
                );
            }

            if let Some((action, decision)) = generated_negative_action(&matched) {
                push_generated_case(
                    &mut cases,
                    format!("{prefix}::negative::{}", action.as_key()),
                    case_match,
                    &matched,
                    Some(action),
                    Some(decision),
                );
            }
        }

        for profile in &matrix.generic_profiles {
            let matched = MatchedContract::Generic(profile);
            let case_match = GeneratedContractMatch::Generic(profile.name.clone());
            let prefix = format!("generic::{}", profile.name);

            push_generated_case(
                &mut cases,
                format!("{prefix}::evidence_shape"),
                case_match.clone(),
                &matched,
                None,
                None,
            );

            if let Some(action) = generated_allowed_action(&matched) {
                let decision = matched.action_policy(&action);
                push_generated_case(
                    &mut cases,
                    format!("{prefix}::allowed::{}", action.as_key()),
                    case_match.clone(),
                    &matched,
                    Some(action),
                    Some(decision),
                );
            }

            if let Some((action, decision)) = generated_negative_action(&matched) {
                push_generated_case(
                    &mut cases,
                    format!("{prefix}::negative::{}", action.as_key()),
                    case_match,
                    &matched,
                    Some(action),
                    Some(decision),
                );
            }
        }

        assert!(
            cases.len() >= minimum_count,
            "matrix generated only {} cases, expected at least {minimum_count}",
            cases.len()
        );
        cases
    }

    fn matched_for_generated_case<'a>(
        matrix: &'a ContractMatrix,
        case: &GeneratedMatrixCase,
    ) -> MatchedContract<'a> {
        match &case.matched {
            GeneratedContractMatch::Semantic(kind) => MatchedContract::Semantic(
                matrix
                    .semantic_contract(*kind)
                    .expect("semantic contract exists"),
            ),
            GeneratedContractMatch::Generic(name) => MatchedContract::Generic(
                matrix
                    .generic_profiles
                    .iter()
                    .find(|profile| profile.name == *name)
                    .expect("generic profile exists"),
            ),
        }
    }

    #[test]
    fn workspace_contract_matrix_loads_and_has_shape() {
        let matrix = load_workspace_matrix();

        assert!(matrix.validate_shape().is_empty());
        assert_eq!(matrix.schema_version, 1);
        assert!(!matrix.matrix_version_hash().is_empty());
        assert!(matrix
            .failure_attribution
            .contains(&"model_error".to_string()));
        assert_eq!(matrix.policy.unknown_semantic, "reject");
        assert_eq!(
            matrix.trace_policy.evidence_storage,
            "redacted_excerpt_hash"
        );
        assert_eq!(
            matrix.trace_policy.provider_evidence_view,
            "provider_safe_redacted"
        );
    }

    #[test]
    fn bundled_contract_matrix_result_exposes_load_errors() {
        let matrix = bundled_contract_matrix_result().expect("bundled matrix should load");

        assert_eq!(matrix.schema_version, 1);

        let err = parse_contract_matrix_source(
            r#"schema_version = 1
matrix_version = "broken"
"#,
        )
        .expect_err("invalid matrix should report a concrete error");
        assert!(err.contains("contract matrix shape invalid"));
        assert!(err.contains("missing failure attribution"));
    }

    #[test]
    fn existence_contract_can_express_negative_evidence() {
        let matrix = load_workspace_matrix();
        let contract = matrix
            .semantic_contract(OutputSemanticKind::ExistenceWithPath)
            .expect("existence contract");
        let expression = contract.evidence_expression();

        assert_eq!(expression.all_of, vec!["kind", "path"]);
        assert_eq!(expression.one_of, vec!["exists_false", "exists_true"]);
        assert_eq!(expression.negative_evidence, vec!["exists_false"]);
    }

    #[test]
    fn trace_snapshot_includes_evidence_expression_trace_policy_and_sources() {
        let snapshot = trace_snapshot_for_output_contract(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FileNames,
            ..IntentOutputContract::default()
        })
        .expect("trace snapshot");

        assert_eq!(
            snapshot
                .get("trace_policy")
                .and_then(|value| value.get("provider_evidence_view"))
                .and_then(Value::as_str),
            Some("provider_safe_redacted")
        );
        assert_eq!(
            snapshot
                .get("evidence_expression")
                .and_then(|value| value.get("all_of"))
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_str),
            Some("candidates")
        );
        assert!(snapshot
            .get("observation_sources")
            .and_then(Value::as_array)
            .is_some_and(|items| items
                .iter()
                .any(|item| item.as_str() == Some("fs_basic.list_dir"))));
    }

    #[test]
    fn runtime_contract_snapshot_binds_matrix_and_compact_prompt_block() {
        let snapshot = runtime_contract_snapshot_for_output_contract(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FileNames,
            ..IntentOutputContract::default()
        })
        .expect("runtime contract snapshot");

        assert_eq!(
            snapshot
                .get("matrix")
                .and_then(|value| value.get("source"))
                .and_then(Value::as_str),
            Some("bundled:configs/task_contract_matrix.toml")
        );
        assert!(snapshot
            .get("matrix")
            .and_then(|value| value.get("hash"))
            .and_then(Value::as_str)
            .is_some_and(|hash| !hash.is_empty()));
        assert_eq!(
            snapshot
                .get("registry")
                .and_then(|value| value.get("source"))
                .and_then(Value::as_str),
            Some("bundled:configs/skills_registry.toml")
        );
        assert!(snapshot
            .get("registry")
            .and_then(|value| value.get("hash"))
            .and_then(Value::as_str)
            .is_some_and(|hash| !hash.is_empty()));
        assert_eq!(
            snapshot
                .get("prompt_layer")
                .and_then(|value| value.get("source"))
                .and_then(Value::as_str),
            Some("bundled:prompts/layers/manifest.toml")
        );
        assert!(snapshot
            .get("prompt_layer")
            .and_then(|value| value.get("hash"))
            .and_then(Value::as_str)
            .is_some_and(|hash| !hash.is_empty()));
        assert!(snapshot
            .get("compact_contract_block")
            .and_then(|value| value.get("hash"))
            .and_then(Value::as_str)
            .is_some_and(|hash| !hash.is_empty()));
        assert_eq!(
            snapshot
                .get("contract")
                .and_then(|value| value.get("contract_match"))
                .and_then(Value::as_str),
            Some("file_names")
        );
    }

    #[test]
    fn contract_matrix_final_answer_shapes_are_typed() {
        let matrix = load_workspace_matrix();
        let configured = matrix
            .contracts
            .values()
            .map(|contract| contract.final_answer_shape.as_str())
            .chain(
                matrix
                    .generic_profiles
                    .iter()
                    .map(|profile| profile.final_answer_shape.as_str()),
            )
            .collect::<BTreeSet<_>>();
        let typed = FinalAnswerShape::ALL
            .iter()
            .map(|shape| shape.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(configured, typed);
        for shape in configured {
            assert_eq!(
                FinalAnswerShape::parse(shape).map(FinalAnswerShape::as_str),
                Some(shape)
            );
        }
    }

    #[test]
    fn contract_matrix_evidence_tokens_are_typed() {
        let matrix = load_workspace_matrix();
        let mut configured = BTreeSet::new();

        for contract in matrix.contracts.values() {
            configured.extend(
                contract
                    .normalized_required_evidence()
                    .into_iter()
                    .filter(|field| !field.is_empty()),
            );
            configured.extend(evidence_expression_tokens(&contract.evidence_expression));
        }
        for profile in &matrix.generic_profiles {
            configured.extend(
                profile
                    .normalized_required_evidence()
                    .into_iter()
                    .filter(|field| !field.is_empty()),
            );
            configured.extend(evidence_expression_tokens(&profile.evidence_expression));
        }

        let typed = EvidenceToken::ALL
            .iter()
            .map(|token| token.as_str().to_string())
            .collect::<BTreeSet<_>>();

        assert_eq!(configured, typed);
        for token in configured {
            assert_eq!(
                EvidenceToken::parse(&token).map(EvidenceToken::as_str),
                Some(token.as_str())
            );
        }
    }

    #[test]
    fn bundled_contract_matrix_renders_prompt_line() {
        let line = compact_prompt_line_for_output_contract(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FileNames,
            ..IntentOutputContract::default()
        })
        .expect("contract prompt line");

        assert!(line.contains("contract_matrix"));
        assert!(line.contains("match=file_names"));
        assert!(line.contains("required_evidence=candidates"));
    }

    #[test]
    fn contract_matrix_covers_all_output_semantic_kinds() {
        let matrix = load_workspace_matrix();

        let missing = OutputSemanticKind::ALL
            .iter()
            .filter(|kind| matrix.semantic_contract(**kind).is_none())
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>();

        assert!(
            missing.is_empty(),
            "missing semantic contracts: {missing:?}"
        );
    }

    #[test]
    fn contract_matrix_evidence_matches_task_contract_defaults() {
        let matrix = load_workspace_matrix();

        for kind in OutputSemanticKind::ALL {
            let output_contract = IntentOutputContract {
                semantic_kind: *kind,
                ..IntentOutputContract::default()
            };
            let expected = fallback_required_evidence_fields_for_output_contract(&output_contract);
            let actual = matrix
                .semantic_contract(*kind)
                .expect("semantic contract")
                .normalized_required_evidence();

            assert_eq!(
                actual,
                expected,
                "evidence mismatch for semantic `{}`",
                kind.as_str()
            );
        }
    }

    #[test]
    fn route_specific_evidence_augments_matrix_base_contract() {
        let required = required_evidence_for_output_contract(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::QuantityComparison,
            locator_kind: OutputLocatorKind::Filename,
            ..IntentOutputContract::default()
        })
        .expect("required evidence");

        assert_eq!(
            required,
            vec!["exists", "field_value", "kind", "size_bytes"]
        );
    }

    #[test]
    fn generic_profile_matches_untyped_path_content_contract() {
        let matrix = load_workspace_matrix();
        let matched = matrix
            .match_output_contract(&IntentOutputContract {
                semantic_kind: OutputSemanticKind::None,
                requires_content_evidence: true,
                locator_kind: OutputLocatorKind::Path,
                response_shape: OutputResponseShape::Free,
                ..IntentOutputContract::default()
            })
            .expect("generic profile match");

        assert_eq!(matched.required_evidence(), vec!["content_excerpt", "path"]);
        assert_eq!(matched.final_answer_shape(), "summary_with_evidence");
    }

    #[test]
    fn semantic_none_rejects_forbidden_action() {
        let matrix = load_workspace_matrix();
        let contract = matrix
            .match_output_contract(&IntentOutputContract {
                semantic_kind: OutputSemanticKind::None,
                requires_content_evidence: true,
                locator_kind: OutputLocatorKind::Path,
                ..IntentOutputContract::default()
            })
            .expect("matched contract");
        let action = ActionRef::parse("run_cmd").expect("action ref");

        assert_eq!(
            contract.action_policy(&action),
            ActionPolicyDecision::RejectedForbidden
        );
    }

    #[test]
    fn action_policy_blocks_disallowed_structured_action_for_semantic_contract() {
        let policy = action_policy_for_output_contract(
            Some(&IntentOutputContract {
                semantic_kind: OutputSemanticKind::FileNames,
                requires_content_evidence: true,
                ..IntentOutputContract::default()
            }),
            "run_cmd",
            &serde_json::json!({"command":"ls"}),
        )
        .expect("policy decision");

        assert_eq!(policy.decision, ActionPolicyDecision::RejectedNotAllowed);
        assert_eq!(policy.contract_match, "file_names");
        assert_eq!(policy.required_evidence, vec!["candidates"]);
    }

    #[test]
    fn action_policy_skips_unstructured_none_contracts() {
        let policy = action_policy_for_output_contract(
            Some(&IntentOutputContract::default()),
            "run_cmd",
            &serde_json::json!({"command":"echo ok"}),
        );

        assert!(policy.is_none());
    }

    #[test]
    fn action_ref_prefers_structured_action_from_args() {
        let action =
            ActionRef::from_skill_args("fs-basic", &serde_json::json!({"action":"list_dir"}))
                .expect("action ref");

        assert_eq!(action.as_key(), "fs_basic.list_dir");
    }

    #[test]
    fn contract_matrix_references_registered_skills() {
        let matrix = load_workspace_matrix();
        let registry_path = workspace_root().join("configs/skills_registry.toml");
        let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");

        let unknown = matrix.unknown_matrix_skills(&registry);

        assert!(unknown.is_empty(), "unknown matrix skills: {unknown:?}");
    }

    #[test]
    fn contract_matrix_action_refs_are_declared_in_registry() {
        let matrix = load_workspace_matrix();
        let registry_path = workspace_root().join("configs/skills_registry.toml");
        let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");

        let unknown = matrix.unknown_matrix_action_refs(&registry);

        assert!(
            unknown.is_empty(),
            "unknown matrix action refs: {unknown:?}"
        );
    }

    #[test]
    fn contract_matrix_action_refs_have_registry_schemas() {
        let matrix = load_workspace_matrix();
        let registry_path = workspace_root().join("configs/skills_registry.toml");
        let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");
        let mut missing = Vec::new();

        for token in matrix.all_action_tokens() {
            let Some(action_ref) = ActionRef::parse(&token) else {
                continue;
            };
            let Some(skill) = registry.resolve_canonical(&action_ref.skill) else {
                continue;
            };
            let Some(manifest) = registry.manifest(skill) else {
                continue;
            };
            if manifest.input_schema.is_none() {
                missing.push(format!("{}.input_schema", action_ref.skill));
            }
            if manifest.output_schema.is_none() {
                missing.push(format!("{}.output_schema", action_ref.skill));
            }
        }
        missing.sort();
        missing.dedup();

        assert!(missing.is_empty(), "missing registry schemas: {missing:?}");
    }

    #[test]
    fn contract_matrix_main_contracts_do_not_reference_backing_tools() {
        let matrix = load_workspace_matrix();

        let backing_refs = matrix.backing_tool_refs_in_main_contracts();

        assert!(
            backing_refs.is_empty(),
            "matrix should use planner-facing actions, not backing tools: {backing_refs:?}"
        );
    }

    #[test]
    fn registry_action_index_contains_skill_level_and_action_level_refs() {
        let registry_path = workspace_root().join("configs/skills_registry.toml");
        let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");
        let refs = available_action_refs_from_registry(&registry);

        assert!(refs.contains("fs_basic"));
        assert!(refs.contains("fs_basic.list_dir"));
        assert!(refs.contains("archive_basic.pack"));
    }

    #[test]
    fn matrix_generated_cases_cover_at_least_100_unique_contract_paths() {
        let matrix = load_workspace_matrix();
        let cases = generated_contract_cases(&matrix, 100);

        let mut ids = BTreeSet::new();
        let mut semantic_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut generic_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut decisions = BTreeSet::new();

        for case in &cases {
            assert!(
                ids.insert(case.id.as_str()),
                "duplicate case id: {}",
                case.id
            );

            match &case.matched {
                GeneratedContractMatch::Semantic(kind) => {
                    *semantic_counts.entry(kind.as_str()).or_default() += 1;
                }
                GeneratedContractMatch::Generic(name) => {
                    *generic_counts.entry(name.clone()).or_default() += 1;
                }
            }

            let matched = matched_for_generated_case(&matrix, case);
            assert_eq!(
                case.expected_required_evidence,
                matched.required_evidence(),
                "required evidence drift in generated case {}",
                case.id
            );
            assert_eq!(
                case.expected_final_answer_shape,
                matched.final_answer_shape(),
                "final answer shape drift in generated case {}",
                case.id
            );

            if let Some(action) = &case.action {
                let expected = case
                    .expected_decision
                    .expect("action case has expected decision");
                let actual = matched.action_policy(action);
                assert_eq!(
                    actual, expected,
                    "action decision drift in generated case {}",
                    case.id
                );
                decisions.insert(actual.as_str());
            }
        }

        assert!(
            OutputSemanticKind::ALL
                .iter()
                .all(|kind| semantic_counts.contains_key(kind.as_str())),
            "generated cases must cover every semantic kind"
        );
        assert!(
            matrix
                .generic_profiles
                .iter()
                .all(|profile| generic_counts.contains_key(&profile.name)),
            "generated cases must cover every generic profile"
        );
        assert!(
            decisions.contains(ActionPolicyDecision::Allowed.as_str()),
            "generated cases must include allowed action decisions"
        );
        assert!(
            decisions.contains(ActionPolicyDecision::RejectedForbidden.as_str()),
            "generated cases must include forbidden action decisions"
        );
        assert!(
            decisions.contains(ActionPolicyDecision::RejectedNotAllowed.as_str()),
            "generated cases must include not-allowed action decisions"
        );
    }
}

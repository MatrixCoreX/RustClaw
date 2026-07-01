use serde_json::{json, Value};

use crate::runtime::ask_mode::{ActFinalizeStyle, AskMode};
use crate::runtime::types::{AgentAction, FirstLayerDecision, ScheduleIntentOutput};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OutputResponseShape {
    #[default]
    Free,
    OneSentence,
    Strict,
    Scalar,
    FileToken,
}

impl OutputResponseShape {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::OneSentence => "one_sentence",
            Self::Strict => "strict",
            Self::Scalar => "scalar",
            Self::FileToken => "file_token",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OutputLocatorKind {
    #[default]
    None,
    Path,
    CurrentWorkspace,
    Url,
    Filename,
}

impl OutputLocatorKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Path => "path",
            Self::CurrentWorkspace => "current_workspace",
            Self::Url => "url",
            Self::Filename => "filename",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OutputDeliveryIntent {
    #[default]
    None,
    FileSingle,
    DirectoryLookup,
    DirectoryBatchFiles,
}

impl OutputDeliveryIntent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::FileSingle => "file_single",
            Self::DirectoryLookup => "directory_lookup",
            Self::DirectoryBatchFiles => "directory_batch_files",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OutputSemanticKind {
    #[default]
    None,
    RawCommandOutput,
    CommandOutputSummary,
    ServiceStatus,
    HiddenEntriesCheck,
    FileNames,
    DirectoryNames,
    DirectoryEntryGroups,
    FilePaths,
    DirectoryPurposeSummary,
    ContentExcerptSummary,
    DocumentHeading,
    ContentExcerptWithSummary,
    ContentPresenceCheck,
    ExcerptKindJudgment,
    RecentArtifactsJudgment,
    WorkspaceProjectSummary,
    ScalarCount,
    QuantityComparison,
    ExecutionFailedStep,
    GeneratedFileDelivery,
    GeneratedFilePathReport,
    FilesystemMutationResult,
    ScalarPathOnly,
    FileBasename,
    ExistenceWithPath,
    ExistenceWithPathSummary,
    RecentScalarEqualityCheck,
    GitCommitSubject,
    GitRepositoryState,
    StructuredKeys,
    ConfigValidation,
    ConfigMutation,
    ConfigRiskAssessment,
    SqliteTableListing,
    SqliteTableNamesOnly,
    SqliteDatabaseKindJudgment,
    SqliteSchemaVersion,
    RssNewsFetch,
    WebPageSummary,
    WebSearchSummary,
    WeatherQuery,
    MarketQuote,
    ImageUnderstanding,
    PhotoOrganization,
    PublishingPreview,
    PackageManagerDetection,
    ToolDiscovery,
    ArchiveList,
    ArchiveRead,
    ArchivePack,
    ArchiveUnpack,
    DockerPs,
    DockerImages,
    DockerLogs,
    DockerContainerLifecycle,
}

impl OutputSemanticKind {
    pub(crate) const ALL: &'static [Self] = &[
        Self::None,
        Self::RawCommandOutput,
        Self::CommandOutputSummary,
        Self::ServiceStatus,
        Self::HiddenEntriesCheck,
        Self::FileNames,
        Self::DirectoryNames,
        Self::DirectoryEntryGroups,
        Self::FilePaths,
        Self::DirectoryPurposeSummary,
        Self::ContentExcerptSummary,
        Self::DocumentHeading,
        Self::ContentExcerptWithSummary,
        Self::ContentPresenceCheck,
        Self::ExcerptKindJudgment,
        Self::RecentArtifactsJudgment,
        Self::WorkspaceProjectSummary,
        Self::ScalarCount,
        Self::QuantityComparison,
        Self::ExecutionFailedStep,
        Self::GeneratedFileDelivery,
        Self::GeneratedFilePathReport,
        Self::FilesystemMutationResult,
        Self::ScalarPathOnly,
        Self::FileBasename,
        Self::ExistenceWithPath,
        Self::ExistenceWithPathSummary,
        Self::RecentScalarEqualityCheck,
        Self::GitCommitSubject,
        Self::GitRepositoryState,
        Self::StructuredKeys,
        Self::ConfigValidation,
        Self::ConfigMutation,
        Self::ConfigRiskAssessment,
        Self::SqliteTableListing,
        Self::SqliteTableNamesOnly,
        Self::SqliteDatabaseKindJudgment,
        Self::SqliteSchemaVersion,
        Self::RssNewsFetch,
        Self::WebPageSummary,
        Self::WebSearchSummary,
        Self::WeatherQuery,
        Self::MarketQuote,
        Self::ImageUnderstanding,
        Self::PhotoOrganization,
        Self::PublishingPreview,
        Self::PackageManagerDetection,
        Self::ToolDiscovery,
        Self::ArchiveList,
        Self::ArchiveRead,
        Self::ArchivePack,
        Self::ArchiveUnpack,
        Self::DockerPs,
        Self::DockerImages,
        Self::DockerLogs,
        Self::DockerContainerLifecycle,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RawCommandOutput => "raw_command_output",
            Self::CommandOutputSummary => "command_output_summary",
            Self::ServiceStatus => "service_status",
            Self::HiddenEntriesCheck => "hidden_entries_check",
            Self::FileNames => "file_names",
            Self::DirectoryNames => "directory_names",
            Self::DirectoryEntryGroups => "directory_entry_groups",
            Self::FilePaths => "file_paths",
            Self::DirectoryPurposeSummary => "directory_purpose_summary",
            Self::ContentExcerptSummary => "content_excerpt_summary",
            Self::DocumentHeading => "document_heading",
            Self::ContentExcerptWithSummary => "content_excerpt_with_summary",
            Self::ContentPresenceCheck => "content_presence_check",
            Self::ExcerptKindJudgment => "excerpt_kind_judgment",
            Self::RecentArtifactsJudgment => "recent_artifacts_judgment",
            Self::WorkspaceProjectSummary => "workspace_project_summary",
            Self::ScalarCount => "scalar_count",
            Self::QuantityComparison => "quantity_comparison",
            Self::ExecutionFailedStep => "execution_failed_step",
            Self::GeneratedFileDelivery => "generated_file_delivery",
            Self::GeneratedFilePathReport => "generated_file_path_report",
            Self::FilesystemMutationResult => "filesystem_mutation_result",
            Self::ScalarPathOnly => "scalar_path_only",
            Self::FileBasename => "file_basename",
            Self::ExistenceWithPath => "existence_with_path",
            Self::ExistenceWithPathSummary => "existence_with_path_summary",
            Self::RecentScalarEqualityCheck => "recent_scalar_equality_check",
            Self::GitCommitSubject => "git_commit_subject",
            Self::GitRepositoryState => "git_repository_state",
            Self::StructuredKeys => "structured_keys",
            Self::ConfigValidation => "config_validation",
            Self::ConfigMutation => "config_mutation",
            Self::ConfigRiskAssessment => "config_risk_assessment",
            Self::SqliteTableListing => "sqlite_table_listing",
            Self::SqliteTableNamesOnly => "sqlite_table_names_only",
            Self::SqliteDatabaseKindJudgment => "sqlite_database_kind_judgment",
            Self::SqliteSchemaVersion => "sqlite_schema_version",
            Self::RssNewsFetch => "rss_news_fetch",
            Self::WebPageSummary => "web_page_summary",
            Self::WebSearchSummary => "web_search_summary",
            Self::WeatherQuery => "weather_query",
            Self::MarketQuote => "market_quote",
            Self::ImageUnderstanding => "image_understanding",
            Self::PhotoOrganization => "photo_organization",
            Self::PublishingPreview => "publishing_preview",
            Self::PackageManagerDetection => "package_manager_detection",
            Self::ToolDiscovery => "tool_discovery",
            Self::ArchiveList => "archive_list",
            Self::ArchiveRead => "archive_read",
            Self::ArchivePack => "archive_pack",
            Self::ArchiveUnpack => "archive_unpack",
            Self::DockerPs => "docker_ps",
            Self::DockerImages => "docker_images",
            Self::DockerLogs => "docker_logs",
            Self::DockerContainerLifecycle => "docker_container_lifecycle",
        }
    }

    pub(crate) fn is_content_excerpt_summary(self) -> bool {
        matches!(
            self,
            Self::ContentExcerptSummary | Self::ContentExcerptWithSummary
        )
    }

    pub(crate) fn is_registry_capability_bridge(self) -> bool {
        matches!(
            self,
            Self::RssNewsFetch
                | Self::WebPageSummary
                | Self::WebSearchSummary
                | Self::WeatherQuery
                | Self::MarketQuote
                | Self::ImageUnderstanding
                | Self::PhotoOrganization
                | Self::PublishingPreview
                | Self::PackageManagerDetection
                | Self::DockerPs
                | Self::DockerImages
                | Self::DockerLogs
                | Self::DockerContainerLifecycle
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SelfExtensionMode {
    #[default]
    None,
    TemporaryFix,
    PermanentExtension,
}

impl SelfExtensionMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::TemporaryFix => "temporary_fix",
            Self::PermanentExtension => "permanent_extension",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SelfExtensionTrigger {
    #[default]
    None,
    ExplicitUserRequest,
    CapabilityGap,
}

impl SelfExtensionTrigger {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ExplicitUserRequest => "explicit_user_request",
            Self::CapabilityGap => "capability_gap",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SelfExtensionContract {
    pub(crate) mode: SelfExtensionMode,
    pub(crate) trigger: SelfExtensionTrigger,
    pub(crate) execute_now: bool,
    pub(crate) scalar_count_filter: OutputScalarCountFilter,
    pub(crate) list_selector: OutputListSelector,
    pub(crate) structured_field_selector: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OutputScalarCountTargetKind {
    #[default]
    Any,
    File,
    Dir,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct OutputScalarCountFilter {
    pub(crate) target_kind: OutputScalarCountTargetKind,
    pub(crate) include_hidden: Option<bool>,
    pub(crate) recursive: Option<bool>,
    pub(crate) extensions: Vec<String>,
}

impl OutputScalarCountFilter {
    pub(crate) fn has_constraints(&self) -> bool {
        self.target_kind != OutputScalarCountTargetKind::Any
            || self.include_hidden.is_some()
            || self.recursive.is_some()
            || !self.extensions.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct OutputListSelector {
    pub(crate) target_kind: OutputScalarCountTargetKind,
    pub(crate) target_kind_specified: bool,
    pub(crate) limit: Option<u64>,
    pub(crate) sort_by: Option<String>,
    pub(crate) include_metadata: Option<bool>,
    pub(crate) include_hidden: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct IntentOutputContract {
    pub(crate) response_shape: OutputResponseShape,
    pub(crate) exact_sentence_count: Option<usize>,
    pub(crate) requires_content_evidence: bool,
    pub(crate) delivery_required: bool,
    pub(crate) locator_kind: OutputLocatorKind,
    pub(crate) delivery_intent: OutputDeliveryIntent,
    pub(crate) semantic_kind: OutputSemanticKind,
    pub(crate) locator_hint: String,
    pub(crate) self_extension: SelfExtensionContract,
}

impl IntentOutputContract {
    pub(crate) fn semantic_kind_is(&self, semantic_kind: OutputSemanticKind) -> bool {
        self.semantic_kind == semantic_kind
    }

    pub(crate) fn semantic_kind_is_any(&self, semantic_kinds: &[OutputSemanticKind]) -> bool {
        semantic_kinds
            .iter()
            .copied()
            .any(|semantic_kind| self.semantic_kind_is(semantic_kind))
    }

    pub(crate) fn semantic_kind_is_unclassified(&self) -> bool {
        self.semantic_kind_is(OutputSemanticKind::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeBehavior {
    None,
    ResumeExecute,
    ResumeDiscuss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScheduleKind {
    None,
    Create,
    Update,
    Delete,
    Query,
}

impl ScheduleKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Query => "query",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RiskCeiling {
    #[default]
    Unknown,
    Low,
    Medium,
    High,
}

impl RiskCeiling {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RouteResult {
    /// Runtime mode for ask-flow dispatch. Legacy route labels for logs and
    /// journals are derived from this field.
    pub(crate) ask_mode: AskMode,
    pub(crate) resolved_intent: String,
    pub(crate) needs_clarify: bool,
    pub(crate) clarify_question: String,
    pub(crate) route_reason: String,
    pub(crate) route_confidence: Option<f64>,
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) visible_skill_candidates: Vec<String>,
    pub(crate) risk_ceiling: RiskCeiling,
    pub(crate) resume_behavior: ResumeBehavior,
    pub(crate) schedule_kind: ScheduleKind,
    pub(crate) schedule_intent: Option<ScheduleIntentOutput>,
    pub(crate) wants_file_delivery: bool,
    pub(crate) should_refresh_long_term_memory: bool,
    pub(crate) agent_display_name_hint: String,
    pub(crate) output_contract: IntentOutputContract,
}

impl RouteResult {
    pub(crate) fn set_ask_mode(&mut self, ask_mode: AskMode) {
        self.ask_mode = ask_mode;
    }

    pub(crate) fn legacy_route_label_for_trace(&self) -> &'static str {
        self.ask_mode.legacy_route_label_for_trace()
    }

    #[cfg(test)]
    pub(crate) fn set_chat_gate(&mut self) {
        self.set_ask_mode(AskMode::direct_answer());
    }

    #[cfg(test)]
    pub(crate) fn set_clarify_gate(&mut self) {
        self.set_ask_mode(AskMode::clarify());
    }

    pub(crate) fn set_execute_gate(&mut self) {
        let finalize = self
            .ask_mode
            .act_finalize_style()
            .unwrap_or(ActFinalizeStyle::Plain);
        self.set_ask_mode(AskMode::Act { finalize });
    }

    pub(crate) fn set_planner_execute_finalize(&mut self, finalize: ActFinalizeStyle) {
        self.set_ask_mode(AskMode::Act { finalize });
    }

    pub(crate) fn route_trace_decision_for_legacy_journal(&self) -> FirstLayerDecision {
        self.ask_mode.route_trace_decision_for_legacy_journal()
    }

    pub(crate) fn gate_kind(&self) -> crate::RouteGateKind {
        self.ask_mode.gate_kind()
    }

    pub(crate) fn is_chat_gate(&self) -> bool {
        matches!(self.gate_kind(), crate::RouteGateKind::Chat)
    }

    pub(crate) fn is_execute_gate(&self) -> bool {
        matches!(self.gate_kind(), crate::RouteGateKind::Execute)
    }

    pub(crate) fn uses_chat_finalizer(&self) -> bool {
        self.ask_mode.finalize_chat_wrapped()
    }

    pub(crate) fn uses_pure_chat_agent_loop_submode(&self) -> bool {
        self.uses_chat_finalizer()
            || self.has_route_reason_machine_marker("pure_chat_agent_loop_submode")
    }

    pub(crate) fn has_route_reason_machine_marker(&self, marker: &str) -> bool {
        self.route_reason.split(';').map(str::trim).any(|part| {
            part == marker
                || part
                    .rsplit_once(':')
                    .is_some_and(|(_, suffix)| suffix.trim() == marker)
        })
    }

    pub(crate) fn output_contract_marker_is(&self, semantic_kind: OutputSemanticKind) -> bool {
        if self.explicit_output_contract_marker_kind().is_some() {
            return self.has_route_reason_machine_marker(semantic_kind.as_str());
        }
        self.output_contract.semantic_kind == semantic_kind
            || self.has_route_reason_machine_marker(semantic_kind.as_str())
    }

    pub(crate) fn output_contract_marker_is_any(
        &self,
        semantic_kinds: &[OutputSemanticKind],
    ) -> bool {
        semantic_kinds
            .iter()
            .copied()
            .any(|semantic_kind| self.output_contract_marker_is(semantic_kind))
    }

    pub(crate) fn has_any_output_contract_marker(&self) -> bool {
        OutputSemanticKind::ALL
            .iter()
            .copied()
            .filter(|semantic_kind| *semantic_kind != OutputSemanticKind::None)
            .any(|semantic_kind| self.has_route_reason_machine_marker(semantic_kind.as_str()))
    }

    pub(crate) fn output_contract_marker_kind(&self) -> Option<OutputSemanticKind> {
        if let Some(semantic_kind) = self.explicit_output_contract_marker_kind() {
            return Some(semantic_kind);
        }
        if self.output_contract.semantic_kind != OutputSemanticKind::None {
            return Some(self.output_contract.semantic_kind);
        }
        OutputSemanticKind::ALL
            .iter()
            .copied()
            .find(|semantic_kind| {
                *semantic_kind != OutputSemanticKind::None
                    && self.has_route_reason_machine_marker(semantic_kind.as_str())
            })
    }

    fn explicit_output_contract_marker_kind(&self) -> Option<OutputSemanticKind> {
        self.route_reason
            .split(';')
            .map(str::trim)
            .rev()
            .find_map(|part| {
                let marker = part
                    .strip_prefix("output_contract_kind=")
                    .or_else(|| part.strip_prefix("contract:"))?
                    .trim();
                OutputSemanticKind::ALL
                    .iter()
                    .copied()
                    .find(|semantic_kind| {
                        *semantic_kind != OutputSemanticKind::None
                            && semantic_kind.as_str() == marker
                    })
            })
    }

    pub(crate) fn effective_output_contract_semantic_kind(&self) -> OutputSemanticKind {
        self.output_contract_marker_kind()
            .unwrap_or(OutputSemanticKind::None)
    }

    pub(crate) fn effective_output_contract(&self) -> IntentOutputContract {
        let mut contract = self.output_contract.clone();
        contract.semantic_kind = self.effective_output_contract_semantic_kind();
        contract
    }

    pub(crate) fn output_contract_is_unclassified(&self) -> bool {
        self.output_contract.semantic_kind == OutputSemanticKind::None
            && !self.has_any_output_contract_marker()
    }

    #[cfg(test)]
    pub(crate) fn is_clarify_gate(&self) -> bool {
        matches!(self.gate_kind(), crate::RouteGateKind::Clarify)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanKind {
    Single,
    Incremental,
    Repair,
}

impl PlanKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Single => "Single",
            Self::Incremental => "Incremental",
            Self::Repair => "Repair",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PlanStep {
    pub(crate) step_id: String,
    pub(crate) action_type: String,
    pub(crate) skill: String,
    pub(crate) args: Value,
    pub(crate) depends_on: Vec<String>,
    /// Planner rationale kept for journal/debug context; execution consumes machine fields.
    pub(crate) why: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PlanResult {
    pub(crate) goal: String,
    pub(crate) missing_slots: Vec<String>,
    pub(crate) needs_confirmation: bool,
    pub(crate) steps: Vec<PlanStep>,
    pub(crate) planner_notes: String,
    pub(crate) plan_kind: PlanKind,
    pub(crate) raw_plan_text: String,
}

impl PlanStep {
    pub(crate) fn to_agent_action(&self) -> Option<AgentAction> {
        match self.action_type.as_str() {
            "call_skill" => Some(AgentAction::CallSkill {
                skill: self.skill.clone(),
                args: self.args.clone(),
            }),
            "call_tool" => Some(AgentAction::CallTool {
                tool: self.skill.clone(),
                args: self.args.clone(),
            }),
            "call_capability" => Some(AgentAction::CallCapability {
                capability: self.skill.clone(),
                args: self.args.clone(),
            }),
            "respond" => Some(AgentAction::Respond {
                content: self
                    .args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            }),
            "synthesize_answer" => Some(AgentAction::SynthesizeAnswer {
                evidence_refs: self
                    .args
                    .get("evidence_refs")
                    .and_then(|v| v.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            "think" => Some(AgentAction::Think {
                content: self
                    .args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            }),
            _ => None,
        }
    }
}

impl PlanResult {
    pub(crate) fn step_labels(&self) -> Vec<String> {
        self.steps
            .iter()
            .map(|step| match step.action_type.as_str() {
                "respond" => "respond".to_string(),
                "synthesize_answer" => "synthesize_answer".to_string(),
                "think" => "think".to_string(),
                "call_capability" => format!("capability({})", step.skill),
                "call_tool" => format!("tool({})", step.skill),
                _ => format!("skill({})", step.skill),
            })
            .collect()
    }
}

pub(crate) fn plan_step_from_agent_action(
    action: &AgentAction,
    step_id: String,
    depends_on: Vec<String>,
    why: String,
) -> PlanStep {
    match action {
        AgentAction::CallSkill { skill, args } => PlanStep {
            step_id,
            action_type: "call_skill".to_string(),
            skill: skill.clone(),
            args: args.clone(),
            depends_on,
            why,
        },
        AgentAction::CallTool { tool, args } => PlanStep {
            step_id,
            action_type: "call_tool".to_string(),
            skill: tool.clone(),
            args: args.clone(),
            depends_on,
            why,
        },
        AgentAction::CallCapability { capability, args } => PlanStep {
            step_id,
            action_type: "call_capability".to_string(),
            skill: capability.clone(),
            args: args.clone(),
            depends_on,
            why,
        },
        AgentAction::Respond { content } => PlanStep {
            step_id,
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({ "content": content }),
            depends_on,
            why,
        },
        AgentAction::SynthesizeAnswer { evidence_refs } => PlanStep {
            step_id,
            action_type: "synthesize_answer".to_string(),
            skill: "synthesize_answer".to_string(),
            args: json!({ "evidence_refs": evidence_refs }),
            depends_on,
            why,
        },
        AgentAction::Think { content } => PlanStep {
            step_id,
            action_type: "think".to_string(),
            skill: "think".to_string(),
            args: json!({ "content": content }),
            depends_on,
            why,
        },
    }
}

#[cfg(test)]
#[path = "pipeline_types_tests.rs"]
mod tests;

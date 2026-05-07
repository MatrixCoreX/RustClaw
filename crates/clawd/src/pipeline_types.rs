use serde_json::{json, Value};

use crate::runtime::ask_mode::AskMode;
use crate::runtime::types::{AgentAction, RoutedMode, ScheduleIntentOutput};

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
    ServiceStatus,
    HiddenEntriesCheck,
    FileNames,
    DirectoryNames,
    FilePaths,
    DirectoryPurposeSummary,
    ContentExcerptSummary,
    ExcerptKindJudgment,
    RecentArtifactsJudgment,
    WorkspaceProjectSummary,
    ScalarCount,
    QuantityComparison,
    ExecutionFailedStep,
    GeneratedFileDelivery,
    ScalarPathOnly,
    ExistenceWithPath,
    ExistenceWithPathSummary,
    RecentScalarEqualityCheck,
    GitCommitSubject,
    StructuredKeys,
    SqliteTableListing,
    SqliteTableNamesOnly,
    SqliteDatabaseKindJudgment,
    SqliteSchemaVersion,
}

impl OutputSemanticKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RawCommandOutput => "raw_command_output",
            Self::ServiceStatus => "service_status",
            Self::HiddenEntriesCheck => "hidden_entries_check",
            Self::FileNames => "file_names",
            Self::DirectoryNames => "directory_names",
            Self::FilePaths => "file_paths",
            Self::DirectoryPurposeSummary => "directory_purpose_summary",
            Self::ContentExcerptSummary => "content_excerpt_summary",
            Self::ExcerptKindJudgment => "excerpt_kind_judgment",
            Self::RecentArtifactsJudgment => "recent_artifacts_judgment",
            Self::WorkspaceProjectSummary => "workspace_project_summary",
            Self::ScalarCount => "scalar_count",
            Self::QuantityComparison => "quantity_comparison",
            Self::ExecutionFailedStep => "execution_failed_step",
            Self::GeneratedFileDelivery => "generated_file_delivery",
            Self::ScalarPathOnly => "scalar_path_only",
            Self::ExistenceWithPath => "existence_with_path",
            Self::ExistenceWithPathSummary => "existence_with_path_summary",
            Self::RecentScalarEqualityCheck => "recent_scalar_equality_check",
            Self::GitCommitSubject => "git_commit_subject",
            Self::StructuredKeys => "structured_keys",
            Self::SqliteTableListing => "sqlite_table_listing",
            Self::SqliteTableNamesOnly => "sqlite_table_names_only",
            Self::SqliteDatabaseKindJudgment => "sqlite_database_kind_judgment",
            Self::SqliteSchemaVersion => "sqlite_schema_version",
        }
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

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct RouteResult {
    pub(crate) routed_mode: RoutedMode,
    /// Phase 3.2 Stage B：与 `routed_mode` 双轨并存。
    /// 此字段只反映 `RoutedMode` 维度的折叠，不携带 worker 层 resume flag；
    /// resume 信息在 `worker::ask_prepare::PreparedAskRouting::ask_mode` 里合并。
    pub(crate) ask_mode: AskMode,
    pub(crate) resolved_intent: String,
    pub(crate) needs_clarify: bool,
    pub(crate) clarify_question: String,
    pub(crate) route_reason: String,
    pub(crate) route_confidence: Option<f64>,
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
    pub(crate) fn set_routed_mode(&mut self, mode: RoutedMode) {
        self.routed_mode = mode;
        self.ask_mode = AskMode::from_routed_mode(mode);
    }

    pub(crate) fn gate_kind(&self) -> crate::RouteGateKind {
        let ask_gate = self.ask_mode.gate_kind();
        let routed_gate = match self.routed_mode {
            RoutedMode::Chat => crate::RouteGateKind::Chat,
            RoutedMode::AskClarify => crate::RouteGateKind::Clarify,
            RoutedMode::Act | RoutedMode::ChatAct => crate::RouteGateKind::Execute,
        };
        if ask_gate == routed_gate {
            ask_gate
        } else {
            routed_gate
        }
    }

    pub(crate) fn is_chat_gate(&self) -> bool {
        matches!(self.gate_kind(), crate::RouteGateKind::Chat)
    }

    pub(crate) fn is_execute_gate(&self) -> bool {
        matches!(self.gate_kind(), crate::RouteGateKind::Execute)
    }

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

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct PlanStep {
    pub(crate) step_id: String,
    pub(crate) action_type: String,
    pub(crate) skill: String,
    pub(crate) args: Value,
    pub(crate) depends_on: Vec<String>,
    pub(crate) why: String,
}

#[allow(dead_code)]
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
mod tests {
    use super::{plan_step_from_agent_action, AgentAction, PlanStep};
    use serde_json::json;

    #[test]
    fn plan_step_to_agent_action_parses_synthesize_answer() {
        let step = PlanStep {
            step_id: "step_1".to_string(),
            action_type: "synthesize_answer".to_string(),
            skill: "synthesize_answer".to_string(),
            args: json!({ "evidence_refs": ["last_output", "step_1"] }),
            depends_on: vec![],
            why: "synthesize".to_string(),
        };

        assert!(matches!(
            step.to_agent_action(),
            Some(AgentAction::SynthesizeAnswer { evidence_refs })
                if evidence_refs == vec!["last_output".to_string(), "step_1".to_string()]
        ));
    }

    #[test]
    fn plan_step_from_agent_action_serializes_synthesize_answer() {
        let action = AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        };
        let step = plan_step_from_agent_action(
            &action,
            "step_2".to_string(),
            vec!["step_1".to_string()],
            "why".to_string(),
        );

        assert_eq!(step.action_type, "synthesize_answer");
        assert_eq!(step.skill, "synthesize_answer");
        assert_eq!(step.args, json!({ "evidence_refs": ["last_output"] }));
        assert_eq!(step.depends_on, vec!["step_1".to_string()]);
    }
}

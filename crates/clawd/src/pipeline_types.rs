use serde_json::{json, Value};

use crate::runtime::types::{AgentAction, RoutedMode, ScheduleIntentOutput};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OutputResponseShape {
    #[default]
    Free,
    OneSentence,
    Scalar,
    FileToken,
}

impl OutputResponseShape {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::OneSentence => "one_sentence",
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
    DirectoryPurposeSummary,
    ContentExcerptSummary,
    RecentArtifactsJudgment,
    WorkspaceProjectSummary,
    ScalarCount,
    QuantityComparison,
    ScalarPathOnly,
    ExistenceWithPath,
    RecentScalarEqualityCheck,
}

impl OutputSemanticKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RawCommandOutput => "raw_command_output",
            Self::ServiceStatus => "service_status",
            Self::HiddenEntriesCheck => "hidden_entries_check",
            Self::DirectoryPurposeSummary => "directory_purpose_summary",
            Self::ContentExcerptSummary => "content_excerpt_summary",
            Self::RecentArtifactsJudgment => "recent_artifacts_judgment",
            Self::WorkspaceProjectSummary => "workspace_project_summary",
            Self::ScalarCount => "scalar_count",
            Self::QuantityComparison => "quantity_comparison",
            Self::ScalarPathOnly => "scalar_path_only",
            Self::ExistenceWithPath => "existence_with_path",
            Self::RecentScalarEqualityCheck => "recent_scalar_equality_check",
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
}

impl RiskCeiling {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct RouteResult {
    pub(crate) routed_mode: RoutedMode,
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
    pub(crate) fn to_agent_actions(&self) -> Vec<AgentAction> {
        self.steps
            .iter()
            .filter_map(PlanStep::to_agent_action)
            .collect()
    }

    pub(crate) fn step_labels(&self) -> Vec<String> {
        self.steps
            .iter()
            .map(|step| match step.action_type.as_str() {
                "respond" => "respond".to_string(),
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

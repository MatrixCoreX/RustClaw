use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::runtime::types::AgentAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutputSemanticKind {
    #[default]
    None,
    RawCommandOutput,
    CommandOutputSummary,
    FileNames,
    DirectoryNames,
    DirectoryEntryGroups,
    FilePaths,
    ContentExcerptSummary,
    ContentExcerptWithSummary,
    ScalarCount,
    ExecutionFailedStep,
    GeneratedFileDelivery,
    GeneratedFilePathReport,
    FilesystemMutationResult,
    ExistenceWithPath,
}

impl OutputSemanticKind {
    pub(crate) const ALL: &'static [Self] = &[
        Self::None,
        Self::RawCommandOutput,
        Self::CommandOutputSummary,
        Self::FileNames,
        Self::DirectoryNames,
        Self::DirectoryEntryGroups,
        Self::FilePaths,
        Self::ContentExcerptSummary,
        Self::ContentExcerptWithSummary,
        Self::ScalarCount,
        Self::ExecutionFailedStep,
        Self::GeneratedFileDelivery,
        Self::GeneratedFilePathReport,
        Self::FilesystemMutationResult,
        Self::ExistenceWithPath,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RawCommandOutput => "raw_command_output",
            Self::CommandOutputSummary => "command_output_summary",
            Self::FileNames => "file_names",
            Self::DirectoryNames => "directory_names",
            Self::DirectoryEntryGroups => "directory_entry_groups",
            Self::FilePaths => "file_paths",
            Self::ContentExcerptSummary => "content_excerpt_summary",
            Self::ContentExcerptWithSummary => "content_excerpt_with_summary",
            Self::ScalarCount => "scalar_count",
            Self::ExecutionFailedStep => "execution_failed_step",
            Self::GeneratedFileDelivery => "generated_file_delivery",
            Self::GeneratedFilePathReport => "generated_file_path_report",
            Self::FilesystemMutationResult => "filesystem_mutation_result",
            Self::ExistenceWithPath => "existence_with_path",
        }
    }

    pub(crate) fn is_content_excerpt_summary(self) -> bool {
        matches!(
            self,
            Self::ContentExcerptSummary | Self::ContentExcerptWithSummary
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct OutputSelectionContract {
    pub(crate) list_selector: OutputListSelector,
    pub(crate) structured_field_selector: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutputScalarCountTargetKind {
    #[default]
    Any,
    File,
    Dir,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct OutputListSelector {
    pub(crate) target_kind: OutputScalarCountTargetKind,
    pub(crate) target_kind_specified: bool,
    pub(crate) limit: Option<u64>,
    pub(crate) sort_by: Option<String>,
    pub(crate) include_metadata: Option<bool>,
    pub(crate) include_hidden: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct IntentOutputContract {
    pub(crate) response_shape: OutputResponseShape,
    pub(crate) exact_sentence_count: Option<usize>,
    pub(crate) requires_content_evidence: bool,
    pub(crate) delivery_required: bool,
    pub(crate) locator_kind: OutputLocatorKind,
    pub(crate) delivery_intent: OutputDeliveryIntent,
    pub(crate) semantic_kind: OutputSemanticKind,
    pub(crate) locator_hint: String,
    pub(crate) selection: OutputSelectionContract,
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
pub(crate) struct MachineTokenMarkers<'a> {
    machine_text: &'a str,
}

impl<'a> MachineTokenMarkers<'a> {
    pub(crate) fn new(machine_text: &'a str) -> Self {
        Self { machine_text }
    }

    pub(crate) fn has_machine_marker(self, marker: &str) -> bool {
        self.tokens().any(|part| {
            part == marker
                || part
                    .rsplit_once(':')
                    .is_some_and(|(_, suffix)| suffix.trim() == marker)
        })
    }

    pub(crate) fn machine_value(self, key: &str) -> Option<&'a str> {
        let key = key.trim();
        if key.is_empty() {
            return None;
        }
        self.tokens()
            .filter_map(|part| {
                let value = part
                    .strip_prefix(key)?
                    .trim_start()
                    .strip_prefix(':')
                    .or_else(|| part.strip_prefix(key)?.trim_start().strip_prefix('='))?;
                let value = value.trim().trim_matches(|ch: char| {
                    matches!(ch, '"' | '\'' | '`' | ',' | ';' | ':' | ')' | '(')
                });
                (!value.is_empty()).then_some(value)
            })
            .last()
    }

    fn tokens(self) -> impl Iterator<Item = &'a str> {
        self.machine_text
            .split(|ch: char| {
                ch.is_whitespace() || matches!(ch, ';' | ',' | '|' | '[' | ']' | '(' | ')')
            })
            .map(str::trim)
            .filter(|part| !part.is_empty())
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
    pub(crate) output_contract: Option<IntentOutputContract>,
    pub(crate) steps: Vec<PlanStep>,
    pub(crate) planner_notes: String,
    pub(crate) plan_kind: PlanKind,
    pub(crate) raw_plan_text: String,
}

impl PlanStep {
    pub(crate) fn is_skill_invocation(&self) -> bool {
        self.action_type == "call_skill"
    }

    pub(crate) fn to_agent_action(&self) -> Option<AgentAction> {
        match self.action_type.as_str() {
            "call_skill" => terminal_agent_action_from_wrapped_ref(&self.skill, &self.args)
                .or_else(|| {
                    Some(AgentAction::CallSkill {
                        skill: self.skill.clone(),
                        args: self.args.clone(),
                    })
                }),
            "call_tool" => {
                terminal_agent_action_from_wrapped_ref(&self.skill, &self.args).or_else(|| {
                    Some(AgentAction::CallTool {
                        tool: self.skill.clone(),
                        args: self.args.clone(),
                    })
                })
            }
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

fn terminal_agent_action_from_wrapped_ref(raw_ref: &str, args: &Value) -> Option<AgentAction> {
    match raw_ref.trim().to_ascii_lowercase().as_str() {
        "synthesize_answer" => Some(AgentAction::SynthesizeAnswer {
            evidence_refs: evidence_refs_from_value(args),
        }),
        "respond" => Some(AgentAction::Respond {
            content: terminal_content_from_value(args).unwrap_or_default(),
        }),
        _ => None,
    }
}

fn evidence_refs_from_value(value: &Value) -> Vec<String> {
    let value = value.get("evidence_refs").unwrap_or(value);
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        Value::String(item) => {
            let item = item.trim();
            if item.is_empty() {
                Vec::new()
            } else {
                vec![item.to_string()]
            }
        }
        _ => Vec::new(),
    }
}

fn terminal_content_from_value(value: &Value) -> Option<String> {
    ["content", "text", "message", "body"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .map(str::to_string)
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

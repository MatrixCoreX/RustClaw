use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

use crate::{AppState, AskReply, ClaimedTask, RoutedMode};

const DIRECT_ANSWER_GATE_PROMPT_LOGICAL_PATH: &str = "prompts/direct_answer_gate_prompt.md";

#[derive(Debug, Clone, Deserialize)]
struct DirectAnswerGateOut {
    #[serde(default)]
    decision: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    clarify_question: String,
    #[serde(default)]
    resolved_user_intent: String,
    output_contract: DirectAnswerGateContractOut,
}

#[derive(Debug, Clone, Deserialize)]
struct DirectAnswerGateContractOut {
    #[serde(default)]
    response_shape: String,
    #[serde(default)]
    exact_sentence_count: Option<usize>,
    #[serde(default)]
    requires_content_evidence: bool,
    #[serde(default)]
    delivery_required: bool,
    #[serde(default)]
    locator_kind: String,
    #[serde(default)]
    delivery_intent: String,
    #[serde(default)]
    semantic_kind: String,
    #[serde(default)]
    locator_hint: String,
    #[serde(default)]
    self_extension: DirectAnswerGateSelfExtensionOut,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DirectAnswerGateSelfExtensionOut {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    trigger: String,
    #[serde(default)]
    execute_now: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectAnswerGateDecision {
    DirectAnswer,
    PlannerExecute,
    Clarify,
}

enum DirectAnswerPreflight {
    DirectAnswer,
    PlannerExecute(crate::agent_engine::AgentRunContext),
    Clarify(String),
}

fn build_resume_continue_execute_prompt_from_parts(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resume_context: &Value,
    resume_instruction: &str,
    resume_steps: Option<&Value>,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    let resume_steps = resume_steps
        .cloned()
        .filter(|v| v.as_array().map(|arr| !arr.is_empty()).unwrap_or(false))
        .unwrap_or_else(|| {
            resume_context
                .get("remaining_actions")
                .cloned()
                .filter(|v| v.as_array().map(|arr| !arr.is_empty()).unwrap_or(false))
                .unwrap_or_else(|| {
                    resume_context
                        .get("remaining_steps")
                        .cloned()
                        .unwrap_or_else(|| json!([]))
                })
        });
    let resume_context_json =
        serde_json::to_string_pretty(resume_context).unwrap_or_else(|_| resume_context.to_string());
    let resume_steps_json =
        serde_json::to_string_pretty(&resume_steps).unwrap_or_else(|_| resume_steps.to_string());

    let (prompt_template, _) = crate::bootstrap::load_required_prompt_template_for_state(
        state,
        "prompts/resume_continue_execute_prompt.md",
    )?;
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    Ok(crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_TEXT__", user_text),
            ("__RESUME_CONTEXT__", &resume_context_json),
            ("__RESUME_STEPS__", &resume_steps_json),
            ("__RESUME_INSTRUCTION__", resume_instruction),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
        ],
    ))
}

fn normalizer_answer_candidate_from_resolved_prompt(resolved_prompt: &str) -> Option<String> {
    let (_intent, candidate) = resolved_prompt.rsplit_once("\nanswer_candidate:")?;
    let candidate = candidate.trim();
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

fn paths_refer_to_same_existing_location(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn normalizer_answer_candidate_matches_runtime_fact(state: &AppState, candidate: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() || candidate.contains('\n') {
        return false;
    }
    let candidate_path = Path::new(candidate);
    if !candidate_path.is_absolute() {
        return false;
    }
    if paths_refer_to_same_existing_location(candidate_path, &state.skill_rt.workspace_root) {
        return true;
    }
    std::env::current_dir()
        .ok()
        .is_some_and(|cwd| paths_refer_to_same_existing_location(candidate_path, &cwd))
}

fn normalizer_chat_direct_answer_candidate(
    state: &AppState,
    resolved_prompt: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    allow_budget_fallback: bool,
) -> Option<String> {
    let route = agent_run_context?.route_result.as_ref()?;
    if route.needs_clarify || route.is_execute_gate() {
        return None;
    }
    let contract = &route.output_contract;
    if contract.requires_content_evidence
        || contract.delivery_required
        || !matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
    {
        return None;
    }
    let candidate = normalizer_answer_candidate_from_resolved_prompt(resolved_prompt)?;
    if normalizer_answer_candidate_matches_runtime_fact(state, &candidate) {
        return Some(candidate);
    }
    allow_budget_fallback.then_some(candidate)
}

fn runtime_scalar_path_direct_answer_candidate(
    state: &AppState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context?.route_result.as_ref()?;
    if route.needs_clarify || !route.is_execute_gate() {
        return None;
    }
    let contract = &route.output_contract;
    if !matches!(contract.response_shape, crate::OutputResponseShape::Scalar)
        || !matches!(
            contract.semantic_kind,
            crate::OutputSemanticKind::ScalarPathOnly
        )
        || !matches!(
            contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
        )
        || contract.delivery_required
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
    {
        return None;
    }
    let candidate = contract.locator_hint.trim();
    normalizer_answer_candidate_matches_runtime_fact(state, candidate)
        .then(|| candidate.to_string())
}

fn parse_direct_answer_gate_decision(raw: &str) -> DirectAnswerGateDecision {
    match raw.trim().to_ascii_lowercase().as_str() {
        "planner_execute" | "execute" | "planner" => DirectAnswerGateDecision::PlannerExecute,
        "clarify" | "ask_clarify" => DirectAnswerGateDecision::Clarify,
        _ => DirectAnswerGateDecision::DirectAnswer,
    }
}

fn parse_gate_response_shape(raw: &str) -> crate::OutputResponseShape {
    match raw.trim().to_ascii_lowercase().as_str() {
        "one_sentence" => crate::OutputResponseShape::OneSentence,
        "strict" => crate::OutputResponseShape::Strict,
        "scalar" => crate::OutputResponseShape::Scalar,
        "file_token" => crate::OutputResponseShape::FileToken,
        _ => crate::OutputResponseShape::Free,
    }
}

fn parse_gate_locator_kind(raw: &str) -> crate::OutputLocatorKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "path" => crate::OutputLocatorKind::Path,
        "current_workspace" => crate::OutputLocatorKind::CurrentWorkspace,
        "url" => crate::OutputLocatorKind::Url,
        "filename" => crate::OutputLocatorKind::Filename,
        _ => crate::OutputLocatorKind::None,
    }
}

fn parse_gate_delivery_intent(raw: &str) -> crate::OutputDeliveryIntent {
    match raw.trim().to_ascii_lowercase().as_str() {
        "file_single" => crate::OutputDeliveryIntent::FileSingle,
        "directory_lookup" => crate::OutputDeliveryIntent::DirectoryLookup,
        "directory_batch_files" => crate::OutputDeliveryIntent::DirectoryBatchFiles,
        _ => crate::OutputDeliveryIntent::None,
    }
}

fn parse_gate_semantic_kind(raw: &str) -> crate::OutputSemanticKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "raw_command_output" => crate::OutputSemanticKind::RawCommandOutput,
        "service_status" => crate::OutputSemanticKind::ServiceStatus,
        "hidden_entries_check" => crate::OutputSemanticKind::HiddenEntriesCheck,
        "file_names" => crate::OutputSemanticKind::FileNames,
        "directory_names" => crate::OutputSemanticKind::DirectoryNames,
        "file_paths" => crate::OutputSemanticKind::FilePaths,
        "directory_purpose_summary" => crate::OutputSemanticKind::DirectoryPurposeSummary,
        "content_excerpt_summary" => crate::OutputSemanticKind::ContentExcerptSummary,
        "excerpt_kind_judgment" => crate::OutputSemanticKind::ExcerptKindJudgment,
        "recent_artifacts_judgment" => crate::OutputSemanticKind::RecentArtifactsJudgment,
        "workspace_project_summary" => crate::OutputSemanticKind::WorkspaceProjectSummary,
        "scalar_count" => crate::OutputSemanticKind::ScalarCount,
        "quantity_comparison" => crate::OutputSemanticKind::QuantityComparison,
        "execution_failed_step" => crate::OutputSemanticKind::ExecutionFailedStep,
        "generated_file_delivery" => crate::OutputSemanticKind::GeneratedFileDelivery,
        "scalar_path_only" => crate::OutputSemanticKind::ScalarPathOnly,
        "existence_with_path" => crate::OutputSemanticKind::ExistenceWithPath,
        "existence_with_path_summary" => crate::OutputSemanticKind::ExistenceWithPathSummary,
        "recent_scalar_equality_check" => crate::OutputSemanticKind::RecentScalarEqualityCheck,
        "git_commit_subject" => crate::OutputSemanticKind::GitCommitSubject,
        "structured_keys" => crate::OutputSemanticKind::StructuredKeys,
        "sqlite_table_listing" => crate::OutputSemanticKind::SqliteTableListing,
        "sqlite_table_names_only" => crate::OutputSemanticKind::SqliteTableNamesOnly,
        "sqlite_database_kind_judgment" => crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
        "sqlite_schema_version" => crate::OutputSemanticKind::SqliteSchemaVersion,
        "archive_list" => crate::OutputSemanticKind::ArchiveList,
        "archive_pack" => crate::OutputSemanticKind::ArchivePack,
        "archive_unpack" => crate::OutputSemanticKind::ArchiveUnpack,
        "docker_ps" => crate::OutputSemanticKind::DockerPs,
        "docker_images" => crate::OutputSemanticKind::DockerImages,
        "docker_logs" => crate::OutputSemanticKind::DockerLogs,
        "docker_container_lifecycle" => crate::OutputSemanticKind::DockerContainerLifecycle,
        _ => crate::OutputSemanticKind::None,
    }
}

fn parse_gate_self_extension(
    raw: DirectAnswerGateSelfExtensionOut,
) -> crate::SelfExtensionContract {
    let mode = match raw.mode.trim().to_ascii_lowercase().as_str() {
        "temporary_fix" => crate::SelfExtensionMode::TemporaryFix,
        "permanent_extension" => crate::SelfExtensionMode::PermanentExtension,
        _ => crate::SelfExtensionMode::None,
    };
    let trigger = match raw.trigger.trim().to_ascii_lowercase().as_str() {
        "explicit_user_request" => crate::SelfExtensionTrigger::ExplicitUserRequest,
        "capability_gap" => crate::SelfExtensionTrigger::CapabilityGap,
        _ => crate::SelfExtensionTrigger::None,
    };
    crate::SelfExtensionContract {
        mode,
        trigger,
        execute_now: raw.execute_now,
    }
}

fn output_contract_from_direct_answer_gate(
    raw: DirectAnswerGateContractOut,
    fallback: &crate::IntentOutputContract,
) -> crate::IntentOutputContract {
    crate::IntentOutputContract {
        response_shape: parse_gate_response_shape(&raw.response_shape),
        exact_sentence_count: raw.exact_sentence_count,
        requires_content_evidence: raw.requires_content_evidence,
        delivery_required: raw.delivery_required,
        locator_kind: parse_gate_locator_kind(&raw.locator_kind),
        delivery_intent: parse_gate_delivery_intent(&raw.delivery_intent),
        semantic_kind: parse_gate_semantic_kind(&raw.semantic_kind),
        locator_hint: raw.locator_hint.trim().to_string(),
        self_extension: parse_gate_self_extension(raw.self_extension),
    }
    .with_fallback_shape(fallback)
}

fn ordered_entry_looks_like_workspace_artifact(entry: &str) -> bool {
    let trimmed = entry.trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return false;
    }
    let path = Path::new(trimmed);
    path.extension().is_some()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.starts_with('.')
}

fn direct_answer_candidate_looks_like_artifact_listing(resolved_prompt: &str) -> bool {
    let Some(candidate) = normalizer_answer_candidate_from_resolved_prompt(resolved_prompt) else {
        return false;
    };
    let entries = crate::followup_frame::extract_ordered_entries_from_text(&candidate);
    entries.len() >= 2
        && entries
            .iter()
            .all(|entry| ordered_entry_looks_like_workspace_artifact(entry))
}

fn promote_artifact_listing_candidate_contract(
    resolved_prompt: &str,
    contract: &mut crate::IntentOutputContract,
) -> bool {
    if output_contract_requires_planner_execution(contract)
        || !direct_answer_candidate_looks_like_artifact_listing(resolved_prompt)
    {
        return false;
    }
    contract.requires_content_evidence = true;
    contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    if matches!(
        contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        contract.response_shape = crate::OutputResponseShape::Strict;
    }
    true
}

fn output_contract_requires_planner_execution(contract: &crate::IntentOutputContract) -> bool {
    contract.requires_content_evidence
        || contract.delivery_required
        || !matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
        || !matches!(contract.semantic_kind, crate::OutputSemanticKind::None)
}

fn planner_mode_for_output_contract(contract: &crate::IntentOutputContract) -> RoutedMode {
    if matches!(
        contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    ) {
        RoutedMode::Act
    } else {
        RoutedMode::ChatAct
    }
}

fn promote_direct_answer_gate_to_planner(
    ctx: &mut crate::agent_engine::AgentRunContext,
    gate: &DirectAnswerGateOut,
    mut contract: crate::IntentOutputContract,
    reason_tag: &str,
) -> DirectAnswerPreflight {
    let Some(route) = ctx.route_result.as_mut() else {
        return DirectAnswerPreflight::DirectAnswer;
    };
    contract.requires_content_evidence = true;
    let mode = planner_mode_for_output_contract(&contract);
    route.output_contract = contract;
    route.set_routed_mode(mode);
    route.needs_clarify = false;
    route.clarify_question.clear();
    if !gate.resolved_user_intent.trim().is_empty() {
        route.resolved_intent = gate.resolved_user_intent.trim().to_string();
    }
    append_route_reason(route, &format!("{reason_tag}:{}", gate.reason.trim()));
    DirectAnswerPreflight::PlannerExecute(ctx.clone())
}

trait OutputContractFallbackShape {
    fn with_fallback_shape(self, fallback: &crate::IntentOutputContract) -> Self;
}

impl OutputContractFallbackShape for crate::IntentOutputContract {
    fn with_fallback_shape(mut self, fallback: &crate::IntentOutputContract) -> Self {
        if matches!(self.response_shape, crate::OutputResponseShape::Free)
            && !matches!(fallback.response_shape, crate::OutputResponseShape::Free)
        {
            self.response_shape = fallback.response_shape;
            self.exact_sentence_count = fallback.exact_sentence_count;
        }
        if self.locator_hint.is_empty()
            && matches!(
                self.locator_kind,
                crate::OutputLocatorKind::Path
                    | crate::OutputLocatorKind::Filename
                    | crate::OutputLocatorKind::Url
            )
        {
            self.locator_hint = fallback.locator_hint.clone();
        }
        self
    }
}

fn append_route_reason(route: &mut crate::RouteResult, addition: &str) {
    let addition = addition.trim();
    if addition.is_empty() || route.route_reason.contains(addition) {
        return;
    }
    if route.route_reason.trim().is_empty() {
        route.route_reason = addition.to_string();
    } else {
        route.route_reason.push_str("; ");
        route.route_reason.push_str(addition);
    }
}

fn apply_direct_answer_gate_outcome(
    ctx: &mut crate::agent_engine::AgentRunContext,
    gate: DirectAnswerGateOut,
) -> DirectAnswerPreflight {
    let decision = parse_direct_answer_gate_decision(&gate.decision);
    if gate.confidence < 0.60 {
        return DirectAnswerPreflight::DirectAnswer;
    }
    let Some(route) = ctx.route_result.as_mut() else {
        return DirectAnswerPreflight::DirectAnswer;
    };
    match decision {
        DirectAnswerGateDecision::DirectAnswer => {
            let fallback_contract = route.output_contract.clone();
            let resolved_prompt = route.resolved_intent.clone();
            let mut contract = output_contract_from_direct_answer_gate(
                gate.output_contract.clone(),
                &fallback_contract,
            );
            let promoted_artifact_listing =
                promote_artifact_listing_candidate_contract(&resolved_prompt, &mut contract);
            if output_contract_requires_planner_execution(&contract) {
                let reason_tag = if promoted_artifact_listing {
                    "direct_answer_gate_artifact_listing_execute"
                } else {
                    "direct_answer_gate_contract_execute"
                };
                promote_direct_answer_gate_to_planner(ctx, &gate, contract, reason_tag)
            } else {
                DirectAnswerPreflight::DirectAnswer
            }
        }
        DirectAnswerGateDecision::Clarify => {
            let question = gate.clarify_question.trim();
            if question.is_empty() {
                DirectAnswerPreflight::DirectAnswer
            } else {
                route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
                route.needs_clarify = true;
                route.clarify_question = question.to_string();
                append_route_reason(
                    route,
                    &format!("direct_answer_gate_clarify:{}", gate.reason.trim()),
                );
                DirectAnswerPreflight::Clarify(question.to_string())
            }
        }
        DirectAnswerGateDecision::PlannerExecute => {
            let fallback_contract = route.output_contract.clone();
            let contract = output_contract_from_direct_answer_gate(
                gate.output_contract.clone(),
                &fallback_contract,
            );
            promote_direct_answer_gate_to_planner(
                ctx,
                &gate,
                contract,
                "direct_answer_gate_execute",
            )
        }
    }
}

fn direct_answer_gate_route_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> String {
    chat_route_resolution_context(agent_run_context).unwrap_or_else(|| "<none>".to_string())
}

fn direct_answer_gate_runtime_context(state: &AppState) -> String {
    let current_process_cwd = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    format!(
        "workspace_root: {}\ncurrent_process_cwd: {}\nruntime_has_tools: true",
        state.skill_rt.workspace_root.display(),
        current_process_cwd
    )
}

async fn run_direct_answer_gate(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<DirectAnswerGateOut> {
    let resolved = match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        DIRECT_ANSWER_GATE_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            tracing::info!(
                "{} direct_answer_gate prompt_missing task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            return None;
        }
    };
    let route_context = direct_answer_gate_route_context(agent_run_context);
    let runtime_context = direct_answer_gate_runtime_context(state);
    let prompt = crate::render_prompt_template(
        &resolved.template,
        &[
            ("__REQUEST__", user_request.trim()),
            ("__ROUTE_CONTEXT__", &route_context),
            ("__RUNTIME_CONTEXT__", &runtime_context),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "direct_answer_gate_prompt",
        &resolved.source,
        resolved.version.as_deref(),
        None,
    );
    let prompt_source = resolved.source;
    let llm_out = match crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        &prompt_source,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::info!(
                "{} direct_answer_gate llm_failed task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            return None;
        }
    };
    match crate::prompt_utils::validate_against_schema::<DirectAnswerGateOut>(
        &llm_out,
        crate::prompt_utils::PromptSchemaId::DirectAnswerGate,
    ) {
        Ok(validated) => Some(validated.value),
        Err(err) => {
            tracing::info!(
                "{} direct_answer_gate schema_validation_failed task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            None
        }
    }
}

fn ask_reply_with_chat_process(text: String, _language_hint: &str) -> AskReply {
    let answer = text.trim().to_string();
    if answer.is_empty() || crate::finalize::is_execution_summary_message(&answer) {
        AskReply::llm(text)
    } else {
        AskReply::llm(answer)
    }
}

fn state_patch_alias_bindings_ack(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    language_hint: &str,
) -> Option<AskReply> {
    let analysis = agent_run_context?.turn_analysis.as_ref()?;
    if analysis.turn_type != Some(crate::intent_router::TurnType::PreferenceOrMemory) {
        return None;
    }
    let bindings = analysis
        .state_patch
        .as_ref()?
        .get("alias_bindings")?
        .as_array()?;
    let pairs = bindings
        .iter()
        .filter_map(|binding| {
            let alias = binding.get("alias")?.as_str()?.trim();
            let target = binding.get("target")?.as_str()?.trim();
            if alias.is_empty() || target.is_empty() {
                None
            } else {
                Some((alias, target))
            }
        })
        .collect::<Vec<_>>();
    if pairs.is_empty() {
        return None;
    }
    let answer = if language_hint == "en" {
        if pairs.len() == 1 {
            format!("Remembered: `{}` -> `{}`.", pairs[0].0, pairs[0].1)
        } else {
            let lines = pairs
                .iter()
                .map(|(alias, target)| format!("- `{alias}` -> `{target}`"))
                .collect::<Vec<_>>()
                .join("\n");
            format!("Remembered:\n{lines}")
        }
    } else if pairs.len() == 1 {
        format!("已记住：`{}` -> `{}`。", pairs[0].0, pairs[0].1)
    } else {
        let lines = pairs
            .iter()
            .map(|(alias, target)| format!("- `{alias}` -> `{target}`"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("已记住：\n{lines}")
    };
    Some(ask_reply_with_chat_process(answer, language_hint))
}

pub(crate) fn build_resume_continue_execute_prompt(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    fallback_user_text: &str,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    let user_text = payload
        .get("resume_user_text")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_user_text);
    let resume_context = payload
        .get("resume_context")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let resume_instruction = payload
        .get("resume_instruction")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let resume_steps = payload.get("resume_steps");
    build_resume_continue_execute_prompt_from_parts(
        state,
        task,
        user_text,
        &resume_context,
        resume_instruction,
        resume_steps,
    )
}

pub(crate) fn build_resume_continue_execute_prompt_from_context(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resume_context: &Value,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    build_resume_continue_execute_prompt_from_parts(
        state,
        task,
        user_text,
        resume_context,
        "",
        None,
    )
}

fn build_resume_followup_discussion_prompt_from_parts(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resume_context: &Value,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    let resume_context_json =
        serde_json::to_string_pretty(resume_context).unwrap_or_else(|_| resume_context.to_string());
    let (prompt_template, _) = crate::bootstrap::load_required_prompt_template_for_state(
        state,
        crate::RESUME_FOLLOWUP_DISCUSSION_PROMPT_LOGICAL_PATH,
    )?;
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    Ok(crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_TEXT__", user_text.trim()),
            ("__RESUME_CONTEXT__", &resume_context_json),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
        ],
    ))
}

pub(crate) fn build_resume_followup_discussion_prompt(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    fallback_user_text: &str,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    let user_text = payload
        .get("resume_user_text")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_user_text)
        .trim();
    let resume_context = payload
        .get("resume_context")
        .cloned()
        .unwrap_or_else(|| json!({}));
    build_resume_followup_discussion_prompt_from_parts(state, task, user_text, &resume_context)
}

pub(crate) fn build_resume_followup_discussion_prompt_from_context(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resume_context: &Value,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    build_resume_followup_discussion_prompt_from_parts(state, task, user_text, resume_context)
}

fn chat_act_goal_from_prompt(prompt_with_memory: &str) -> String {
    format!(
        "{}\n\nMode hint: chat_act. Complete required actions first, then return a concise user-facing reply that confirms results naturally.",
        prompt_with_memory
    )
}

fn fuzzy_locator_clarify_context(candidates: &[String]) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    let candidate_block = candidates
        .iter()
        .enumerate()
        .map(|(idx, value)| format!("candidate_{}: {}", idx + 1, value))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "clarify_case: fuzzy_locator_candidates\nexact_target_found: false\ncandidate_count: {}\n{}",
        candidates.len(),
        candidate_block
    ))
}

fn preferred_route_clarify_question(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    // Phase 0.3: 单入口复用 normalizer 的 clarify_question。
    //
    // 原先这里先 filter `route.needs_clarify=true`，导致 `post_route_policy`
    // 将 routed_mode 强制覆写为 `AskClarify`（例如缺少 locator）但 normalizer
    // 自己没把 `needs_clarify` 设为 true 的场景下，`clarify_question` 被丢弃，
    // 后续 `generate_or_reuse_clarify_question` 会带 `AllowModel` 策略再次触发
    // 一次 LLM 调用。只要 normalizer 已经给出 clarify_question，就直接复用，
    // 把"这一轮澄清问题由谁出"收敛到单一入口。
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    let question = route.clarify_question.trim();
    if !question.is_empty() {
        return Some(question.to_string());
    }
    None
}

fn route_structured_clarify_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    if let Some(context) = fuzzy_locator_clarify_context(&ctx.fuzzy_locator_suggestions) {
        return Some(context);
    }
    if !route.needs_clarify || !route.output_contract.locator_hint.trim().is_empty() {
        return None;
    }
    let clarify_case = if route.output_contract.delivery_required {
        Some("missing_file_locator")
    } else if route.output_contract.requires_content_evidence
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        )
    {
        Some("missing_read_target")
    } else {
        None
    }?;
    Some(
        [
            format!("clarify_case: {clarify_case}"),
            format!(
                "locator_kind: {}",
                route.output_contract.locator_kind.as_str()
            ),
            format!(
                "response_shape: {}",
                route.output_contract.response_shape.as_str()
            ),
            format!(
                "requires_content_evidence: {}",
                route.output_contract.requires_content_evidence
            ),
            format!(
                "delivery_required: {}",
                route.output_contract.delivery_required
            ),
        ]
        .join("\n"),
    )
}

fn chat_route_resolution_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    let mut lines = Vec::new();
    let resolved_intent = route.resolved_intent.trim();
    if !resolved_intent.is_empty() {
        lines.push(format!("resolved_user_intent: {resolved_intent}"));
    }
    let locator_hint = route.output_contract.locator_hint.trim();
    if !locator_hint.is_empty() {
        lines.push(format!("locator_hint: {locator_hint}"));
    }
    lines.push(format!(
        "response_shape: {}",
        route.output_contract.response_shape.as_str()
    ));
    lines.push(format!(
        "semantic_kind: {}",
        route.output_contract.semantic_kind.as_str()
    ));
    lines.push(format!(
        "requires_content_evidence: {}",
        route.output_contract.requires_content_evidence
    ));
    lines.push(format!(
        "delivery_required: {}",
        route.output_contract.delivery_required
    ));
    let route_reason = route.route_reason.trim();
    if !route_reason.is_empty() {
        lines.push(format!("route_reason: {route_reason}"));
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "### ROUTE_RESOLUTION\nTreat the following route resolution as authoritative for this turn. It is resolved context, not missing context. If older memory or unrelated assistant history conflicts with it, prefer this resolution unless the user explicitly asks about older history.\n{}\n",
        lines.join("\n")
    ))
}

fn chat_prompt_context_with_route_resolution(
    chat_prompt_context: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> String {
    let route_context = chat_route_resolution_context(agent_run_context);
    let recent_execution_context = chat_recent_execution_context(agent_run_context);
    if route_context.is_none() && recent_execution_context.is_none() {
        return chat_prompt_context.to_string();
    };
    let trimmed_context = chat_prompt_context.trim();
    let mut blocks = Vec::new();
    if !(trimmed_context.is_empty() || trimmed_context == "<none>") {
        blocks.push(chat_prompt_context.to_string());
    }
    if let Some(route_context) = route_context {
        blocks.push(route_context);
    }
    if let Some(recent_execution_context) = recent_execution_context {
        blocks.push(recent_execution_context);
    }
    blocks.join("\n\n")
}

fn chat_recent_execution_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    if !route.output_contract.requires_content_evidence {
        return None;
    }
    let context = ctx
        .cross_turn_recent_execution_context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "<none>")?;
    Some(format!(
        "### RECENT_EXECUTION_CONTEXT\nUse this observed execution context as evidence for this turn when the route contract requires content evidence. Do not invent details beyond it.\n{context}"
    ))
}

fn chat_user_request<'a>(resolved_prompt: &'a str, execution_user_request: &'a str) -> &'a str {
    if execution_user_request.trim() != resolved_prompt.trim() {
        execution_user_request
    } else {
        resolved_prompt
    }
}

fn chat_request_for_prompt(original_user_request: &str, semantic_request: &str) -> String {
    let original = original_user_request.trim();
    let semantic = semantic_request.trim();
    if original.is_empty() || original == semantic {
        return semantic.to_string();
    }
    format!(
        "Original user request:\n{original}\n\nResolved semantic intent / answer candidate:\n{semantic}\n\nUse the resolved semantic intent to answer the original request. If the original request asks for only a value, ID, path, name, or one short answer, output only the resolved value with no preamble."
    )
}

fn task_payload_text(task: &ClaimedTask) -> Option<String> {
    crate::task_payload_value(task)?
        .get("text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

async fn execute_via_planner_loop(
    state: &AppState,
    task: &ClaimedTask,
    prompt_with_memory: &str,
    execution_user_request: &str,
    ask_mode: &crate::AskMode,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<AskReply, String> {
    let planner_goal = if ask_mode.finalize_chat_wrapped() {
        chat_act_goal_from_prompt(prompt_with_memory)
    } else {
        prompt_with_memory.to_string()
    };
    crate::agent_engine::run_agent_with_tools(
        state,
        task,
        &planner_goal,
        execution_user_request,
        agent_run_context,
    )
    .await
}

pub(crate) async fn execute_ask_routed(
    state: &AppState,
    task: &ClaimedTask,
    chat_prompt_context: &str,
    prompt_with_memory: &str,
    resolved_prompt: &str,
    execution_user_request: &str,
    agent_mode: bool,
    resume_force_chat: bool,
    route_ask_mode: Option<crate::AskMode>,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<AskReply, String> {
    // Phase 2.7: legacy `route_request_mode` (second-LLM router) was removed. Callers now
    // pass the folded route ask_mode directly; if for some reason a caller drops it, default
    // to AskClarify rather than burning another LLM round-trip.
    let route_ask_mode_for_log = route_ask_mode.clone();
    let (ask_mode, override_reason) = if resume_force_chat {
        (
            crate::AskMode::from_routed_mode(RoutedMode::Chat),
            Some("resume_force_chat"),
        )
    } else if let Some(mode) = route_ask_mode {
        (mode, None)
    } else if agent_mode {
        (
            crate::AskMode::from_routed_mode(RoutedMode::AskClarify),
            Some("route_ask_mode=None and agent_mode=true"),
        )
    } else {
        (
            crate::AskMode::from_routed_mode(RoutedMode::Chat),
            Some("route_ask_mode=None and agent_mode=false"),
        )
    };
    let routed_mode = ask_mode.to_routed_mode();
    tracing::info!(
        "{} worker_once: ask task_id={} first_layer_decision={} ask_mode={} routed_mode={:?} agent_mode={} override={}",
        crate::highlight_tag("routing"),
        task.task_id,
        ask_mode.first_layer_decision().as_str(),
        route_ask_mode_for_log
            .as_ref()
            .map(crate::AskMode::as_str)
            .unwrap_or("none"),
        routed_mode,
        agent_mode,
        override_reason.unwrap_or("")
    );
    if let Some(reply) = crate::self_extension::maybe_handle_ask_self_extension(
        state,
        task,
        resolved_prompt,
        execution_user_request,
        agent_run_context.as_ref(),
    )
    .await?
    {
        return Ok(reply);
    }
    let current_turn_user_request_for_process =
        task_payload_text(task).unwrap_or_else(|| execution_user_request.to_string());
    let process_language_hint = crate::language_policy::task_response_language_hint(
        state,
        task,
        &current_turn_user_request_for_process,
    );
    if let Some(candidate) =
        runtime_scalar_path_direct_answer_candidate(state, agent_run_context.as_ref())
    {
        tracing::info!(
            "{} worker_once: ask runtime_scalar_path_direct_answer task_id={} len={}",
            crate::highlight_tag("routing"),
            task.task_id,
            candidate.len()
        );
        return Ok(ask_reply_with_chat_process(
            candidate,
            &process_language_hint,
        ));
    }
    match ask_mode.first_layer_decision() {
        crate::FirstLayerDecision::DirectAnswer => {
            let allow_candidate_budget_fallback =
                state.task_llm_budget_exceeded(&task.task_id).is_some();
            if let Some(candidate) = normalizer_chat_direct_answer_candidate(
                state,
                resolved_prompt,
                agent_run_context.as_ref(),
                allow_candidate_budget_fallback,
            ) {
                tracing::info!(
                    "{} worker_once: ask normalizer_answer_candidate_budget_fallback task_id={} len={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    candidate.len()
                );
                return Ok(ask_reply_with_chat_process(
                    candidate,
                    &process_language_hint,
                ));
            }
            let chat_prompt_context = chat_prompt_context_with_route_resolution(
                chat_prompt_context,
                agent_run_context.as_ref(),
            );
            let resolved_chat_prompt =
                crate::bootstrap::load_required_prompt_template_for_state_with_meta(
                    state,
                    crate::CHAT_RESPONSE_PROMPT_LOGICAL_PATH,
                )
                .map_err(|e| e.to_string())?;
            let chat_prompt_template = resolved_chat_prompt.template;
            let chat_prompt_source = resolved_chat_prompt.source;
            let chat_prompt_version = resolved_chat_prompt.version;
            crate::log_prompt_render_with_version(
                state,
                &task.task_id,
                "chat_response_prompt",
                &chat_prompt_source,
                chat_prompt_version.as_deref(),
                None,
            );
            let task_persona_prompt = state.task_persona_prompt(task);
            let chat_user_request = chat_user_request(resolved_prompt, execution_user_request);
            let current_turn_user_request =
                task_payload_text(task).unwrap_or_else(|| chat_user_request.to_string());
            let request_language_hint = crate::language_policy::task_response_language_hint(
                state,
                task,
                &current_turn_user_request,
            );
            if let Some(reply) =
                state_patch_alias_bindings_ack(agent_run_context.as_ref(), &request_language_hint)
            {
                tracing::info!(
                    "{} worker_once: ask state_patch_alias_bindings_ack task_id={}",
                    crate::highlight_tag("routing"),
                    task.task_id
                );
                return Ok(reply);
            }
            if state.task_llm_budget_exceeded(&task.task_id).is_none() {
                if let Some(mut gate_ctx) = agent_run_context.clone() {
                    if let Some(gate) = run_direct_answer_gate(
                        state,
                        task,
                        &current_turn_user_request,
                        Some(&gate_ctx),
                    )
                    .await
                    {
                        match apply_direct_answer_gate_outcome(&mut gate_ctx, gate) {
                            DirectAnswerPreflight::DirectAnswer => {}
                            DirectAnswerPreflight::Clarify(question) => {
                                tracing::info!(
                                    "{} worker_once: ask direct_answer_gate_clarify task_id={}",
                                    crate::highlight_tag("routing"),
                                    task.task_id
                                );
                                return Ok(ask_reply_with_chat_process(
                                    question,
                                    &request_language_hint,
                                ));
                            }
                            DirectAnswerPreflight::PlannerExecute(promoted_ctx) => {
                                tracing::info!(
                                    "{} worker_once: ask direct_answer_gate_promoted_to_planner task_id={}",
                                    crate::highlight_tag("routing"),
                                    task.task_id
                                );
                                let promoted_prompt_with_memory = promoted_ctx
                                    .route_result
                                    .as_ref()
                                    .map(|route| route.resolved_intent.trim())
                                    .filter(|intent| {
                                        !intent.is_empty() && *intent != prompt_with_memory.trim()
                                    })
                                    .map(|intent| {
                                        format!(
                                            "{}\n\nDirect answer gate resolved execution intent:\n{}",
                                            prompt_with_memory.trim(),
                                            intent
                                        )
                                    })
                                    .unwrap_or_else(|| prompt_with_memory.to_string());
                                return execute_via_planner_loop(
                                    state,
                                    task,
                                    &promoted_prompt_with_memory,
                                    execution_user_request,
                                    &crate::AskMode::from_routed_mode(RoutedMode::ChatAct),
                                    Some(promoted_ctx),
                                )
                                .await;
                            }
                        }
                    }
                }
            }
            let request_for_chat_prompt =
                chat_request_for_prompt(&current_turn_user_request, chat_user_request);
            let chat_prompt = crate::render_prompt_template(
                &chat_prompt_template,
                &[
                    ("__PERSONA_PROMPT__", &task_persona_prompt),
                    ("__CONTEXT__", &chat_prompt_context),
                    (
                        "__CONFIG_RESPONSE_LANGUAGE__",
                        &state.policy.command_intent.default_locale,
                    ),
                    ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
                    ("__REQUEST__", &request_for_chat_prompt),
                ],
            );
            crate::llm_gateway::run_with_fallback_with_prompt_source(
                state,
                task,
                &chat_prompt,
                &chat_prompt_source,
            )
            .await
            .map(|answer| ask_reply_with_chat_process(answer, &request_language_hint))
            .map_err(|e| e.to_string())
        }
        crate::FirstLayerDecision::PlannerExecute => {
            execute_via_planner_loop(
                state,
                task,
                prompt_with_memory,
                execution_user_request,
                &ask_mode,
                agent_run_context.clone(),
            )
            .await
        }
        crate::FirstLayerDecision::Clarify => {
            let clarify_reason = agent_run_context
                .as_ref()
                .and_then(|ctx| ctx.route_result.as_ref())
                .map(|route| route.route_reason.as_str())
                .unwrap_or("router_selected_ask_clarify");
            let preferred_clarify = preferred_route_clarify_question(agent_run_context.as_ref());
            let structured_clarify_context =
                route_structured_clarify_context(agent_run_context.as_ref());
            let clarify_policy = if structured_clarify_context.is_some()
                || (preferred_clarify.is_none()
                    && agent_run_context
                        .as_ref()
                        .and_then(|ctx| ctx.route_result.as_ref())
                        .is_some_and(|route| route.clarify_question.trim().is_empty()))
            {
                crate::intent_router::ClarifyQuestionPolicy::SafeFallback
            } else {
                crate::intent_router::ClarifyQuestionPolicy::AllowModel
            };
            let clarify = crate::intent_router::generate_or_reuse_clarify_question(
                state,
                task,
                resolved_prompt,
                clarify_reason,
                structured_clarify_context.as_deref(),
                preferred_clarify.as_deref(),
                clarify_policy,
                // §7.2: ask_flow 路由到 AskClarify 但 route_result 也没给 clarify_question
                // → IntentUnresolved（与 ask_pipeline 同语义）。
                crate::fallback::ClarifyFallbackSource::IntentUnresolved,
            )
            .await;
            Ok(ask_reply_with_chat_process(clarify, &process_language_hint))
        }
    }
}

pub(crate) async fn analyze_attached_images_for_ask(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    resolved_prompt: &str,
) -> anyhow::Result<Option<String>> {
    let Some(images) = payload.get("images").and_then(|v| v.as_array()) else {
        return Ok(None);
    };
    if images.is_empty() {
        return Ok(None);
    }
    let mut args = json!({
        "action": "describe",
        "images": images,
    });
    let instruction = resolved_prompt.trim();
    if let Some(obj) = args.as_object_mut() {
        if !instruction.is_empty() {
            obj.insert(
                "instruction".to_string(),
                Value::String(instruction.to_string()),
            );
        }
        if let Some(language) = payload
            .get("response_language")
            .or_else(|| payload.get("language"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            obj.insert(
                "response_language".to_string(),
                Value::String(language.to_string()),
            );
        }
    }
    crate::skills::run_skill_with_runner(state, task, "image_vision", args)
        .await
        .map_err(anyhow::Error::msg)
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::{
        apply_direct_answer_gate_outcome, ask_reply_with_chat_process,
        chat_prompt_context_with_route_resolution, chat_request_for_prompt, chat_user_request,
        normalizer_chat_direct_answer_candidate, preferred_route_clarify_question,
        route_structured_clarify_context, runtime_scalar_path_direct_answer_candidate,
        state_patch_alias_bindings_ack, task_payload_text, DirectAnswerGateContractOut,
        DirectAnswerGateOut, DirectAnswerGateSelfExtensionOut, DirectAnswerPreflight,
    };

    fn chat_route_for_gate() -> crate::RouteResult {
        crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
            resolved_intent: "帮我写一篇关于 RustClaw 的长文".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(0.86),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        }
    }

    fn gate_contract(
        requires_content_evidence: bool,
        locator_kind: &str,
        semantic_kind: &str,
    ) -> DirectAnswerGateContractOut {
        DirectAnswerGateContractOut {
            response_shape: "free".to_string(),
            exact_sentence_count: None,
            requires_content_evidence,
            delivery_required: false,
            locator_kind: locator_kind.to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: semantic_kind.to_string(),
            locator_hint: String::new(),
            self_extension: DirectAnswerGateSelfExtensionOut::default(),
        }
    }

    fn gate_out(decision: &str, contract: DirectAnswerGateContractOut) -> DirectAnswerGateOut {
        DirectAnswerGateOut {
            decision: decision.to_string(),
            reason: "test".to_string(),
            confidence: 0.9,
            clarify_question: String::new(),
            resolved_user_intent: "Write a grounded RustClaw article using workspace evidence."
                .to_string(),
            output_contract: contract,
        }
    }

    #[test]
    fn direct_answer_gate_promotes_chat_to_planner_execute() {
        let route = chat_route_for_gate();
        let mut ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let gate = gate_out(
            "planner_execute",
            gate_contract(true, "current_workspace", "workspace_project_summary"),
        );

        let outcome = apply_direct_answer_gate_outcome(&mut ctx, gate);

        assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
        let route = ctx.route_result.expect("route");
        assert_eq!(route.routed_mode, crate::RoutedMode::ChatAct);
        assert!(route.is_execute_gate());
        assert!(route.output_contract.requires_content_evidence);
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
        );
        assert_eq!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::WorkspaceProjectSummary
        );
        assert!(route.route_reason.contains("direct_answer_gate_execute"));
    }

    #[test]
    fn direct_answer_gate_keeps_direct_chat_when_decision_is_direct() {
        let route = chat_route_for_gate();
        let mut ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));

        let outcome = apply_direct_answer_gate_outcome(&mut ctx, gate);

        assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
        let route = ctx.route_result.expect("route");
        assert_eq!(route.routed_mode, crate::RoutedMode::Chat);
        assert!(route.is_chat_gate());
        assert!(!route.output_contract.requires_content_evidence);
    }

    #[test]
    fn direct_answer_gate_promotes_artifact_listing_candidate_to_planner() {
        let mut route = chat_route_for_gate();
        route.resolved_intent = concat!(
            "List the first five entries under the selected workspace directory\n",
            "answer_candidate: act_plan.log, clawd.log, clawd.run.log, clawd.test.log, clawd_manual.log"
        )
        .to_string();
        let mut ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));

        let outcome = apply_direct_answer_gate_outcome(&mut ctx, gate);

        assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
        let route = ctx.route_result.expect("route");
        assert_eq!(route.routed_mode, crate::RoutedMode::ChatAct);
        assert!(route.is_execute_gate());
        assert!(route.output_contract.requires_content_evidence);
        assert_eq!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
        );
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
        );
        assert!(route
            .route_reason
            .contains("direct_answer_gate_artifact_listing_execute"));
    }

    #[test]
    fn direct_answer_gate_does_not_promote_non_artifact_example_list() {
        let mut route = chat_route_for_gate();
        route.resolved_intent =
            "Give simple examples\nanswer_candidate: apple, banana, cherry".to_string();
        let mut ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));

        let outcome = apply_direct_answer_gate_outcome(&mut ctx, gate);

        assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
        let route = ctx.route_result.expect("route");
        assert!(route.is_chat_gate());
        assert!(!route.output_contract.requires_content_evidence);
    }

    #[test]
    fn direct_answer_gate_promotes_contract_evidence_even_when_decision_is_direct() {
        let route = chat_route_for_gate();
        let mut ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut contract = gate_contract(true, "path", "content_excerpt_summary");
        contract.locator_hint = "/tmp/clawd.log".to_string();
        let gate = gate_out("direct_answer", contract);

        let outcome = apply_direct_answer_gate_outcome(&mut ctx, gate);

        assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
        let route = ctx.route_result.expect("route");
        assert_eq!(route.routed_mode, crate::RoutedMode::ChatAct);
        assert!(route.is_execute_gate());
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
        assert_eq!(route.output_contract.locator_hint, "/tmp/clawd.log");
        assert!(route
            .route_reason
            .contains("direct_answer_gate_contract_execute"));
    }

    #[test]
    fn direct_answer_gate_promotes_chat_to_clarify_when_blocker_is_missing() {
        let route = chat_route_for_gate();
        let mut ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut gate = gate_out("clarify", gate_contract(false, "none", "none"));
        gate.clarify_question = "要创建的文件夹叫什么名字？".to_string();

        let outcome = apply_direct_answer_gate_outcome(&mut ctx, gate);

        assert!(
            matches!(outcome, DirectAnswerPreflight::Clarify(question) if question == "要创建的文件夹叫什么名字？")
        );
        let route = ctx.route_result.expect("route");
        assert_eq!(route.routed_mode, crate::RoutedMode::AskClarify);
        assert!(route.is_clarify_gate());
        assert!(route.needs_clarify);
        assert_eq!(route.clarify_question, "要创建的文件夹叫什么名字？");
        assert!(route.route_reason.contains("direct_answer_gate_clarify"));
    }

    #[test]
    fn chat_prompt_context_appends_authoritative_route_resolution() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
            resolved_intent: "上一个和上上个哪个更多，只回答目录名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "'上一个'=assistant[-1](document,17), '上上个'=assistant[-2](scripts,48); scripts 更多".to_string(),
            route_confidence: Some(0.94),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                locator_hint: "scripts".to_string(),
                ..Default::default()
            },
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let rendered = chat_prompt_context_with_route_resolution(
            "### MEMORY_CONTEXT\nRECENT_ASSISTANT_RESULTS\n- old summary",
            Some(&ctx),
        );
        assert!(rendered.contains("### ROUTE_RESOLUTION"));
        assert!(rendered.contains("resolved_user_intent: 上一个和上上个哪个更多，只回答目录名"));
        assert!(rendered.contains("locator_hint: scripts"));
        assert!(rendered.contains("scripts 更多"));
    }

    #[test]
    fn chat_prompt_context_replaces_empty_placeholder_with_route_resolution() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
            resolved_intent: "client-like-continuous-20260428_144029".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(0.94),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let rendered = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));
        assert!(!rendered.contains("<none>"));
        assert!(rendered.contains("### ROUTE_RESOLUTION"));
        assert!(rendered.contains("client-like-continuous-20260428_144029"));
    }

    #[test]
    fn chat_prompt_context_includes_recent_execution_when_contract_requires_evidence() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
            resolved_intent: "Summarize the observed README excerpt in one sentence".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "prior observed content is available".to_string(),
            route_confidence: Some(0.94),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                requires_content_evidence: true,
                ..Default::default()
            },
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            cross_turn_recent_execution_context: Some(
                "read_range path=/tmp/README.md\n# RustClaw\nlocal Rust agent runtime".to_string(),
            ),
            ..Default::default()
        };

        let rendered = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));

        assert!(rendered.contains("### ROUTE_RESOLUTION"));
        assert!(rendered.contains("### RECENT_EXECUTION_CONTEXT"));
        assert!(rendered.contains("local Rust agent runtime"));
    }

    #[test]
    fn chat_user_request_preserves_inline_structured_prompt_when_resolution_dropped_payload() {
        let prompt = r#"sort this JSON array by score descending and render it as a markdown table: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
        let resolved = "Sort the provided JSON array by score in descending order and output as a markdown table";
        assert_eq!(chat_user_request(resolved, prompt), prompt);
    }

    #[test]
    fn chat_request_for_prompt_keeps_original_constraints_and_semantic_anchor() {
        let request = chat_request_for_prompt(
            "刚才我让你记住的测试编号是什么？只回答编号。",
            "client-like-continuous-20260428_144029",
        );
        assert!(request.contains("Original user request:"));
        assert!(request.contains("只回答编号"));
        assert!(request.contains("Resolved semantic intent / answer candidate:"));
        assert!(request.contains("client-like-continuous-20260428_144029"));
        assert!(request.contains("output only the resolved value"));
    }

    #[test]
    fn task_payload_text_preserves_raw_current_turn_for_chat_language_hint() {
        let task = crate::ClaimedTask {
            task_id: "task".to_string(),
            user_id: 1,
            chat_id: 1,
            user_key: None,
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: serde_json::json!({"text":"先只看登录模块"}).to_string(),
        };
        assert_eq!(task_payload_text(&task).as_deref(), Some("先只看登录模块"));
    }

    #[test]
    fn chat_reply_does_not_attach_context_process_message() {
        let reply =
            ask_reply_with_chat_process("RustClaw 是本地 agent 运行时。".to_string(), "zh-CN");

        assert_eq!(reply.text, "RustClaw 是本地 agent 运行时。");
        assert!(reply.messages.is_empty());
    }

    #[test]
    fn english_chat_reply_does_not_attach_execution_process_message() {
        let reply =
            ask_reply_with_chat_process("RustClaw is a local agent runtime.".to_string(), "en");

        assert_eq!(reply.text, "RustClaw is a local agent runtime.");
        assert!(reply.messages.is_empty());
    }

    #[test]
    fn alias_state_patch_uses_structured_ack_without_chat_llm() {
        let ctx = crate::agent_engine::AgentRunContext {
            turn_analysis: Some(crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
                should_interrupt_active_run: false,
                state_patch: Some(serde_json::json!({
                    "alias_bindings": [
                        {
                            "alias": "that docs dir",
                            "target": "/tmp/docs"
                        }
                    ]
                })),
                attachment_processing_required: false,
            }),
            ..Default::default()
        };

        let reply = state_patch_alias_bindings_ack(Some(&ctx), "zh-CN").unwrap();

        assert_eq!(reply.text, "已记住：`that docs dir` -> `/tmp/docs`。");
        assert!(reply.messages.is_empty());
    }

    #[test]
    fn alias_state_patch_ack_requires_memory_turn() {
        let ctx = crate::agent_engine::AgentRunContext {
            turn_analysis: Some(crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskRequest),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
                should_interrupt_active_run: false,
                state_patch: Some(serde_json::json!({
                    "alias_bindings": [
                        {
                            "alias": "that docs dir",
                            "target": "/tmp/docs"
                        }
                    ]
                })),
                attachment_processing_required: false,
            }),
            ..Default::default()
        };

        assert!(state_patch_alias_bindings_ack(Some(&ctx), "zh-CN").is_none());
    }

    #[test]
    fn response_language_hint_prefers_current_request_language() {
        assert_eq!(
            crate::language_policy::preferred_response_language_hint("写个两句短诗", None),
            "zh-CN"
        );
        assert_eq!(
            crate::language_policy::preferred_response_language_hint(
                "do not run anything, just tell me a very short joke",
                None
            ),
            "en"
        );
        assert_eq!(
            crate::language_policy::preferred_response_language_hint(
                "用 English 解释 README",
                None
            ),
            "mixed"
        );
        assert_eq!(
            crate::language_policy::preferred_response_language_hint("12345", None),
            "config_default"
        );
    }

    #[test]
    fn normalizer_chat_direct_answer_uses_candidate_only_without_evidence_contract() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
            resolved_intent:
                "写一首两句的打工人短诗\nanswer_candidate: 早出晚归血汗钱\n苦中作乐笑开颜"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer supplied candidate".to_string(),
            route_confidence: Some(0.95),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert_eq!(
            normalizer_chat_direct_answer_candidate(
                &state,
                "写一首两句的打工人短诗\nanswer_candidate: 早出晚归血汗钱\n苦中作乐笑开颜",
                Some(&ctx),
                true,
            )
            .as_deref(),
            Some("早出晚归血汗钱\n苦中作乐笑开颜")
        );

        assert_eq!(
            normalizer_chat_direct_answer_candidate(
                &state,
                "写一首两句的打工人短诗\nanswer_candidate: 早出晚归血汗钱\n苦中作乐笑开颜",
                Some(&ctx),
                false,
            ),
            None
        );
    }

    #[test]
    fn normalizer_chat_direct_answer_does_not_bypass_evidence_contract() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
            resolved_intent:
                "检查当前目录是否有隐藏文件\nanswer_candidate: 有，例如 .git、.gitignore、.pids"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "needs local evidence".to_string(),
            route_confidence: Some(0.95),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Medium,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Strict,
                requires_content_evidence: true,
                semantic_kind: crate::OutputSemanticKind::HiddenEntriesCheck,
                ..Default::default()
            },
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert_eq!(
            normalizer_chat_direct_answer_candidate(
                &state,
                "检查当前目录是否有隐藏文件\nanswer_candidate: 有，例如 .git、.gitignore、.pids",
                Some(&ctx),
                true,
            ),
            None
        );
    }

    #[test]
    fn normalizer_chat_direct_answer_uses_runtime_fact_candidate_without_budget_fallback() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let runtime_path = state.skill_rt.workspace_root.to_string_lossy().to_string();
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
            resolved_intent: format!(
                "User request: output absolute path of current working directory\nanswer_candidate: {runtime_path}"
            ),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer supplied runtime fact".to_string(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert_eq!(
            normalizer_chat_direct_answer_candidate(
                &state,
                &format!(
                    "User request: output absolute path of current working directory\nanswer_candidate: {runtime_path}"
                ),
                Some(&ctx),
                false,
            )
            .as_deref(),
            Some(runtime_path.as_str())
        );
    }

    #[test]
    fn runtime_scalar_path_direct_answer_uses_verified_contract_locator() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let runtime_path = state.skill_rt.workspace_root.to_string_lossy().to_string();
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "Output the current workspace path".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "runtime scalar path".to_string(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Scalar,
                semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                locator_hint: runtime_path.clone(),
                ..Default::default()
            },
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert_eq!(
            runtime_scalar_path_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
            Some(runtime_path.as_str())
        );
    }

    #[test]
    fn runtime_scalar_path_direct_answer_rejects_unverified_locator() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "Output the current workspace path".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "runtime scalar path".to_string(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Scalar,
                semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                locator_hint: "/tmp/not-the-rustclaw-workspace".to_string(),
                ..Default::default()
            },
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert_eq!(
            runtime_scalar_path_direct_answer_candidate(&state, Some(&ctx)),
            None
        );
    }

    #[test]
    fn preferred_route_clarify_question_respects_explicit_route_question_before_generic_fallback() {
        let mut route = crate::RouteResult {
            routed_mode: crate::RoutedMode::AskClarify,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::AskClarify),
            resolved_intent: "看看那个目录下面都有什么".to_string(),
            needs_clarify: true,
            clarify_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
            route_reason: "fresh_deictic_missing_locator:directory_lookup".to_string(),
            route_confidence: Some(0.95),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                locator_kind: crate::OutputLocatorKind::Path,
                ..Default::default()
            },
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route.clone()),
            ..Default::default()
        };
        assert_eq!(
            preferred_route_clarify_question(Some(&ctx)).as_deref(),
            Some("LOCATOR_CLARIFY_PROMPT")
        );

        route.clarify_question.clear();
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert_eq!(preferred_route_clarify_question(Some(&ctx)), None);
        let context = route_structured_clarify_context(Some(&ctx)).expect("structured context");
        assert!(context.contains("clarify_case: missing_read_target"));
        assert!(context.contains("locator_kind: path"));
    }

    #[test]
    fn fuzzy_locator_candidates_are_structured_context_not_hard_question() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::AskClarify,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::AskClarify),
            resolved_intent: "读取 Cargo.toml 的 package.name，只输出值".to_string(),
            needs_clarify: true,
            clarify_question: String::new(),
            route_reason: "llm_contract:generic_filename_scalar_extract".to_string(),
            route_confidence: Some(0.95),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Scalar,
                requires_content_evidence: true,
                locator_kind: crate::OutputLocatorKind::Filename,
                ..Default::default()
            },
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            fuzzy_locator_suggestions: vec![
                "/tmp/a/Cargo.toml".to_string(),
                "/tmp/b/Cargo.toml".to_string(),
            ],
            ..Default::default()
        };
        assert_eq!(preferred_route_clarify_question(Some(&ctx)), None);
        let context = route_structured_clarify_context(Some(&ctx)).expect("structured context");
        assert!(context.contains("clarify_case: fuzzy_locator_candidates"));
        assert!(context.contains("candidate_1: /tmp/a/Cargo.toml"));
        assert!(context.contains("candidate_2: /tmp/b/Cargo.toml"));
    }
}

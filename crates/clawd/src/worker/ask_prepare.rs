use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tracing::info;

use crate::{schedule_service, AppState};

pub(super) struct PreparedAskExecutionContext {
    pub(super) context_bundle: crate::task_context_builder::TaskContextBundle,
    pub(super) chat_prompt_context: String,
    pub(super) resolved_prompt_for_execution: String,
    pub(super) prompt_with_memory_for_execution: String,
    pub(super) memory_context_for_execution: String,
    pub(super) recent_execution_context: String,
}

pub(super) struct PreparedAskRouting {
    pub(super) route_result: crate::RouteResult,
    pub(super) execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    pub(super) turn_analysis: Option<crate::intent_router::TurnAnalysis>,
    pub(super) clarify_fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    pub(super) resolved_prompt: String,
    pub(super) agent_mode: bool,
    /// Final runtime ask mode after first-layer routing and resume overrides.
    /// All dispatch branches should use `ask_mode` predicates.
    pub(super) ask_mode: crate::AskMode,
}

pub(super) struct PreparedAskInput {
    pub(super) prompt: String,
    pub(super) source: String,
}

pub(super) struct PreparedRunSkillInput {
    pub(super) skill_name: String,
    pub(super) args: Value,
}

fn merged_prompt_from_task_turn_analysis(
    prior_primary_task_prompt: Option<&str>,
    prior_primary_task_output: Option<&str>,
    current_prompt: &str,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<String> {
    let prior = prior_primary_task_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let current = current_prompt.trim();
    if current.is_empty() || current == prior || current.contains(prior) {
        return None;
    }
    let analysis = turn_analysis?;
    let policy = analysis.target_task_policy?;
    let turn_type = analysis.turn_type?;
    let structured_patch = analysis
        .state_patch
        .as_ref()
        .and_then(render_task_state_patch);
    let include_prior_output = matches!(
        (turn_type, policy),
        (
            crate::intent_router::TurnType::TaskAppend
                | crate::intent_router::TurnType::TaskCorrect
                | crate::intent_router::TurnType::TaskScopeUpdate,
            crate::intent_router::TargetTaskPolicy::ReuseActive,
        )
    );
    let prior_output = if include_prior_output {
        prior_primary_task_output
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(truncate_task_output_for_merge)
    } else {
        None
    };
    match (turn_type, policy) {
        (
            crate::intent_router::TurnType::TaskAppend,
            crate::intent_router::TargetTaskPolicy::ReuseActive,
        ) => Some(merged_reuse_active_prompt(
            prior,
            prior_output.as_deref(),
            current,
            structured_patch.as_deref(),
            "Keep the same task and append this new instruction.",
        )),
        (
            crate::intent_router::TurnType::TaskCorrect,
            crate::intent_router::TargetTaskPolicy::ReuseActive,
        ) => Some(merged_reuse_active_prompt(
            prior,
            prior_output.as_deref(),
            current,
            structured_patch.as_deref(),
            "Keep the same task, but treat the new instruction as a correction that overrides conflicting earlier details.",
        )),
        (
            crate::intent_router::TurnType::TaskScopeUpdate,
            crate::intent_router::TargetTaskPolicy::ReuseActive,
        ) => Some(merged_reuse_active_prompt(
            prior,
            prior_output.as_deref(),
            current,
            structured_patch.as_deref(),
            "Keep the same task, but update its scope, priorities, or boundaries using the new instruction. Treat conceptual scope terms that describe content area, topic, section, audience, or emphasis as content constraints, not filesystem targets, unless the user explicitly asks to inspect files, code, or logs. If the updated scope is enough to produce a useful generic draft/plan/answer, produce that scoped result now instead of asking for optional platform/system subtype details.",
        )),
        (
            crate::intent_router::TurnType::TaskReplace,
            crate::intent_router::TargetTaskPolicy::ReplaceActive,
        ) => Some(merged_replace_active_prompt(
            prior,
            prior_output.as_deref(),
            current,
            structured_patch.as_deref(),
        )),
        _ => None,
    }
}

fn task_turn_merge_prior_context(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> (Option<&str>, Option<&str>) {
    if let Some(clarify_state) = session_snapshot.active_clarify_state.as_ref() {
        let prompt = non_empty_str(&clarify_state.source_request);
        let output = non_empty_str(&clarify_state.pending_question);
        if prompt.is_some() || output.is_some() {
            return (prompt, output);
        }
    }
    (
        session_snapshot
            .conversation_state
            .as_ref()
            .and_then(|state| state.last_primary_task_prompt.as_deref()),
        session_snapshot
            .conversation_state
            .as_ref()
            .and_then(|state| state.last_primary_task_output.as_deref()),
    )
}

fn non_empty_str(value: &str) -> Option<&str> {
    (!value.trim().is_empty()).then_some(value)
}

fn active_clarify_run_control_prompt(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    current_prompt: &str,
) -> Option<String> {
    if !route_result.is_chat_gate()
        || route_result.output_contract.delivery_required
        || !matches!(
            turn_analysis.and_then(|analysis| analysis.turn_type),
            Some(crate::intent_router::TurnType::RunControl)
        )
    {
        return None;
    }
    let clarify_state = session_snapshot.active_clarify_state.as_ref()?;
    if !matches!(
        clarify_state.missing_slot,
        crate::clarify_state::ClarifyMissingSlot::Locator
    ) {
        return None;
    }
    let source_request = clarify_state.source_request.trim();
    let pending_question = clarify_state.pending_question.trim();
    if source_request.is_empty() && pending_question.is_empty() {
        return None;
    }
    let candidate_targets = if clarify_state.candidate_targets.is_empty() {
        "<none>".to_string()
    } else {
        clarify_state.candidate_targets.join("\n")
    };
    Some(format!(
        "Previous request is waiting for clarification:\n{source_request}\n\nMissing information to confirm:\n{pending_question}\n\nCandidate targets from that clarification only:\n{candidate_targets}\n\nThe new user instruction changes this into a chat-only response and asks not to execute or deliver anything:\n{current_prompt}\n\nAnswer in the user's language by restating the missing information to confirm. Do not select a concrete file, alias, or path unless it is listed under Candidate targets from that clarification only."
    ))
}

fn should_apply_task_turn_merge(
    clarify_followup_resolution: &crate::intent::continuation_resolver::ClarifyFollowupResolution,
) -> bool {
    matches!(
        clarify_followup_resolution,
        crate::intent::continuation_resolver::ClarifyFollowupResolution::None
    )
}

fn truncate_task_output_for_merge(output: &str) -> String {
    const MAX_CHARS: usize = 2000;
    let trimmed = output.trim();
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_string();
    }
    trimmed.chars().take(MAX_CHARS).collect::<String>()
}

fn render_task_state_patch(state_patch: &Value) -> Option<String> {
    match state_patch {
        Value::Null => None,
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        other => serde_json::to_string(other)
            .ok()
            .filter(|serialized| !serialized.is_empty()),
    }
}

fn merged_reuse_active_prompt(
    prior: &str,
    prior_output: Option<&str>,
    current: &str,
    structured_patch: Option<&str>,
    merge_instruction: &str,
) -> String {
    let recent_output_block = prior_output
        .map(|output| format!("\n\nMost recent generated output:\n{output}"))
        .unwrap_or_default();
    let continuity_rules = "\n\nContinuity rules:\n- Preserve all active prior subject, scope, audience, tone, key facts, and safety constraints unless the new instruction explicitly overrides them.\n- Continuity does not preserve reply language when the current turn has a clear language. The current user instruction's language hint remains authoritative; translate or rewrite the prior deliverable into that language while preserving facts, scope, and format.\n- Treat the latest output-shape constraints as highest priority: exact bullet/table row counts, word/character limits, and output-only/body-only requests must be followed.\n- For table requests, row counts mean data rows only, excluding the header and separator. A two-row table must contain exactly two data rows.\n- When the latest instruction specifies a table, bullet count, final sentence, body-only, or another exact output shape, emit only that requested shape; do not append explanatory notes or summaries outside it.\n- For a latest length limit, compress the deliverable body comfortably below the stated limit instead of preserving all prior coverage. Runtime-visible process/execution framing is separate from the deliverable body and must not be used as an excuse to exceed the requested body length.\n- A format/count-only change must not broaden a narrowed scope. If an exact count needs more items than the recent output has, split, combine, or elaborate within the current scope instead of adding unrelated categories.\n- If the most recent generated output is a clarification question, visibly incomplete, starts mid-document, or relies on a continued marker, do not preserve its question shape, broken numbering, continuation marker, or fragment boundary. Rebuild a coherent compact deliverable for the current task scope and latest instruction, while preserving valid facts and constraints.\n- Style or quality feedback means rewrite the deliverable itself. Do not answer with meta-commentary like \"it already meets that\" unless the user explicitly asks for evaluation.\n- Do not invent unobserved project setup commands, package names, dependency lines, version numbers, paths, or configuration values. If such details are not provided or observed, keep them neutral/generic or say to follow the repo's documented setup path.\n- For a project-specific setup/deployment note with no observed setup evidence, do not include command blocks, backticked command invocations, package names, fake CLI steps, settings-file claims, or assigned installer roles. If recent output already contains unsupported setup commands or setup artifacts, remove or replace them with neutral documented-path wording instead of preserving them.\n- When rewriting setup/deployment/onboarding text for a simpler audience, do not introduce alternate OS scripts, download methods, websites, ports, Bot platforms, API-key locations, installer roles, or launch commands unless they already appear in recent output or authoritative context. Do not present shell scripts (.sh) as GUI-only actions unless that GUI flow was explicitly observed. Simplify by replacing commands with neutral documented-step wording, not by inventing easier-looking steps.\n- When shortening, reformatting, or asking for the final sentence/body, synthesize a complete standalone answer from the current task and recent output. Do not return only a heading, label, dangling fragment, or trailing sentence if that would drop required facts.\n- If the recent output is a clarification question and the new instruction only adds constraints without answering the missing slot, do not repeat the same clarification indefinitely. For low-risk writing or chat-only drafting tasks, produce a best-effort draft using a neutral, reasonable assumption. For file, code, command, system, credential, delivery, or other concrete-action tasks, keep clarifying instead of guessing.";
    match structured_patch {
        Some(patch) => format!(
            "Current task:\n{prior}{recent_output_block}{continuity_rules}\n\nStructured task updates:\n{patch}\n\n{merge_instruction}\nNew user instruction:\n{current}"
        ),
        None => format!(
            "Current task:\n{prior}{recent_output_block}{continuity_rules}\n\n{merge_instruction}\nNew user instruction:\n{current}"
        ),
    }
}

fn merged_replace_active_prompt(
    prior: &str,
    prior_output: Option<&str>,
    current: &str,
    structured_patch: Option<&str>,
) -> String {
    let recent_output_block = prior_output
        .map(|output| format!("\n\nMost recent generated output:\n{output}"))
        .unwrap_or_default();
    match structured_patch {
        Some(patch) => format!(
            "Previous task:\n{prior}{recent_output_block}\n\nStructured replacement details:\n{patch}\n\nDiscard that task and replace it with this new goal. Preserve the prior subject/topic unless the new instruction explicitly changes it, and treat the replacement as a deliverable/style update rather than a filesystem lookup unless the user explicitly asks to inspect files, code, or logs:\n{current}"
        ),
        None => format!(
            "Previous task:\n{prior}{recent_output_block}\n\nDiscard that task and replace it with this new goal. Preserve the prior subject/topic unless the new instruction explicitly changes it, and treat the replacement as a deliverable/style update rather than a filesystem lookup unless the user explicitly asks to inspect files, code, or logs:\n{current}"
        ),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn should_probe_transcript_for_clarify_fallback(
    prompt: &str,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    should_probe_transcript_for_clarify_fallback_with_surface(session_snapshot, &surface)
}

fn should_probe_transcript_for_clarify_fallback_with_surface(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    if session_snapshot
        .conversation_state
        .as_ref()
        .and_then(|state| state.last_primary_task_prompt.as_deref())
        .is_some_and(|prompt| !prompt.trim().is_empty())
    {
        return false;
    }
    if session_snapshot.active_clarify_state.is_some()
        || session_snapshot.active_followup_frame.is_some()
        || session_snapshot.active_observed_facts.is_some()
    {
        return false;
    }
    if surface.is_structural_locator_only_reply() {
        return true;
    }
    false
}

fn log_ask_memory_snapshot(
    task: &crate::ClaimedTask,
    long_term_log: &str,
    preferences_log: &str,
    trigger_log: &str,
    fact_log: &str,
    related_log: &str,
    recalled_count: usize,
    recalled_log: &str,
) {
    info!(
        "worker_once: ask memory task_id={} memory.long_term_summary={} memory.preferences={} memory.similar_triggers={} memory.relevant_facts={} memory.related_events={} memory.recalled_recent_count={} memory.recalled_recent={}",
        task.task_id,
        long_term_log,
        preferences_log,
        trigger_log,
        fact_log,
        related_log,
        recalled_count,
        recalled_log,
    );
}

pub(super) async fn prepare_ask_execution_context(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    route_result: &crate::RouteResult,
    resolved_prompt: &str,
) -> anyhow::Result<PreparedAskExecutionContext> {
    let chat_memory_budget_chars =
        crate::dynamic_chat_memory_budget_chars(state, task, resolved_prompt);
    let mut context_bundle = crate::task_context_builder::build_execution_task_context_bundle(
        state,
        task,
        route_result,
        resolved_prompt,
        chat_memory_budget_chars,
    );
    let execution_view = context_bundle
        .execution_view
        .as_ref()
        .expect("execution context bundle should include execution_view");
    let long_term_summary = execution_view.memory_ctx.long_term_summary.clone();
    let preferences = execution_view.memory_ctx.preferences.clone();
    let recalled = execution_view.memory_ctx.recalled.clone();
    let similar_triggers = execution_view.memory_ctx.similar_triggers.clone();
    let relevant_facts = execution_view.memory_ctx.relevant_facts.clone();
    let recent_related_events = execution_view.memory_ctx.recent_related_events.clone();
    let prompt_with_memory = execution_view.memory_ctx.prompt_with_memory.clone();
    let mut chat_prompt_context = execution_view.memory_ctx.chat_prompt_context.clone();
    let mut resolved_prompt_for_execution = resolved_prompt.to_string();
    let mut prompt_with_memory_for_execution = prompt_with_memory.clone();
    let recent_execution_context = execution_view.recent_execution_context.clone();
    if let Some(image_context) =
        crate::analyze_attached_images_for_ask(state, task, payload, resolved_prompt).await?
    {
        crate::task_context_builder::set_execution_image_context(
            &mut context_bundle,
            Some(image_context),
        );
    }
    crate::task_context_builder::apply_execution_context_to_prompts(
        &context_bundle,
        &mut chat_prompt_context,
        &mut resolved_prompt_for_execution,
        &mut prompt_with_memory_for_execution,
    );
    let long_term_log = long_term_summary
        .as_deref()
        .map(crate::truncate_for_log)
        .unwrap_or_else(|| "<none>".to_string());
    let recalled_log = if recalled.is_empty() {
        "<none>".to_string()
    } else {
        let merged = recalled
            .iter()
            .map(|(role, content)| format!("{role}:{content}"))
            .collect::<Vec<_>>()
            .join(" | ");
        crate::truncate_for_log(&merged)
    };
    let preferences_log = if preferences.is_empty() {
        "<none>".to_string()
    } else {
        let merged = preferences
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" | ");
        crate::truncate_for_log(&merged)
    };
    let trigger_log = if similar_triggers.is_empty() {
        "<none>".to_string()
    } else {
        crate::truncate_for_log(
            &similar_triggers
                .iter()
                .map(|v| v.text.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        )
    };
    let fact_log = if relevant_facts.is_empty() {
        "<none>".to_string()
    } else {
        crate::truncate_for_log(
            &relevant_facts
                .iter()
                .map(|v| v.text.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        )
    };
    let related_log = if recent_related_events.is_empty() {
        "<none>".to_string()
    } else {
        crate::truncate_for_log(
            &recent_related_events
                .iter()
                .map(|v| v.text.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        )
    };
    log_ask_memory_snapshot(
        task,
        &long_term_log,
        &preferences_log,
        &trigger_log,
        &fact_log,
        &related_log,
        recalled.len(),
        &recalled_log,
    );
    Ok(PreparedAskExecutionContext {
        context_bundle,
        chat_prompt_context,
        resolved_prompt_for_execution,
        prompt_with_memory_for_execution,
        memory_context_for_execution: prompt_with_memory,
        recent_execution_context,
    })
}

pub(super) async fn prepare_ask_input(
    _state: &AppState,
    _task: &crate::ClaimedTask,
    payload: &mut Value,
) -> PreparedAskInput {
    let prompt = payload
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let source = payload
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    PreparedAskInput { prompt, source }
}

pub(super) fn prepare_run_skill_input(payload: &Value) -> PreparedRunSkillInput {
    let skill_name = payload
        .get("skill_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let args = payload.get("args").cloned().unwrap_or_else(|| json!(""));
    PreparedRunSkillInput { skill_name, args }
}

pub(super) async fn maybe_finalize_schedule_direct_text_success(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
) -> anyhow::Result<bool> {
    let is_schedule_triggered = payload
        .get("schedule_triggered")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let schedule_task_mode = payload
        .get("schedule_task_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let schedule_force_agent = payload
        .get("schedule_force_agent")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let schedule_direct_text_mode = is_schedule_triggered
        && !schedule_force_agent
        && (schedule_task_mode.is_empty() || schedule_task_mode == "direct_text");
    if !schedule_direct_text_mode {
        return Ok(false);
    }
    let direct_text = prompt.trim();
    if direct_text.is_empty() {
        return Ok(false);
    }
    let answer_text = crate::intercept_response_text_for_delivery(direct_text);
    crate::finalize::finalize_ask_direct_success(
        state,
        task,
        payload,
        prompt,
        &answer_text,
        "schedule_direct_text",
        false,
        "",
    )
    .await?;
    Ok(true)
}

fn parse_clarify_state_response_shape(value: Option<&str>) -> Option<crate::OutputResponseShape> {
    match value?.trim() {
        "free" => Some(crate::OutputResponseShape::Free),
        "one_sentence" => Some(crate::OutputResponseShape::OneSentence),
        "scalar" => Some(crate::OutputResponseShape::Scalar),
        "file_token" => Some(crate::OutputResponseShape::FileToken),
        "strict" => Some(crate::OutputResponseShape::Strict),
        _ => None,
    }
}

fn parse_clarify_state_semantic_kind(value: Option<&str>) -> Option<crate::OutputSemanticKind> {
    match value?.trim() {
        "content_excerpt_summary" => Some(crate::OutputSemanticKind::ContentExcerptSummary),
        "content_excerpt_with_summary" => {
            Some(crate::OutputSemanticKind::ContentExcerptWithSummary)
        }
        "scalar_path_only" => Some(crate::OutputSemanticKind::ScalarPathOnly),
        "raw_command_output" => Some(crate::OutputSemanticKind::RawCommandOutput),
        "file_names" => Some(crate::OutputSemanticKind::FileNames),
        "directory_names" => Some(crate::OutputSemanticKind::DirectoryNames),
        "directory_entry_groups" => Some(crate::OutputSemanticKind::DirectoryEntryGroups),
        "file_paths" => Some(crate::OutputSemanticKind::FilePaths),
        "existence_with_path" => Some(crate::OutputSemanticKind::ExistenceWithPath),
        "existence_with_path_summary" => Some(crate::OutputSemanticKind::ExistenceWithPathSummary),
        "hidden_entries_check" => Some(crate::OutputSemanticKind::HiddenEntriesCheck),
        "execution_failed_step" => Some(crate::OutputSemanticKind::ExecutionFailedStep),
        "generated_file_delivery" => Some(crate::OutputSemanticKind::GeneratedFileDelivery),
        "recent_scalar_equality_check" => {
            Some(crate::OutputSemanticKind::RecentScalarEqualityCheck)
        }
        "git_commit_subject" => Some(crate::OutputSemanticKind::GitCommitSubject),
        "git_repository_state"
        | "git_workspace_state"
        | "git_state"
        | "git_status"
        | "git_branch"
        | "git_current_branch"
        | "git_remote"
        | "git_changed_files" => Some(crate::OutputSemanticKind::GitRepositoryState),
        "structured_keys" => Some(crate::OutputSemanticKind::StructuredKeys),
        "config_validation" | "structured_config_validation" => {
            Some(crate::OutputSemanticKind::ConfigValidation)
        }
        "config_mutation" | "config_write" | "config_set" | "structured_config_mutation" => {
            Some(crate::OutputSemanticKind::ConfigMutation)
        }
        "config_risk_assessment" | "config_risk" | "structured_config_risk" | "config_guard" => {
            Some(crate::OutputSemanticKind::ConfigRiskAssessment)
        }
        "sqlite_table_listing" => Some(crate::OutputSemanticKind::SqliteTableListing),
        "sqlite_table_names_only" => Some(crate::OutputSemanticKind::SqliteTableNamesOnly),
        "sqlite_database_kind_judgment" => {
            Some(crate::OutputSemanticKind::SqliteDatabaseKindJudgment)
        }
        "sqlite_schema_version" => Some(crate::OutputSemanticKind::SqliteSchemaVersion),
        "web_page_summary" => Some(crate::OutputSemanticKind::WebPageSummary),
        "web_search_summary" => Some(crate::OutputSemanticKind::WebSearchSummary),
        "weather_query" => Some(crate::OutputSemanticKind::WeatherQuery),
        "market_quote" => Some(crate::OutputSemanticKind::MarketQuote),
        "image_understanding" => Some(crate::OutputSemanticKind::ImageUnderstanding),
        "publishing_preview" => Some(crate::OutputSemanticKind::PublishingPreview),
        "archive_list" => Some(crate::OutputSemanticKind::ArchiveList),
        "archive_read" => Some(crate::OutputSemanticKind::ArchiveRead),
        "archive_pack" => Some(crate::OutputSemanticKind::ArchivePack),
        "archive_unpack" => Some(crate::OutputSemanticKind::ArchiveUnpack),
        "docker_ps" => Some(crate::OutputSemanticKind::DockerPs),
        "docker_images" => Some(crate::OutputSemanticKind::DockerImages),
        "docker_logs" => Some(crate::OutputSemanticKind::DockerLogs),
        "docker_container_lifecycle" => Some(crate::OutputSemanticKind::DockerContainerLifecycle),
        "service_status" => Some(crate::OutputSemanticKind::ServiceStatus),
        "package_manager_detection" => Some(crate::OutputSemanticKind::PackageManagerDetection),
        _ => None,
    }
}

fn route_requests_file_delivery(route_result: &crate::RouteResult) -> bool {
    route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
}

fn file_delivery_has_concrete_locator(route_result: &crate::RouteResult) -> bool {
    !route_result.output_contract.locator_hint.trim().is_empty()
        || matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
        )
}

fn generated_file_delivery_can_choose_target(route_result: &crate::RouteResult) -> bool {
    route_requests_file_delivery(route_result)
        && route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::GeneratedFileDelivery
        && route_result.output_contract.delivery_intent == crate::OutputDeliveryIntent::FileSingle
        && route_result.output_contract.response_shape == crate::OutputResponseShape::FileToken
}

fn normalize_output_shape_text(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn json_value_requests_filename_only_output(value: &Value) -> bool {
    match value {
        Value::String(text) => matches!(
            normalize_output_shape_text(text).as_str(),
            "filename"
                | "file_name"
                | "basename"
                | "filename_only"
                | "file_name_only"
                | "basename_only"
        ),
        Value::Array(items) => items.iter().any(json_value_requests_filename_only_output),
        Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                normalize_output_shape_text(key).as_str(),
                "output_format" | "output_shape" | "format" | "answer_format" | "delivery_format"
            ) && json_value_requests_filename_only_output(value)
        }),
        _ => false,
    }
}

fn turn_analysis_requests_filename_only_output(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    turn_analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(json_value_requests_filename_only_output)
}

fn clear_file_delivery_contract_for_filename_only(
    route_result: &mut crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) {
    if !turn_analysis_requests_filename_only_output(turn_analysis) {
        return;
    }
    route_result.wants_file_delivery = false;
    route_result.output_contract.delivery_required = false;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    if matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    ) {
        route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    }
    route_result
        .route_reason
        .push_str("; filename_only_output_clears_file_delivery_contract");
}

fn json_usize(value: &Value) -> Option<usize> {
    value
        .as_u64()
        .and_then(|raw| usize::try_from(raw).ok())
        .or_else(|| value.as_i64().and_then(|raw| usize::try_from(raw).ok()))
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
}

fn ordered_entry_index_from_state_patch(
    state_patch: Option<&Value>,
    frame: &crate::followup_frame::FollowupFrame,
) -> Option<usize> {
    let len = frame.ordered_entries.len();
    if len == 0 {
        return None;
    }
    let reference = state_patch?
        .get("ordered_entry_ref")
        .or_else(|| state_patch?.get("ordered_entry_reference"))?;
    let reference = reference.as_object()?;
    if let Some(index_value) = reference.get("index") {
        let index = json_usize(index_value)?;
        let index_base = reference
            .get("index_base")
            .and_then(json_usize)
            .unwrap_or(1);
        let zero_based_index = index.checked_sub(index_base)?;
        return (zero_based_index < len).then_some(zero_based_index);
    }

    let offset = reference
        .get("relative_offset")
        .or_else(|| reference.get("offset_from_selected"))
        .and_then(json_i64)?;
    let selected = i64::try_from(frame.selected_entry_index?).ok()?;
    let target = selected.checked_add(offset)?;
    usize::try_from(target).ok().filter(|index| *index < len)
}

fn ordered_entry_state_patch(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<&Value> {
    turn_analysis.and_then(|analysis| analysis.state_patch.as_ref())
}

fn has_ordered_entry_state_patch(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    ordered_entry_state_patch(turn_analysis).is_some_and(|state_patch| {
        state_patch.get("ordered_entry_ref").is_some()
            || state_patch.get("ordered_entry_reference").is_some()
    })
}

fn ordered_entry_reference_from_active_frame_index(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    index: usize,
) -> bool {
    let Some(frame) = session_snapshot.active_followup_frame.as_ref() else {
        return false;
    };
    let Some(target) = crate::followup_frame::ordered_entry_target_at(frame, index) else {
        return false;
    };
    if target.trim().is_empty() {
        return false;
    }
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = target.clone();
    if route_result.route_reason.trim().is_empty() {
        route_result.route_reason = "ordered_entry_reference_bound_from_active_frame".to_string();
    } else if !route_result
        .route_reason
        .contains("ordered_entry_reference_bound_from_active_frame")
    {
        route_result
            .route_reason
            .push_str("; ordered_entry_reference_bound_from_active_frame");
    }
    if route_result.resolved_intent.trim().is_empty() {
        route_result.resolved_intent = format!("Use ordered entry {}: {target}", index + 1);
    } else if !route_result.resolved_intent.contains(&target) {
        route_result
            .resolved_intent
            .push_str(&format!("\nordered_entry_target: {target}"));
    }
    true
}

fn bind_ordered_entry_reference_from_active_frame(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let supported_ordered_entry_contract = route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || (route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar
            && route_result.output_contract.semantic_kind
                == crate::OutputSemanticKind::ScalarPathOnly);
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !supported_ordered_entry_contract
        || !has_ordered_entry_state_patch(turn_analysis)
    {
        return false;
    }
    let Some(frame) = session_snapshot.active_followup_frame.as_ref() else {
        return false;
    };
    let Some(index) =
        ordered_entry_index_from_state_patch(ordered_entry_state_patch(turn_analysis), frame)
    else {
        return false;
    };
    ordered_entry_reference_from_active_frame_index(route_result, session_snapshot, index)
}

fn active_read_bound_target(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> Option<String> {
    session_snapshot
        .active_observed_facts
        .as_ref()
        .and_then(|facts| facts.bound_target.as_deref())
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            session_snapshot
                .active_followup_frame
                .as_ref()
                .filter(|frame| {
                    matches!(frame.op_kind, crate::followup_frame::FollowupOpKind::Read)
                })
                .and_then(|frame| frame.bound_target.as_deref())
                .map(str::trim)
                .filter(|target| !target.is_empty())
                .map(ToString::to_string)
        })
}

fn bind_structural_file_delivery_to_recent_read_target(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if !route_requests_file_delivery(route_result)
        || route_result.needs_clarify
        || file_delivery_has_concrete_locator(route_result)
    {
        return false;
    }
    let Some(bound_target) = active_read_bound_target(session_snapshot) else {
        return false;
    };
    route_result.needs_clarify = false;
    route_result.set_first_layer_decision(crate::FirstLayerDecision::PlannerExecute);
    if route_result.resolved_intent.trim().is_empty() {
        route_result.resolved_intent = format!("file_delivery_target: {bound_target}");
    } else if !route_result.resolved_intent.contains(&bound_target) {
        route_result
            .resolved_intent
            .push_str(&format!("\nfile_delivery_target: {bound_target}"));
    }
    route_result.clarify_question.clear();
    route_result.wants_file_delivery = true;
    route_result.output_contract.delivery_required = true;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route_result.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route_result.output_contract.requires_content_evidence = false;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route_result.output_contract.locator_hint = bound_target;
    route_result
        .route_reason
        .push_str("; structural_file_delivery_bound_to_recent_read_target");
    true
}

fn force_unresolved_file_delivery_clarify(route_result: &mut crate::RouteResult) {
    route_result.needs_clarify = true;
    route_result.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route_result.clarify_question = "请提供要发送的文件路径或文件名。".to_string();
    route_result.wants_file_delivery = false;
    route_result.output_contract.delivery_required = false;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route_result.output_contract.response_shape = crate::OutputResponseShape::Free;
    route_result.output_contract.requires_content_evidence = false;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route_result.output_contract.locator_hint.clear();
    route_result
        .route_reason
        .push_str("; unresolved_file_delivery_requires_clarify");
}

fn allow_generated_file_delivery_without_locator(route_result: &mut crate::RouteResult) {
    route_result.needs_clarify = false;
    if route_result.is_clarify_gate() {
        route_result.set_first_layer_decision(crate::FirstLayerDecision::PlannerExecute);
    }
    route_result.clarify_question.clear();
    route_result.wants_file_delivery = true;
    route_result.output_contract.delivery_required = true;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route_result.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route_result.output_contract.requires_content_evidence = true;
    if matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    ) {
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    }
    route_result
        .route_reason
        .push_str("; generated_file_delivery_allows_runtime_target");
}

fn repair_structural_file_delivery_resolution(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) {
    if !route_requests_file_delivery(route_result) {
        return;
    }
    if generated_file_delivery_can_choose_target(route_result) {
        allow_generated_file_delivery_without_locator(route_result);
        return;
    }
    if file_delivery_has_concrete_locator(route_result) {
        return;
    }
    if bind_structural_file_delivery_to_recent_read_target(route_result, session_snapshot) {
        return;
    }
    force_unresolved_file_delivery_clarify(route_result);
}

fn preserve_active_clarify_output_contract_for_locator_reply(
    route_result: &mut crate::RouteResult,
    clarify_followup_resolution: &crate::intent::continuation_resolver::ClarifyFollowupResolution,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) {
    let crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(hit) =
        clarify_followup_resolution
    else {
        return;
    };
    if active_clarify_locator_reply_is_structured_payload(&hit.current_user_text) {
        return;
    }
    let Some(clarify_state) = session_snapshot.active_clarify_state.as_ref() else {
        return;
    };
    if hit.prior_user_text.trim() != clarify_state.source_request.trim() {
        return;
    }
    let prior_shape = parse_clarify_state_response_shape(clarify_state.output_shape.as_deref());
    let prior_semantic = parse_clarify_state_semantic_kind(clarify_state.semantic_kind.as_deref());
    let prior_requested_file_delivery = clarify_state.delivery_required
        || matches!(prior_shape, Some(crate::OutputResponseShape::FileToken));
    if prior_requested_file_delivery {
        return;
    }
    if prior_shape.is_none() && prior_semantic.is_none() {
        return;
    }

    let current_requested_file_delivery = route_requests_file_delivery(route_result);
    if current_requested_file_delivery
        && !prior_non_file_contract_should_override_current_file_delivery(
            prior_shape,
            prior_semantic,
        )
    {
        route_result
            .route_reason
            .push_str("; keep_current_file_delivery_over_weak_active_clarify_shape");
        return;
    }
    if current_requested_file_delivery {
        route_result.wants_file_delivery = false;
        route_result.output_contract.delivery_required = false;
        route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    }
    if let Some(shape) =
        prior_shape.filter(|shape| !matches!(shape, crate::OutputResponseShape::FileToken))
    {
        route_result.output_contract.response_shape = shape;
    } else if current_requested_file_delivery {
        route_result.output_contract.response_shape = crate::OutputResponseShape::Free;
    }
    if let Some(semantic) = prior_semantic {
        route_result.output_contract.semantic_kind = semantic;
    } else if prior_shape.is_some()
        && !current_requested_file_delivery
        && route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None
    {
        route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route_result
            .route_reason
            .push_str("; drop_untrusted_locator_reply_semantic_kind");
    }
    route_result
        .route_reason
        .push_str("; preserve_active_clarify_output_contract");
}

fn active_clarify_locator_reply_is_structured_payload(text: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(text);
    surface.inline_json_shape.is_some()
        || crate::intent::surface_signals::inline_csv_record_block(text).is_some()
}

fn prior_non_file_contract_should_override_current_file_delivery(
    prior_shape: Option<crate::OutputResponseShape>,
    prior_semantic: Option<crate::OutputSemanticKind>,
) -> bool {
    if prior_semantic.is_some() {
        return true;
    }
    matches!(
        prior_shape,
        Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Strict)
    )
}

fn structural_locator_kind_from_reply(locator: &str) -> crate::OutputLocatorKind {
    let trimmed = locator.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return crate::OutputLocatorKind::Url;
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return crate::OutputLocatorKind::Path;
    }
    crate::OutputLocatorKind::Filename
}

fn promote_active_clarify_locator_reply_to_execute(
    route_result: &mut crate::RouteResult,
    clarify_followup_resolution: &crate::intent::continuation_resolver::ClarifyFollowupResolution,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) {
    let crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(hit) =
        clarify_followup_resolution
    else {
        return;
    };
    let Some(clarify_state) = session_snapshot.active_clarify_state.as_ref() else {
        return;
    };
    if hit.prior_user_text.trim() != clarify_state.source_request.trim() {
        return;
    }
    let locator = hit.current_user_text.trim();
    if locator.is_empty() {
        return;
    }
    if active_clarify_locator_reply_is_structured_payload(locator) {
        return;
    }
    let already_executable = route_result.is_execute_gate() && !route_result.needs_clarify;
    if !already_executable {
        route_result.set_first_layer_decision(crate::FirstLayerDecision::PlannerExecute);
        route_result.needs_clarify = false;
        route_result.clarify_question.clear();
        route_result.resolved_intent = hit.resolved_intent.clone();
    }
    route_result.output_contract.locator_hint = locator.to_string();
    if clarify_state.delivery_required
        || matches!(
            parse_clarify_state_response_shape(clarify_state.output_shape.as_deref()),
            Some(crate::OutputResponseShape::FileToken)
        )
    {
        route_result.wants_file_delivery = true;
        route_result.output_contract.delivery_required = true;
        route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route_result.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route_result.output_contract.requires_content_evidence = false;
        route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    }
    if matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
    ) {
        route_result.output_contract.locator_kind = structural_locator_kind_from_reply(locator);
    }
    if route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None {
        route_result.output_contract.requires_content_evidence = true;
    }
    if let Some(pair) =
        archive_unpack_pair_from_active_clarify_locator_reply(locator, clarify_state)
    {
        route_result.wants_file_delivery = false;
        route_result.output_contract.delivery_required = false;
        route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
        route_result.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route_result.output_contract.requires_content_evidence = true;
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route_result.output_contract.locator_hint = pair;
        route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ArchiveUnpack;
        route_result
            .route_reason
            .push_str("; active_clarify_archive_unpack_pair_repaired");
    }
    route_result
        .route_reason
        .push_str("; active_clarify_locator_reply_execute");
}

fn archive_unpack_pair_from_active_clarify_locator_reply(
    current_locator: &str,
    clarify_state: &crate::clarify_state::ClarifyState,
) -> Option<String> {
    let archive = first_supported_archive_locator(current_locator)?;
    let destination = first_structural_non_archive_locator(&clarify_state.source_request)?;
    Some(format!("{} | {}", archive.trim(), destination.trim()))
}

fn first_supported_archive_locator(text: &str) -> Option<String> {
    crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        .into_iter()
        .map(|locator| locator.locator_hint)
        .find(|path| supported_archive_locator_path(path))
}

fn first_structural_non_archive_locator(text: &str) -> Option<String> {
    crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        .into_iter()
        .map(|locator| locator.locator_hint)
        .find(|path| {
            !supported_archive_locator_path(path) && archive_unpack_destination_path_like(path)
        })
}

fn supported_archive_locator_path(path: &str) -> bool {
    let path = path.trim().to_ascii_lowercase();
    path.ends_with(".zip") || path.ends_with(".tar.gz") || path.ends_with(".tgz")
}

fn archive_unpack_destination_path_like(path: &str) -> bool {
    let path = path.trim();
    if !(path.starts_with("./")
        || path.starts_with("../")
        || path.starts_with('/')
        || path.starts_with("~/")
        || path.contains('/')
        || path.contains('\\'))
    {
        return false;
    }
    !path_basename_looks_like_file(path)
}

fn path_basename_looks_like_file(path: &str) -> bool {
    let basename = path.trim().rsplit(['/', '\\']).next().unwrap_or("").trim();
    let Some((stem, ext)) = basename.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=16).contains(&ext.len())
        && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn route_reason_has_structural_marker(route_result: &crate::RouteResult, marker: &str) -> bool {
    route_result
        .route_reason
        .split(';')
        .map(str::trim)
        .any(|part| {
            part == marker
                || part
                    .rsplit_once(':')
                    .is_some_and(|(_, suffix)| suffix.trim() == marker)
        })
}

fn repair_scalar_field_value_contract_for_locator_reply(route_result: &mut crate::RouteResult) {
    if route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::StructuredKeys
        || !route_result.output_contract.requires_content_evidence
        || !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
    {
        return;
    }
    let marker_matches_field_value_request = [
        "contract_valid_minor_repair_fields_only",
        "single_path_field_extraction_semantic_kind_none_is_valid",
        "structured_field_selector_requires_scalar_value",
        "structured_keys_scalar_response_requires_field_value",
    ]
    .iter()
    .any(|marker| route_reason_has_structural_marker(route_result, marker));
    if !marker_matches_field_value_request {
        return;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route_result
        .route_reason
        .push_str("; scalar_field_value_contract_repair");
}

fn active_clarify_existing_workspace_locator_reply(
    workspace_root: &Path,
    default_search_dir: &Path,
    prompt: &str,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> Option<crate::intent::continuation_resolver::ClarifyFollowupResolution> {
    let clarify_state = session_snapshot.active_clarify_state.as_ref()?;
    if clarify_state.missing_slot != crate::clarify_state::ClarifyMissingSlot::Locator {
        return None;
    }
    if !active_clarify_state_has_structural_binding_contract(clarify_state) {
        return None;
    }
    let locator = prompt.trim();
    if !minimal_locator_reply_candidate(locator) {
        return None;
    }
    let Some(resolved_locator) =
        resolve_existing_workspace_locator_candidate(workspace_root, default_search_dir, locator)
    else {
        return None;
    };
    Some(
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent: format!(
                    "Continue the previous request that was waiting for clarification: {}\nUser now provides the missing target/content: {}",
                    clarify_state.source_request.trim(),
                    resolved_locator
                ),
                prior_user_text: clarify_state.source_request.trim().to_string(),
                current_user_text: resolved_locator,
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        ),
    )
}

fn active_clarify_state_has_structural_binding_contract(
    clarify_state: &crate::clarify_state::ClarifyState,
) -> bool {
    clarify_state.delivery_required
        || clarify_state.output_shape.is_some()
        || clarify_state.semantic_kind.is_some()
        || !clarify_state.candidate_targets.is_empty()
        || (!clarify_state.source_request.trim().is_empty()
            && !clarify_state.pending_question.trim().is_empty())
}

fn minimal_locator_reply_candidate(locator: &str) -> bool {
    let trimmed = locator.trim();
    !trimmed.is_empty()
        && !trimmed.contains('\n')
        && trimmed.chars().count() <= 260
        && trimmed.split_whitespace().count() <= 1
        && !trimmed.contains("://")
        && !trimmed.chars().any(|ch| matches!(ch, '*' | '?' | '|'))
}

fn resolve_existing_workspace_locator_candidate(
    workspace_root: &Path,
    default_search_dir: &Path,
    locator: &str,
) -> Option<String> {
    let candidate = Path::new(locator.trim());
    if candidate.is_absolute() {
        return candidate.exists().then(|| locator.trim().to_string());
    }
    if [
        default_search_dir.join(candidate),
        workspace_root.join(candidate),
    ]
    .into_iter()
    .any(|path| path.exists())
    {
        return Some(locator.trim().to_string());
    }
    unique_workspace_basename_match(workspace_root, candidate)
        .and_then(|path| workspace_relative_display_path(workspace_root, &path))
}

fn unique_workspace_basename_match(workspace_root: &Path, candidate: &Path) -> Option<PathBuf> {
    let name = candidate.file_name()?.to_str()?.trim();
    if name.is_empty() || candidate.components().count() != 1 || matches!(name, "." | "..") {
        return None;
    }
    const MAX_VISITED: usize = 20_000;
    let mut visited = 0usize;
    let mut matches = Vec::new();
    let mut queue = VecDeque::from([workspace_root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        visited += 1;
        if visited > MAX_VISITED || matches.len() > 1 {
            break;
        }
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if file_name == ".git" || file_name == "target" {
                continue;
            }
            if file_name == name {
                matches.push(path.clone());
                if matches.len() > 1 {
                    break;
                }
            }
            if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                queue.push_back(path);
            }
        }
    }
    (matches.len() == 1).then(|| matches.remove(0))
}

fn workspace_relative_display_path(workspace_root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(workspace_root)
        .ok()
        .and_then(|relative| relative.to_str())
        .map(|value| value.replace('\\', "/"))
        .filter(|value| !value.trim().is_empty())
}

pub(super) async fn prepare_ask_routing(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    source: &str,
) -> PreparedAskRouting {
    let agent_mode = payload
        .get("agent_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let is_resume_continue = super::is_resume_continue_source(source);
    let (now_iso, timezone_str, schedule_rules) =
        schedule_service::schedule_context_for_normalizer(state);
    let session_snapshot = crate::conversation_state::load_active_session_snapshot(state, task);
    let routed_prompt = prompt.to_string();
    let routed_prompt_surface =
        crate::intent::surface_signals::analyze_prompt_surface(&routed_prompt);
    let mut clarify_followup_resolution =
        crate::intent::continuation_resolver::resolve_clarify_followup_from_session_with_surface(
            &routed_prompt,
            None,
            Some(&session_snapshot),
            &routed_prompt_surface,
        );
    if matches!(
        clarify_followup_resolution,
        crate::intent::continuation_resolver::ClarifyFollowupResolution::None
    ) {
        if let Some(resolution) = active_clarify_existing_workspace_locator_reply(
            &state.skill_rt.workspace_root,
            &state.skill_rt.default_locator_search_dir,
            &routed_prompt,
            &session_snapshot,
        ) {
            clarify_followup_resolution = resolution;
        }
    }
    if matches!(
        clarify_followup_resolution,
        crate::intent::continuation_resolver::ClarifyFollowupResolution::None
    ) && should_probe_transcript_for_clarify_fallback_with_surface(
        &session_snapshot,
        &routed_prompt_surface,
    ) {
        let built_last_turn_full = crate::memory::build_last_turn_full_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            400,
            1200,
        );
        clarify_followup_resolution =
            crate::intent::continuation_resolver::resolve_clarify_followup_from_session_with_surface(
                &routed_prompt,
                Some(&built_last_turn_full),
                Some(&session_snapshot),
                &routed_prompt_surface,
            );
    }
    let normalizer_prompt = match &clarify_followup_resolution {
        crate::intent::continuation_resolver::ClarifyFollowupResolution::NormalizerRewrite {
            rewritten_prompt,
            ..
        } => rewritten_prompt.clone(),
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            hit,
        ) => {
            crate::clarify_followup::emit_clarify_rewrite_event(&task.task_id, hit);
            info!(
                "{} worker_once: ask clarify_locator_reply_rewrite task_id={} reason={} normalizer_rewrite=true",
                crate::highlight_tag("routing"),
                task.task_id,
                hit.reason.as_metric_label()
            );
            hit.resolved_intent.clone()
        }
        _ => routed_prompt.clone(),
    };
    let explicit_resume_binding =
        crate::intent::resume_policy::explicit_resume_context_binding(payload, is_resume_continue);
    let recent_failed_resume_binding = crate::intent::resume_policy::recent_failed_resume_candidate(
        state,
        task,
        explicit_resume_binding.is_some(),
    );
    let resume_binding = explicit_resume_binding
        .clone()
        .or_else(|| recent_failed_resume_binding.clone());
    let binding_context_value = crate::intent::resume_policy::binding_context_json(
        source,
        is_resume_continue,
        resume_binding.as_ref(),
    );
    let normalizer_out = crate::intent_router::run_intent_normalizer(
        state,
        task,
        &normalizer_prompt,
        Some(&session_snapshot),
        resume_binding
            .as_ref()
            .map(|binding| &binding.resume_context),
        Some(&binding_context_value),
        &now_iso,
        &timezone_str,
        &schedule_rules,
    )
    .await;
    // Phase 0.4: 若 normalizer 已给出 schedule_intent，缓存起来，后续
    // `schedule.compile` 技能可以直接复用，避免对同一段文本再跑一次
    // `schedule_intent_prompt` LLM 调用。
    if let Some(intent) = normalizer_out.schedule_intent.as_ref() {
        state.cache_task_schedule_intent(&task.task_id, &normalizer_prompt, intent);
    }
    let turn_analysis = normalizer_out.turn_analysis.clone();
    let clarify_fallback_source = normalizer_out.fallback_source;
    let mut execution_recipe_hint = normalizer_out.execution_recipe_hint;
    let mut route_result =
        crate::intent_router::route_result_from_normalizer(state, task, &normalizer_out);
    preserve_active_clarify_output_contract_for_locator_reply(
        &mut route_result,
        &clarify_followup_resolution,
        &session_snapshot,
    );
    promote_active_clarify_locator_reply_to_execute(
        &mut route_result,
        &clarify_followup_resolution,
        &session_snapshot,
    );
    repair_scalar_field_value_contract_for_locator_reply(&mut route_result);
    clear_file_delivery_contract_for_filename_only(&mut route_result, turn_analysis.as_ref());
    bind_ordered_entry_reference_from_active_frame(
        &mut route_result,
        &session_snapshot,
        turn_analysis.as_ref(),
    );
    repair_structural_file_delivery_resolution(&mut route_result, &session_snapshot);
    let resume_runtime_binding = crate::intent::resume_policy::select_resume_runtime_binding(
        &route_result,
        resume_binding.as_ref(),
    );
    info!(
        "worker_once: ask raw_message task_id={} user_id={} chat_id={} text={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        crate::truncate_for_log(prompt)
    );
    let resume_runtime = crate::intent::resume_policy::resolve_resume_runtime_prompt(
        state,
        task,
        payload,
        prompt,
        &route_result,
        resume_runtime_binding,
    );
    let mut runtime_prompt = resume_runtime.runtime_prompt;
    if should_apply_task_turn_merge(&clarify_followup_resolution) {
        let (merge_prior_prompt, merge_prior_output) =
            task_turn_merge_prior_context(&session_snapshot);
        if let Some(merged_prompt) = merged_prompt_from_task_turn_analysis(
            merge_prior_prompt,
            merge_prior_output,
            prompt,
            turn_analysis.as_ref(),
        ) {
            info!(
            "{} worker_once: ask task_turn_merge task_id={} turn_type={:?} target_task_policy={:?} merged_prompt={}",
            crate::highlight_tag("routing"),
            task.task_id,
            turn_analysis.as_ref().and_then(|analysis| analysis.turn_type),
            turn_analysis
                .as_ref()
                .and_then(|analysis| analysis.target_task_policy),
            crate::truncate_for_log(&merged_prompt)
        );
            runtime_prompt = merged_prompt;
            route_result.resolved_intent = runtime_prompt.clone();
        }
    }
    if let Some(clarify_control_prompt) = active_clarify_run_control_prompt(
        &route_result,
        turn_analysis.as_ref(),
        &session_snapshot,
        prompt,
    ) {
        info!(
            "{} worker_once: ask active_clarify_run_control_prompt task_id={} prompt={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(&clarify_control_prompt)
        );
        runtime_prompt = clarify_control_prompt;
        route_result.resolved_intent = runtime_prompt.clone();
    }
    info!(
        "worker_once: ask received_message task_id={} user_id={} chat_id={} text={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        crate::truncate_for_log(&runtime_prompt)
    );
    let context_resolution = crate::intent_router::ContextResolution {
        resolved_user_intent: runtime_prompt.clone(),
        needs_clarify: route_result.needs_clarify,
        confidence: route_result.route_confidence,
        reason: route_result.route_reason.clone(),
    };
    let resolved_prompt = context_resolution.resolved_user_intent.clone();
    if route_result.needs_clarify || !route_result.is_execute_gate() {
        execution_recipe_hint = None;
    }
    crate::intent::safety_class::apply_route_risk_ceiling(
        &mut route_result,
        execution_recipe_hint.as_ref(),
    );
    info!(
        "{} worker_once: ask resolved_message task_id={} needs_clarify={} confidence={} reason={} resolved_text={}",
        crate::highlight_tag("routing"),
        task.task_id,
        context_resolution.needs_clarify,
        context_resolution.confidence.unwrap_or(-1.0),
        crate::truncate_for_log(&context_resolution.reason),
        crate::truncate_for_log(&resolved_prompt)
    );
    if let Some(analysis) = turn_analysis.as_ref() {
        info!(
            "{} worker_once: ask turn_analysis task_id={} turn_type={:?} target_task_policy={:?} should_interrupt_active_run={} has_state_patch={} attachment_processing_required={}",
            crate::highlight_tag("routing"),
            task.task_id,
            analysis.turn_type,
            analysis.target_task_policy,
            analysis.should_interrupt_active_run,
            analysis.state_patch.is_some(),
            analysis.attachment_processing_required
        );
    }
    let ask_mode = route_result.ask_mode.clone().with_resume_overrides(
        resume_runtime.should_discuss_context,
        resume_runtime.should_apply_context,
    );
    // When resume flags do not override the route, RouteResult should already
    // carry the normalized ask_mode. Route labels are derived only for logs.
    if !resume_runtime.should_discuss_context && !resume_runtime.should_apply_context {
        debug_assert_eq!(
            ask_mode,
            route_result.ask_mode,
            "prepared ask_mode should come from normalized RouteResult when no resume flag dominates"
        );
    }
    PreparedAskRouting {
        route_result,
        execution_recipe_hint,
        turn_analysis,
        clarify_fallback_source,
        resolved_prompt,
        agent_mode,
        ask_mode,
    }
}

#[cfg(test)]
#[path = "ask_prepare_tests.rs"]
mod tests;

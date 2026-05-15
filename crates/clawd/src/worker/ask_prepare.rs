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
        "structured_keys" => Some(crate::OutputSemanticKind::StructuredKeys),
        "sqlite_table_listing" => Some(crate::OutputSemanticKind::SqliteTableListing),
        "sqlite_table_names_only" => Some(crate::OutputSemanticKind::SqliteTableNamesOnly),
        "sqlite_database_kind_judgment" => {
            Some(crate::OutputSemanticKind::SqliteDatabaseKindJudgment)
        }
        "sqlite_schema_version" => Some(crate::OutputSemanticKind::SqliteSchemaVersion),
        "archive_list" => Some(crate::OutputSemanticKind::ArchiveList),
        "archive_pack" => Some(crate::OutputSemanticKind::ArchivePack),
        "archive_unpack" => Some(crate::OutputSemanticKind::ArchiveUnpack),
        "docker_ps" => Some(crate::OutputSemanticKind::DockerPs),
        "docker_images" => Some(crate::OutputSemanticKind::DockerImages),
        "docker_logs" => Some(crate::OutputSemanticKind::DockerLogs),
        "docker_container_lifecycle" => Some(crate::OutputSemanticKind::DockerContainerLifecycle),
        "service_status" => Some(crate::OutputSemanticKind::ServiceStatus),
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
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || (!route_result.output_contract.requires_content_evidence
            && !route_result.output_contract.delivery_required)
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
    route_result
        .route_reason
        .push_str("; active_clarify_locator_reply_execute");
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
mod tests {
    use super::{
        active_clarify_existing_workspace_locator_reply, active_clarify_run_control_prompt,
        bind_ordered_entry_reference_from_active_frame, merged_prompt_from_task_turn_analysis,
        preserve_active_clarify_output_contract_for_locator_reply,
        promote_active_clarify_locator_reply_to_execute,
        repair_structural_file_delivery_resolution, should_apply_task_turn_merge,
        should_probe_transcript_for_clarify_fallback, task_turn_merge_prior_context,
    };

    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_root(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rustclaw_ask_prepare_{label}_{}_{}",
            std::process::id(),
            nonce
        ));
        std::fs::create_dir_all(&path).expect("temp root");
        path
    }

    #[test]
    fn binding_context_marks_recent_failed_candidate_without_mutating_source() {
        let binding = crate::intent::resume_policy::ResumeContextBinding {
            source: crate::intent::resume_policy::ResumeContextSource::RecentFailedCandidate,
            resume_context: json!({"resume_context_id":"ctx-1"}),
            failed_ts: Some(42),
            has_newer_successful_ask_after_failed_task: true,
        };
        let value =
            crate::intent::resume_policy::binding_context_json("manual", false, Some(&binding));
        assert_eq!(
            value.get("resume_context_source").and_then(|v| v.as_str()),
            Some("recent_failed_resume_candidate")
        );
        assert_eq!(
            value
                .get("is_resume_continue_source")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            value
                .get("has_newer_successful_ask_after_failed_task")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn task_turn_merge_prior_prefers_active_clarify_over_stale_primary_task() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some(
                    "读取 scripts/nl_tests/fixtures/device_local/package.json 的 name 字段"
                        .to_string(),
                ),
                last_primary_task_output: Some("rustclaw-nl-fixture".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "请提供要发送的文件路径或文件名。".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: true,
                output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
                semantic_kind: None,
                source_request: "把那个最大的发给我。".to_string(),
                source_task_id: "task-clarify".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_followup_frame: None,
            active_observed_facts: None,
        };

        let (prompt, output) = task_turn_merge_prior_context(&snapshot);

        assert_eq!(prompt, Some("把那个最大的发给我。"));
        assert_eq!(output, Some("请提供要发送的文件路径或文件名。"));
    }

    #[test]
    fn active_clarify_accepts_existing_workspace_child_as_locator_reply() {
        let root = make_temp_root("clarify_existing_child");
        std::fs::create_dir_all(root.join("scripts")).expect("scripts dir");
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "请提供具体目标或路径。".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: false,
                output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
                semantic_kind: None,
                source_request: "数一下那个目录里有多少个直接子项，只输出数字".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };

        let resolution =
            active_clarify_existing_workspace_locator_reply(&root, &root, "scripts", &snapshot)
                .expect("existing workspace child should fill locator clarify");

        match resolution {
            crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
                hit,
            ) => {
                assert_eq!(hit.current_user_text, "scripts");
                assert!(hit
                    .resolved_intent
                    .contains("数一下那个目录里有多少个直接子项，只输出数字"));
            }
            other => panic!("expected locator rewrite, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn active_clarify_existing_locator_reply_requires_existing_path() {
        let root = make_temp_root("clarify_missing_child");
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "Target?".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: false,
                output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
                semantic_kind: None,
                source_request: "Count that directory".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };

        assert!(active_clarify_existing_workspace_locator_reply(
            &root,
            &root,
            "missing_child",
            &snapshot
        )
        .is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn active_clarify_resolves_unique_nested_filename_reply() {
        let root = make_temp_root("clarify_unique_nested_file");
        std::fs::create_dir_all(root.join("scripts")).expect("scripts dir");
        std::fs::write(root.join("scripts").join("restart_once.sh"), "#!/bin/sh\n")
            .expect("fixture file");
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "Target?".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: false,
                output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
                semantic_kind: Some(
                    crate::OutputSemanticKind::ExistenceWithPath
                        .as_str()
                        .to_string(),
                ),
                source_request: "检查那个重启脚本在不在".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };

        let resolution = active_clarify_existing_workspace_locator_reply(
            &root,
            &root,
            "restart_once.sh",
            &snapshot,
        )
        .expect("unique nested filename should fill locator clarify");

        match resolution {
            crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
                hit,
            ) => {
                assert_eq!(hit.current_user_text, "scripts/restart_once.sh");
                assert!(hit.resolved_intent.contains("scripts/restart_once.sh"));
            }
            other => panic!("expected locator rewrite, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn active_clarify_accepts_locator_reply_without_explicit_output_contract() {
        let root = make_temp_root("clarify_plain_locator");
        std::fs::create_dir_all(root.join("scripts")).expect("scripts dir");
        std::fs::write(
            root.join("scripts").join("restart_clawd_latest.sh"),
            "#!/bin/sh\n",
        )
        .expect("fixture file");
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "请提供具体目标或路径。".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: false,
                output_shape: None,
                semantic_kind: None,
                source_request: "看看那个重启脚本在不在".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };

        let resolution = active_clarify_existing_workspace_locator_reply(
            &root,
            &root,
            "restart_clawd_latest.sh",
            &snapshot,
        )
        .expect("plain locator clarify should fill an existing unique workspace entry");

        match resolution {
            crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
                hit,
            ) => {
                assert_eq!(hit.current_user_text, "scripts/restart_clawd_latest.sh");
                assert!(hit.resolved_intent.contains("看看那个重启脚本在不在"));
            }
            other => panic!("expected locator rewrite, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn active_clarify_does_not_guess_ambiguous_nested_filename_reply() {
        let root = make_temp_root("clarify_ambiguous_nested_file");
        std::fs::create_dir_all(root.join("a")).expect("dir a");
        std::fs::create_dir_all(root.join("b")).expect("dir b");
        std::fs::write(root.join("a").join("same.md"), "a").expect("fixture a");
        std::fs::write(root.join("b").join("same.md"), "b").expect("fixture b");
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "Target?".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: false,
                output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
                semantic_kind: Some(
                    crate::OutputSemanticKind::ExistenceWithPath
                        .as_str()
                        .to_string(),
                ),
                source_request: "检查那个文件在不在".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };

        assert!(active_clarify_existing_workspace_locator_reply(
            &root, &root, "same.md", &snapshot
        )
        .is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn active_clarify_run_control_prompt_blocks_unrelated_alias_selection() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::direct_answer(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(0.9),
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
        let turn_analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::RunControl),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReplaceActive),
            should_interrupt_active_run: true,
            state_patch: None,
            attachment_processing_required: false,
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                    alias: "甲文件".to_string(),
                    target: "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
                        .to_string(),
                    updated_at_ts: 1,
                }],
                ..crate::conversation_state::ConversationState::default()
            }),
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "请提供要发送的文件路径或文件名。".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: true,
                output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
                semantic_kind: None,
                source_request: "把那个最大的发给我。".to_string(),
                source_task_id: "task-clarify".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_followup_frame: None,
            active_observed_facts: None,
        };

        let prompt = active_clarify_run_control_prompt(
            &route,
            Some(&turn_analysis),
            &snapshot,
            "停一下，不要发文件，改为只告诉我你需要我确认哪个文件。",
        )
        .expect("clarify control prompt");

        assert!(prompt.contains("Missing information to confirm"));
        assert!(prompt.contains("Candidate targets from that clarification only:\n<none>"));
        assert!(!prompt.contains("release_checklist.md"));
    }

    #[test]
    fn runtime_resume_binding_is_disabled_when_normalizer_rejects_resume() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "list current workspace".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        let binding = crate::intent::resume_policy::ResumeContextBinding {
            source: crate::intent::resume_policy::ResumeContextSource::RecentFailedCandidate,
            resume_context: json!({"resume_context_id":"ctx-2"}),
            failed_ts: Some(7),
            has_newer_successful_ask_after_failed_task: false,
        };
        assert!(crate::intent::resume_policy::select_resume_runtime_binding(
            &route,
            Some(&binding)
        )
        .is_none());
    }

    #[test]
    fn clarify_locator_reply_preserves_prior_content_excerpt_contract() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "读取文件最后 10 行并发送内容".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: true,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "/tmp/model_io.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let clarify_state = crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供日志路径".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: None,
            semantic_kind: Some(
                crate::OutputSemanticKind::ContentExcerptSummary
                    .as_str()
                    .to_string(),
            ),
            source_request: "看下那个最近 10 行".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(clarify_state),
            active_observed_facts: None,
        };
        let resolution =
            crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
                crate::clarify_followup::ClarifyLocatorReplyRewrite {
                    resolved_intent: "Continue...".to_string(),
                    prior_user_text: "看下那个最近 10 行".to_string(),
                    current_user_text: "/tmp/model_io.log".to_string(),
                    reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
                },
            );

        preserve_active_clarify_output_contract_for_locator_reply(
            &mut route,
            &resolution,
            &snapshot,
        );

        assert!(!route.wants_file_delivery);
        assert!(!route.output_contract.delivery_required);
        assert_eq!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        );
        assert_eq!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free
        );
        assert_eq!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ContentExcerptSummary
        );
        assert!(route
            .route_reason
            .contains("preserve_active_clarify_output_contract"));
    }

    #[test]
    fn clarify_locator_reply_promotes_bare_path_back_to_execution() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::clarify(),
            resolved_intent: "scripts/nl_tests/fixtures/device_local/logs/model_io.log".to_string(),
            needs_clarify: true,
            route_reason: "bare_path_no_verb".to_string(),
            route_confidence: Some(0.8),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: "What would you like me to do with the file?".to_string(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::None,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let clarify_state = crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供日志路径".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: None,
            semantic_kind: Some(
                crate::OutputSemanticKind::ContentExcerptSummary
                    .as_str()
                    .to_string(),
            ),
            source_request: "看看那个模型日志最后 5 行".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(clarify_state),
            active_observed_facts: None,
        };
        let resolution =
            crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
                crate::clarify_followup::ClarifyLocatorReplyRewrite {
                    resolved_intent:
                        "Continue the previous request that was waiting for clarification: 看看那个模型日志最后 5 行\nUser now provides the missing target/content: scripts/nl_tests/fixtures/device_local/logs/model_io.log"
                            .to_string(),
                    prior_user_text: "看看那个模型日志最后 5 行".to_string(),
                    current_user_text: "scripts/nl_tests/fixtures/device_local/logs/model_io.log"
                        .to_string(),
                    reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
                },
            );

        preserve_active_clarify_output_contract_for_locator_reply(
            &mut route,
            &resolution,
            &snapshot,
        );
        promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

        assert!(route.is_execute_gate());
        assert!(!route.needs_clarify);
        assert!(route.clarify_question.is_empty());
        assert_eq!(
            route.output_contract.locator_hint,
            "scripts/nl_tests/fixtures/device_local/logs/model_io.log"
        );
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
        assert_eq!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ContentExcerptSummary
        );
        assert!(route.output_contract.requires_content_evidence);
        assert!(route
            .route_reason
            .contains("active_clarify_locator_reply_execute"));
    }

    #[test]
    fn clarify_locator_reply_preserves_prior_file_delivery_contract() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::clarify(),
            resolved_intent: "README.md".to_string(),
            needs_clarify: true,
            route_reason: "bare_path_no_verb".to_string(),
            route_confidence: Some(0.8),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: "What file?".to_string(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "Which file?".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: true,
                output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
                semantic_kind: None,
                source_request: "Send me the file".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };
        let resolution =
            crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
                crate::clarify_followup::ClarifyLocatorReplyRewrite {
                    resolved_intent:
                        "Continue the previous request that was waiting for clarification: Send me the file\nUser now provides the missing target/content: README.md"
                            .to_string(),
                    prior_user_text: "Send me the file".to_string(),
                    current_user_text: "README.md".to_string(),
                    reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
                },
            );

        promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

        assert!(route.is_execute_gate());
        assert!(!route.needs_clarify);
        assert!(route.wants_file_delivery);
        assert!(route.output_contract.delivery_required);
        assert_eq!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::FileSingle
        );
        assert_eq!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        );
        assert_eq!(route.output_contract.locator_hint, "README.md");
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Filename
        );
    }

    #[test]
    fn clarify_locator_reply_injects_locator_into_existing_execute_route() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "read and deliver config file".to_string(),
            needs_clarify: false,
            route_reason: "semantic_contract_requires_evidence".to_string(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: crate::OutputSemanticKind::FilePaths,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
                ..Default::default()
            },
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "Which file?".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: true,
                output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
                semantic_kind: None,
                source_request: "Send that config file".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };
        let resolution =
            crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
                crate::clarify_followup::ClarifyLocatorReplyRewrite {
                    resolved_intent:
                        "Continue the previous request that was waiting for clarification: Send that config file\nUser now provides the missing target/content: /tmp/app_config.toml"
                            .to_string(),
                    prior_user_text: "Send that config file".to_string(),
                    current_user_text: "/tmp/app_config.toml".to_string(),
                    reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
                },
            );

        promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

        assert!(route.is_execute_gate());
        assert!(!route.needs_clarify);
        assert_eq!(route.output_contract.locator_hint, "/tmp/app_config.toml");
        assert_eq!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        );
        assert!(route.output_contract.delivery_required);
        assert_eq!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::FileSingle
        );
    }

    #[test]
    fn clarify_locator_reply_does_not_promote_stale_prior_request() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::clarify(),
            resolved_intent: "/tmp/a.log".to_string(),
            needs_clarify: true,
            route_reason: "bare_path_no_verb".to_string(),
            route_confidence: Some(0.8),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: "path?".to_string(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "path?".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: false,
                output_shape: None,
                semantic_kind: None,
                source_request: "上一轮请求".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };
        let resolution =
            crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
                crate::clarify_followup::ClarifyLocatorReplyRewrite {
                    resolved_intent: "Continue...".to_string(),
                    prior_user_text: "另一轮请求".to_string(),
                    current_user_text: "/tmp/a.log".to_string(),
                    reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
                },
            );

        promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

        assert_eq!(route.ask_mode, crate::AskMode::clarify());
        assert!(route.needs_clarify);
    }

    #[test]
    fn clarify_locator_reply_drops_untrusted_current_semantic_when_prior_only_shape() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "Continue the prior task using scripts".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::QuantityComparison,
                locator_hint: "scripts".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
                ..Default::default()
            },
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "Provide the missing target path.".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: false,
                output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
                semantic_kind: None,
                source_request: "Count direct children in the target directory.".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };
        let resolution =
            crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
                crate::clarify_followup::ClarifyLocatorReplyRewrite {
                    resolved_intent:
                        "Continue the previous request that was waiting for clarification."
                            .to_string(),
                    prior_user_text: "Count direct children in the target directory.".to_string(),
                    current_user_text: "scripts".to_string(),
                    reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
                },
            );

        preserve_active_clarify_output_contract_for_locator_reply(
            &mut route,
            &resolution,
            &snapshot,
        );

        assert_eq!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        );
        assert_eq!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
        );
        assert!(route.output_contract.requires_content_evidence);
        assert!(route
            .route_reason
            .contains("drop_untrusted_locator_reply_semantic_kind"));
        assert!(route
            .route_reason
            .contains("preserve_active_clarify_output_contract"));
    }

    #[test]
    fn clarify_locator_reply_preserves_prior_scalar_path_contract_without_delivery() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "在目录 fixtures/stem_unique 中查找 abcd".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "fixtures/stem_unique".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let clarify_state = crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供要搜索的目录或目标文件的具体路径。".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: Some(
                crate::OutputSemanticKind::ScalarPathOnly
                    .as_str()
                    .to_string(),
            ),
            source_request: "去那个 stem_unique 目录里找 abcd，只输出路径".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(clarify_state),
            active_observed_facts: None,
        };
        let resolution =
            crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
                crate::clarify_followup::ClarifyLocatorReplyRewrite {
                    resolved_intent: "Continue...".to_string(),
                    prior_user_text: "去那个 stem_unique 目录里找 abcd，只输出路径".to_string(),
                    current_user_text: "fixtures/stem_unique".to_string(),
                    reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
                },
            );

        preserve_active_clarify_output_contract_for_locator_reply(
            &mut route,
            &resolution,
            &snapshot,
        );

        assert!(!route.wants_file_delivery);
        assert!(!route.output_contract.delivery_required);
        assert_eq!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        );
        assert_eq!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ScalarPathOnly
        );
        assert!(route
            .route_reason
            .contains("preserve_active_clarify_output_contract"));
    }

    #[test]
    fn file_delivery_with_structured_locator_is_preserved() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "send the routed file".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: false,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "/tmp/model_io.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                source_request: "read the last 10 lines".to_string(),
                op_kind: crate::followup_frame::FollowupOpKind::Read,
                bound_target: Some("/tmp/model_io.log".to_string()),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
                ..Default::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };

        repair_structural_file_delivery_resolution(&mut route, &snapshot);

        assert!(!route.needs_clarify);
        assert!(route.is_execute_gate());
        assert!(route.wants_file_delivery);
        assert!(route.output_contract.delivery_required);
        assert_eq!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::FileSingle
        );
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
        assert_eq!(route.output_contract.locator_hint, "/tmp/model_io.log");
        assert!(route.clarify_question.is_empty());
    }

    #[test]
    fn unresolved_file_delivery_without_locator_requires_clarify() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "send the file".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: true,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::None,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        repair_structural_file_delivery_resolution(&mut route, &snapshot);

        assert!(route.needs_clarify);
        assert!(route.is_clarify_gate());
        assert!(!route.wants_file_delivery);
        assert!(!route.output_contract.delivery_required);
        assert_eq!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        );
        assert_eq!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        );
        assert!(route.clarify_question.contains("文件路径"));
        assert!(route
            .route_reason
            .contains("unresolved_file_delivery_requires_clarify"));
    }

    #[test]
    fn generated_file_delivery_without_locator_can_choose_runtime_target() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::clarify(),
            resolved_intent: "create a shell script, save it, and deliver the generated file"
                .to_string(),
            needs_clarify: true,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: "please provide a filename".to_string(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: true,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: crate::OutputSemanticKind::GeneratedFileDelivery,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        repair_structural_file_delivery_resolution(&mut route, &snapshot);

        assert!(!route.needs_clarify);
        assert!(route.is_execute_gate());
        assert!(route.wants_file_delivery);
        assert!(route.output_contract.delivery_required);
        assert_eq!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::GeneratedFileDelivery
        );
        assert_eq!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::FileSingle
        );
        assert!(route.clarify_question.is_empty());
        assert!(route
            .route_reason
            .contains("generated_file_delivery_allows_runtime_target"));
    }

    #[test]
    fn structurally_resolved_file_delivery_binds_recent_read_target_without_text_match() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "deliver the active file target".to_string(),
            needs_clarify: false,
            route_reason: "normalizer resolved delivery from immediate context".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: true,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::None,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                source_request: "read README.md head".to_string(),
                op_kind: crate::followup_frame::FollowupOpKind::Read,
                bound_target: Some("/tmp/README.md".to_string()),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
                ..Default::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };

        repair_structural_file_delivery_resolution(&mut route, &snapshot);

        assert!(!route.needs_clarify);
        assert!(route.is_execute_gate());
        assert!(route.wants_file_delivery);
        assert!(route.output_contract.delivery_required);
        assert_eq!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        );
        assert_eq!(
            route.output_contract.locator_hint,
            "/tmp/README.md".to_string()
        );
        assert!(route.resolved_intent.contains("/tmp/README.md"));
        assert!(route
            .route_reason
            .contains("structural_file_delivery_bound_to_recent_read_target"));
    }

    #[test]
    fn ordered_entry_reference_binds_third_delivery_from_active_frame() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "deliver the third listed file".to_string(),
            needs_clarify: false,
            route_reason: "normalizer selected an ordinal follow-up".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: true,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "/home/guagua/rustclaw/logs/clawd.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                source_request: "先列出 logs 目录下前 4 个文件名".to_string(),
                op_kind: crate::followup_frame::FollowupOpKind::List,
                bound_target: Some("logs".to_string()),
                ordered_entries: vec![
                    "act_plan.log".to_string(),
                    "clawd.log".to_string(),
                    "clawd.run.log".to_string(),
                    "clawd.test.log".to_string(),
                ],
                source_task_id: "task-list".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
                ..Default::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskAppend),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(json!({"ordered_entry_ref":{"index":3,"index_base":1}})),
            attachment_processing_required: false,
        };

        assert!(bind_ordered_entry_reference_from_active_frame(
            &mut route,
            &snapshot,
            Some(&analysis)
        ));

        assert_eq!(route.output_contract.locator_hint, "logs/clawd.run.log");
        assert!(route
            .route_reason
            .contains("ordered_entry_reference_bound_from_active_frame"));
        assert!(route.resolved_intent.contains("logs/clawd.run.log"));
    }

    #[test]
    fn ordered_entry_reference_binds_previous_from_selected_entry() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "read previous selected file tail".to_string(),
            needs_clarify: false,
            route_reason: "normalizer selected a relative ordinal follow-up".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "/home/guagua/rustclaw/logs/clawd.run.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                source_request: "把第三个发给我".to_string(),
                op_kind: crate::followup_frame::FollowupOpKind::Delivery,
                bound_target: Some("logs/clawd.run.log".to_string()),
                ordered_entries: vec![
                    "act_plan.log".to_string(),
                    "clawd.log".to_string(),
                    "clawd.run.log".to_string(),
                    "clawd.test.log".to_string(),
                ],
                selected_entry_index: Some(2),
                source_task_id: "task-delivery".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
                ..Default::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(json!({"ordered_entry_ref":{"relative_offset":-1}})),
            attachment_processing_required: false,
        };

        assert!(bind_ordered_entry_reference_from_active_frame(
            &mut route,
            &snapshot,
            Some(&analysis)
        ));

        assert_eq!(route.output_contract.locator_hint, "logs/clawd.log");
        assert!(route.resolved_intent.contains("logs/clawd.log"));
    }

    #[test]
    fn filename_only_output_patch_clears_file_delivery_contract() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "only output the basename of the previously delivered file"
                .to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: false,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "/tmp/README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskAppend),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(json!({"output_format": "filename_only"})),
            attachment_processing_required: false,
        };

        super::clear_file_delivery_contract_for_filename_only(&mut route, Some(&analysis));

        assert!(!route.wants_file_delivery);
        assert!(!route.output_contract.delivery_required);
        assert_eq!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        );
        assert_eq!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        );
        assert!(route
            .route_reason
            .contains("filename_only_output_clears_file_delivery_contract"));
    }

    #[test]
    fn immediate_last_turn_clarify_placeholder_is_detected() {
        assert!(crate::intent::continuation_resolver::immediate_prior_turn_was_clarify(
            "### LAST_TURN_FULL\n[TURN -1]\nUser: 读取待确认文件里的名字字段，只输出值\nAssistant: [clarification_requested]\n[/TURN]"
        ));
        assert!(!crate::intent::continuation_resolver::immediate_prior_turn_was_clarify(
            "### LAST_TURN_FULL\n[TURN -1]\nUser: 看看那个重启脚本在不在\nAssistant: 有，路径：scripts/restart_clawd_latest.sh\n[/TURN]"
        ));
    }

    #[test]
    fn transcript_probe_is_enabled_for_locator_only_reply_without_session_state() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(should_probe_transcript_for_clarify_fallback(
            "/tmp/device_local/logs/model_io.log",
            &snapshot,
        ));
    }

    #[test]
    fn transcript_probe_is_skipped_when_session_state_already_exists() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: false,
                output_shape: None,
                semantic_kind: None,
                source_request: "看一下那个日志最后 5 行".to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };
        assert!(!should_probe_transcript_for_clarify_fallback(
            "/tmp/device_local/logs/model_io.log",
            &snapshot,
        ));
    }

    #[test]
    fn transcript_probe_is_skipped_for_regular_new_request() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!should_probe_transcript_for_clarify_fallback(
            "读取 /tmp/device_local/logs/model_io.log 最后 5 行",
            &snapshot,
        ));
    }

    #[test]
    fn transcript_probe_is_skipped_when_primary_task_prompt_exists() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Help me write a proposal".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!should_probe_transcript_for_clarify_fallback(
            "It is for executives",
            &snapshot,
        ));
    }

    #[test]
    fn clarify_followup_routing_prompt_merges_previous_operation_for_non_locator_reply_target() {
        let merged = crate::intent::continuation_resolver::resolve_clarify_followup(
            "就在 scripts/restart_clawd_latest.sh",
            Some("[LAST_TURN_FULL]\nUser: 把那个重启脚本发给我\nAssistant: [clarification_requested]\n[/LAST_TURN_FULL]"),
            None,
            None,
            None,
        );
        match merged {
            crate::intent::continuation_resolver::ClarifyFollowupResolution::NormalizerRewrite {
                rewritten_prompt,
            } => {
                assert!(rewritten_prompt.contains("把那个重启脚本发给我"));
                assert!(rewritten_prompt.contains("就在 scripts/restart_clawd_latest.sh"));
            }
            other => panic!("expected normalizer rewrite, got {other:?}"),
        }
    }

    #[test]
    fn clarify_followup_routing_prompt_skips_unrelated_new_request() {
        assert!(matches!(
            crate::intent::continuation_resolver::resolve_clarify_followup(
                "今天天气怎么样",
                Some(
                    "[LAST_TURN_FULL]\nUser: 把那个 JSON 数组按 score 排一下并转成表格\nAssistant: [clarification_requested]\n[/LAST_TURN_FULL]"
                ),
                None,
                None,
                None,
            ),
            crate::intent::continuation_resolver::ClarifyFollowupResolution::None
        ));
    }

    #[test]
    fn clarify_followup_resolution_disables_active_task_merge() {
        let resolution =
            crate::intent::continuation_resolver::ClarifyFollowupResolution::NormalizerRewrite {
                rewritten_prompt:
                    "Continue the previous request that was waiting for clarification: 看看日志最后 5 行"
                        .to_string(),
            };
        assert!(!should_apply_task_turn_merge(&resolution));
        assert!(should_apply_task_turn_merge(
            &crate::intent::continuation_resolver::ClarifyFollowupResolution::None
        ));
    }

    #[test]
    fn task_append_merge_reuses_prior_primary_task_prompt() {
        let merged = merged_prompt_from_task_turn_analysis(
            Some("帮我写个方案"),
            None,
            "面向老板",
            Some(&crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskAppend),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
                should_interrupt_active_run: false,
                state_patch: Some(json!({"audience":"boss"})),
                attachment_processing_required: false,
            }),
        )
        .expect("merged prompt");
        assert!(merged.contains("帮我写个方案"));
        assert!(merged.contains("面向老板"));
        assert!(merged.contains("\"audience\":\"boss\""));
        assert!(merged.contains("append this new instruction"));
        assert!(merged.contains("Continuity rules"));
        assert!(merged.contains("Continuity does not preserve reply language"));
        assert!(merged.contains("do not preserve its question shape"));
        assert!(merged.contains("do not repeat the same clarification indefinitely"));
    }

    #[test]
    fn task_replace_merge_discards_prior_goal() {
        let merged = merged_prompt_from_task_turn_analysis(
            Some("别写长文，先做方案"),
            None,
            "算了，改成 X thread",
            Some(&crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskReplace),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReplaceActive),
                should_interrupt_active_run: false,
                state_patch: Some(json!({"deliverable":"thread"})),
                attachment_processing_required: false,
            }),
        )
        .expect("merged prompt");
        assert!(merged.contains("别写长文，先做方案"));
        assert!(merged.contains("算了，改成 X thread"));
        assert!(merged.contains("\"deliverable\":\"thread\""));
        assert!(merged.contains("replace it with this new goal"));
    }

    #[test]
    fn task_correct_merge_marks_conflicting_details_as_overrides() {
        let merged = merged_prompt_from_task_turn_analysis(
            Some("帮我写安装说明，面向 Python 3.10"),
            None,
            "不对，不是 Python 3.10，是 Python 3.11",
            Some(&crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
                should_interrupt_active_run: false,
                state_patch: Some(json!({"python_version":"3.11"})),
                attachment_processing_required: false,
            }),
        )
        .expect("merged prompt");
        assert!(merged.contains("Python 3.10"));
        assert!(merged.contains("Python 3.11"));
        assert!(merged.contains("\"python_version\":\"3.11\""));
        assert!(merged.contains("overrides conflicting earlier details"));
    }

    #[test]
    fn task_append_merge_includes_recent_generated_output_when_normalizer_reuses_active() {
        let merged = merged_prompt_from_task_turn_analysis(
            Some("Write one deployment note that mentions Python 3.11"),
            Some("Deployment note: use Python 3.11."),
            "Output only that sentence",
            Some(&crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskAppend),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
                should_interrupt_active_run: false,
                state_patch: None,
                attachment_processing_required: false,
            }),
        )
        .expect("merged prompt");
        assert!(merged.contains("Most recent generated output"));
        assert!(merged.contains("Deployment note: use Python 3.11."));
    }
}

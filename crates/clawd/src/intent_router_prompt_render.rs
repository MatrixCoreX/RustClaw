use tracing::{info, warn};

use crate::llm_gateway;

use super::{
    normalize_intent_normalizer_raw_for_schema_with_report, AppState, ClaimedTask,
    ContractRepairReport, IntentNormalizerOut, ROUTING_POLICY_PERSONA_PROMPT,
};

pub(super) fn render_intent_normalizer_prompt_for_route(
    state: &AppState,
    task: &ClaimedTask,
    route_view: &crate::task_context_builder::RouteContextView,
    context_bundle: &crate::task_context_builder::TaskContextBundle,
    prompt_template: &str,
    auth_policy_context: &str,
    self_extension_runtime: &str,
    request_language_hint: &str,
    req: &str,
) -> String {
    let max_prompt_bytes = intent_normalizer_max_prompt_bytes(state, task);
    let compact_prompt_required = intent_normalizer_uses_compact_prompt(state, task);
    let mut prompt = if compact_prompt_required {
        warn!(
            "intent_normalizer using compact prompt for small-context provider: task_id={}",
            task.task_id
        );
        render_compact_intent_normalizer_prompt(
            route_view,
            context_bundle,
            auth_policy_context,
            request_language_hint,
            req,
        )
    } else {
        crate::render_prompt_template(
            prompt_template,
            &[
                ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
                ("__AUTH_POLICY_CONTEXT__", auth_policy_context),
                ("__CAPABILITY_MAP__", &route_view.capability_map),
                ("__SELF_EXTENSION_RUNTIME__", self_extension_runtime),
                (
                    "__RESUME_CONTEXT__",
                    &context_bundle.raw_sources.resume_context,
                ),
                (
                    "__BINDING_CONTEXT__",
                    &context_bundle.raw_sources.binding_context,
                ),
                ("__ACTIVE_TASK_CONTEXT__", &route_view.active_task_context),
                (
                    "__ACTIVE_EXECUTION_ANCHOR__",
                    &route_view.active_execution_anchor_context,
                ),
                (
                    "__SESSION_ALIAS_CONTEXT__",
                    &route_view.session_alias_context,
                ),
                (
                    "__REQUEST_SURFACE_HINTS__",
                    &route_view.request_surface_hints,
                ),
                (
                    "__RECENT_EXECUTION_CONTEXT__",
                    &route_view.recent_execution_context,
                ),
                ("__MEMORY_CONTEXT__", &route_view.memory_context),
                ("__RECENT_TURNS_FULL__", &route_view.recent_turns_full),
                ("__LAST_TURN_FULL__", &route_view.last_turn_full),
                (
                    "__RECENT_ASSISTANT_REPLIES__",
                    &route_view.recent_assistant_replies,
                ),
                ("__NOW__", &context_bundle.raw_sources.now_iso),
                ("__TIMEZONE__", &context_bundle.raw_sources.timezone),
                (
                    "__SCHEDULE_RULES__",
                    &context_bundle.raw_sources.schedule_rules,
                ),
                ("__REQUEST_LANGUAGE_HINT__", request_language_hint),
                ("__REQUEST__", req),
            ],
        )
    };
    if !compact_prompt_required && prompt.len() > max_prompt_bytes {
        warn!(
            "intent_normalizer full prompt exceeds provider budget, switching to compact prompt: task_id={} bytes_before={} bytes_budget={}",
            task.task_id,
            prompt.len(),
            max_prompt_bytes
        );
        prompt = render_compact_intent_normalizer_prompt(
            route_view,
            context_bundle,
            auth_policy_context,
            request_language_hint,
            req,
        );
    }
    cap_intent_normalizer_prompt_for_llm_budget(state, task, prompt)
}

pub(super) fn intent_normalizer_max_prompt_bytes(state: &AppState, task: &ClaimedTask) -> usize {
    let providers = state.task_llm_providers(task);
    if providers.is_empty() {
        return 192 * 1024;
    }
    let min_tokens = providers
        .iter()
        .map(|provider| crate::memory::service::estimate_context_window_tokens(provider.as_ref()))
        .min()
        .unwrap_or(32_000);
    if min_tokens <= 4_096 {
        return min_tokens.saturating_mul(2).clamp(2_048, 3_300);
    }
    min_tokens
        .saturating_sub(1_400)
        .max(512)
        .saturating_mul(2)
        .min(512 * 1024)
        .max(2_048)
}

pub(super) fn intent_normalizer_uses_compact_prompt(state: &AppState, task: &ClaimedTask) -> bool {
    state
        .task_llm_providers(task)
        .iter()
        .map(|provider| crate::memory::service::estimate_context_window_tokens(provider.as_ref()))
        .min()
        .is_some_and(|tokens| tokens <= 4_096)
}

pub(super) fn compact_prompt_slot(label: &str, value: &str, max_bytes: usize) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "<none>" {
        return format!("{label}: <none>");
    }
    if trimmed.len() <= max_bytes {
        let visible = crate::providers::utf8_safe_prefix(trimmed, max_bytes);
        format!("{label}: {visible}")
    } else {
        let marker = "\n...<snip>...\n";
        if max_bytes <= marker.len().saturating_add(16) {
            let visible = crate::providers::utf8_safe_prefix(trimmed, max_bytes);
            return format!("{label}: {visible}...(truncated)");
        }
        let content_budget = max_bytes.saturating_sub(marker.len());
        let head_budget = content_budget / 2;
        let tail_budget = content_budget.saturating_sub(head_budget);
        let head = crate::providers::utf8_safe_prefix(trimmed, head_budget);
        let tail = crate::providers::utf8_safe_suffix(trimmed, tail_budget);
        format!("{label}: {head}{marker}{tail}")
    }
}

fn compact_runtime_context_from_auth(auth_policy_context: &str) -> String {
    let mut lines = Vec::new();
    for line in auth_policy_context.lines().map(str::trim) {
        if line.starts_with("current_process_cwd:") || line.starts_with("workspace_root:") {
            lines.push(line.to_string());
        }
    }
    if lines.is_empty() {
        "<none>".to_string()
    } else {
        format!("### RUNTIME_CONTEXT\n{}", lines.join("\n"))
    }
}

pub(super) fn render_compact_intent_normalizer_prompt(
    route_view: &crate::task_context_builder::RouteContextView,
    context_bundle: &crate::task_context_builder::TaskContextBundle,
    auth_policy_context: &str,
    request_language_hint: &str,
    req: &str,
) -> String {
    let mut parts = Vec::new();
    parts.push(
        "Compact intent normalizer. Output exactly one raw JSON object and then stop. No markdown, no answer text after JSON.".to_string(),
    );
    parts.push("Normalizer protocol is internal only: the raw-JSON/no-markdown requirement applies only to this classifier response. Never treat it as a user-visible output-format limit; preserve requested final formats such as markdown tables or CSV in the route contract/resolved intent.".to_string());
    parts.push("Always include all top-level schema keys: resolved_user_intent, answer_candidate, resume_behavior, schedule_kind, schedule_intent, wants_file_delivery, should_refresh_long_term_memory, agent_display_name_hint, needs_clarify, clarify_question, reason, confidence, decision, output_contract, execution_recipe, turn_type, target_task_policy, should_interrupt_active_run, state_patch, attachment_processing_required.".to_string());
    parts.push("Use decision as an initial machine hint for the legacy first-layer boundary: clarify, direct_answer, planner_execute. When planner-loop routing authority is active, the agent decision envelope owns ordinary respond/clarify/execute semantics; keep this field consistent with the contract but do not treat it as the final user-visible action.".to_string());
    parts.push("Prefer decision=direct_answer for greetings, confirmations, memory-only requests, and pure discussion. Use decision=clarify only when a required target/action is truly missing.".to_string());
    parts.push("High-priority: if REQUEST asks to summarize, explain, conclude, judge, or state what a current topic/test/conversation mainly verifies or means, do not treat a prior exact ID/value as the answer. Keep answer_candidate empty unless REQUEST explicitly asks for that scalar, and copy any relevant recent background/goals/purpose into resolved_user_intent. In MEMORY lists, leading decimal numbers are retrieval scores, never user facts or answer candidates.".to_string());
    parts.push("If ACTIVE_TASK is <none>, do not use task_append, task_correct, or task_scope_update. Classify a fresh user goal as task_request or leave turn_type empty for pure chat/memory/status turns.".to_string());
    parts.push("A complete current REQUEST with its own deliverable, topic, object, audience, scope, or factual constraints is standalone unless it semantically modifies the active deliverable. Shared chat identity, similar task type, same product name, or nearby memory is not enough to merge independent tasks.".to_string());
    parts.push("If ACTIVE_TASK or LAST shows an active writing/drafting/planning task and REQUEST only adds audience, tone, length, body-only, wording, count, format, scope, or presentation constraints, keep it attached: turn_type=\"task_append\", target_task_policy=\"reuse_active\", decision=\"direct_answer\", execution_recipe.kind=\"none\", requires_content_evidence=false, locator_kind=\"none\". Do not route such presentation-only follow-ups to planner_execute unless the REQUEST explicitly requires fresh local/system/file/web evidence.".to_string());
    parts.push("If ACTIVE_TASK/LAST shows a low-risk writing/drafting/planning clarification was already asked and REQUEST adds more constraints without answering every optional detail, prefer a best-effort generic draft over repeating clarification: decision=\"direct_answer\", turn_type=\"task_append\" or \"task_scope_update\", target_task_policy=\"reuse_active\", no evidence/delivery.".to_string());
    parts.push("ACTIVE_TASK_PATCH: for active-task corrections/refinements, put exact current-turn content values that must remain visible in state_patch.required_content_literals. For concrete visible replacements, set state_patch.replacement_pairs=[{\"from\":\"old literal\",\"to\":\"new literal\"}] and state_patch.forbidden_visible_literals for old/rejected literals that must disappear. Use exact content literals from the request; do not include generic output-control wording, length limits, body-only/output-only constraints, tone, count, or format instructions.".to_string());
    parts.push("Do not treat a bare acknowledgement request as active-task output refinement. If REQUEST only asks for a short acknowledgement/confirmation and does not explicitly reference ACTIVE_TASK/LAST/the prior answer/result/rewrite target, use standalone chat with no state_patch; answer the acknowledgement itself, not the active task output.".to_string());
    parts.push("If REQUEST's apparent missing topic is only the generic acknowledgement/short-reply target itself, do not ask what topic to answer. Use standalone chat, needs_clarify=false, empty turn_type/target_task_policy, no evidence/delivery, and put the minimal acknowledgement/short reply in answer_candidate when inferable from REQUEST. This is semantic, not phrase-list based.".to_string());
    parts.push("If the same active writing/drafting task is still missing its topic or core subject, keep the new constraint in resolved_user_intent and ask one concise clarification with decision=\"clarify\"; never force planner_execute for a presentation-only constraint.".to_string());
    parts.push("Do not ask optional preference clarifications for harmless creative/chat requests; answer generically when the deliverable is clear. For a negative constraint plus positive deliverable, preserve the constraint and route the positive deliverable.".to_string());
    parts.push("If CAPABILITIES or a visible skill contract says a missing target/parameter can be handled by safe discovery, default behavior, bounded lookup, or a candidate-returning prepare step, keep the request executable instead of asking a front-door clarification; execution can return observed candidates when it cannot choose uniquely.".to_string());
    parts.push("Example pattern: if a photo-organization capability declares external-drive discovery, route the request to execution without a source_dir so the skill can inspect mounts and either preview the unique candidate or return observed candidates. This is a contract example, not a phrase trigger.".to_string());
    parts.push("Inline-data transform invariant: if REQUEST embeds complete structured data and asks to sort, filter, project, aggregate, convert, or render it, do not clarify because of the requested output format. Use an enabled structured transform capability when visible; otherwise direct-answer from the inline data when no local/external evidence is needed.".to_string());
    parts.push("Current REQUEST overrides RECENT/MEMORY. Prior assistant refusals, tool failures, exact IDs, scalar values, or capability claims in history are background only unless the current request explicitly asks for them.".to_string());
    parts.push("Do not import a prior directory/path scope from RECENT/MEMORY into the current REQUEST when the current REQUEST names its own file/dir target. Reuse prior scope only for explicit follow-ups like same directory, that file, or previous result.".to_string());
    parts.push("Fresh unresolved deictic executable targets are missing locators: for a fresh filesystem/log/document/service/process/runtime-component request whose target is only a pronoun/deictic role and no unique immediate binding exists, set decision=\"clarify\", turn_type=\"task_request\", state_patch.deictic_reference={\"target\":\"missing_locator\"} or {\"target\":\"ambiguous_locator\"}, locator_kind=\"none\", locator_hint=\"\". Do not convert it to current_workspace, status_query, all-services status, generic health check, or a default component list unless the current REQUEST itself semantically names that broad scope. Do not resolve a fresh deictic target from MEMORY alone; MEMORY may explain ambiguity, but execution needs an immediate binding or current-turn locator.".to_string());
    parts.push("When decision=\"clarify\" only because a concrete locator/target is missing, still preserve the intended final-answer contract in output_contract: keep requires_content_evidence=true for tasks that must read/list/inspect local evidence after the user supplies the locator, keep the requested response_shape, and keep the synthesis semantic_kind when the final answer needs explanation, summary, judgment, conclusion, or another model-language answer grounded in that future evidence. Only locator_kind/locator_hint should express the missing target.".to_string());
    parts.push("If REQUEST asks for observable local/system/workspace state, filesystem inspection, command output, file content, directory listing, counts, or extracting a value, choose decision=\"planner_execute\". Do not claim the assistant cannot execute; the runtime has tools and the AUTH block describes permission.".to_string());
    parts.push("If REQUEST asks what capabilities, tools, skills, integrations, actions, or callable functions the assistant/runtime can currently use, choose decision=\"planner_execute\", output_contract.semantic_kind=\"tool_discovery\", response_shape=\"free\", requires_content_evidence=false, delivery_required=false, locator_kind=\"none\", delivery_intent=\"none\", and execution_recipe.kind=\"none\". A no-system-check or no-file-inspection constraint only forbids fake status/file observation; it does not make capability discovery chat-only.".to_string());
    parts.push("For generic baseline diagnostics, local runtime health, service/process status, or system health requests with no narrower unknown target, use decision=\"planner_execute\", turn_type=\"status_query\", output_contract.semantic_kind=\"service_status\", requires_content_evidence=true, delivery_required=false, locator_kind=\"none\", locator_hint=\"\", and execution_recipe.kind=\"none\". If the final answer scope is only host operating-system/system-health fields or excludes assistant/runtime/self service state, set state_patch.structured_field_selector=\"system_health.*\" so runtime can project that machine field family. Do not leave these as semantic_kind=\"none\" locatorless content-evidence clarifications, and do not rely on localized resolved_user_intent/reason prose to carry field exclusions.".to_string());
    parts.push("For RSS/feed/latest-news requests covered by rss_fetch, use decision=\"planner_execute\", output_contract.semantic_kind=\"rss_news_fetch\", requires_content_evidence=true, delivery_required=false, locator_kind=\"none\", locator_hint=\"\", and execution_recipe.kind=\"none\". Do not ask for a file path merely because no feed URL was supplied; configured RSS categories provide the default source set.".to_string());
    parts.push("For URL/web-page observation requests that need opening, extracting, title reading, or summarizing page content through browser_web, use decision=\"planner_execute\", output_contract.semantic_kind=\"web_page_summary\", requires_content_evidence=true, delivery_required=false, locator_kind=\"url\", locator_hint=<the concrete URL>, and execution_recipe.kind=\"none\".".to_string());
    parts.push("For web-search result requests covered by web_search_extract, use decision=\"planner_execute\", output_contract.semantic_kind=\"web_search_summary\", requires_content_evidence=true, delivery_required=false, locator_kind=\"none\", locator_hint=\"\", and execution_recipe.kind=\"none\"; keep the search query and requested result limit in resolved_user_intent.".to_string());
    parts.push("For current-weather or forecast requests covered by weather, use decision=\"planner_execute\", output_contract.semantic_kind=\"weather_query\", requires_content_evidence=true, delivery_required=false, locator_kind=\"none\", locator_hint=\"\", and execution_recipe.kind=\"none\"; keep the place, date/day target, forecast window, and output language constraints in resolved_user_intent.".to_string());
    parts.push("For stock, crypto, or other market quote/price requests covered by stock or crypto, use decision=\"planner_execute\", output_contract.semantic_kind=\"market_quote\", requires_content_evidence=true, delivery_required=false, locator_kind=\"none\", locator_hint=\"\", and execution_recipe.kind=\"none\"; keep the concrete symbol/code/name, market type if known, and requested brevity/language in resolved_user_intent. Crypto account and order workflows covered by crypto are executable observations or guarded low-risk skill outcomes too: account balances/positions, open orders, order status, trade history, trade previews/dry runs, cancel previews/requests, and other exchange-order workflows use the same market_quote contract with locator_kind=\"none\". Do not direct-answer from assumed missing account context, default exchange, or credentials before planner/skill execution; the planner/skill owns credential/default-exchange checks, confirmation gates, and structured clarification/account-access errors.".to_string());
    parts.push("For image/photo/screenshot understanding requests covered by image_vision, use decision=\"planner_execute\", output_contract.semantic_kind=\"image_understanding\", requires_content_evidence=true, delivery_required=false, locator_kind=\"url\" with locator_hint set to the concrete image URL when one is supplied, otherwise locator_kind=\"none\", and execution_recipe.kind=\"none\"; keep the requested visual task and response language in resolved_user_intent.".to_string());
    parts.push("For knowledge-base ingest/index/build/import requests covered by kb.ingest, use decision=\"planner_execute\", output_contract.semantic_kind=\"filesystem_mutation_result\", requires_content_evidence=true, delivery_required=false, locator_kind=\"path\" or \"current_workspace\" for the source document/directory, and execution_recipe.kind=\"none\" unless an ops closed-loop mutation recipe is separately required. Keep the target namespace and source paths in resolved_user_intent; do not classify these as service_status.".to_string());
    parts.push("If REQUEST asks about the assistant/runtime's current unfinished task queue, running tasks, queued tasks, or canceling those tasks, use the existing task_control capability with its default current-user/current-chat scope; do not ask the user to choose a queue type just because no separate system name was supplied. For readonly status/list and cancel/end requests in this task-control family, use decision=\"planner_execute\", turn_type=\"status_query\", output_contract.semantic_kind=\"service_status\", output_contract.requires_content_evidence=true, output_contract.locator_kind=\"none\", output_contract.delivery_required=false, execution_recipe.kind=\"none\", and state_patch=null unless REQUEST separately gives a real task-state update.".to_string());
    parts.push("If REQUEST only asks whether this assistant is currently waiting for user approval, answer from runtime invariants with decision=\"direct_answer\", turn_type=\"status_query\", execution_recipe.kind=\"none\", no evidence/delivery, and state_patch.runtime_status_query={\"kind\":\"approval_wait\",\"scope\":\"current_task\"}; leave answer_candidate empty unless that runtime fact is provided as structured context. Do not use runtime_status_query.kind=\"approval_wait\" for task queue/list/running/cancel semantics; those belong to task_control.".to_string());
    parts.push("Never ask the user to paste local file contents when REQUEST names a local file/dir/workspace target; route the request for tool execution. Capability refusals are only valid after an actual tool failure, not inside this normalizer.".to_string());
    parts.push("Always include output_contract as a JSON object, never as a string token. It is the final answer contract, not a place to invent a task-specific schema. Put exact scalar recall/direct-answer values in answer_candidate as a string only when the current request itself asks for that exact value; never put answer_candidate as an object or inside output_contract. If unsure, still emit the full default output_contract object with response_shape=\"free\", requires_content_evidence=false, delivery_required=false, locator_kind=\"none\", delivery_intent=\"none\", semantic_kind=\"none\", locator_hint=\"\", and self_extension set to none.".to_string());
    parts.push("Allowed output_contract keys only: response_shape, exact_sentence_count, requires_content_evidence, delivery_required, locator_kind, delivery_intent, semantic_kind, locator_hint, scalar_count_filter, list_selector, self_extension. Do not emit exact_format, required_evidence, fields, examples, post_processing, or custom keys.".to_string());
    parts.push("locator_hint must be a clean concrete locator value or concrete target pair, not a full instruction sentence and not explanatory prose. If no clean locator is known, leave it empty and let needs_clarify/decision express the missing target.".to_string());
    parts.push("Allowed response_shape: free, one_sentence, strict, scalar, file_token. Allowed locator_kind: none, path, current_workspace, url, filename. Allowed delivery_intent: none, file_single, directory_lookup, directory_batch_files.".to_string());
    parts.push("Allowed semantic_kind: none, raw_command_output, command_output_summary, service_status, hidden_entries_check, file_names, directory_names, directory_entry_groups, file_paths, directory_purpose_summary, content_excerpt_summary, document_heading, content_excerpt_with_summary, content_presence_check, excerpt_kind_judgment, recent_artifacts_judgment, workspace_project_summary, scalar_count, quantity_comparison, execution_failed_step, generated_file_delivery, generated_file_path_report, filesystem_mutation_result, scalar_path_only, file_basename, existence_with_path, existence_with_path_summary, recent_scalar_equality_check, git_commit_subject, git_repository_state, structured_keys, config_validation, config_mutation, config_risk_assessment, rss_news_fetch, web_page_summary, web_search_summary, weather_query, market_quote, image_understanding, publishing_preview, package_manager_detection, tool_discovery, sqlite_table_listing, sqlite_table_names_only, sqlite_database_kind_judgment, sqlite_schema_version, archive_list, archive_pack, archive_unpack, docker_ps, docker_images, docker_logs, docker_container_lifecycle.".to_string());
    parts.push("For requests asking what capabilities, tools, skills, integrations, or actions the assistant/runtime can currently use, route to the planner with output_contract.semantic_kind=\"tool_discovery\", response_shape=\"free\", requires_content_evidence=false, delivery_required=false, locator_kind=\"none\", delivery_intent=\"none\", and execution_recipe.kind=\"none\". This is answered from planner-visible capability context, not filesystem evidence or a fake health/status observation.".to_string());
    parts.push("If the current REQUEST semantically targets the present repository, project, workspace, or current directory and does not name another path, this is resolved current workspace scope, not a missing locator. Use decision=\"planner_execute\", needs_clarify=false, output_contract.locator_kind=\"current_workspace\", and include machine token current_workspace_scope_from_current_request in reason. Keep unresolved pronoun/deictic local targets as clarify with state_patch.deictic_reference.".to_string());
    parts.push("For output_contract.semantic_kind=\"scalar_count\", output_contract.scalar_count_filter is the mandatory machine field whenever the counted object class, extension filter, or scope is semantically known. Use exactly {\"target_kind\":\"any|file|dir\",\"include_hidden\":true|false|null,\"recursive\":true|false|null,\"extensions\":[\"ext\"]}. target_kind=\"file\" means files only; target_kind=\"dir\" means directories/folders only; target_kind=\"any\" means files plus directories. Put file extension tokens such as md/json/log/toml in extensions without dots; extension-filtered counts should use target_kind=\"file\". recursive=true means the full subtree/all descendants below the target directory, including extension/file-type counts over a directory as a whole; recursive=false means direct/immediate/top-level children only. If directory scope is known but the user did not explicitly restrict counting to direct/immediate/top-level children, prefer recursive=true for files-only or extension-filtered file counts. If a directory-count request explicitly excludes the target/root directory itself from the count, treat that as an all-descendant scope and set recursive=true unless the same request also explicitly limits the scope to direct/immediate/top-level children. Do not set recursive=false for root-excluding directory counts: direct-child counts normally do not need root exclusion. Mirror the same object in state_patch.scalar_count_filter for standalone filesystem counts, set turn_type=\"task_request\" and target_task_policy=\"standalone\", and do not rely on localized wording in resolved_user_intent or reason to carry these filters. For structured JSON/TOML/YAML field-value extraction or service-status field-family projection, state_patch.structured_field_selector is the mandatory machine field whenever the selected field/key/path is semantically known, even if the file/path locator is still missing and decision=\"clarify\"; use exact machine tokens such as name, package.version, workspace.package.repository, or system_health.*, and do not rely on localized wording in resolved_user_intent or reason to carry the selected field.".to_string());
    parts.push("Allowed top-level schedule_kind: none, create, update, delete, query. Schedule type/frequency tokens once/daily/weekly/interval/cron belong only in schedule_intent.schedule.type. Scheduled reminders default to the current conversation/task delivery context; do not clarify only to ask for a receive channel. Schedule runtime handles schedule create/update/delete/query; ordinary scheduled reminders use output_contract.semantic_kind=\"none\", requires_content_evidence=false, locator_kind=\"none\", delivery_required=false, delivery_intent=\"none\".".to_string());
    parts.push("Allowed turn_type: task_request, task_append, task_replace, task_correct, task_scope_update, run_control, approval_decision, status_query, feedback_or_error, preference_or_memory, or empty string. clarify is a decision, never a turn_type or resume_behavior.".to_string());
    parts.push("state_patch must be a JSON object or null. Use null when there is no structured update; never output an empty string for state_patch. For ordered-entry follow-ups against an active ordered list, set state_patch.ordered_entry_ref to {\"index\":N,\"index_base\":1} for absolute item selection or {\"relative_offset\":K} for signed relative selection. For recent count_inventory comparisons, use semantic_kind=\"quantity_comparison\" and state_patch.quantity_comparison={\"selection\":\"max\"|\"min\",\"source\":\"recent_count_inventory\"}. When a standalone current REQUEST creates a new user-visible deliverable that later short corrections should edit, set state_patch.primary_task_update=\"replace\" and state_patch.active_task_boundary=\"new_deliverable\". For structured file field-value extraction, set state_patch.structured_field_selector to the exact machine field/key/path to read when it is known, such as a JSON/TOML/YAML field_path; preserve it even when the request must clarify for a missing locator. Leave it absent only when the selected field is genuinely unknown. For active-task visible corrections, set required_content_literals / replacement_pairs / forbidden_visible_literals as structured exact content literals, not language-specific phrase markers. Keep output-only/body-only/length/tone/count/format constraints in resolved_user_intent and output_contract, not in required_content_literals. For a clear deictic reference, set state_patch.deictic_reference={\"target\":\"current_action_result\"|\"current_turn_locator\"|\"comparison_result\"|\"unresolved_prior_object\"|\"missing_locator\"|\"ambiguous_locator\"}; unresolved/missing/ambiguous targets mean safe clarify. For runtime self-state questions about whether this assistant is waiting for user approval, set state_patch.runtime_status_query={\"kind\":\"approval_wait\",\"scope\":\"current_task\"}. For current local identity/environment scalar status, set state_patch.runtime_status_query with machine kinds such as current_user, host_name, or kernel_release. The runtime consumes structured numbers/targets/status tokens, not language-specific ordinal words, pronouns, connectors, or status wording.".to_string());
    parts.push("Every enum field must be exactly one listed schema token. Do not output aliases, combined values, or explanatory prose in decision/output_contract/execution_recipe/turn_type/target_task_policy.".to_string());
    parts.push("Boolean fields must be JSON true/false, not prose. self_extension must be an object with mode/trigger/execute_now; use {\"mode\":\"none\",\"trigger\":\"none\",\"execute_now\":false} unless the user explicitly asks for self-extension. If locator_kind=\"none\", locator_hint must be \"\".".to_string());
    parts.push("If the user asks to observe/list/read first but only return a scalar result, set response_shape=\"scalar\" and use a matching semantic_kind only when one applies: scalar_count for generic counts, hidden_entries_check for hidden/dot-prefixed entry counts, scalar_path_only only for a path/current-directory/workspace-location answer, file_basename only for the basename/name of the active or selected local file target, document_heading when the final answer is only the observed document/file/page heading or title value, sqlite_schema_version for SQLite schema-version metadata. For active-file basename/name follow-ups, keep the answer bound to the active file target itself, not filenames that appear inside the displayed content. For config field values, package names, usernames, hostnames, IDs, or other non-path scalar values, keep semantic_kind=\"none\" unless another specific enum applies. If the final answer must include both a structured field/key/path identifier and its value, it is not a scalar-only value response: use response_shape=\"strict\" and preserve the key/value shape in resolved_user_intent. If the request requires an exact non-scalar output format with fixed count, body-only delivery, one-line fixed format, placeholder format, or no-extra-output delivery, set response_shape=\"strict\" and preserve the exact format in resolved_user_intent. For any exact counted-sentence requirement, also set exact_sentence_count to that positive integer; use response_shape=\"strict\" when the count is greater than 1. Never put natural-language format descriptions in response_shape.".to_string());
    parts.push("For command/tool execution where the final answer is about execution failure itself, including a single failed command/action, ordered failed step(s), or an ordered success/failure report for each step, set response_shape=\"strict\", semantic_kind=\"execution_failed_step\", requires_content_evidence=true, delivery_required=false. For ordered command/tool requests, preserve the whole ordered action sequence in resolved_user_intent and do not downgrade failed-action or success/failure-step reporting to command_output_summary. This is a semantic judgment from the requested final answer shape, not a phrase-list trigger.".to_string());
    parts.push("For bounded file or log excerpt observations, choose the semantic_kind from the final answer, not from the tool used to gather evidence. A direct request to display a bounded line slice, head/tail slice, or exact range must use semantic_kind=\"raw_command_output\" with the exact slice/count preserved in resolved_user_intent and response_shape=\"strict\" when the final answer should paste/show the observed lines themselves. When a follow-up names a different local file but repeats the previous bounded-read operation, inherit only the slice/count constraint and still use raw_command_output unless the current deliverable asks for interpretation. If the answer must only explain, summarize, conclude, judge, describe a phenomenon, or provide a one-sentence takeaway from the observed excerpt, use semantic_kind=\"content_excerpt_summary\" or semantic_kind=\"excerpt_kind_judgment\" for excerpt classification. If the final answer must include both the bounded observed slice and the requested synthesis, use semantic_kind=\"content_excerpt_with_summary\". Preserve bounded-read constraints as machine tokens in resolved_user_intent or reason: slice_mode=head|tail|range, slice_n=N, slice_start=N, slice_end=N. These tokens are strict; never emit aliases or prose values for slice_mode. Do not classify a plain bounded line read as content_excerpt_summary unless model-language interpretation is part of the requested deliverable.".to_string());
    parts.push("For requests to create/save/write a new artifact and then send/deliver it as an attachment/artifact, set response_shape=\"file_token\", semantic_kind=\"generated_file_delivery\", delivery_required=true, delivery_intent=\"file_single\", requires_content_evidence=true. If the user instead wants only the saved path reported in chat, set response_shape=\"scalar\", semantic_kind=\"generated_file_path_report\", delivery_required=false, delivery_intent=\"none\", requires_content_evidence=true, and preserve the target path/filename when supplied. If the user did not supply a filename but the artifact type/content is clear, do not ask for one; let execution planning choose a safe workspace filename.".to_string());
    parts.push("If REQUEST only asks to send/deliver/receive a named path or filename and does not ask to create/save/write new content for that target, treat it as existing-file delivery rather than generated_file_delivery. Preserve the named target as a locator and let execution either deliver the existing file or return structured missing-file evidence.".to_string());
    parts.push("For filesystem mutations where the final answer should report the action result instead of deliver a file artifact, set response_shape=\"one_sentence\", semantic_kind=\"filesystem_mutation_result\", delivery_required=false, delivery_intent=\"none\", and requires_content_evidence=true. Keep the concrete path as locator_hint when known. Use this for structured local path mutations such as creating a directory, writing/appending/removing a path, or similar filesystem lifecycle actions when the requested deliverable is the observed success/failure result. Filesystem lifecycle mutation contracts outrank command_output_summary even when the final visible answer is a brief structured summary; command_output_summary is for observed command/tool output that does not itself require a filesystem lifecycle mutation contract.".to_string());
    parts.push("For archive pack/create/compress or unpack/extract/decompress requests, use semantic_kind=\"archive_pack\" or semantic_kind=\"archive_unpack\" even when the final answer asks only for the resulting path or status. For archive extraction, keep semantic_kind=\"archive_unpack\" even when the source archive locator is deictic or missing; then use decision=\"clarify\", requires_content_evidence=true, locator_hint=\"\", and do not downgrade it to filesystem_mutation_result. Do not classify archive operations as generated_file_delivery; they have dedicated archive contracts and actions.".to_string());
    parts.push("For requests to send/deliver/receive an existing or selected local file, including a file selected from an observed or target directory by ordinal/order such as first/last/newest/largest, set wants_file_delivery=true, response_shape=\"file_token\", delivery_required=true, delivery_intent=\"file_single\", requires_content_evidence=true. The final answer must be a file token, not a bare filename, file_path_and_content answer_candidate, or pasted file content. This is a semantic delivery contract, not a phrase list.".to_string());
    parts.push("For selected existing-file delivery from a directory, preserve the directory as locator scope and put the child-selection rule in output_contract.list_selector instead of route_reason prose. Use target_kind=\"file\", limit=1, and a strict sort_by token such as name/name_desc, mtime_desc/mtime_asc, or size_desc/size_asc when the selector is bounded. This lets the planner observe the directory and resolve the file without front-door clarification.".to_string());
    parts.push("Text drafting/composition is not file delivery by default. If REQUEST asks to write/draft/compose an article, note, proposal, summary, checklist, tutorial, guide, or long-form text for the chat, but does not explicitly ask to save it to a file/path/document or send/deliver it as an attachment/artifact, do not use response_shape=\"file_token\" or semantic_kind=\"generated_file_delivery\". Keep delivery_required=false, wants_file_delivery=false, and use response_shape=\"free\" or \"strict\" according to the requested prose format. If the text is project-bound and needs workspace facts, use decision=\"planner_execute\", requires_content_evidence=true, locator_kind=\"current_workspace\"; still keep file delivery disabled. Examples: \"帮我写一篇关于 RustClaw 的长文\" / \"Write a long article about RustClaw\" means pasted prose in chat, while \"帮我写成 md 文件并发给我\" / \"Create a markdown file and send it to me\" means generated file delivery.".to_string());
    parts.push("If REQUEST drafts or composes text for an external publishing channel or platform workflow owned by a visible publishing skill, use decision=\"planner_execute\", semantic_kind=\"publishing_preview\", requires_content_evidence=true even when the requested mode is preview-only, draft-only, dry-run, or no-publish. Keep delivery_required=false and preserve the preview/no-send constraint in resolved_user_intent; ordinary chat-only drafting still follows the direct-answer drafting rule.".to_string());
    parts.push("For exact same/different comparison of two scalar/field values that still need observation, use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, response_shape=\"strict\", semantic_kind=\"recent_scalar_equality_check\". Keep the requested final line format in resolved_user_intent.".to_string());
    parts.push("For a comparison where one side is a scalar field/value from a structured manifest or config file and the other side is the corresponding value mentioned in a README/docs file, use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, semantic_kind=\"recent_scalar_equality_check\", and response_shape=\"one_sentence\"/\"strict\" according to the requested final answer. This is a semantic contract for field/document evidence, not generic document summarization.".to_string());
    parts.push("For comparison or classification of prose excerpts/opening sections by audience, purpose, document role, or content type, use semantic_kind=\"excerpt_kind_judgment\" with content evidence. Do not route these as scalar equality checks because the compared evidence is prose, not scalar fields.".to_string());
    parts.push("When REQUEST compares, classifies, or selects among already observed recent execution results/excerpts and the final answer only asks for the chosen name/path/label, bind it to RECENT_EXECUTION_CONTEXT or RECENT turns, not to a fresh workspace listing. Use semantic_kind=\"excerpt_kind_judgment\" or \"recent_artifacts_judgment\" while evidence is still needed; if the recent observed context already contains enough evidence and the selected scalar is clear, use decision=\"direct_answer\", response_shape=\"scalar\", requires_content_evidence=false, semantic_kind=\"none\", locator_kind=\"none\", and put the selected scalar in answer_candidate.".to_string());
    parts.push("For recent-file listings plus any grounded type, category, purpose, use, or role judgment about those selected recent entries, use semantic_kind=\"recent_artifacts_judgment\" with content evidence. Preserve both the recent-entry selection and the judgment/explanation deliverable in resolved_user_intent so planning can first observe the sorted entries and then read bounded content when needed.".to_string());
    parts.push("For bounded, filtered, or ordered entry listings plus any grounded category, purpose, use, role, or artifact/document-style judgment about the selected entries, choose the synthesis contract from the judgment deliverable instead of the listing step. Use semantic_kind=\"recent_artifacts_judgment\" when the selection is explicitly recent/modified-time based; otherwise use semantic_kind=\"directory_purpose_summary\" with response_shape=\"one_sentence\" or \"free\", requires_content_evidence=true, delivery_required=false, and locator_kind=\"current_workspace\" or \"path\". Preserve selection constraints such as target_kind, limit, order, and include_metadata as machine fields or tokens in output_contract.list_selector/resolved_user_intent so planning can observe the entries before synthesis. Do not route these as semantic_kind=\"file_names\" or semantic_kind=\"directory_entry_groups\" because strict listing contracts discard the judgment deliverable. This is semantic final-deliverable priority, not a language-specific trigger.".to_string());
    parts.push("When output_contract.list_selector is present, its machine tokens are strict: target_kind must be exactly \"file\", \"dir\", or \"any\"; sort_by must be exactly \"name\", \"name_desc\", \"size_desc\", \"size_asc\", \"mtime_desc\", or \"mtime_asc\" when supplied; include_hidden and include_metadata must be booleans or null. Never emit aliases such as mixed, both, default, auto, files, folders, hidden, or prose. For selected-entry synthesis, use target_kind=\"file\" when the selected set is files only, target_kind=\"dir\" when it is directories only, and target_kind=\"any\" only when files and directories are both valid final evidence. If a listing selector semantically includes dot-prefixed entries, set include_hidden=true as a machine field.".to_string());
    parts.push("For file/path metadata comparisons across concrete local targets (for example size/大小, modified time/修改时间, existence state, or other observable path facts), use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, semantic_kind=\"quantity_comparison\", and response_shape=\"scalar\"/\"one_sentence\"/\"strict\" according to the requested final answer. If the same comparison deliverable also asks for a reason, explanation, likely cause, summary, or judgment, keep semantic_kind=\"quantity_comparison\" but use response_shape=\"free\" instead of a scalar/one-sentence verdict shape unless the user explicitly asks for JSON, a table, or another exact structured format, and include machine token quantity_comparison_requires_model_language_synthesis in reason, so the final answer includes both observed comparison values and the requested model-language synthesis. This is a semantic contract decision, not a phrase-list trigger; do not treat metadata comparison as document content summarization just because the user also asks for a short explanation.".to_string());
    parts.push("For local project package-manager, dependency-manager, frontend package-manager, or build-tool detection, use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, semantic_kind=\"package_manager_detection\", and locator_kind=\"current_workspace\" or \"path\" when the request names a project directory. This is a project capability contract based on manifest/lock-file observation; do not route it as generic file_names merely because marker filenames are inspected.".to_string());
    parts.push("For git commit subject/title requests, use decision=\"planner_execute\", requires_content_evidence=true, response_shape=\"scalar\" or \"strict\" according to the user's requested format, and semantic_kind=\"git_commit_subject\". Do not publish the raw git oneline hash when the final answer asks for the subject/title only.".to_string());
    parts.push("For read-only Git repository state observation such as current branch, branch list, status, remotes, changed files, or revision metadata, use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, locator_kind=\"current_workspace\" or \"none\", semantic_kind=\"git_repository_state\", and response_shape=\"scalar\"/\"strict\"/\"free\" according to the requested final answer. This is a tool capability contract; do not require a file/path locator unless the Git action needs a concrete path, such as reading a file at a revision.".to_string());
    parts.push("For structured document key-name requests against JSON/TOML/YAML/config files, use decision=\"planner_execute\", requires_content_evidence=true, response_shape=\"strict\" when the user asks only for the keys, and semantic_kind=\"structured_keys\". Keep locator_kind/locator_hint pointed at the structured file; do not treat key-name requests as file excerpts.".to_string());
    parts.push("For hidden or dot-prefixed directory entry checks, use decision=\"planner_execute\", requires_content_evidence=true, locator_kind=\"current_workspace\" or \"path\", and semantic_kind=\"hidden_entries_check\". This remains hidden_entries_check even when the final answer asks for example names, two examples, a short list, or yes/no plus names; do not downgrade it to file_names. When the final answer is constrained to count only, use response_shape=\"scalar\" with this same semantic_kind. When the final answer is constrained to yes/no plus a limited set of entries, use response_shape=\"strict\" so later stages do not prepend execution traces.".to_string());
    parts.push("For existence checks whose final answer is a presence judgment over a concrete file, directory, path, or local artifact, use decision=\"planner_execute\", requires_content_evidence=true, semantic_kind=\"existence_with_path\", and the narrowest locator_kind that matches the target scope. If the final answer asks only for yes/no or exists/not-exists, use response_shape=\"scalar\"; if it asks to include a path/locator or other evidence fields, use response_shape=\"strict\". Do not use semantic_kind=\"scalar_count\" merely because the requested final answer is short or binary; presence judgment is not numeric counting. If the same request also asks for a brief content-grounded purpose, summary, role, or explanation when found, use semantic_kind=\"existence_with_path_summary\" instead so planning observes both the path and bounded content before synthesis. Preserve the final answer wording constraint in resolved_user_intent so later stages do not prepend execution traces.".to_string());
    parts.push("For directory/file inventory with name or extension filtering, set requires_content_evidence=true and locator_kind=\"current_workspace\" or \"path\". Use semantic_kind=\"file_names\" when the final answer is restricted to exact file names or file-only entries with requested metadata columns. Use semantic_kind=\"directory_names\" when the final answer is exact folder/directory names only. Use semantic_kind=\"directory_entry_groups\" for direct child entry names or inventory from one directory when files and directories may both be valid, even if the final visible answer should be names-only rather than grouped prose. Use semantic_kind=\"file_paths\" when the final answer must be file paths, especially repository/workspace-wide extension searches, basename/stem/name-match candidate discovery, or representative matching file path lists; a brief representative path mention is still file_paths when no purpose/use explanation is requested. For basename/stem/name-match candidate discovery, keep the search root as locator_kind=\"current_workspace\" or the named directory path and preserve the target name/stem as a selector token in resolved_user_intent; do not convert a single auto-located candidate into a missing-read-target clarify or content_excerpt_summary contract. If the request explicitly asks for a top-k candidate set, normalize it to output_contract.list_selector={\"target_kind\":\"file\",\"limit\":N,\"include_metadata\":false} and preserve selector_limit=N in resolved_user_intent; otherwise do not invent a limit for requests that ask for all matching paths. If the same request also asks for explanation, purpose, judgment, comparison, or a brief conclusion about the files, do not use an exact names/paths contract; use directory_purpose_summary for grounded purpose/category/style judgment over the selected entries, otherwise keep semantic_kind=\"none\" and preserve the combined listing+synthesis requirement in resolved_user_intent/reason. If a nuance has no enum, keep response_shape=\"free\" or semantic_kind=\"none\" instead of inventing enum values.".to_string());
    parts.push("For compound workspace-structure requests that ask to observe/list top-level directories, folders, or workspace sections and then explain the partition, organization, purpose, role, or beginner-friendly meaning of that structure, route as synthesis: decision=\"planner_execute\", response_shape=\"one_sentence\" or \"free\", semantic_kind=\"directory_purpose_summary\", requires_content_evidence=true, delivery_required=false, locator_kind=\"current_workspace\". Do not use semantic_kind=\"directory_entry_groups\" for these because a strict grouped listing would discard the requested explanation. This is a semantic final-deliverable contract, not a fixed phrase trigger.".to_string());
    parts.push("For bounded or ordered direct child inventory of a directory/workspace, including name, modification-time, or recency ordering, keep the route executable with response_shape=\"strict\", requires_content_evidence=true, delivery_required=false, and semantic_kind=\"directory_entry_groups\" unless the final answer is restricted to files-only or directories-only. If the final answer is a file-only metadata-ranked list, use semantic_kind=\"file_names\", set output_contract.list_selector={\"target_kind\":\"file\",\"limit\":N,\"sort_by\":\"name|name_desc|size_desc|size_asc|mtime_desc|mtime_asc\",\"include_metadata\":true|false} when clear, preserve matching selector_target_kind/file selector_limit/sort tokens in resolved_user_intent, and include machine token file_names_contract_preserves_bounded_ordered_files_only_listing_with_size_format in reason for size-ranked or size-column lists. Preserve the ordering/count requirement in resolved_user_intent; do not downgrade such requests to semantic_kind=\"none\" or a generic tree/workspace overview.".to_string());
    parts.push("Use decision=\"planner_execute\" when the request inspects local/system/workspace state, whether the final answer is direct raw/scalar/list output or a narrative synthesis. For current-directory or workspace-location scalar answers, set output_contract.response_shape=\"scalar\" and output_contract.semantic_kind=\"scalar_path_only\" from the request meaning, not from local phrase-classifier hints.".to_string());
    parts.push("For directory-scoped locator search where the user wants the resolved entry path itself, use response_shape=\"scalar\", semantic_kind=\"scalar_path_only\", requires_content_evidence=true, delivery_required=false, and bind the concrete directory as locator context while preserving the target entry name/stem in resolved_user_intent.".to_string());
    parts.push("For recall questions, use exact values from RECENT/MEMORY. If found, put the value in answer_candidate and resolved_user_intent, set needs_clarify=false, and set decision=\"direct_answer\". Never invent recall-specific decisions. A request for a summary, recap, explanation, conclusion, judgment, or what something verifies/means is not a recall question; keep that deliverable in resolved_user_intent and leave answer_candidate empty unless the current request also explicitly asks for an exact scalar.".to_string());
    parts.push("For requests that depend on prior context, copy the relevant RECENT/MEMORY facts into resolved_user_intent so the next stage has enough context.".to_string());
    parts.push("Use ALIASES only for temporary references already defined in this session. When the current message mentions one, resolve it in resolved_user_intent and locator fields when relevant.".to_string());
    parts.push("For explicit temporary alias/reference mappings in the current turn, set state_patch.alias_bindings to objects with alias and target string fields. Do not infer aliases from vague references.".to_string());
    parts.push("Keep resolved_user_intent concise; preserve exact IDs, but summarize long user text instead of copying it.".to_string());
    parts.push(compact_prompt_slot(
        "ACTIVE_TASK",
        &route_view.active_task_context,
        760,
    ));
    parts.push(compact_prompt_slot(
        "ANCHOR",
        &route_view.active_execution_anchor_context,
        520,
    ));
    parts.push(compact_prompt_slot(
        "ALIASES",
        &route_view.session_alias_context,
        220,
    ));
    parts.push(compact_prompt_slot(
        "HINTS",
        &route_view.request_surface_hints,
        120,
    ));
    parts.push(compact_prompt_slot(
        "CAPABILITIES",
        &route_view.capability_map,
        1800,
    ));
    parts.push(compact_prompt_slot("AUTH", auth_policy_context, 100));
    parts.push("Required keys: resolved_user_intent, needs_clarify, clarify_question, reason, confidence, decision. If unsure: use decision=\"direct_answer\" only for non-observable discussion; use decision=\"planner_execute\" for clear observable local/system/workspace requests.".to_string());
    parts.push("For ordinary chat, greetings, and confirmations: decision=\"direct_answer\", needs_clarify=false, turn_type=\"\". Never use turn_type=\"chat\".".to_string());
    parts.push(compact_prompt_slot(
        "RECENT",
        &route_view.recent_turns_full,
        1040,
    ));
    parts.push(compact_prompt_slot("LAST", &route_view.last_turn_full, 180));
    let runtime_context = context_bundle
        .execution_view
        .as_ref()
        .map(|view| view.runtime_context.as_str())
        .filter(|runtime| {
            let trimmed = runtime.trim();
            !trimmed.is_empty() && trimmed != "<none>"
        })
        .map(str::to_string)
        .unwrap_or_else(|| compact_runtime_context_from_auth(auth_policy_context));
    parts.push(format!("LANG={}", request_language_hint));
    parts.push("SCALAR_COUNT_GUARD: For semantic_kind=\"scalar_count\", set output_contract.scalar_count_filter with machine tokens. target_kind=file for files, dir for directories, any for files+dirs. recursive=true means full subtree/all descendants, including directory-wide files-only or extension-filtered counts unless direct/immediate/top-level scope is explicitly requested; recursive=false means direct/immediate/top-level children. If a directory-count request excludes the target/root directory itself, use recursive=true unless direct/immediate/top-level scope is also explicitly requested; root-excluding directory counts must not use recursive=false.".to_string());
    parts.push("CONTRACT: output_contract must be a JSON object. hidden/dot-entry check => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"hidden_entries_check\" even when the answer asks for example names; never classify that as file_names. ordered command/tool success-failure step reports => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"execution_failed_step\"; never classify those as command_output_summary. yes/no-only existence check => response_shape=\"scalar\", semantic_kind=\"existence_with_path\"; existence check that must return a path/locator/evidence field => response_shape=\"strict\", semantic_kind=\"existence_with_path\"; if it also needs a content-grounded purpose/summary/explanation when found, use semantic_kind=\"existence_with_path_summary\". URL/web-page content or title summary => locator_kind=\"url\", requires_content_evidence=true, semantic_kind=\"web_page_summary\". Web search result summary => locator_kind=\"none\", requires_content_evidence=true, semantic_kind=\"web_search_summary\". Weather current/forecast observation => locator_kind=\"none\", requires_content_evidence=true, semantic_kind=\"weather_query\". Stock/crypto market quote, crypto account observation, or crypto order/preview workflow => locator_kind=\"none\", requires_content_evidence=true, semantic_kind=\"market_quote\". Image/photo/screenshot understanding => requires_content_evidence=true, semantic_kind=\"image_understanding\", locator_kind=\"url\" only when a concrete image URL is supplied. External publishing-channel draft/preview => requires_content_evidence=true, semantic_kind=\"publishing_preview\", locator_kind=\"none\". command output that needs explanation/diagnosis/judgment/rewrite/synthesis => requires_content_evidence=true, semantic_kind=\"command_output_summary\". generated file that must be delivered as an attachment/artifact => response_shape=\"file_token\", delivery_required=true, semantic_kind=\"generated_file_delivery\". generated file whose saved path is the scalar chat answer => response_shape=\"scalar\", delivery_required=false, semantic_kind=\"generated_file_path_report\". exact file-only names list or file-only metadata-ranked list => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"file_names\"; for file-only metadata-ranked lists set output_contract.list_selector with target_kind=file, limit/sort_by/include_metadata when clear and include machine token file_names_contract_preserves_bounded_ordered_files_only_listing_with_size_format in reason for size-ranked or size-column lists. exact folder/directory names list => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"directory_names\". direct directory child inventory or mixed entry names list => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"directory_entry_groups\". exact file paths list => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"file_paths\". exact bounded file/log line slice => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"raw_command_output\". local document/file/page heading/title only => response_shape=\"scalar\", requires_content_evidence=true, semantic_kind=\"document_heading\". local file/path metadata comparison => requires_content_evidence=true, semantic_kind=\"quantity_comparison\". git commit subject/title only => response_shape=\"scalar\", requires_content_evidence=true, semantic_kind=\"git_commit_subject\". read-only Git repository state => requires_content_evidence=true, semantic_kind=\"git_repository_state\". current path only => response_shape=\"scalar\", semantic_kind=\"scalar_path_only\"; active/selected file basename only => response_shape=\"scalar\", semantic_kind=\"file_basename\"; never use scalar_path_only for directory listings.".to_string());
    parts.push("PATH_LIST_SELECTOR: For file path candidate discovery where the request explicitly asks for top-k results, keep semantic_kind=\"file_paths\" and set output_contract.list_selector={\"target_kind\":\"file\",\"limit\":N,\"include_metadata\":false}; also preserve selector_limit=N in resolved_user_intent. Do not set a limit for all-matches path-list requests.".to_string());
    parts.push("FILE_DELIVERY_SELECTOR: For existing-file delivery where the concrete file must be selected from a directory by a bounded ordinal, ordering, recency, or metadata selector, keep the turn executable with response_shape=\"file_token\", delivery_required=true, delivery_intent=\"file_single\", requires_content_evidence=true, locator_hint set to the directory scope, and output_contract.list_selector={\"target_kind\":\"file\",\"limit\":1,\"sort_by\":\"name|name_desc|mtime_desc|mtime_asc|size_desc|size_asc\",\"include_metadata\":false}. The planner/runtime can list the directory and choose the selected file; do not clarify only because the exact child filename is not known before that observation.".to_string());
    // Keep memory and assistant recall context close to the current request so
    // compact head+tail truncation preserves both structure labels and goals.
    parts.push(compact_prompt_slot(
        "MEMORY",
        &route_view.memory_context,
        560,
    ));
    // Keep recent assistant replies closest to the request so exact scalar
    // recall can use the assistant's visible answer rather than memory scores.
    parts.push(compact_prompt_slot(
        "ASSISTANT",
        &route_view.recent_assistant_replies,
        260,
    ));
    parts.push(compact_prompt_slot("RUNTIME", &runtime_context, 260));
    parts.push(compact_prompt_slot("RUNTIME", &runtime_context, 240));
    // Keep memory, assistant replies, active-task state, and the request in the
    // compact tail together; small-context providers often preserve only
    // head+tail around the final request.
    parts.push(compact_prompt_slot(
        "MEMORY",
        &route_view.memory_context,
        560,
    ));
    // Keep recent assistant replies after memory so exact scalar recall can use
    // the assistant's visible answer rather than memory scores.
    parts.push(compact_prompt_slot(
        "ASSISTANT",
        &route_view.recent_assistant_replies,
        240,
    ));
    parts.push(compact_prompt_slot(
        "ACTIVE_TASK",
        &route_view.active_task_context,
        320,
    ));
    parts.push(compact_prompt_slot(
        "ANCHOR",
        &route_view.active_execution_anchor_context,
        280,
    ));
    parts.push("TAIL_GUARDS: SUMMARY_RECALL summary!=ID and memory scores are metadata; RECENT_OBSERVED_JUDGMENT use recent observed context when enough evidence exists, do not turn them into fresh file_names/path lookup; RUNTIME_STATUS approval_wait=>direct_answer status_query, task_control queue/running/cancel status=>planner_execute service_status, kb.ingest=>planner_execute filesystem_mutation_result; FOLLOWUP_ANCHOR_PRIORITY ANCHOR/ACTIVE_TASK beat MEMORY for ordinal/deictic/refinement; LOCAL_EXEC local file/dir/command/count/metadata/read/list/summarize=>planner_execute, no cannot-access-FS reply.".to_string());
    parts.push(compact_prompt_slot("REQUEST", req, 480));
    parts.join("\n")
}

fn render_intent_normalizer_json_retry_prompt(
    route_view: &crate::task_context_builder::RouteContextView,
    context_bundle: &crate::task_context_builder::TaskContextBundle,
    auth_policy_context: &str,
    request_language_hint: &str,
    req: &str,
) -> String {
    let runtime_context = context_bundle
        .execution_view
        .as_ref()
        .map(|view| view.runtime_context.as_str())
        .filter(|runtime| {
            let trimmed = runtime.trim();
            !trimmed.is_empty() && trimmed != "<none>"
        })
        .map(str::to_string)
        .unwrap_or_else(|| compact_runtime_context_from_auth(auth_policy_context));
    let parts = vec![
        "JSON-only retry. Output one object now; start with `{` and stop after `}`. No reasoning, no markdown, no `<think>`.".to_string(),
        "Fill this route schema; use only listed machine tokens.".to_string(),
        "{\"resolved_user_intent\":\"...\",\"answer_candidate\":\"\",\"resume_behavior\":\"none\",\"schedule_kind\":\"none\",\"schedule_intent\":null,\"wants_file_delivery\":false,\"should_refresh_long_term_memory\":false,\"agent_display_name_hint\":\"\",\"needs_clarify\":false,\"clarify_question\":\"\",\"reason\":\"...\",\"confidence\":0.9,\"decision\":\"clarify|direct_answer|planner_execute\",\"output_contract\":{\"response_shape\":\"free|strict|scalar|one_sentence|file_token\",\"exact_sentence_count\":null,\"requires_content_evidence\":false,\"delivery_required\":false,\"locator_kind\":\"none|path|current_workspace|url|filename\",\"delivery_intent\":\"none|file_single|directory_lookup|directory_batch_files\",\"semantic_kind\":\"none|service_status|file_names|directory_names|directory_entry_groups|file_paths|raw_command_output|command_output_summary|execution_failed_step|generated_file_delivery|generated_file_path_report|filesystem_mutation_result|document_heading|scalar_count|quantity_comparison|git_repository_state|structured_keys|config_validation|config_mutation|config_risk_assessment|rss_news_fetch|web_page_summary|web_search_summary|weather_query|market_quote|image_understanding|publishing_preview|package_manager_detection|existence_with_path|existence_with_path_summary|file_basename\",\"locator_hint\":\"\",\"self_extension\":{\"mode\":\"none\",\"trigger\":\"none\",\"execute_now\":false}},\"execution_recipe\":{\"kind\":\"none\",\"profile\":\"none\",\"target_scope\":\"none\"},\"turn_type\":\"task_request|status_query|\",\"target_task_policy\":\"standalone|\",\"should_interrupt_active_run\":false,\"state_patch\":null,\"attachment_processing_required\":false}".to_string(),
        "Observable local/system/workspace inspection, command output, file/config reads, validation, risk assessment, listings, counts, and metadata => decision=\"planner_execute\" with requires_content_evidence=true.".to_string(),
        "Schedule operations use top-level schedule_kind only as none/create/update/delete/query; put once/daily/weekly/interval/cron under schedule_intent.schedule.type, use the current conversation/task as the default scheduled-reminder delivery context, and keep ordinary schedule output_contract semantic_kind=\"none\" with no local evidence or delivery requirement.".to_string(),
        "Main application configuration risk/security/audit/guard assessment => semantic_kind=\"config_risk_assessment\", locator_kind=\"path\", locator_hint=\"configs/config.toml\" unless another concrete config path is named. Preserve no-secret-leak requirements in resolved_user_intent; do not expose secret values.".to_string(),
        "Only use decision=\"clarify\" when a required target/action is genuinely missing. Do not ask the user to paste local files when a local target is named or implied by the application config contract.".to_string(),
        format!("LANG={request_language_hint}"),
        "For scalar_count, include output_contract.scalar_count_filter and state_patch.scalar_count_filter when object/scope is known. Directory-wide files-only or extension-filtered counts mean recursive=true unless direct/immediate/top-level scope is explicitly requested. Root-excluding directory counts mean recursive=true unless direct/immediate/top-level scope is explicitly requested; do not use recursive=false for that shape.".to_string(),
        compact_prompt_slot("RUNTIME", &runtime_context, 240),
        compact_prompt_slot("ACTIVE_TASK", &route_view.active_task_context, 180),
        compact_prompt_slot("ANCHOR", &route_view.active_execution_anchor_context, 180),
        compact_prompt_slot("CAPABILITIES", &route_view.capability_map, 600),
        compact_prompt_slot("RECENT", &route_view.recent_turns_full, 180),
        compact_prompt_slot("REQUEST", req, 680),
    ];
    parts.join("\n")
}

pub(super) async fn retry_intent_normalizer_json_parse(
    state: &AppState,
    task: &ClaimedTask,
    route_view: &crate::task_context_builder::RouteContextView,
    context_bundle: &crate::task_context_builder::TaskContextBundle,
    auth_policy_context: &str,
    request_language_hint: &str,
    req: &str,
    prompt_source: &str,
    base_repair_report: &ContractRepairReport,
) -> Option<(IntentNormalizerOut, ContractRepairReport)> {
    let prompt = render_intent_normalizer_json_retry_prompt(
        route_view,
        context_bundle,
        auth_policy_context,
        request_language_hint,
        req,
    );
    let retry_prompt_source = format!("{prompt_source}#retry=json_only");
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "intent_normalizer_retry_prompt",
        &retry_prompt_source,
        None,
        None,
    );
    let retry_out = match llm_gateway::run_with_fallback_with_hints(
        state,
        task,
        &prompt,
        &retry_prompt_source,
        crate::ChatRequestHints {
            temperature: Some(0.0),
            max_tokens: Some(4096),
        },
    )
    .await
    {
        Ok(out) => out,
        Err(err) => {
            warn!(
                "intent_normalizer parse retry llm failed: task_id={} err={}",
                task.task_id, err
            );
            return None;
        }
    };
    let (retry_out_for_parse, retry_report) =
        normalize_intent_normalizer_raw_for_schema_with_report(&retry_out, req);
    let parsed = crate::prompt_utils::validate_against_schema::<IntentNormalizerOut>(
        &retry_out_for_parse,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    );
    match parsed {
        Ok(validated) => {
            let mut report = base_repair_report.clone();
            report.add("llm_retry", "normalizer_parse_retry");
            report.merge(&retry_report);
            if !validated.raw_parse_ok || validated.schema_normalized {
                info!(
                    "{} intent_normalizer task_id={} parse_retry_recovery raw_parse_ok={} schema_normalized={} repair_source={} repair_detail={} input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    validated.raw_parse_ok,
                    validated.schema_normalized,
                    report.source_csv(),
                    report.detail_csv(),
                    crate::truncate_for_log(req)
                );
            } else {
                info!(
                    "{} intent_normalizer task_id={} parse_retry_success input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    crate::truncate_for_log(req)
                );
            }
            Some((validated.value, report))
        }
        Err(err) => {
            warn!(
                "intent_normalizer parse retry schema failed: task_id={} err={} normalized_raw={}",
                task.task_id,
                err,
                crate::truncate_for_log(&retry_out_for_parse)
            );
            None
        }
    }
}

pub(super) fn cap_intent_normalizer_prompt_for_llm_budget(
    state: &AppState,
    task: &ClaimedTask,
    prompt: String,
) -> String {
    let max_bytes = intent_normalizer_max_prompt_bytes(state, task);
    if prompt.len() <= max_bytes {
        return prompt;
    }
    warn!(
        "intent_normalizer_prompt oversized vs provider budget — truncating head+tail task_id={} bytes_before={} bytes_budget={}",
        task.task_id,
        prompt.len(),
        max_bytes
    );
    let head_take = (max_bytes * 35) / 100;
    let tail_take = (max_bytes * 55) / 100;
    let note_budget = max_bytes
        .saturating_sub(head_take)
        .saturating_sub(tail_take)
        .max(32);
    let note = format!(
        "\n\n[RustClaw: omitted {} bytes of middle context to fit provider window]\n\n",
        prompt
            .len()
            .saturating_sub(head_take.saturating_add(tail_take))
    );
    let head = crate::providers::utf8_safe_prefix(&prompt, head_take);
    let note = crate::providers::utf8_safe_prefix(&note, note_budget);
    let tail = crate::providers::utf8_safe_suffix(&prompt, tail_take);
    let capped = format!("{head}{note}{tail}");
    state.note_task_prompt_truncation_with_label(
        &task.task_id,
        "normalizer",
        prompt.len(),
        max_bytes,
        capped.len(),
    );
    capped
}

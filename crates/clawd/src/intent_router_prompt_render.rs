use tracing::{info, warn};

use crate::llm_gateway;

use super::{
    normalize_intent_normalizer_raw_for_schema_with_report, parse_output_contract, AppState,
    ClaimedTask, ContractRepairReport, IntentExecutionRecipeOut, IntentNormalizerOut,
    IntentOutputContractOut, OutputLocatorKind, SelfExtensionMode, ROUTING_POLICY_PERSONA_PROMPT,
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
    let _ = (state, task);
    intent_normalizer_compact_prompt_default_enabled()
}

pub(super) fn intent_normalizer_compact_prompt_default_enabled() -> bool {
    true
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

    let mut parts = Vec::new();
    parts.push(
        "Compact boundary normalizer. Output exactly one raw JSON object and then stop. No markdown, no user-visible answer text after JSON."
            .to_string(),
    );
    parts.push("This stage extracts boundaries only. The agent loop owns ordinary respond / clarify / act / synthesize decisions, capability choice, argument completion, and final wording.".to_string());
    parts.push("The raw-JSON requirement is internal to this normalizer. Preserve requested final formats, language, brevity, tables, CSV, files, or artifacts in resolved_user_intent and structured fields for the agent loop.".to_string());
    parts.push("Prefer the compact boundary envelope. Runtime fills missing compatibility schema slots with neutral defaults, so only emit extra compatibility fields when they carry explicit boundary facts.".to_string());
    parts.push("boundary_envelope is the primary machine-only output: schema_version=1, raw_chars as request character count, optional language_hint, schedule_intent, attachment_refs, explicit_locators, active_task_reference, session_binding, and safety_budget_hint. Never put raw user text, answer text, or route decisions inside it.".to_string());
    parts.push("Do not emit legacy decision fields; runtime derives any route trace from machine boundary fields.".to_string());
    parts.push("Do not emit answer_candidate. Runtime may clear legacy provider output, but live normalizer prompts must carry recall facts, IDs, constraints, and visible prior values through resolved_user_intent or state_patch for the loop.".to_string());
    parts.push("Boundary extraction scope: language hint, explicit locators, attachment references, schedule intent, delivery/artifact intent, active-task references, temporary alias bindings, missing boundary blockers, memory-refresh preference, safety/budget hints, and current-request constraints.".to_string());
    parts.push("Do not classify ordinary capability families in this prompt. Preserve any machine capability_ref token already present, but let the planner/resolver choose from CAPABILITIES.".to_string());
    parts.push("output_contract is an optional compatibility evidence/delivery envelope, not a capability router. If emitted, keep contract_marker=\"none\" unless an existing machine context already provided a compatibility marker; never create or select feature contract markers to make one natural-language case pass.".to_string());
    parts.push("Use execution-signal machine fields only when the current request genuinely needs observation, tool execution, side effects, delivery, scheduling, or attachment processing. Signals include requires_content_evidence=true, delivery_required=true, wants_file_delivery=true, attachment_processing_required=true, schedule_kind!=none, execution_recipe.kind!=none, or structured state_patch runtime fields.".to_string());
    parts.push("needs_clarify is only for a missing required boundary that the loop cannot safely infer: absent target/locator, ambiguous referenced object, unsafe scope, missing approval choice, or incomplete schedule fields. Do not ask for optional style/preferences before the loop can proceed.".to_string());
    parts.push("When needs_clarify=true, ask exactly one concise question in clarify_question using REQUEST language hint when clear. Preserve the intended future boundary contract; do not erase delivery, schedule, locator, attachment, or evidence fields only because one required slot is missing.".to_string());
    parts.push("Active-task and resume binding are boundary facts. Reuse active task only when the current request explicitly modifies, narrows, corrects, resumes, or asks about that task. A complete current request with its own object, deliverable, scope, or constraints is standalone.".to_string());
    parts.push("Temporary aliases are session bindings only. Resolve aliases already defined in ALIASES when relevant; for new alias mappings, write state_patch.alias_bindings with machine fields. Do not infer aliases from vague references.".to_string());
    parts.push("Current REQUEST is authoritative. RECENT, ASSISTANT, MEMORY, and LAST are background for deictic binding and recall; memory scores are metadata, not user facts. Do not import stale paths, stale failures, or old capability claims unless the current request explicitly points to them.".to_string());
    parts.push("For observable local/system/workspace requests, do not answer from model knowledge inside the normalizer and do not ask the user to paste local file contents. Expose boundary/evidence fields and let the agent loop call tools or capabilities.".to_string());
    parts.push("For ordinary chat, greetings, confirmations, memory-only statements, preferences, and harmless discussion that do not require fresh evidence or side effects: needs_clarify=false, execution_recipe.kind=\"none\" when present, no evidence/delivery flags, and state_patch null unless memory/alias binding is requested.".to_string());
    parts.push("If output_contract is emitted, allowed keys only: response_shape, exact_sentence_count, requires_content_evidence, delivery_required, locator_kind, delivery_intent, contract_marker, locator_hint, scalar_count_filter, list_selector, self_extension. Do not emit custom keys or natural-language mini-schemas.".to_string());
    parts.push("Allowed response_shape: free, one_sentence, strict, scalar, file_token. Allowed locator_kind: none, path, current_workspace, url, filename. Allowed delivery_intent: none, file_single, directory_lookup, directory_batch_files.".to_string());
    parts.push("Allowed top-level schedule_kind: none, create, update, delete, query. Schedule frequency/type belongs inside schedule_intent. A scheduled reminder uses the current conversation/task as default delivery context unless the user supplies another channel.".to_string());
    parts.push("Allowed execution_recipe.kind: none or ops_closed_loop. Use ops_closed_loop only when the current request requires a change plus a separate machine-verifiable validation step; read-only inspection stays kind=none.".to_string());
    parts.push("state_patch may carry only machine fields such as alias_bindings, deictic_reference, ordered_entry_ref, scalar_count_filter, structured_field_selector, runtime_status_query, runtime_async_job_start, required_machine_fields, required_content_literals, forbidden_visible_literals, replacement_pairs, quantity_comparison, or primary_task_update. Do not put localized prose in machine fields.".to_string());
    parts.push("Every enum field must contain one exact schema token. Put nuance in resolved_user_intent or structured machine fields; never output aliases, combined tokens, localized words, or explanatory prose as enum values.".to_string());
    parts.push("Keep resolved_user_intent concise but complete: include the user's actual goal, constraints, explicit locators, output/delivery requirements, and relevant bound context. Preserve exact IDs, paths, URLs, field names, counts, and quoted literals.".to_string());
    parts.push(compact_prompt_slot(
        "ACTIVE_TASK",
        &route_view.active_task_context,
        640,
    ));
    parts.push(compact_prompt_slot(
        "ANCHOR",
        &route_view.active_execution_anchor_context,
        420,
    ));
    parts.push(compact_prompt_slot(
        "ALIASES",
        &route_view.session_alias_context,
        260,
    ));
    parts.push(compact_prompt_slot(
        "HINTS",
        &route_view.request_surface_hints,
        180,
    ));
    parts.push(compact_prompt_slot(
        "CAPABILITIES",
        &route_view.capability_map,
        1400,
    ));
    parts.push(compact_prompt_slot("AUTH", auth_policy_context, 220));
    parts.push(compact_prompt_slot(
        "RECENT",
        &route_view.recent_turns_full,
        900,
    ));
    parts.push(compact_prompt_slot("LAST", &route_view.last_turn_full, 240));
    parts.push(format!("LANG={request_language_hint}"));
    parts.push(compact_prompt_slot(
        "MEMORY",
        &route_view.memory_context,
        560,
    ));
    parts.push(compact_prompt_slot(
        "ASSISTANT",
        &route_view.recent_assistant_replies,
        280,
    ));
    parts.push(compact_prompt_slot("RUNTIME", &runtime_context, 300));
    parts.push(compact_prompt_slot(
        "ACTIVE_TASK",
        &route_view.active_task_context,
        320,
    ));
    parts.push(compact_prompt_slot(
        "ANCHOR",
        &route_view.active_execution_anchor_context,
        300,
    ));
    parts.push("TAIL_GUARDS: SUMMARY_RECALL uses visible recent/memory facts without emitting answer_candidate; memory scores are metadata. FOLLOWUP_ANCHOR_PRIORITY uses ANCHOR/ACTIVE_TASK/ALIASES before stale MEMORY for deictic references. BOUNDARY_ONLY no ordinary capability-family routing, no feature contract-marker selection, no user-visible normalizer answer. LOCAL_EXEC observable local/system/workspace requests expose machine evidence boundaries and let the loop act. RUNTIME_STATUS only emits structured runtime_status_query when the requested status is already a runtime boundary fact.".to_string());
    parts.push(compact_prompt_slot("REQUEST", req, 560));
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
        "JSON-only retry for boundary normalizer. Output one object now; start with `{` and stop after `}`. No reasoning, markdown, or user-visible answer.".to_string(),
        "Emit boundary machine tokens only. Runtime fills missing compatibility schema slots with neutral defaults; the agent loop owns ordinary respond/clarify/act decisions and capability choice.".to_string(),
        "Preserve capability_ref tokens only when already present in context; do not choose a feature family here. If output_contract is emitted, keep contract_marker=\"none\" unless an existing machine context already provided a compatibility marker.".to_string(),
        "Do not emit answer_candidate. Put user goals, constraints, exact locators, IDs, paths, URLs, schedule details, delivery requirements, and missing boundary blockers in boundary_envelope, resolved_user_intent, schedule_intent, execution_recipe, or state_patch.".to_string(),
        "{\"boundary_envelope\":{\"schema_version\":1,\"raw_chars\":0,\"language_hint\":null,\"schedule_intent\":null,\"attachment_refs\":[],\"explicit_locators\":[],\"active_task_reference\":null,\"session_binding\":null,\"safety_budget_hint\":null},\"resolved_user_intent\":\"...\",\"needs_clarify\":false,\"clarify_question\":\"\",\"reason\":\"boundary_only\",\"confidence\":0.9}".to_string(),
        "Set needs_clarify=true only for a missing required boundary such as absent locator, ambiguous referenced object, unsafe scope, incomplete schedule fields, or missing approval choice. Ask one concise question.".to_string(),
        "If observation, side effects, delivery, scheduling, attachment processing, or background execution is required, expose the relevant machine boundary fields; do not synthesize results in the normalizer.".to_string(),
        format!("LANG={request_language_hint}"),
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
    base_llm_out_for_parse: &str,
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
            let mut value = validated.value;
            preserve_base_execution_recipe_for_retry(
                &mut value,
                base_llm_out_for_parse,
                &mut report,
            );
            preserve_base_output_contract_for_retry(
                &mut value,
                base_llm_out_for_parse,
                &mut report,
            );
            Some((value, report))
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

pub(super) fn preserve_base_execution_recipe_for_retry(
    retry_out: &mut IntentNormalizerOut,
    base_llm_out_for_parse: &str,
    report: &mut ContractRepairReport,
) {
    if execution_recipe_declares_agent_loop_execution(retry_out.execution_recipe.as_ref()) {
        return;
    }
    let Some(base_recipe) = serde_json::from_str::<serde_json::Value>(base_llm_out_for_parse)
        .ok()
        .and_then(|value| value.get("execution_recipe").cloned())
        .and_then(|value| serde_json::from_value::<IntentExecutionRecipeOut>(value).ok())
    else {
        return;
    };
    if !execution_recipe_declares_agent_loop_execution(Some(&base_recipe)) {
        return;
    }
    retry_out.execution_recipe = Some(base_recipe);
    report.add("llm_retry", "preserved_base_execution_recipe");
}

pub(super) fn preserve_base_output_contract_for_retry(
    retry_out: &mut IntentNormalizerOut,
    base_llm_out_for_parse: &str,
    report: &mut ContractRepairReport,
) {
    if output_contract_declares_retry_boundary_signal(retry_out.output_contract.as_ref()) {
        return;
    }
    let Some(base_contract) = serde_json::from_str::<serde_json::Value>(base_llm_out_for_parse)
        .ok()
        .and_then(|value| value.get("output_contract").cloned())
        .and_then(|value| serde_json::from_value::<IntentOutputContractOut>(value).ok())
    else {
        return;
    };
    if !output_contract_declares_retry_boundary_signal(Some(&base_contract)) {
        return;
    }
    retry_out.output_contract = Some(base_contract);
    report.add("llm_retry", "preserved_base_output_contract");
}

fn output_contract_declares_retry_boundary_signal(
    contract: Option<&IntentOutputContractOut>,
) -> bool {
    let Some(contract) = contract else {
        return false;
    };
    let parsed = parse_output_contract(Some(contract.clone()), false);
    let has_locator_boundary = match parsed.locator_kind {
        OutputLocatorKind::None => !parsed.locator_hint.trim().is_empty(),
        OutputLocatorKind::CurrentWorkspace => true,
        _ => !parsed.locator_hint.trim().is_empty(),
    };
    parsed.requires_content_evidence
        || has_locator_boundary
        || parsed.self_extension.scalar_count_filter.has_constraints()
        || parsed.self_extension.list_selector.target_kind_specified
        || parsed.self_extension.list_selector.limit.is_some()
        || parsed.self_extension.list_selector.sort_by.is_some()
        || parsed
            .self_extension
            .list_selector
            .include_metadata
            .is_some()
        || parsed.self_extension.list_selector.include_hidden.is_some()
        || !matches!(parsed.self_extension.mode, SelfExtensionMode::None)
        || parsed.self_extension.execute_now
        || parsed
            .self_extension
            .structured_field_selector
            .as_deref()
            .is_some_and(|selector| !selector.trim().is_empty())
}

fn execution_recipe_declares_agent_loop_execution(
    recipe: Option<&IntentExecutionRecipeOut>,
) -> bool {
    let Some(recipe) = recipe else {
        return false;
    };
    let kind = recipe.kind.trim();
    (!kind.is_empty() && !kind.eq_ignore_ascii_case("none"))
        || [
            recipe.command.as_str(),
            recipe.cmd.as_str(),
            recipe.shell_command.as_str(),
            recipe.execution_mode.as_str(),
            recipe.async_adapter_kind.as_str(),
        ]
        .iter()
        .any(|value| !value.trim().is_empty())
        || recipe.attachment_processing_required
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

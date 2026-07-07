use tracing::info;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{
    direct_scalar_observed_answer, direct_structured_observed_answer,
    generated_delivery_existing_file_content_synthesis_token,
    last_respond_matches_single_line_observation, latest_publishable_respond_step_output,
    latest_tail_read_range_answer_from_loop, log_deterministic_delivery_record,
    looks_like_raw_command_snapshot, looks_like_structured_machine_output,
    output_contract_requests_exact_delivery, planned_delivery_is_publishable_model_language_answer,
    publishable_summary_has_multi_source_observation,
    route_requires_compound_content_file_delivery, route_requires_raw_tail_read_passthrough,
    strict_raw_command_output_exact_observation_answer, valid_publishable_synthesis_output,
};

pub(super) async fn enforce_delivery_output_contract(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return;
    };
    if loop_state.delivery_messages.is_empty()
        && loop_state
            .last_user_visible_respond
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
    {
        return;
    }
    let publishable_synthesis = valid_publishable_synthesis_output(loop_state).map(str::to_string);
    if let Some(synthesis) = publishable_synthesis
        .as_deref()
        .filter(|_| route_requires_compound_content_file_delivery(agent_run_context))
    {
        let mut delivery_messages = loop_state
            .delivery_messages
            .iter()
            .filter(|message| crate::finalize::is_execution_summary_message(message))
            .cloned()
            .collect::<Vec<_>>();
        delivery_messages.push(synthesis.to_string());
        loop_state.last_user_visible_respond = Some(synthesis.to_string());
        loop_state.delivery_messages = delivery_messages;
        return;
    }
    if let Some(synthesis) = publishable_synthesis
        .as_deref()
        .filter(|text| route_accepts_filesystem_mutation_synthesis(route, text))
    {
        let mut delivery_messages = loop_state
            .delivery_messages
            .iter()
            .filter(|message| crate::finalize::is_execution_summary_message(message))
            .cloned()
            .collect::<Vec<_>>();
        append_delivery_message(&task.task_id, &mut delivery_messages, synthesis.to_string());
        loop_state.last_user_visible_respond = Some(synthesis.to_string());
        loop_state.delivery_messages = delivery_messages;
        log_deterministic_delivery_record(
            &task.task_id,
            "final_result_use_filesystem_mutation_synthesis",
            "kept",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }
    if let (Some(synthesis), Some(token)) = (
        publishable_synthesis.as_deref(),
        generated_delivery_existing_file_content_synthesis_token(
            state,
            loop_state,
            agent_run_context,
        ),
    ) {
        let mut delivery_messages = loop_state
            .delivery_messages
            .iter()
            .filter(|message| crate::finalize::is_execution_summary_message(message))
            .cloned()
            .collect::<Vec<_>>();
        append_delivery_message(&task.task_id, &mut delivery_messages, synthesis.to_string());
        append_delivery_message(&task.task_id, &mut delivery_messages, token);
        loop_state.last_user_visible_respond = Some(synthesis.to_string());
        loop_state.delivery_messages = delivery_messages;
        return;
    }
    if let Some(synthesis) = publishable_synthesis
        .as_deref()
        .or_else(|| latest_publishable_respond_step_output(loop_state))
        .filter(|text| route_prefers_content_evidence_synthesis(route, text))
    {
        let mut delivery_messages = loop_state
            .delivery_messages
            .iter()
            .filter(|message| crate::finalize::is_execution_summary_message(message))
            .cloned()
            .collect::<Vec<_>>();
        append_delivery_message(&task.task_id, &mut delivery_messages, synthesis.to_string());
        loop_state.last_user_visible_respond = Some(synthesis.to_string());
        loop_state.delivery_messages = delivery_messages;
        log_deterministic_delivery_record(
            &task.task_id,
            "content_evidence_keep_publishable_synthesis",
            "kept",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }
    let seed_text = if publishable_synthesis.is_some() {
        loop_state
            .last_publishable_synthesis_output
            .clone()
            .or_else(|| loop_state.delivery_messages.last().cloned())
            .or_else(|| loop_state.last_user_visible_respond.clone())
            .unwrap_or_default()
    } else {
        loop_state
            .last_user_visible_respond
            .clone()
            .or_else(|| loop_state.delivery_messages.last().cloned())
            .unwrap_or_default()
    };
    if route_requires_raw_tail_read_passthrough(Some(route)) {
        if let Some(answer) = latest_tail_read_range_answer_from_loop(loop_state, false) {
            let answer = answer.trim().to_string();
            let seed_matches = seed_text.trim() == answer;
            let delivery_matches = loop_state
                .delivery_messages
                .iter()
                .rev()
                .find(|message| !crate::finalize::is_execution_summary_message(message))
                .is_some_and(|message| message.trim() == answer);
            if !answer.is_empty() && (seed_matches || delivery_matches) {
                loop_state.last_user_visible_respond = Some(answer.clone());
                loop_state.delivery_messages = vec![answer];
                log_deterministic_delivery_record(
                    &task.task_id,
                    "final_result_keep_strict_raw_tail_read_observation",
                    "kept",
                    agent_run_context,
                    loop_state.executed_step_results.len(),
                );
                return;
            }
        }
    }
    if strict_raw_command_output_exact_observation_answer(route, loop_state, &seed_text) {
        let answer = seed_text.trim().to_string();
        loop_state.last_user_visible_respond = Some(answer.clone());
        loop_state.delivery_messages = vec![answer];
        log_deterministic_delivery_record(
            &task.task_id,
            "final_result_use_exact_raw_command_observation",
            "kept",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }
    let (mut normalized_text, mut normalized_messages) =
        crate::intercept_response_payload_for_delivery(
            state,
            user_text,
            route.wants_file_delivery,
            &route.output_contract,
            seed_text,
            loop_state.delivery_messages.clone(),
        );
    if publishable_synthesis.is_some() && !normalized_text.trim().is_empty() {
        let mut rewritten_messages = normalized_messages
            .into_iter()
            .filter(|message| crate::finalize::is_execution_summary_message(message))
            .collect::<Vec<_>>();
        rewritten_messages.push(normalized_text.clone());
        normalized_messages = rewritten_messages;
    }

    // §7.1 output_contract verifier hook：在 enforce_output_contract 的"shape 整形"
    // 之后再做一层最小结构合规性判定。不要在这里用自然语言词表判断 yes/no、
    // same/different、语气或意图；这些交给 LLM composer/prompt。
    // 三态结果：
    // - Pass：已合规，原文直出。
    // - Reshape：候选基本合规但可结构化抽取严格值（如 scalar path/count），verifier
    //   给出已修复文本，直接覆盖 normalized_text。
    // - Reject：候选明显违反结构 contract（如 strict scalar 缺路径/整数），走 §7.2
    //   ClarifyFallbackSource::VerifyRejected fallback，丢弃 candidate。
    // 三种情况都打 tracing event verify_contract_emitted，便于 inspect_task.sh 关联。
    if !normalized_text.trim().is_empty()
        && !model_language_evidence_summary_should_skip_low_level_reshape(
            route,
            loop_state,
            &normalized_text,
        )
    {
        let verdict = crate::output_contract_verifier::verify_output_contract(
            &route.output_contract,
            &normalized_text,
            user_text,
        );
        match &verdict {
            crate::output_contract_verifier::OutputContractVerdict::Pass => {
                info!(
                    "verify_contract_emitted task_id={} owner_layer={} verdict=pass response_shape={:?} contract_marker={:?}",
                    task.task_id,
                    verdict.owner_layer(),
                    route.output_contract.response_shape,
                    route.effective_output_contract_semantic_kind(),
                );
            }
            crate::output_contract_verifier::OutputContractVerdict::Reshape {
                reason_code,
                reason,
                reshaped,
            } => {
                info!(
                    "verify_contract_emitted task_id={} owner_layer={} verdict=reshape response_shape={:?} contract_marker={:?} reason_code={} reason={} from={} to={}",
                    task.task_id,
                    verdict.owner_layer(),
                    route.output_contract.response_shape,
                    route.effective_output_contract_semantic_kind(),
                    reason_code,
                    reason,
                    crate::truncate_for_log(&normalized_text),
                    crate::truncate_for_log(reshaped),
                );
                normalized_text = reshaped.clone();
                if let Some(last) = normalized_messages.last_mut() {
                    *last = reshaped.clone();
                } else {
                    normalized_messages.push(reshaped.clone());
                }
            }
            crate::output_contract_verifier::OutputContractVerdict::Reject {
                reason_code,
                reason,
            } => {
                info!(
                    "verify_contract_emitted task_id={} owner_layer={} verdict=reject response_shape={:?} contract_marker={:?} reason_code={} reason={} dropped_candidate={}",
                    task.task_id,
                    verdict.owner_layer(),
                    route.output_contract.response_shape,
                    route.effective_output_contract_semantic_kind(),
                    reason_code,
                    reason,
                    crate::truncate_for_log(&normalized_text),
                );
                let language_hint =
                    crate::language_policy::task_response_language_hint(state, task, user_text);
                let contract = crate::fallback::UserResponseContract::verify_rejected(
                    user_text,
                    &route.resolved_intent,
                    &format!("{:?}", route.output_contract.response_shape),
                    &format!("{:?}", route.effective_output_contract_semantic_kind()),
                    reason_code,
                    reason,
                    &language_hint,
                );
                let fallback_text = crate::fallback::compose_user_response_from_contract(
                    state,
                    task,
                    &contract,
                    crate::fallback::ClarifyFallbackSource::VerifyRejected,
                );
                let fallback_text = fallback_text.await;
                normalized_text = fallback_text.clone();
                normalized_messages = vec![fallback_text];
            }
        }
    }

    loop_state.last_user_visible_respond =
        (!normalized_text.trim().is_empty()).then_some(normalized_text);
    loop_state.delivery_messages = normalized_messages;
}

fn model_language_evidence_summary_should_skip_low_level_reshape(
    route: &crate::RouteResult,
    loop_state: &LoopState,
    candidate: &str,
) -> bool {
    let contract = route.effective_output_contract();
    if contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return false;
    }
    if !matches!(
        crate::evidence_policy::final_answer_shape_for_route(route),
        Some(
            crate::evidence_policy::FinalAnswerShape::SummaryWithEvidence
                | crate::evidence_policy::FinalAnswerShape::RawOutputOrShortSummary
        )
    ) {
        return false;
    }
    let candidate = candidate.trim();
    if !planned_delivery_is_publishable_model_language_answer(candidate) {
        return false;
    }
    if route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::RawCommandOutput,
        crate::OutputSemanticKind::CommandOutputSummary,
    ]) && !publishable_summary_has_multi_source_observation(loop_state)
    {
        return false;
    }
    let nonempty_lines = candidate
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();
    let token_count = candidate.split_whitespace().count();
    let char_count = candidate.chars().count();
    nonempty_lines > 1 || token_count >= 8 || char_count >= 64
}

pub(super) fn route_accepts_filesystem_mutation_synthesis(
    route: &crate::RouteResult,
    synthesis: &str,
) -> bool {
    route.output_contract_marker_is(crate::OutputSemanticKind::FilesystemMutationResult)
        && filesystem_mutation_synthesis_payload_is_complete(synthesis)
}

fn filesystem_mutation_synthesis_payload_is_complete(synthesis: &str) -> bool {
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(synthesis.trim()) else {
        return false;
    };
    payload
        .pointer("/contract_marker")
        .and_then(serde_json::Value::as_str)
        == Some("filesystem_mutation_result")
        && payload
            .pointer("/status")
            .and_then(serde_json::Value::as_str)
            == Some("ok")
        && payload
            .pointer("/steps")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|steps| !steps.is_empty())
}

pub(super) fn route_prefers_content_evidence_synthesis(
    route: &crate::RouteResult,
    synthesis: &str,
) -> bool {
    let contract = route.effective_output_contract();
    let content_summary_contract = route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::ContentExcerptSummary,
        crate::OutputSemanticKind::ContentExcerptWithSummary,
        crate::OutputSemanticKind::ExcerptKindJudgment,
        crate::OutputSemanticKind::WorkspaceProjectSummary,
    ]) && !matches!(
        contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    );
    if !contract.requires_content_evidence
        || contract.delivery_required
        || (output_contract_requests_exact_delivery(route) && !content_summary_contract)
        || synthesis.trim().is_empty()
        || crate::finalize::parse_delivery_token(synthesis).is_some()
        || crate::finalize::looks_like_planner_artifact(synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(synthesis)
    {
        return false;
    }
    content_summary_contract
}
pub(super) async fn discard_meta_respond_placeholder_for_content_evidence(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    requires_content_evidence: bool,
    agent_run_context: Option<&AgentRunContext>,
) {
    let Some(last_respond) = loop_state.last_user_visible_respond.as_deref() else {
        return;
    };
    let respond = last_respond.trim();
    let Some(raw_passthrough) = should_drop_passthrough_delivery_for_content_evidence(
        loop_state,
        requires_content_evidence,
        agent_run_context,
        respond,
    ) else {
        return;
    };
    if content_evidence_terminal_respond_is_contractual_answer(
        loop_state,
        agent_run_context,
        respond,
    ) {
        info!(
            "content_evidence_keep_contractual_terminal_respond task_id={} text={}",
            task.task_id,
            crate::truncate_for_log(respond)
        );
        return;
    }
    // §3.4 finalize-tier: drop_passthrough_delivery 是 finalize 决策层。
    let meta_placeholder =
        crate::semantic_judge::is_meta_respond_instruction(state, task, respond).await;
    if !raw_passthrough && !meta_placeholder {
        return;
    }
    info!(
        "content_evidence_drop_passthrough_respond task_id={} raw_passthrough={} meta_placeholder={} text={}",
        task.task_id,
        raw_passthrough,
        meta_placeholder,
        crate::truncate_for_log(respond)
    );
    loop_state.delivery_messages.clear();
    loop_state.last_user_visible_respond = None;
}

pub(super) fn should_drop_passthrough_delivery_for_content_evidence(
    loop_state: &LoopState,
    requires_content_evidence: bool,
    agent_run_context: Option<&AgentRunContext>,
    respond: &str,
) -> Option<bool> {
    if loop_state.pending_user_input_required {
        return None;
    }
    if !requires_content_evidence {
        return None;
    }
    if !loop_state.has_tool_or_skill_output {
        return None;
    }
    if loop_state.delivery_messages.len() != 1 {
        return None;
    }
    let delivery = loop_state.delivery_messages[0].trim();
    let respond = respond.trim();
    if delivery.is_empty() || respond.is_empty() || delivery != respond {
        return None;
    }

    let route_has_semantic_answer_contract = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| !route.output_contract_is_unclassified());
    let direct_structured_answer = route_has_semantic_answer_contract
        .then(|| direct_structured_observed_answer(None, loop_state, agent_run_context))
        .flatten()
        .map(|(answer, _)| answer);
    let direct_observed_answer_matches =
        direct_scalar_observed_answer(None, loop_state, agent_run_context)
            .map(|(answer, _)| answer)
            .into_iter()
            .chain(direct_structured_answer)
            .any(|answer| answer.trim() == respond);
    if direct_observed_answer_matches {
        return Some(false);
    }
    if last_respond_matches_single_line_observation(loop_state, respond) {
        return Some(false);
    }

    let raw_passthrough = loop_state
        .executed_step_results
        .iter()
        .rfind(|step| {
            step.is_ok() && !matches!(step.skill.as_str(), "respond" | "synthesize_answer")
        })
        .and_then(|step| {
            let body = step.output.as_deref()?.trim();
            if body.is_empty() {
                return None;
            }
            if respond == body {
                return Some(true);
            }
            (step.skill == "list_dir"
                && crate::agent_engine::observed_output::normalized_observed_listing(body)
                    .is_some_and(|listing| {
                        listing.trim() == respond
                            || listing
                                .lines()
                                .map(str::trim)
                                .any(|entry| !entry.is_empty() && entry == respond)
                    }))
            .then_some(true)
        })
        .unwrap_or(false);
    Some(raw_passthrough)
}

pub(super) fn content_evidence_terminal_respond_is_contractual_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    respond: &str,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route.output_contract.requires_content_evidence {
        return false;
    }
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free
            | crate::OutputResponseShape::OneSentence
            | crate::OutputResponseShape::Strict
    ) {
        return false;
    }
    if matches!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::RawCommandOutput
    ) {
        return strict_raw_command_output_exact_observation_answer(route, loop_state, respond);
    }
    let has_answer_semantic = !matches!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::None
    );
    let has_constrained_answer_shape = matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Strict
    );
    if !has_answer_semantic && !has_constrained_answer_shape {
        return false;
    }
    let answer = respond.trim();
    if answer.is_empty()
        || answer.chars().count() > 800
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || crate::finalize::is_execution_summary_message(answer)
        || looks_like_structured_machine_output(answer)
        || looks_like_raw_command_snapshot(answer)
    {
        return false;
    }
    if crate::finalize::parse_delivery_token(answer).is_some() {
        return true;
    }
    let has_successful_observation = loop_state.executed_step_results.iter().any(|step| {
        step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
            && step
                .output
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| !output.is_empty())
    });
    if !has_successful_observation {
        return false;
    }
    !matches!(
        crate::output_contract_verifier::verify_output_contract(
            &route.output_contract,
            answer,
            &route.resolved_intent,
        ),
        crate::output_contract_verifier::OutputContractVerdict::Reject { .. }
    )
}

pub(super) fn discard_raw_passthrough_delivery_when_structured_answer_available(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    if loop_state.pending_user_input_required {
        return;
    }
    if loop_state.delivery_messages.len() != 1 {
        return;
    }
    let Some(current_delivery) = loop_state.delivery_messages.last().map(|v| v.trim()) else {
        return;
    };
    if current_delivery.is_empty() {
        return;
    }
    let raw_passthrough = loop_state
        .executed_step_results
        .iter()
        .rfind(|step| {
            step.is_ok() && !matches!(step.skill.as_str(), "respond" | "synthesize_answer")
        })
        .and_then(|step| {
            let body = step.output.as_deref()?.trim();
            if body.is_empty() {
                return None;
            }
            if current_delivery == body {
                return Some(true);
            }
            let first_line = body.lines().map(str::trim).find(|line| !line.is_empty())?;
            (current_delivery == first_line).then_some(true)
        })
        .unwrap_or(false);
    if !raw_passthrough {
        return;
    }
    if last_respond_matches_single_line_observation(loop_state, current_delivery) {
        return;
    }

    let structured_answer = direct_structured_observed_answer(None, loop_state, agent_run_context)
        .map(|(answer, _)| answer.trim().to_string())
        .filter(|answer| !answer.is_empty() && answer != current_delivery);

    let exact_delivery_requested = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(output_contract_requests_exact_delivery)
        .unwrap_or(false);
    if structured_answer.is_none()
        && (exact_delivery_requested
            || !crate::agent_engine::observed_output::has_observed_answer_candidates(loop_state))
    {
        return;
    }

    info!(
        "drop_raw_passthrough_delivery_for_structured_answer task_id={} raw={} structured={}",
        task.task_id,
        crate::truncate_for_log(current_delivery),
        crate::truncate_for_log(structured_answer.as_deref().unwrap_or("<synthesis>"))
    );
    loop_state.delivery_messages.clear();
    loop_state.last_user_visible_respond = None;
}

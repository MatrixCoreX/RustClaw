async fn run_agent_with_loop_seeded_and_initial_plan(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    resume_checkpoint: Option<&crate::task_lifecycle::TaskCheckpoint>,
    initial_plan: Option<&crate::PlanResult>,
    initial_task_observations: &[Value],
) -> Result<AskReply, String> {
    let base_policy = load_agent_loop_guard_policy(state);
    let mut task_budget_policy =
        crate::task_budget_contract::load_task_budget_policy(&state.skill_rt.workspace_root);
    clamp_child_task_budget_policy(task, &mut task_budget_policy);
    let mut loop_state = LoopState::new();
    super::seed_loop_state_for_agent_run(&mut loop_state, agent_run_context, resume_checkpoint);
    loop_state
        .task_observations
        .extend(initial_task_observations.iter().cloned());
    record_session_start_hooks(state, task, user_text, &mut loop_state).await;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        initial_execution_recipe_spec(goal, user_text, agent_run_context),
    );
    let budget_profile =
        AgentLoopGuardPolicy::budget_profile_for_context(loop_state.execution_recipe, None);
    let mut policy = base_policy.adjusted_for_context(loop_state.execution_recipe, None);
    clamp_child_loop_guard_policy(task, &mut policy);
    base_policy.apply_recipe_runtime_overrides(&mut loop_state.execution_recipe);
    let enabled_rollout_switches = policy.enabled_rollout_switches();
    if !enabled_rollout_switches.is_empty() {
        loop_state.output_vars.insert(
            "rollout_switches_enabled".to_string(),
            enabled_rollout_switches.join(","),
        );
    }
    info!(
        "loop_budget_profile task_id={} profile={} max_actions_per_turn={} repeat_action_limit={}",
        task.task_id,
        budget_profile.as_str(),
        policy.max_actions_per_turn,
        policy.repeat_action_limit
    );
    // A resumed checkpoint carries settled `turn:<round>` allocations. Reusing
    // round 1 makes TaskBudgetSlice::allocate reject the restored planner turn
    // as a duplicate and skips directly to finalization.
    let mut round = initial_round_for_agent_loop(&loop_state);
    let initial_plan_round = round;
    let loop_started_at = Instant::now();
    initialize_task_budget_slice(&mut loop_state, budget_profile, &task_budget_policy);
    let mut effective_user_text = user_text.to_string();
    restore_applied_task_steering(state, task, &mut effective_user_text);
    restore_applied_conversation_inputs(state, task, &mut effective_user_text, &mut loop_state);
    let mut skip_planner_rounds = false;
    loop {
        if !skip_planner_rounds {
            loop {
                ensure_task_running(state, task)?;
                if let ActiveTaskBoundaryControl::Pause {
                    control_seq,
                    resume_after,
                } = apply_active_task_boundary_controls(
                    state,
                    task,
                    &mut effective_user_text,
                    &mut loop_state,
                )? {
                    loop_state.last_stop_signal = Some("user_pause_requested".to_string());
                    publish_agent_loop_pause_checkpoint(state, task, &mut loop_state, resume_after);
                    crate::repo::apply_task_control_directive(
                        state,
                        &task.task_id,
                        control_seq,
                        "pause_checkpoint_created",
                    )
                    .map_err(|error| format!("task_pause_apply_failed:{error}"))?;
                    break;
                }
                loop_state.round_no = round;
                if task_budget_soft_slice_exhausted(loop_started_at, &loop_state) {
                    let decision = observe_task_budget(
                        state,
                        task,
                        &mut loop_state,
                        None,
                        loop_started_at,
                        true,
                    );
                    loop_state.last_stop_signal = Some("task_budget_slice_exhausted".to_string());
                    if matches!(
                        decision,
                        crate::task_budget_contract::BudgetDecision::CheckpointRequeue
                    ) {
                        publish_agent_loop_checkpoint_progress(
                            state,
                            task,
                            &mut loop_state,
                            "task_budget_slice_exhausted",
                        );
                    }
                    break;
                }
                super::maybe_publish_execution_recipe_phase_hint(state, task, &mut loop_state);
                let allocation_id = format!("turn:{}", round);
                let model_turns_before = state.task_llm_call_count(&task.task_id) as u64;
                let tool_calls_before = loop_state.tool_calls_total as u64;
                let elapsed_before = loop_started_at
                    .elapsed()
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64;
                let cost_before = state.task_llm_cost_summary(&task.task_id);
                let turn_allocated = loop_state
                    .task_budget_slice
                    .as_mut()
                    .and_then(|slice| {
                        let remaining_turns = slice
                            .hard_ceilings
                            .model_turns
                            .saturating_sub(slice.cumulative_model_turns)
                            .max(1);
                        let remaining_tokens = slice.hard_ceilings.total_tokens.saturating_sub(
                            slice
                                .cumulative_input_tokens
                                .saturating_add(slice.cumulative_output_tokens),
                        );
                        slice.allocate(
                            allocation_id.clone(),
                            format!("round:{round}"),
                            crate::task_budget_contract::BudgetAllocationKind::ModelTurn,
                            crate::task_budget_contract::BudgetUnits {
                                model_turns: 1,
                                tool_calls: policy.max_actions_per_turn as u64,
                                tokens: remaining_tokens.saturating_add(remaining_turns - 1)
                                    / remaining_turns,
                                elapsed_ms: slice.soft_slice_ms,
                            },
                        )
                    })
                    .is_some();
                if !turn_allocated {
                    observe_task_budget(state, task, &mut loop_state, None, loop_started_at, false);
                    loop_state.last_stop_signal =
                        Some("task_budget_allocation_exhausted".to_string());
                    break;
                }
                let outcome = run_agent_round(
                    state,
                    task,
                    goal,
                    &effective_user_text,
                    &mut policy,
                    &task_budget_policy,
                    &mut loop_state,
                    agent_run_context,
                    initial_plan_for_round(round, initial_plan_round, initial_plan),
                )
                .await;
                let cost_after = state.task_llm_cost_summary(&task.task_id);
                let elapsed_after = loop_started_at
                    .elapsed()
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64;
                if let Some(slice) = loop_state.task_budget_slice.as_mut() {
                    slice.settle_allocation(
                        &allocation_id,
                        crate::task_budget_contract::BudgetUnits {
                            model_turns: (state.task_llm_call_count(&task.task_id) as u64)
                                .saturating_sub(model_turns_before),
                            tool_calls: (loop_state.tool_calls_total as u64)
                                .saturating_sub(tool_calls_before),
                            tokens: cost_after
                                .input_tokens
                                .saturating_add(cost_after.output_tokens)
                                .saturating_sub(
                                    cost_before
                                        .input_tokens
                                        .saturating_add(cost_before.output_tokens),
                                ),
                            elapsed_ms: elapsed_after.saturating_sub(elapsed_before),
                        },
                    );
                }
                let outcome = match outcome {
                    Ok(outcome) => outcome,
                    Err(error)
                        if error == crate::llm_gateway::CONVERSATION_INPUT_INTERRUPTED_ERR =>
                    {
                        loop_state.last_stop_signal = Some(error);
                        round = round.saturating_add(1);
                        continue;
                    }
                    Err(error) => {
                        if super::model_blocker_checkpoint::checkpoint_blocked_model_error(
                            state,
                            task,
                            &mut loop_state,
                        )? {
                            break;
                        }
                        if !claim_conversation_terminal_boundary(state, task, &loop_state)? {
                            loop_state.last_stop_signal = Some(
                                crate::llm_gateway::CONVERSATION_INPUT_INTERRUPTED_ERR.to_string(),
                            );
                            round = round.saturating_add(1);
                            continue;
                        }
                        return Err(error);
                    }
                };
                loop_state.last_stop_signal = outcome.stop_signal.clone();
                if outcome.no_progress {
                    loop_state.consecutive_no_progress =
                        loop_state.consecutive_no_progress.saturating_add(1);
                } else {
                    loop_state.consecutive_no_progress = 0;
                }
                let soft_slice_exhausted =
                    task_budget_soft_slice_exhausted(loop_started_at, &loop_state);
                let decision = observe_task_budget(
                    state,
                    task,
                    &mut loop_state,
                    Some(&outcome),
                    loop_started_at,
                    soft_slice_exhausted,
                );
                if outcome.executed_actions > 0 {
                    super::support::persist_agent_loop_recovery_snapshot(state, task, &loop_state);
                }
                match decision {
                    crate::task_budget_contract::BudgetDecision::Continue => {
                        round = round.saturating_add(1);
                    }
                    crate::task_budget_contract::BudgetDecision::CheckpointRequeue => {
                        loop_state.last_stop_signal =
                            Some("task_budget_slice_exhausted".to_string());
                        publish_agent_loop_checkpoint_progress(
                            state,
                            task,
                            &mut loop_state,
                            "task_budget_slice_exhausted",
                        );
                        break;
                    }
                    crate::task_budget_contract::BudgetDecision::Waiting => {
                        if let Some(resume_reason) =
                            recoverable_machine_blocker_resume_reason(&loop_state)
                        {
                            publish_agent_loop_checkpoint_progress(
                                state,
                                task,
                                &mut loop_state,
                                resume_reason,
                            );
                        }
                        break;
                    }
                    crate::task_budget_contract::BudgetDecision::NeedsUser
                    | crate::task_budget_contract::BudgetDecision::Finish
                    | crate::task_budget_contract::BudgetDecision::Terminal => break,
                }
            }
        }
        if loop_state_has_checkpoint_handoff(&loop_state) {
            return Ok(checkpoint_handoff_reply(
                task,
                &effective_user_text,
                &loop_state,
                agent_run_context,
            ));
        }
        if super::task_plan_reconciliation::prepare_task_plan_reconciliation(
            state,
            task,
            &mut loop_state,
        )? {
            round = round.saturating_add(1);
            skip_planner_rounds = false;
            continue;
        }
        let pre_finalize_loop_state = loop_state.clone();
        let finalization = crate::finalize::finalize_loop_reply(
            state,
            task,
            &effective_user_text,
            loop_state,
            agent_run_context,
        )
        .await;
        let mut reply = match finalization {
            Ok(reply) => reply,
            Err(error) if error == crate::llm_gateway::CONVERSATION_INPUT_INTERRUPTED_ERR => {
                loop_state = pre_finalize_loop_state;
                loop_state.last_stop_signal = Some(error);
                round = round.saturating_add(1);
                skip_planner_rounds = false;
                continue;
            }
            Err(error) => {
                loop_state = pre_finalize_loop_state;
                if super::model_blocker_checkpoint::checkpoint_blocked_model_error(
                    state,
                    task,
                    &mut loop_state,
                )? {
                    return Ok(checkpoint_handoff_reply(
                        task,
                        &effective_user_text,
                        &loop_state,
                        agent_run_context,
                    ));
                }
                if !claim_conversation_terminal_boundary(state, task, &loop_state)? {
                    loop_state.last_stop_signal =
                        Some(crate::llm_gateway::CONVERSATION_INPUT_INTERRUPTED_ERR.to_string());
                    round = round.saturating_add(1);
                    skip_planner_rounds = false;
                    continue;
                }
                return Err(error);
            }
        };
        if active_task_boundary_control_pending(state, task)? {
            loop_state = pre_finalize_loop_state;
            loop_state.last_stop_signal =
                Some(crate::llm_gateway::CONVERSATION_INPUT_INTERRUPTED_ERR.to_string());
            round = round.saturating_add(1);
            skip_planner_rounds = false;
            continue;
        }
        if loop_state_has_checkpoint_handoff(&pre_finalize_loop_state) {
            return Ok(reply);
        }
        let mut blocked_loop_state = pre_finalize_loop_state.clone();
        if super::model_blocker_checkpoint::checkpoint_blocked_model_error(
            state,
            task,
            &mut blocked_loop_state,
        )? {
            return Ok(checkpoint_handoff_reply(
                task,
                &effective_user_text,
                &blocked_loop_state,
                agent_run_context,
            ));
        }
        let answer_contract = answer_contract_for_reply(&effective_user_text, &reply);
        prefer_terminal_model_answer_for_verifier_candidate(&mut reply, answer_contract.as_ref());
        enforce_post_write_content_evidence_guard(&mut reply);
        enforce_workspace_mutation_validation_success_guard(&mut reply);
        let mut pre_verifier_recovery_loop_state = pre_finalize_loop_state.clone();
        let reserve_recovery = try_run_post_write_validation_reserve_recovery(
            state,
            task,
            goal,
            &effective_user_text,
            &policy,
            &mut pre_verifier_recovery_loop_state,
            &reply,
            agent_run_context,
        )
        .await;
        if super::model_blocker_checkpoint::checkpoint_blocked_model_error(
            state,
            task,
            &mut pre_verifier_recovery_loop_state,
        )? {
            return Ok(checkpoint_handoff_reply(
                task,
                &effective_user_text,
                &pre_verifier_recovery_loop_state,
                agent_run_context,
            ));
        }
        if reserve_recovery? {
            loop_state = pre_verifier_recovery_loop_state;
            skip_planner_rounds = true;
            continue;
        }
        attach_answer_verifier_if_missing(
            state,
            task,
            &effective_user_text,
            answer_contract.as_ref(),
            &mut reply,
        )
        .await;
        let mut blocked_loop_state = pre_finalize_loop_state.clone();
        if super::model_blocker_checkpoint::checkpoint_blocked_model_error(
            state,
            task,
            &mut blocked_loop_state,
        )? {
            return Ok(checkpoint_handoff_reply(
                task,
                &effective_user_text,
                &blocked_loop_state,
                agent_run_context,
            ));
        }
        enforce_post_write_content_evidence_guard(&mut reply);
        enforce_workspace_mutation_validation_success_guard(&mut reply);
        let route_result = answer_contract.as_ref();
        if let Some(verifier) = answer_verifier_evidence_replan_summary(&reply).cloned() {
            let mut verifier_replan_loop_state = pre_finalize_loop_state.clone();
            if prepare_answer_verifier_evidence_replan(&mut verifier_replan_loop_state, &verifier) {
                info!(
                    task_id = %task.task_id,
                    missing_evidence_fields = ?verifier.missing_evidence_fields,
                    "answer_verifier_evidence_replan"
                );
                loop_state = verifier_replan_loop_state;
                round = round.saturating_add(1);
                skip_planner_rounds = false;
                continue;
            }
        }
        suppress_answer_verifier_retry_if_structurally_satisfied(&mut reply, route_result);
        if let Some(verifier) = answer_verifier_retry_summary(&reply, route_result).cloned() {
            if let Some(route) = route_result {
                if try_bounded_answer_verifier_synthesis_retry(
                    state,
                    task,
                    &effective_user_text,
                    route,
                    &verifier,
                    &mut reply,
                )
                .await
                {
                    info!("answer_verifier_bounded_synthesis_retry_succeeded");
                    if active_task_boundary_control_pending(state, task)? {
                        loop_state = pre_finalize_loop_state;
                        loop_state.last_stop_signal = Some(
                            crate::llm_gateway::CONVERSATION_INPUT_INTERRUPTED_ERR.to_string(),
                        );
                        round = round.saturating_add(1);
                        skip_planner_rounds = false;
                        continue;
                    }
                    if !claim_conversation_terminal_boundary(state, task, &pre_finalize_loop_state)?
                    {
                        loop_state = pre_finalize_loop_state;
                        loop_state.last_stop_signal = Some(
                            crate::llm_gateway::CONVERSATION_INPUT_INTERRUPTED_ERR.to_string(),
                        );
                        round = round.saturating_add(1);
                        skip_planner_rounds = false;
                        continue;
                    }
                    return Ok(reply);
                }
            }
            warn!(
                task_id = %task.task_id,
                missing_evidence_fields = ?verifier.missing_evidence_fields,
                "answer_verifier_bounded_synthesis_retry_exhausted"
            );
            let mut blocked_loop_state = pre_finalize_loop_state.clone();
            if super::model_blocker_checkpoint::checkpoint_blocked_model_error(
                state,
                task,
                &mut blocked_loop_state,
            )? {
                return Ok(checkpoint_handoff_reply(
                    task,
                    &effective_user_text,
                    &blocked_loop_state,
                    agent_run_context,
                ));
            }
            mark_reply_failed_after_answer_verifier_exhausted(
                &effective_user_text,
                &mut reply,
                &verifier,
            );
        }
        if active_task_boundary_control_pending(state, task)? {
            loop_state = pre_finalize_loop_state;
            loop_state.last_stop_signal =
                Some(crate::llm_gateway::CONVERSATION_INPUT_INTERRUPTED_ERR.to_string());
            round = round.saturating_add(1);
            skip_planner_rounds = false;
            continue;
        }
        if !claim_conversation_terminal_boundary(state, task, &pre_finalize_loop_state)? {
            loop_state = pre_finalize_loop_state;
            loop_state.last_stop_signal =
                Some(crate::llm_gateway::CONVERSATION_INPUT_INTERRUPTED_ERR.to_string());
            round = round.saturating_add(1);
            skip_planner_rounds = false;
            continue;
        }
        return Ok(reply);
    }
}

fn initial_plan_for_round<'a>(
    round: usize,
    initial_plan_round: usize,
    initial_plan: Option<&'a crate::PlanResult>,
) -> Option<&'a crate::PlanResult> {
    (round == initial_plan_round).then_some(initial_plan).flatten()
}

#[cfg(test)]
#[path = "loop_runtime_tests.rs"]
mod loop_runtime_tests;

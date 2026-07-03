use super::*;

/// Unified intent normalizer: one LLM call for boundary hints such as resume,
/// schedule, clarify state, and route-trace compatibility fields.
pub(crate) async fn run_intent_normalizer(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    resume_context: Option<&Value>,
    binding_context: Option<&Value>,
    now_iso: &str,
    timezone: &str,
    schedule_rules: &str,
) -> IntentNormalizerOutput {
    let req = user_request.trim();
    let surface_req = request_without_contract_test_hint(req);
    let req_surface = crate::intent::surface_signals::analyze_prompt_surface(&surface_req);
    if contract_test_hint_semantic_kind(req).is_some() {
        // This is a machine-readable contract-matrix test hook, not a
        // natural-language intent classifier. Parse it before the normalizer so
        // large NL suites do not spend a model call rediscovering the contract.
        if let Some(fallback) = contract_hint_fallback_decision(
            req,
            &req_surface,
            &state.skill_rt.workspace_root,
            "contract_hint_fast_path",
        ) {
            info!(
                "{} intent_normalizer task_id={} contract_hint_fast_path reason={} input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                fallback.reason,
                crate::truncate_for_log(req)
            );
            return normalizer_output_from_fallback(
                req,
                "structured_contract_hint_fast_path",
                fallback,
                None,
            );
        }
    }
    if let Some((fallback, turn_analysis)) = structural_alias_binding_fallback_decision(req) {
        info!(
            "{} intent_normalizer task_id={} structured_alias_binding_fast_path input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback_with_turn_analysis(
            req,
            "structured_alias_binding_fast_path",
            fallback,
            None,
            Some(turn_analysis),
        );
    }
    let context_bundle = crate::task_context_builder::build_route_task_context_bundle(
        state,
        task,
        user_request,
        session_snapshot,
        resume_context,
        binding_context,
        now_iso,
        timezone,
        schedule_rules,
    );
    let route_view = context_bundle
        .route_view
        .as_ref()
        .expect("route context bundle should include route_view");
    let model_outcome = run_intent_normalizer_model_step(
        state,
        task,
        req,
        &surface_req,
        &req_surface,
        route_view,
        &context_bundle,
        session_snapshot,
    )
    .await;
    let model_success = match model_outcome {
        NormalizerModelOutcome::Success(success) => success,
        NormalizerModelOutcome::Fallback(output) => return output,
    };
    let llm_out = model_success.llm_out;
    let llm_out_for_parse = model_success.llm_out_for_parse;
    let contract_repair_report = model_success.contract_repair_report;
    let parsed = model_success.parsed;
    if let Some(mut out) = parsed {
        let (repaired_out, contract_repair_report) = apply_boundary_contract_judge_repair(
            state,
            task,
            req,
            &req_surface,
            route_view,
            session_snapshot,
            &llm_out,
            &llm_out_for_parse,
            contract_repair_report,
            out,
        )
        .await;
        out = repaired_out;
        let resolved = out.resolved_user_intent.trim();
        let mut resume_behavior = parse_resume_behavior(&out.resume_behavior);
        if resume_context.is_none() && resume_behavior != ResumeBehavior::None {
            warn!(
                "intent_normalizer override resume_behavior to none: task_id={} raw_resume_behavior={}",
                task.task_id, out.resume_behavior
            );
            resume_behavior = ResumeBehavior::None;
        }
        let mut schedule_kind = parse_schedule_kind(&out.schedule_kind);
        let confidence = out.confidence.clamp(0.0, 1.0);
        let parsed_turn_type = parse_turn_type(&out.turn_type);
        let parsed_target_task_policy = parse_target_task_policy(&out.target_task_policy);
        let command_payload_declared =
            contract_repair_report.has_detail("execution_recipe_command_payload");
        let mut wants_file_delivery = out.wants_file_delivery;
        let mut output_contract =
            parse_output_contract(out.output_contract.clone(), wants_file_delivery);
        let mut clarify_question = out.clarify_question.trim().to_string();
        let mut execution_recipe_hint = parse_execution_recipe_hint(out.execution_recipe.clone());
        let mut execution_recipe_plan_hint =
            parse_execution_recipe_plan_hint(out.execution_recipe.as_ref());
        let mut needs_clarify = out.needs_clarify;
        let mut attachment_processing_required = out.attachment_processing_required;
        let mut execution_finalize_style = execution_finalize_style_for_contract(&output_contract);
        let schedule_route_contract_repair = apply_schedule_route_contract_repair(
            schedule_kind,
            &mut output_contract,
            &mut wants_file_delivery,
            &mut execution_finalize_style,
        );
        let structured_contract_hint_repair = apply_structured_contract_hint_repair(
            &mut output_contract,
            req,
            &req_surface,
            &state.skill_rt.workspace_root,
            &mut wants_file_delivery,
            &mut needs_clarify,
            &mut clarify_question,
            &mut execution_finalize_style,
        );
        if let Some(fallback) = parsed_inline_json_transform_repair_decision(
            req,
            needs_clarify,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        ) {
            info!(
                "{} intent_normalizer task_id={} parsed_inline_json_transform_repair reason={} input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                fallback.reason,
                crate::truncate_for_log(req)
            );
            return normalizer_output_from_fallback(
                req,
                "parsed_inline_json_transform_repair",
                fallback,
                None,
            );
        }
        if let Some(fallback) = explicit_surface_path_metadata_clarify_repair_decision(
            req,
            &req_surface,
            &state.skill_rt.workspace_root,
            needs_clarify,
            &output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        ) {
            info!(
                "{} intent_normalizer task_id={} clarify_explicit_surface_metadata_fallback reason={} input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                fallback.reason,
                crate::truncate_for_log(req)
            );
            return normalizer_output_from_fallback(
                req,
                "clarify_structured_surface_metadata_fallback",
                fallback,
                None,
            );
        }
        if let Some(fallback) = explicit_surface_path_facts_clarify_repair_decision(
            req,
            &req_surface,
            &state.skill_rt.workspace_root,
            needs_clarify,
            &output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        ) {
            info!(
                "{} intent_normalizer task_id={} clarify_explicit_surface_fallback reason={} input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                fallback.reason,
                crate::truncate_for_log(req)
            );
            return normalizer_output_from_fallback(
                req,
                "clarify_structured_surface_fallback",
                fallback,
                None,
            );
        }
        let mut synced_route_label = route_trace_label_from_state(
            needs_clarify,
            &output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
            execution_finalize_style,
        );
        let structural_contract_repair = apply_current_turn_structural_contract_repair(
            &out.reason,
            &mut output_contract,
            &surface_req,
            &req_surface,
            &state.skill_rt.workspace_root,
            parsed_turn_type,
            parsed_target_task_policy,
        );
        let fs_basic_lifecycle_contract_repair =
            apply_fs_basic_lifecycle_machine_contract_repair(&mut output_contract, &out.reason);
        if fs_basic_lifecycle_contract_repair.is_some() {
            execution_finalize_style = execution_finalize_style_for_contract(&output_contract);
            synced_route_label = route_trace_label_from_state(
                needs_clarify,
                &output_contract,
                wants_file_delivery,
                schedule_kind,
                execution_recipe_hint,
                execution_finalize_style,
            );
        }
        let command_payload_contract_repair = apply_command_payload_contract_repair(
            command_payload_declared,
            &mut output_contract,
            &mut needs_clarify,
            &mut clarify_question,
            &mut execution_finalize_style,
        );
        if command_payload_contract_repair.is_some() {
            synced_route_label = route_trace_label_from_state(
                needs_clarify,
                &output_contract,
                wants_file_delivery,
                schedule_kind,
                execution_recipe_hint,
                execution_finalize_style,
            );
        }
        let file_delivery_contract_repair = apply_file_delivery_contract_repair(
            wants_file_delivery,
            &mut output_contract,
            &mut needs_clarify,
            &mut clarify_question,
            &mut execution_finalize_style,
        );
        if file_delivery_contract_repair.is_some() {
            synced_route_label = route_trace_label_from_state(
                needs_clarify,
                &output_contract,
                wants_file_delivery,
                schedule_kind,
                execution_recipe_hint,
                execution_finalize_style,
            );
        }
        let generated_file_delivery_attachment_repair =
            clear_spurious_generated_file_delivery_attachment_processing(
                &mut attachment_processing_required,
                &output_contract,
                wants_file_delivery,
            );
        let raw_output_explicit_locator_repair = apply_raw_output_explicit_locator_repair(
            &mut output_contract,
            &out.reason,
            &surface_req,
            &state.policy.command_intent,
        );
        if raw_output_explicit_locator_repair.is_some() {
            synced_route_label = route_trace_label_from_state(
                needs_clarify,
                &output_contract,
                wants_file_delivery,
                schedule_kind,
                execution_recipe_hint,
                execution_finalize_style,
            );
        }
        let state_patch_for_decision = out
            .state_patch
            .as_ref()
            .filter(|value| is_meaningful_state_patch(value));
        let active_ordered_scalar_path_loop_context = active_ordered_scalar_path_loop_context_hint(
            session_snapshot,
            state_patch_for_decision,
            &out.reason,
            needs_clarify,
            &output_contract,
        );
        let active_observed_output_loop_context = active_observed_output_loop_context_hint(
            req,
            session_snapshot,
            parsed_turn_type,
            parsed_target_task_policy,
            attachment_processing_required,
            out.should_refresh_long_term_memory,
            schedule_kind,
            execution_recipe_hint,
            wants_file_delivery,
            needs_clarify,
            &out.reason,
            &output_contract,
        );
        let decision_contract_conflict_repair = structured_execution_signal_for_effective_route(
            &output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        )
        .then_some("executable_contract_preserved_for_agent_loop");
        let explicit_command_execution_repair = apply_explicit_command_execution_contract_repair(
            &state.policy.command_intent,
            req,
            &out.reason,
            &mut needs_clarify,
            &mut clarify_question,
            &mut output_contract,
            &mut execution_finalize_style,
        );
        if explicit_command_execution_repair.is_some() {
            synced_route_label = route_trace_label_from_state(
                needs_clarify,
                &output_contract,
                wants_file_delivery,
                schedule_kind,
                execution_recipe_hint,
                execution_finalize_style,
            );
        }
        let current_turn_anchor_repair_allowed =
            !current_request_mentions_session_alias(session_snapshot, req)
                && current_turn_anchor_drift_repair_allowed(
                    needs_clarify,
                    &out.reason,
                    &output_contract,
                    wants_file_delivery,
                    schedule_kind,
                    execution_recipe_hint,
                    &state.skill_rt.workspace_root,
                );
        let current_turn_anchor_path = current_turn_anchor_repair_allowed
            .then(|| resolve_current_turn_anchor_path(state, req))
            .flatten();
        let current_turn_anchor_drift_repair =
            current_turn_anchor_path.as_deref().and_then(|anchor_path| {
                apply_current_turn_anchor_drift_repair(
                    &mut output_contract,
                    &out.reason,
                    resolved,
                    anchor_path,
                    &state.skill_rt.workspace_root,
                )
            });
        if current_turn_anchor_drift_repair.is_some() {
            schedule_kind = ScheduleKind::None;
            wants_file_delivery = output_contract.delivery_required
                || matches!(
                    output_contract.response_shape,
                    OutputResponseShape::FileToken
                )
                || matches!(
                    output_contract.delivery_intent,
                    OutputDeliveryIntent::FileSingle
                );
            execution_recipe_hint = None;
            execution_recipe_plan_hint = None;
        }
        if let Some(finalize_style) =
            crate::post_route_policy::content_evidence_execution_finalize_style(
                &output_contract,
                needs_clarify,
            )
        {
            execution_finalize_style = finalize_style;
        }
        let mut state_patch = out.state_patch.clone().filter(is_meaningful_state_patch);
        if execution_recipe_plan_hint.is_none() {
            execution_recipe_plan_hint =
                parse_runtime_async_job_start_plan_hint(state_patch.as_ref());
        }
        let state_patch_replacement_literal_conflict_repair =
            repair_state_patch_replacement_literal_conflicts(&mut state_patch);
        let deictic_missing_locator_state_patch_repair =
            apply_deictic_missing_locator_state_patch_clarify_repair(
                &mut output_contract,
                state_patch.as_ref(),
                &mut needs_clarify,
                &mut clarify_question,
                &mut execution_finalize_style,
            );
        let archive_unpack_missing_archive_locator_clarify_repair =
            apply_archive_unpack_missing_archive_locator_clarify(
                &mut output_contract,
                &req_surface,
                session_snapshot,
                &mut needs_clarify,
                &mut clarify_question,
                &mut execution_finalize_style,
            );
        let structured_clarify_repair =
            if archive_unpack_missing_archive_locator_clarify_repair.is_none() {
                apply_spurious_structured_observation_clarify_repair(
                    &out.reason,
                    &mut output_contract,
                    req,
                    &req_surface,
                    &state.skill_rt.workspace_root,
                    state_patch.as_ref(),
                    &mut needs_clarify,
                    &mut clarify_question,
                    &mut execution_finalize_style,
                )
            } else {
                None
            };
        let workspace_default_clarify_repair =
            if archive_unpack_missing_archive_locator_clarify_repair.is_none()
                && structured_clarify_repair.is_none()
            {
                apply_locatorless_observation_clarify_repair(
                    &out.reason,
                    &mut output_contract,
                    resolved,
                    state_patch.as_ref(),
                    &mut needs_clarify,
                    &mut clarify_question,
                    &mut execution_finalize_style,
                )
                .or_else(|| {
                    apply_workspace_default_observation_clarify_repair(
                        &out.reason,
                        &mut output_contract,
                        &state.skill_rt.workspace_root,
                        state_patch.as_ref(),
                        &mut needs_clarify,
                        &mut clarify_question,
                        &mut execution_finalize_style,
                    )
                })
            } else {
                None
            };
        let resolved_directory_clarify_repair =
            if archive_unpack_missing_archive_locator_clarify_repair.is_none()
                && structured_clarify_repair.is_none()
                && workspace_default_clarify_repair.is_none()
            {
                apply_resolved_directory_observation_clarify_repair(
                    state,
                    &mut output_contract,
                    req,
                    &req_surface,
                    state_patch.as_ref(),
                    &mut needs_clarify,
                    &mut clarify_question,
                    &mut execution_finalize_style,
                )
            } else {
                None
            };
        let unbound_workspace_generic_content_clarify_repair = if structured_clarify_repair
            .is_none()
            && archive_unpack_missing_archive_locator_clarify_repair.is_none()
            && workspace_default_clarify_repair.is_none()
            && resolved_directory_clarify_repair.is_none()
            && deictic_missing_locator_state_patch_repair.is_none()
        {
            apply_unbound_workspace_generic_content_clarify_repair(
                &mut output_contract,
                req,
                &req_surface,
                &mut needs_clarify,
                &mut clarify_question,
                &mut execution_finalize_style,
            )
        } else {
            None
        };
        let executionless_finalize_trace_cleanup = cleanup_executionless_finalize_trace(
            &mut execution_finalize_style,
            needs_clarify,
            &output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        );
        let schedule_intent = normalize_schedule_intent_from_normalizer(
            schedule_kind,
            out.schedule_intent.clone(),
            if resolved.is_empty() { req } else { resolved },
            &out.reason,
            needs_clarify,
            &clarify_question,
            confidence,
        );
        let mut target_task_policy = infer_missing_target_policy_from_contract(
            parsed_target_task_policy,
            parsed_turn_type,
            needs_clarify,
            schedule_kind,
            out.should_refresh_long_term_memory,
            &output_contract,
        );
        let mut turn_type = infer_missing_turn_type_from_policy(
            parsed_turn_type,
            target_task_policy,
            needs_clarify,
            schedule_kind,
            out.should_refresh_long_term_memory,
        );
        let mut reason = out.reason;
        let mut force_current_request_resolved_intent = current_turn_anchor_drift_repair.is_some();
        if let Some(repair_reason) = generated_file_delivery_attachment_repair {
            append_route_reason(&mut reason, repair_reason);
        }
        if current_turn_anchor_drift_repair.is_some() {
            turn_type = Some(TurnType::TaskRequest);
            target_task_policy = Some(TargetTaskPolicy::Standalone);
        }
        if should_detach_bare_acknowledgement_from_active_task(
            turn_type,
            target_task_policy,
            &output_contract,
            state_patch.as_ref(),
            out.should_refresh_long_term_memory,
        ) {
            turn_type = None;
            target_task_policy = None;
            force_current_request_resolved_intent = true;
            append_route_reason(
                &mut reason,
                "bare_acknowledgement_detached_active_task_context",
            );
            info!(
                "{} intent_normalizer task_id={} bare_acknowledgement_detached_active_task_context input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        for repair_reason in [
            structural_contract_repair,
            state_patch_replacement_literal_conflict_repair,
            fs_basic_lifecycle_contract_repair,
            active_ordered_scalar_path_loop_context,
            active_observed_output_loop_context,
            structured_contract_hint_repair,
        ]
        .into_iter()
        .flatten()
        {
            append_route_reason(&mut reason, repair_reason);
        }
        if let Some(repair_reason) = current_turn_anchor_drift_repair {
            append_route_reason(&mut reason, repair_reason);
            if let Some(anchor_path) = current_turn_anchor_path.as_deref() {
                info!(
                    "{} intent_normalizer task_id={} current_turn_anchor_overrides_contextual_target anchor={} input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    crate::truncate_for_log(anchor_path),
                    crate::truncate_for_log(req)
                );
            }
        }
        for repair_reason in [
            archive_unpack_missing_archive_locator_clarify_repair,
            deictic_missing_locator_state_patch_repair,
            structured_clarify_repair,
            workspace_default_clarify_repair,
            resolved_directory_clarify_repair,
            unbound_workspace_generic_content_clarify_repair,
            executionless_finalize_trace_cleanup,
        ]
        .into_iter()
        .flatten()
        {
            append_route_reason(&mut reason, repair_reason);
        }
        if contract_repair_report.has_detail("execution_recipe_scalar_runtime_tool_observation") {
            append_route_reason(
                &mut reason,
                "execution_recipe_scalar_runtime_tool_observation",
            );
        }
        if contract_repair_report.has_detail("execution_recipe_service_status_observation") {
            append_route_reason(&mut reason, "execution_recipe_service_status_observation");
        }
        if contract_repair_report.has_detail("execution_recipe_health_check_observation") {
            append_route_reason(&mut reason, "execution_recipe_health_check_observation");
        }
        for repair_reason in [
            command_payload_contract_repair,
            file_delivery_contract_repair,
            raw_output_explicit_locator_repair,
            explicit_command_execution_repair,
            decision_contract_conflict_repair,
        ]
        .into_iter()
        .flatten()
        {
            append_route_reason(&mut reason, repair_reason);
        }
        if let Some(scope_hint) = apply_workspace_scope_patch_to_contract(
            &reason,
            &mut output_contract,
            turn_type,
            target_task_policy,
            state_patch.as_ref(),
        ) {
            append_route_reason(&mut reason, "workspace_scope_patch_locator_hint");
            info!(
                "{} intent_normalizer task_id={} workspace_scope_patch_locator_hint={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(&scope_hint)
            );
        }
        if let Some(loop_context_reason) = orphan_output_shape_loop_context_hint(
            session_snapshot,
            turn_type,
            target_task_policy,
            needs_clarify,
            &output_contract,
            state_patch.as_ref(),
            out.should_refresh_long_term_memory,
            attachment_processing_required,
        ) {
            append_route_reason(&mut reason, loop_context_reason);
            info!(
                "{} intent_normalizer task_id={} turn_analysis_hint=orphan_output_shape_loop_context input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(loop_context_reason) = standalone_freeform_clarify_loop_context_hint(
            session_snapshot,
            turn_type,
            target_task_policy,
            needs_clarify,
            &output_contract,
            state_patch.as_ref(),
            out.should_refresh_long_term_memory,
            attachment_processing_required,
            wants_file_delivery,
            schedule_kind,
        ) {
            append_route_reason(&mut reason, loop_context_reason);
            info!(
                "{} intent_normalizer task_id={} turn_analysis_hint=standalone_freeform_clarify_loop_context input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        let mut resolved_user_intent =
            if force_current_request_resolved_intent || resolved.is_empty() {
                req.to_string()
            } else {
                resolved.to_string()
            };
        if let Some(current_turn_intent) = sanitize_resolved_intent_for_current_turn_locator(
            &resolved_user_intent,
            req,
            &req_surface,
        ) {
            resolved_user_intent = current_turn_intent;
            append_route_reason(
                &mut reason,
                "current_turn_locator_overrides_contextual_path",
            );
            info!(
                "{} intent_normalizer task_id={} current_turn_locator_overrides_contextual_path input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(repair_reason) = apply_missing_active_task_reuse_clarify(
            req,
            &reason,
            session_snapshot,
            turn_type,
            target_task_policy,
            state_patch.as_ref(),
            &mut needs_clarify,
            &mut clarify_question,
            &mut execution_finalize_style,
            &mut output_contract,
        ) {
            append_route_reason(&mut reason, repair_reason);
            info!(
                "{} intent_normalizer task_id={} missing_active_task_reuse_requires_clarify input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(repair_reason) = apply_active_task_structured_patch_repair(
            req,
            &reason,
            session_snapshot,
            &mut turn_type,
            &mut target_task_policy,
            attachment_processing_required,
            &mut execution_finalize_style,
            &mut needs_clarify,
            schedule_kind,
            out.should_refresh_long_term_memory,
            &mut output_contract,
            state_patch.as_ref(),
        ) {
            clarify_question.clear();
            append_route_reason(&mut reason, repair_reason);
            info!(
                "{} intent_normalizer task_id={} active_task_structured_patch_repair input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(repair_reason) = apply_active_task_scope_refinement_repair(
            req,
            &reason,
            session_snapshot,
            &mut turn_type,
            &mut target_task_policy,
            attachment_processing_required,
            &mut execution_finalize_style,
            &mut needs_clarify,
            schedule_kind,
            out.should_refresh_long_term_memory,
            &mut output_contract,
            state_patch.as_ref(),
            crate::worker::try_resolve_workspace_child_locator_from_text(
                &state.skill_rt.workspace_root,
                &state.skill_rt.default_locator_search_dir,
                req,
            )
            .is_some(),
        ) {
            clarify_question.clear();
            append_route_reason(&mut reason, repair_reason);
            info!(
                "{} intent_normalizer task_id={} active_task_scope_refinement_repair input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(loop_context_reason) = active_task_scope_update_loop_context_hint(
            req,
            session_snapshot,
            turn_type,
            target_task_policy,
            attachment_processing_required,
            needs_clarify,
            &output_contract,
            state_patch.as_ref(),
        ) {
            append_route_reason(&mut reason, loop_context_reason);
            info!(
                "{} intent_normalizer task_id={} turn_analysis_hint=active_task_scope_update_loop_context input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(loop_context_reason) = active_task_mutation_loop_context_hint(
            req,
            &reason,
            session_snapshot,
            turn_type,
            target_task_policy,
            attachment_processing_required,
            &output_contract,
            state_patch.as_ref(),
        ) {
            append_route_reason(&mut reason, loop_context_reason);
            info!(
                "{} intent_normalizer task_id={} turn_analysis_hint=active_task_mutation_loop_context input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(loop_context_reason) = active_task_replace_loop_context_hint(
            req,
            session_snapshot,
            turn_type,
            target_task_policy,
            attachment_processing_required,
            needs_clarify,
            &output_contract,
            state_patch.as_ref(),
        ) {
            append_route_reason(&mut reason, loop_context_reason);
            info!(
                "{} intent_normalizer task_id={} turn_analysis_hint=active_task_replace_loop_context input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(loop_context_reason) = active_task_append_loop_context_hint(
            req,
            session_snapshot,
            turn_type,
            target_task_policy,
            attachment_processing_required,
            needs_clarify,
            &output_contract,
            state_patch.as_ref(),
        ) {
            append_route_reason(&mut reason, loop_context_reason);
            info!(
                "{} intent_normalizer task_id={} turn_analysis_hint=active_task_append_loop_context input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(repair_reason) = apply_archive_unpack_missing_archive_locator_clarify(
            &mut output_contract,
            &req_surface,
            session_snapshot,
            &mut needs_clarify,
            &mut clarify_question,
            &mut execution_finalize_style,
        ) {
            append_route_reason(&mut reason, repair_reason);
            info!(
                "{} intent_normalizer task_id={} archive_unpack_missing_archive_late_clarify input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(repair_reason) = apply_missing_read_target_mutation_clarify(
            &reason,
            &mut output_contract,
            &mut needs_clarify,
            &mut clarify_question,
            &mut execution_finalize_style,
        ) {
            append_route_reason(&mut reason, repair_reason);
            info!(
                "{} intent_normalizer task_id={} missing_read_target_mutation_clarify input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        let resolved_user_intent = append_state_patch_slice_tokens_to_resolved_intent(
            resolved_user_intent,
            state_patch.as_ref(),
        );
        let resolved_user_intent = append_state_patch_structured_field_selector_to_resolved_intent(
            resolved_user_intent,
            state_patch.as_ref(),
        );
        let turn_analysis = if turn_type.is_some()
            || target_task_policy.is_some()
            || out.should_interrupt_active_run
            || state_patch.is_some()
            || attachment_processing_required
        {
            Some(TurnAnalysis {
                turn_type,
                target_task_policy,
                should_interrupt_active_run: out.should_interrupt_active_run,
                state_patch,
                attachment_processing_required,
            })
        } else {
            None
        };
        let turn_analysis_log = turn_analysis
            .as_ref()
            .map(|analysis| {
                format!(
                    "type={:?},policy={:?},interrupt={},state_patch={},attachments={}",
                    analysis.turn_type,
                    analysis.target_task_policy,
                    analysis.should_interrupt_active_run,
                    analysis.state_patch.is_some(),
                    analysis.attachment_processing_required
                )
            })
            .unwrap_or_else(|| "none".to_string());
        let derived_route_trace_decision = route_trace_decision_from_state(
            needs_clarify,
            &output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        );
        let route_trace_label =
            route_trace_label_from_decision(derived_route_trace_decision, execution_finalize_style);
        if route_trace_label != synced_route_label {
            info!(
                "{} intent_normalizer task_id={} route_trace_label_override={} -> {} reason=content_evidence_requires_execution locator_kind={:?} shape={:?}",
                crate::highlight_tag("routing"),
                task.task_id,
                synced_route_label,
                route_trace_label,
                output_contract.locator_kind,
                output_contract.response_shape
            );
        }
        info!(
            "{} intent_normalizer task_id={} input={} resolved_user_intent={} resume_behavior={:?} schedule_kind={:?} route_trace_decision={:?} route_trace_label={} wants_file_delivery={} needs_clarify={} reason={} confidence={} output_contract.shape={:?} output_contract.delivery_required={} output_contract.requires_content_evidence={} output_contract.locator_kind={:?} execution_recipe_hint={} contract_repair_source={} contract_repair_detail={} contract_repair_class={} turn_analysis={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req),
            crate::truncate_for_log(&resolved_user_intent),
            resume_behavior,
            schedule_kind,
            derived_route_trace_decision,
            route_trace_label,
            wants_file_delivery,
            needs_clarify,
            crate::truncate_for_log(&reason),
            confidence,
            output_contract.response_shape,
            output_contract.delivery_required,
            output_contract.requires_content_evidence,
            output_contract.locator_kind,
            execution_recipe_hint
                .map(|spec| format!(
                    "{}:{}:{}",
                    spec.kind.as_str(),
                    spec.profile.as_str(),
                    spec.target_scope.as_str()
                ))
                .unwrap_or_else(|| "none".to_string()),
            contract_repair_report.source_csv(),
            contract_repair_report.detail_csv(),
            contract_repair_report.class_csv(),
            turn_analysis_log,
        );
        return build_normalizer_output_with_final_gate(
            task,
            req,
            &req_surface,
            session_snapshot,
            resolved_user_intent,
            resume_behavior,
            schedule_kind,
            schedule_intent,
            wants_file_delivery,
            out.should_refresh_long_term_memory,
            out.agent_display_name_hint.trim().to_string(),
            needs_clarify,
            clarify_question,
            reason,
            confidence,
            output_contract,
            execution_recipe_hint,
            execution_recipe_plan_hint,
            execution_finalize_style,
            turn_analysis,
            turn_type,
            target_task_policy,
            &contract_repair_report,
            &[
                structural_contract_repair,
                active_ordered_scalar_path_loop_context,
                active_observed_output_loop_context,
                structured_contract_hint_repair,
                current_turn_anchor_drift_repair,
                archive_unpack_missing_archive_locator_clarify_repair,
                structured_clarify_repair,
                workspace_default_clarify_repair,
                resolved_directory_clarify_repair,
                unbound_workspace_generic_content_clarify_repair,
                executionless_finalize_trace_cleanup,
                command_payload_contract_repair,
                file_delivery_contract_repair,
                schedule_route_contract_repair,
                raw_output_explicit_locator_repair,
                explicit_command_execution_repair,
                decision_contract_conflict_repair,
                generated_file_delivery_attachment_repair,
            ],
        );
    }
    let _ = (resume_context, binding_context);
    normalizer_parse_failed_fallback_output(state, task, req, &surface_req, &req_surface, &llm_out)
}

/// Derives a legacy-compatible route trace token from machine boundary fields.
/// This is journal/log compatibility only; ordinary respond/clarify/act
/// decisions are owned by the agent loop.
fn route_trace_decision_from_state(
    needs_clarify: bool,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
) -> RouteTraceDecision {
    if needs_clarify {
        RouteTraceDecision::Clarify
    } else if structured_execution_signal_for_effective_route(
        output_contract,
        wants_file_delivery,
        schedule_kind,
        execution_recipe_hint,
    ) {
        RouteTraceDecision::Act
    } else {
        RouteTraceDecision::Respond
    }
}

fn route_trace_label_from_decision(
    decision: RouteTraceDecision,
    finalize_style: ActFinalizeStyle,
) -> &'static str {
    match decision {
        RouteTraceDecision::Clarify => "clarify",
        RouteTraceDecision::Respond => "respond",
        RouteTraceDecision::Act => match finalize_style {
            ActFinalizeStyle::ChatWrapped => "act_chat_finalizer",
            ActFinalizeStyle::Plain | ActFinalizeStyle::ResumeContinue => "act_plain_finalizer",
        },
    }
}

fn route_trace_label_from_state(
    needs_clarify: bool,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    execution_finalize_style: ActFinalizeStyle,
) -> &'static str {
    route_trace_label_from_decision(
        route_trace_decision_from_state(
            needs_clarify,
            output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        ),
        execution_finalize_style,
    )
}

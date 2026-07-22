use super::*;

pub(super) fn extract_answer_from_observed_output_impl(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract());
    let response_shape = route.map(|route| route.response_shape);
    let has_route_contract = route.is_some();
    let locator_hint = route
        .map(|route| route.locator_hint.as_str())
        .filter(|hint| !hint.trim().is_empty());
    let auto_locator_path = agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .filter(|path| !path.trim().is_empty());
    if let Some(answer) =
        selected_capability_result_exact_candidate(route, &loop_state.capability_results)
    {
        return evidence_policy_checked_direct_candidate(
            route,
            loop_state,
            auto_locator_path,
            answer,
        );
    }
    if route.is_some_and(crate::IntentOutputContract::requests_path_inspection) {
        return None;
    }
    let request_language_hint = current_turn_request_text(route, agent_run_context)
        .map(observed_request_language_hint)
        .unwrap_or("config_default");
    let allow_localized_direct_template =
        observed_language_supports_bilingual_template(request_language_hint);
    let prefers_english_free_text =
        observed_request_prefers_english_template(state, request_language_hint);
    let allow_raw_listing_direct_answer = route_allows_raw_listing_direct_answer(route);
    let health_check_prefers_raw_payload = has_route_contract
        && route.is_some_and(crate::IntentOutputContract::requests_exact_command_output)
        && !matches!(
            response_shape,
            Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar)
        );
    if has_successful_step_for_skill(loop_state, "health_check")
        && !health_check_prefers_raw_payload
        && matches!(
            response_shape,
            Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar)
        )
    {
        return None;
    }
    let prefer_full_path = route.is_some_and(route_prefers_plain_fs_search_paths);

    if let Some(route) = route {
        if let Some(answer) =
            multi_status_json_summary_candidate(route, loop_state).and_then(|answer| {
                evidence_policy_checked_direct_candidate(
                    Some(route),
                    loop_state,
                    auto_locator_path,
                    answer,
                )
            })
        {
            return Some(answer);
        }
    }

    let answer = allow_raw_listing_direct_answer
        .then(|| {
            latest_successful_list_dir_answer_candidate(
                loop_state,
                response_shape,
                auto_locator_path,
                prefer_full_path,
            )
        })
        .flatten()
        .or_else(|| {
            let observed_output =
                extract_latest_generic_successful_output_with_state(state, loop_state)?;
            if observed_output.skill == "run_cmd" {
                {
                    allow_raw_listing_direct_answer
                        .then(|| {
                            route
                                .filter(|route| !route.requests_exact_path_list())
                                .and_then(|_| {
                                    run_cmd_listing_text_candidate(
                                        &observed_output.body,
                                        auto_locator_path,
                                    )
                                })
                        })
                        .flatten()
                }
            } else {
                None
            }
            .or_else(|| match observed_output.skill.as_str() {
                "health_check" => {
                    health_check_prefers_raw_payload.then_some(observed_output.body.clone())
                }
                "http_basic" => None,
                "process_basic" | "service_control" => None,
                "fs_search" => serde_json::from_str::<serde_json::Value>(&observed_output.body)
                    .ok()
                    .and_then(|value| {
                        fs_search_output_direct_answer_candidate(
                            state,
                            route,
                            &value,
                            locator_hint,
                            prefers_english_free_text,
                            allow_raw_listing_direct_answer,
                            prefer_full_path,
                        )
                    }),
                "doc_parse" => None,
                "transform" => transform_skill_formatted_output_candidate(&observed_output.body),
                "log_analyze" => None,
                "system_basic" | "config_basic" | "fs_basic" => {
                    let system_basic_info = (observed_output.skill == "system_basic")
                        .then(|| system_basic_info_value("system_basic", &observed_output.body))
                        .flatten();
                    if let Some(info) = system_basic_info.as_ref() {
                        if route.is_some_and(route_requests_exact_scalar_path) {
                            return system_basic_info_scalar_path_candidate(info);
                        }
                    }
                    let value = serde_json::from_str::<serde_json::Value>(&observed_output.body)
                        .ok()
                        .or_else(|| {
                            system_basic_info_value("system_basic", &observed_output.body)
                        })?;
                    let action = value.get("action").and_then(|v| v.as_str());
                    if observed_output.skill == "fs_basic" {
                        if let Some(answer) = fs_search_output_direct_answer_candidate(
                            state,
                            route,
                            &value,
                            locator_hint,
                            prefers_english_free_text,
                            allow_raw_listing_direct_answer,
                            prefer_full_path,
                        ) {
                            return Some(answer);
                        }
                    }
                    let raw_read_range_direct =
                        route_allows_raw_read_range_direct_passthrough(route, response_shape);
                    if action == Some("read_range")
                        && (route_allows_tail_read_range_direct_passthrough(
                            route,
                            response_shape,
                            &value,
                        ) || route_allows_read_range_direct_passthrough(route, response_shape)
                            || raw_read_range_direct)
                    {
                        value
                            .get("excerpt")
                            .and_then(|v| v.as_str())
                            .and_then(|excerpt| {
                                normalize_read_range_excerpt_for_direct_answer(
                                    state,
                                    excerpt,
                                    prefers_english_free_text,
                                    read_range_preserve_blank_lines(&value),
                                )
                            })
                            .filter(|candidate| {
                                raw_read_range_direct
                                    || !read_range_direct_candidate_conflicts_with_request_language(
                                        candidate,
                                        request_language_hint,
                                    )
                            })
                    } else if action == Some("inventory_dir")
                        && inventory_dir_can_use_direct_names(
                            route,
                            &value,
                            loop_state,
                            has_route_contract,
                            allow_raw_listing_direct_answer,
                        )
                    {
                        inventory_dir_direct_answer_candidate(
                            state,
                            route,
                            &value,
                            prefers_english_free_text,
                        )
                    } else if action == Some("tree_summary") {
                        tree_summary_direct_answer_candidate(
                            state,
                            &value,
                            prefers_english_free_text,
                        )
                    } else if action == Some("dir_compare") {
                        dir_compare_direct_answer_candidate(
                            state,
                            &value,
                            prefers_english_free_text,
                        )
                    } else if action == Some("count_inventory") {
                        count_inventory_direct_answer_candidate(&value, route)
                    } else if matches!(action, Some("extract_field" | "read_field")) {
                        extract_field_direct_answer_candidate(
                            state,
                            &value,
                            response_shape,
                            prefers_english_free_text,
                            allow_localized_direct_template,
                        )
                    } else if matches!(action, Some("extract_fields" | "read_fields")) {
                        extract_fields_direct_answer_candidate(
                            state,
                            &value,
                            response_shape,
                            prefers_english_free_text,
                            allow_localized_direct_template,
                        )
                    } else if action == Some("info")
                        || (action.is_none() && system_basic_value_looks_like_info(&value))
                    {
                        if route.is_some_and(route_requests_exact_scalar_path) {
                            system_basic_info_scalar_path_candidate(&value)
                        } else {
                            None
                        }
                    } else if action == Some("path_batch_facts")
                        && route.is_some_and(route_requires_single_file_delivery)
                    {
                        path_batch_file_delivery_token_candidate(route, &value)
                    } else if action == Some("path_batch_facts")
                        && route.is_some_and(route_allows_path_batch_scalar_path_observed_answer)
                    {
                        route
                            .and_then(exact_scalar_path_selector)
                            .and_then(|field| {
                                system_basic_path_batch_scalar_path_candidate(&value, &field)
                            })
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .or_else(|| {
                structured_scalar_candidate(
                    state,
                    route,
                    &observed_output.skill,
                    &observed_output.body,
                    locator_hint,
                    auto_locator_path,
                    prefer_full_path,
                    prefers_english_free_text,
                )
            })
            .or_else(|| {
                allows_normalized_scalar_direct_fallback(
                    &observed_output.skill,
                    route,
                    response_shape,
                )
                .then(|| normalized_scalar_candidate(&observed_output.body))
                .flatten()
            })
        })?;
    if crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    evidence_policy_checked_direct_candidate(route, loop_state, auto_locator_path, answer)
}

pub(super) fn fs_search_output_direct_answer_candidate(
    state: Option<&AppState>,
    route: Option<&crate::IntentOutputContract>,
    value: &serde_json::Value,
    locator_hint: Option<&str>,
    prefer_english: bool,
    allow_multi_result_list: bool,
    prefer_full_path: bool,
) -> Option<String> {
    if route.is_some_and(crate::IntentOutputContract::requests_exact_command_output) {
        return fs_search_direct_answer_candidate(
            state,
            value,
            locator_hint,
            prefer_english,
            true,
            false,
        )
        .map(|answer| {
            absolutize_fs_search_answer_paths(state, route, value, answer, prefer_full_path)
        });
    }
    route
        .and_then(|route| {
            fs_search_route_filtered_listing_candidate(route, value, allow_multi_result_list)
        })
        .or_else(|| {
            fs_search_direct_answer_candidate(
                state,
                value,
                locator_hint,
                prefer_english,
                allow_multi_result_list,
                prefer_full_path,
            )
        })
        .map(|answer| {
            absolutize_fs_search_answer_paths(state, route, value, answer, prefer_full_path)
        })
}

pub(super) fn route_allows_tail_read_range_direct_passthrough(
    route: Option<&crate::IntentOutputContract>,
    response_shape: Option<crate::OutputResponseShape>,
    value: &serde_json::Value,
) -> bool {
    if matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar)
    ) {
        return false;
    }
    let Some(route) = route else {
        return false;
    };
    if route.delivery_required {
        return false;
    }
    if value.get("mode").and_then(|v| v.as_str()) != Some("tail") {
        return false;
    }
    let Some(requested_n) = value.get("requested_n").and_then(|v| v.as_u64()) else {
        return false;
    };
    if requested_n == 0 || requested_n > 50 {
        return false;
    }
    route.requires_content_evidence
}

pub(super) fn route_allows_read_range_direct_passthrough(
    route: Option<&crate::IntentOutputContract>,
    response_shape: Option<crate::OutputResponseShape>,
) -> bool {
    if matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar)
    ) {
        return false;
    }
    let Some(route) = route else {
        return false;
    };
    if !super::output_route_policy::route_is_unclassified_contract(route)
        || route.requires_content_evidence
    {
        return false;
    }
    true
}

pub(super) fn route_allows_raw_read_range_direct_passthrough(
    route: Option<&crate::IntentOutputContract>,
    response_shape: Option<crate::OutputResponseShape>,
) -> bool {
    if matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar)
    ) {
        return false;
    }
    let Some(route) = route else {
        return false;
    };
    route.requests_exact_command_output()
        && route.requires_content_evidence
        && !route.delivery_required
}

pub(super) fn allows_normalized_scalar_direct_fallback(
    skill: &str,
    _route: Option<&crate::IntentOutputContract>,
    response_shape: Option<crate::OutputResponseShape>,
) -> bool {
    match skill {
        "package_manager" => false,
        "http_basic" => !matches!(
            response_shape,
            Some(crate::OutputResponseShape::OneSentence)
        ),
        _ => true,
    }
}

fn inventory_dir_can_use_direct_names(
    route: Option<&crate::IntentOutputContract>,
    value: &serde_json::Value,
    loop_state: &LoopState,
    has_route_contract: bool,
    allow_raw_listing_direct_answer: bool,
) -> bool {
    let has_machine_names =
        value_requests_terminal_inventory_names(value) && inventory_dir_names(value).is_some();
    if has_machine_names && route.is_some_and(crate::IntentOutputContract::requests_exact_name_list)
    {
        return true;
    }
    if has_machine_names
        && route_allows_latest_plan_names_direct(route)
        && latest_plan_requests_names_only_listing(loop_state)
    {
        return true;
    }
    if has_machine_names && latest_plan_requests_observation_only_names_listing(loop_state) {
        return true;
    }
    has_route_contract && allow_raw_listing_direct_answer
}

fn value_requests_terminal_inventory_names(value: &serde_json::Value) -> bool {
    ["names_only", "dirs_only", "files_only"]
        .into_iter()
        .any(|key| value.get(key).and_then(|v| v.as_bool()).unwrap_or(false))
}

fn route_allows_latest_plan_names_direct(route: Option<&crate::IntentOutputContract>) -> bool {
    let Some(route) = route else {
        return false;
    };
    matches!(
        route.response_shape,
        crate::OutputResponseShape::Strict | crate::OutputResponseShape::Scalar
    )
}

fn latest_plan_requests_names_only_listing(loop_state: &LoopState) -> bool {
    let Some(plan) = loop_state
        .round_traces
        .iter()
        .rev()
        .find_map(|round| round.plan_result.as_ref())
    else {
        return false;
    };
    let executable_steps = plan
        .steps
        .iter()
        .filter(|step| {
            matches!(
                step.action_type.as_str(),
                "call_capability" | "call_tool" | "call_skill"
            )
        })
        .collect::<Vec<_>>();
    if executable_steps.len() != 1 {
        return false;
    }
    let step = executable_steps[0];
    match step.action_type.as_str() {
        "call_capability" => {
            matches!(
                step.skill.as_str(),
                "filesystem.list_names"
                    | "filesystem.list_dir"
                    | "filesystem.list_entries"
                    | "fs.list_names"
                    | "fs.list_dir"
                    | "fs.list_entries"
            ) && step
                .args
                .get("names_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(true)
        }
        "call_tool" | "call_skill" => {
            matches!(
                step.skill.as_str(),
                "fs_basic" | "system_basic" | "list_dir"
            ) && matches!(
                step.args.get("action").and_then(|v| v.as_str()),
                Some("inventory_dir" | "list_dir") | None
            ) && step
                .args
                .get("names_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        }
        _ => false,
    }
}

fn latest_plan_requests_observation_only_names_listing(loop_state: &LoopState) -> bool {
    let Some(plan) = loop_state
        .round_traces
        .iter()
        .rev()
        .find_map(|round| round.plan_result.as_ref())
    else {
        return false;
    };
    let executable_steps = plan
        .steps
        .iter()
        .filter(|step| {
            matches!(
                step.action_type.as_str(),
                "call_capability" | "call_tool" | "call_skill"
            )
        })
        .collect::<Vec<_>>();
    if executable_steps.len() != 1 {
        return false;
    }
    if plan
        .steps
        .iter()
        .any(|step| !plan_step_is_observation_only_listing(step))
    {
        return false;
    }
    plan_step_requests_terminal_names_listing(executable_steps[0])
}

fn plan_step_is_observation_only_listing(step: &crate::PlanStep) -> bool {
    matches!(
        step.action_type.as_str(),
        "call_capability" | "call_tool" | "call_skill"
    )
}

fn plan_step_requests_terminal_names_listing(step: &crate::PlanStep) -> bool {
    match step.action_type.as_str() {
        "call_capability" => {
            matches!(
                step.skill.as_str(),
                "filesystem.list_names"
                    | "filesystem.list_dir"
                    | "filesystem.list_entries"
                    | "fs.list_names"
                    | "fs.list_dir"
                    | "fs.list_entries"
            ) && step_args_request_terminal_inventory_names(&step.args)
        }
        "call_tool" | "call_skill" => {
            matches!(
                step.skill.as_str(),
                "fs_basic" | "system_basic" | "list_dir"
            ) && matches!(
                step.args.get("action").and_then(|v| v.as_str()),
                Some("inventory_dir" | "list_dir") | None
            ) && step_args_request_terminal_inventory_names(&step.args)
        }
        _ => false,
    }
}

fn step_args_request_terminal_inventory_names(args: &serde_json::Value) -> bool {
    ["names_only", "dirs_only", "files_only"]
        .into_iter()
        .any(|key| args.get(key).and_then(|v| v.as_bool()).unwrap_or(false))
}

pub(crate) fn extract_answer_from_observed_output(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    extract_answer_from_observed_output_impl(None, loop_state, agent_run_context)
}

pub(crate) fn extract_answer_from_observed_output_i18n(
    loop_state: &LoopState,
    state: &AppState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    extract_answer_from_observed_output_impl(Some(state), loop_state, agent_run_context)
}

pub(crate) fn answer_matches_observed_output_passthrough(
    answer: &str,
    loop_state: &LoopState,
) -> bool {
    let answer = answer.trim();
    if answer.is_empty() {
        return false;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
        })
        .filter_map(|step| step.output.as_deref().map(str::trim))
        .filter(|body| !body.is_empty())
        .any(|body| {
            answer == body
                || super::strict_exact_tail_read_answer_from_output(body)
                    .is_some_and(|observed| answer == observed.trim())
                || normalized_observed_listing(body).is_some_and(|listing| {
                    let listing = listing.trim();
                    answer == listing
                        || listing
                            .lines()
                            .map(str::trim)
                            .any(|line| !line.is_empty() && line == answer)
                })
        })
}

#[cfg(test)]
pub(crate) fn extract_direct_answer_from_generic_output(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    extract_answer_from_observed_output(loop_state, agent_run_context)
}

#[cfg(test)]
pub(crate) fn extract_direct_answer_from_generic_output_i18n(
    loop_state: &LoopState,
    state: &AppState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    extract_answer_from_observed_output_i18n(loop_state, state, agent_run_context)
}

#[cfg(test)]
pub(crate) fn answer_is_direct_observation_passthrough(
    answer: &str,
    loop_state: &LoopState,
) -> bool {
    answer_matches_observed_output_passthrough(answer, loop_state)
}

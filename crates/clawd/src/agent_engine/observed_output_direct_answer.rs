use super::*;

pub(super) fn extract_answer_from_observed_output_impl(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let response_shape = route.map(|route| route.output_contract.response_shape);
    let is_plain_act = route.is_some_and(|route| route.ask_mode.is_plain_act());
    let locator_hint = route
        .map(|route| route.output_contract.locator_hint.as_str())
        .filter(|hint| !hint.trim().is_empty());
    let auto_locator_path = agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .filter(|path| !path.trim().is_empty());
    let request_language_hint = current_turn_request_text(route, agent_run_context)
        .map(observed_request_language_hint)
        .unwrap_or("config_default");
    let allow_localized_direct_template =
        observed_language_supports_bilingual_template(request_language_hint);
    let prefers_english_free_text =
        observed_request_prefers_english_template(state, request_language_hint);
    let prefers_english_presence_answer = route.is_some_and(|route| {
        super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::ExistenceWithPath,
        ) && prefers_english_free_text
    });
    let existence_with_path_should_use_llm_synthesis =
        route_should_synthesize_non_bilingual_existence_with_path(
            route,
            allow_localized_direct_template,
        );
    let hidden_entries_should_use_llm_synthesis = route.is_some_and(|route| {
        route_requests_hidden_entries_check(route)
            && route.output_contract.response_shape != crate::OutputResponseShape::Scalar
    });
    let allow_raw_listing_direct_answer = route_allows_raw_listing_direct_answer(route)
        && !existence_with_path_should_use_llm_synthesis
        && !hidden_entries_should_use_llm_synthesis;
    let health_check_prefers_raw_payload = is_plain_act
        && route.is_some_and(|route| {
            super::output_route_policy::route_contract_marker_is(
                route,
                crate::OutputSemanticKind::RawCommandOutput,
            )
        })
        && !matches!(
            response_shape,
            Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar)
        );
    let health_check_service_status_direct_allowed = route.is_some_and(|route| {
        super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::ServiceStatus,
        )
    });
    if route.is_some_and(|route| {
        super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::ServiceStatus,
        )
    }) {
        if let Some(answer) = latest_process_basic_service_status_direct_answer_candidate(
            state,
            loop_state,
            response_shape,
            prefers_english_free_text,
        )
        .and_then(|answer| {
            evidence_policy_checked_direct_candidate(route, loop_state, auto_locator_path, answer)
        }) {
            return Some(answer);
        }
    }
    if has_successful_step_for_skill(loop_state, "health_check")
        && !health_check_prefers_raw_payload
        && !health_check_service_status_direct_allowed
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
            structured_scalar_equality_direct_answer(state, route, loop_state, agent_run_context)
        {
            return evidence_policy_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = latest_git_repository_state_direct_answer(
            state,
            route,
            loop_state,
            response_shape,
            allow_localized_direct_template,
            prefers_english_free_text,
        ) {
            return evidence_policy_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) =
            hidden_entries_direct_answer(state, route, loop_state, prefers_english_free_text)
        {
            if latest_observation_lacks_required_content_evidence(route, loop_state) {
                return None;
            }
            return evidence_policy_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = db_basic_database_kind_judgment_from_loop_state_candidate(
            route,
            loop_state,
            current_turn_request_text(Some(route), agent_run_context),
            prefers_english_free_text,
        ) {
            return evidence_policy_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = directory_purpose_summary_find_ext_answer_candidate(route, loop_state)
            .and_then(|answer| {
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
                route
                    .and_then(|route| {
                        run_cmd_sqlite_direct_answer_candidate(
                            route,
                            &observed_output.body,
                            current_turn_request_text(Some(route), agent_run_context),
                            prefers_english_free_text,
                        )
                    })
                    .or_else(|| {
                        (!existence_with_path_should_use_llm_synthesis)
                            .then(|| {
                                run_cmd_presence_with_path_candidate(
                                    state,
                                    &observed_output.body,
                                    locator_hint,
                                    auto_locator_path,
                                    prefers_english_presence_answer,
                                )
                            })
                            .flatten()
                    })
                    .or_else(|| {
                        (allow_raw_listing_direct_answer
                            && !existence_with_path_should_use_llm_synthesis)
                            .then(|| {
                                route
                                    .and_then(|route| {
                                        run_cmd_contract_listing_text_candidate(
                                            route,
                                            &observed_output.body,
                                        )
                                    })
                                    .or_else(|| {
                                        run_cmd_listing_text_candidate(
                                            &observed_output.body,
                                            auto_locator_path,
                                        )
                                    })
                            })
                            .flatten()
                    })
                    .or_else(|| {
                        route
                            .filter(|route| {
                                route_allows_strict_plain_observation_passthrough(route)
                            })
                            .and_then(|_| {
                                strict_plain_observation_passthrough_candidate(
                                    &observed_output.body,
                                )
                            })
                    })
            } else {
                None
            }
            .or_else(|| match observed_output.skill.as_str() {
                "health_check" => {
                    health_check_prefers_raw_payload.then_some(observed_output.body.clone())
                }
                "http_basic" => None,
                "process_basic" => route.and_then(|route| {
                    if process_basic_port_list_should_use_llm_synthesis(
                        route,
                        &observed_output.body,
                    ) {
                        return None;
                    }
                    super::output_route_policy::route_contract_marker_is(
                        route,
                        crate::OutputSemanticKind::ServiceStatus,
                    )
                    .then(|| {
                        process_basic_service_status_direct_answer_candidate(
                            state,
                            &observed_output.body,
                            response_shape,
                            prefers_english_free_text,
                        )
                    })
                    .flatten()
                }),
                "service_control" => {
                    serde_json::from_str::<serde_json::Value>(&observed_output.body)
                        .ok()
                        .and_then(|value| {
                            route
                                .filter(|route| {
                                    super::output_route_policy::route_contract_marker_is(
                                        route,
                                        crate::OutputSemanticKind::ServiceStatus,
                                    )
                                })
                                .and_then(|_| {
                                    service_control_status_direct_answer_candidate(
                                        &value,
                                        response_shape,
                                    )
                                })
                        })
                }
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
                "git_basic" => {
                    let branch = latest_git_current_branch(loop_state);
                    git_basic_direct_answer_candidate(
                        state,
                        route,
                        &observed_output.body,
                        branch.as_deref(),
                        response_shape,
                        allow_localized_direct_template,
                        prefers_english_free_text,
                    )
                }
                "doc_parse" => route
                    .and_then(|route| {
                        doc_parse_content_presence_direct_answer_candidate(
                            state,
                            route,
                            &observed_output.body,
                            current_turn_request_text(Some(route), agent_run_context),
                            auto_locator_path.or(locator_hint),
                            prefers_english_free_text,
                        )
                    })
                    .or_else(|| {
                        content_excerpt_summary_direct_answer_candidate(
                            route,
                            &observed_output.body,
                        )
                        .filter(|candidate| {
                            !direct_free_text_conflicts_with_request_language(
                                candidate,
                                request_language_hint,
                            )
                        })
                    }),
                "db_basic" => route.and_then(|route| {
                    db_basic_database_kind_judgment_candidate(
                        route,
                        &observed_output.body,
                        current_turn_request_text(Some(route), agent_run_context),
                        prefers_english_free_text,
                    )
                    .or_else(|| {
                        db_basic_tables_summary_candidate(
                            state,
                            route,
                            &observed_output.body,
                            prefers_english_free_text,
                        )
                    })
                }),
                "transform" => transform_skill_formatted_output_candidate(&observed_output.body),
                "package_manager" => package_manager_summary_candidate(
                    state,
                    &observed_output.body,
                    response_shape,
                    allow_localized_direct_template,
                    prefers_english_free_text,
                ),
                "archive_basic" => {
                    if let Some(answer) = archive_unpack_direct_answer_candidate(
                        route,
                        &observed_output.body,
                        prefers_english_free_text,
                    ) {
                        Some(answer)
                    } else if let Some(answer) =
                        archive_read_direct_answer_candidate(route, &observed_output.body)
                    {
                        Some(answer)
                    } else {
                        archive_list_summary_from_body(&observed_output.body).and_then(|summary| {
                            route.and_then(|route| {
                                archive_entry_existence_direct_answer(
                                    state,
                                    route,
                                    current_turn_request_text(Some(route), agent_run_context),
                                    &summary,
                                    auto_locator_path.or(locator_hint),
                                    prefers_english_presence_answer,
                                )
                            })
                        })
                    }
                }
                "log_analyze" => None,
                "system_basic" | "config_basic" | "fs_basic" => {
                    let system_basic_info = (observed_output.skill == "system_basic")
                        .then(|| system_basic_info_value("system_basic", &observed_output.body))
                        .flatten();
                    if let Some(info) = system_basic_info.as_ref() {
                        if route.is_some_and(route_requests_scalar_path_only) {
                            return system_basic_info_scalar_path_candidate(info);
                        }
                        if route.is_some_and(|route| {
                            super::output_route_policy::route_contract_marker_is(
                                route,
                                crate::OutputSemanticKind::ServiceStatus,
                            )
                        }) {
                            return None;
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
                            is_plain_act,
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
                        if route.is_some_and(|route| {
                            super::output_route_policy::route_contract_marker_is(
                                route,
                                crate::OutputSemanticKind::DirectoryPurposeSummary,
                            )
                        }) {
                            None
                        } else {
                            tree_summary_direct_answer_candidate(
                                state,
                                &value,
                                prefers_english_free_text,
                            )
                        }
                    } else if action == Some("dir_compare") {
                        dir_compare_direct_answer_candidate(
                            state,
                            &value,
                            prefers_english_free_text,
                        )
                    } else if action == Some("count_inventory") {
                        count_inventory_direct_answer_candidate(
                            state,
                            &value,
                            response_shape,
                            prefers_english_free_text,
                        )
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
                    } else if action == Some("structured_keys") {
                        structured_keys_direct_answer_candidate(
                            state,
                            &value,
                            current_turn_request_text(route, agent_run_context),
                            response_shape,
                            prefers_english_free_text,
                        )
                    } else if action == Some("validate_structured") {
                        validate_structured_direct_answer_candidate(
                            state,
                            &value,
                            prefers_english_free_text,
                        )
                    } else if action == Some("info")
                        || (action.is_none() && system_basic_value_looks_like_info(&value))
                    {
                        if route.is_some_and(route_requests_scalar_path_only) {
                            system_basic_info_scalar_path_candidate(&value)
                        } else {
                            None
                        }
                    } else if action == Some("path_batch_facts")
                        && route.is_some_and(route_requires_single_file_delivery)
                    {
                        path_batch_file_delivery_token_candidate(route, &value)
                    } else if action == Some("path_batch_facts")
                        && route.is_some_and(route_allows_path_batch_file_basename_observed_answer)
                    {
                        system_basic_path_batch_file_basename_candidate(&value)
                    } else if action == Some("path_batch_facts")
                        && route.is_some_and(|route| {
                            route_allows_path_batch_scalar_path_observed_answer(route)
                                || route_scalar_has_plain_path_terminal_respond(route, loop_state)
                        })
                    {
                        system_basic_path_batch_scalar_path_candidate(&value)
                    } else if action == Some("path_batch_facts")
                        && route.is_some_and(route_requests_scalar_existence)
                    {
                        system_basic_scalar_existence_candidate(
                            state,
                            &value,
                            prefers_english_presence_answer,
                        )
                    } else if action == Some("path_batch_facts")
                        && !existence_with_path_should_use_llm_synthesis
                        && route.is_some_and(route_prefers_path_kind_fact_answer)
                    {
                        path_batch_fact_path_kind_candidate(&value, prefers_english_free_text)
                            .or_else(|| {
                                (!existence_with_path_should_use_llm_synthesis
                                    && route.is_some_and(|route| {
                                        super::output_route_policy::route_contract_marker_is(
                                            route,
                                            crate::OutputSemanticKind::ExistenceWithPath,
                                        )
                                    }))
                                .then(|| {
                                    system_basic_existence_with_path_candidate(
                                        state,
                                        &value,
                                        locator_hint,
                                        auto_locator_path,
                                        prefers_english_presence_answer,
                                    )
                                })
                                .flatten()
                            })
                    } else if !existence_with_path_should_use_llm_synthesis
                        && route.is_some_and(|route| {
                            super::output_route_policy::route_contract_marker_is(
                                route,
                                crate::OutputSemanticKind::ExistenceWithPath,
                            )
                        })
                    {
                        system_basic_existence_with_path_candidate(
                            state,
                            &value,
                            locator_hint,
                            auto_locator_path,
                            prefers_english_presence_answer,
                        )
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
                    allow_localized_direct_template,
                    prefers_english_free_text,
                )
            })
            .or_else(|| {
                (!existence_with_path_should_use_llm_synthesis
                    && allows_normalized_scalar_direct_fallback(
                        &observed_output.skill,
                        route,
                        response_shape,
                    ))
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
    route: Option<&crate::RouteResult>,
    value: &serde_json::Value,
    locator_hint: Option<&str>,
    prefer_english: bool,
    allow_multi_result_list: bool,
    prefer_full_path: bool,
) -> Option<String> {
    if route.is_some_and(|route| {
        super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::DirectoryPurposeSummary,
        ) && route.output_contract.requires_content_evidence
    }) {
        return None;
    }
    if route.is_some_and(|route| {
        super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::RawCommandOutput,
        )
    }) {
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
            fs_search_content_presence_direct_answer_candidate(state, route, value, prefer_english)
        })
        .or_else(|| {
            route.and_then(|route| {
                fs_search_route_filtered_listing_candidate(route, value, allow_multi_result_list)
            })
        })
        .or_else(|| route.and_then(|route| fs_search_contract_listing_candidate(route, value)))
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
    route: Option<&crate::RouteResult>,
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
    if route.output_contract.delivery_required {
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
    route.output_contract.requires_content_evidence
        && super::output_route_policy::route_contract_marker_is_any(
            route,
            &[
                crate::OutputSemanticKind::ContentExcerptSummary,
                crate::OutputSemanticKind::RawCommandOutput,
            ],
        )
}

pub(super) fn route_allows_read_range_direct_passthrough(
    route: Option<&crate::RouteResult>,
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
    if !super::output_route_policy::route_is_unclassified_contract(route) {
        return false;
    }
    if route.ask_mode.is_plain_act() {
        return true;
    }
    route.ask_mode.finalize_chat_wrapped()
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
}

pub(super) fn route_allows_raw_read_range_direct_passthrough(
    route: Option<&crate::RouteResult>,
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
    super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::RawCommandOutput,
    ) && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
}

pub(super) fn allows_normalized_scalar_direct_fallback(
    skill: &str,
    route: Option<&crate::RouteResult>,
    response_shape: Option<crate::OutputResponseShape>,
) -> bool {
    match skill {
        "git_basic" => false,
        "package_manager" => false,
        "archive_basic" => false,
        "http_basic" => {
            !matches!(
                response_shape,
                Some(crate::OutputResponseShape::OneSentence)
            ) && !route_requires_http_body_synthesis(route)
        }
        _ => true,
    }
}

pub(super) fn route_requires_http_body_synthesis(route: Option<&crate::RouteResult>) -> bool {
    let Some(route) = route else {
        return false;
    };
    if !route.output_contract.requires_content_evidence {
        return false;
    }
    if route_requests_browser_page_body(route) {
        return true;
    }
    let Some(shape) = crate::evidence_policy::final_answer_shape_for_route(route) else {
        return false;
    };
    if super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::ServiceStatus,
    ) {
        return shape.class() == crate::evidence_policy::FinalAnswerShapeClass::Verdict;
    }
    false
}

fn route_requests_browser_page_body(route: &crate::RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_action(
        route,
        &["browser", "http"],
        &["open", "get", "read", "extract"],
    ) && !crate::machine_capability_ref::route_has_capability_action(
        route,
        &["browser", "web"],
        &["search"],
    )
}

fn inventory_dir_can_use_direct_names(
    route: Option<&crate::RouteResult>,
    value: &serde_json::Value,
    loop_state: &LoopState,
    is_plain_act: bool,
    allow_raw_listing_direct_answer: bool,
) -> bool {
    let has_machine_names = value
        .get("names_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        && inventory_dir_names(value).is_some();
    if has_machine_names
        && route.is_some_and(|route| {
            super::output_route_policy::route_contract_marker_is_any(
                route,
                &[
                    crate::OutputSemanticKind::FileNames,
                    crate::OutputSemanticKind::DirectoryNames,
                    crate::OutputSemanticKind::DirectoryEntryGroups,
                    crate::OutputSemanticKind::FilePaths,
                ],
            )
        })
    {
        return true;
    }
    if has_machine_names
        && route_allows_latest_plan_names_direct(route)
        && latest_plan_requests_names_only_listing(loop_state)
    {
        return true;
    }
    (is_plain_act
        || route.is_some_and(|route| {
            super::output_route_policy::route_contract_marker_is(
                route,
                crate::OutputSemanticKind::DirectoryEntryGroups,
            )
        }))
        && allow_raw_listing_direct_answer
}

fn route_allows_latest_plan_names_direct(route: Option<&crate::RouteResult>) -> bool {
    let Some(route) = route else {
        return false;
    };
    matches!(
        route.output_contract.response_shape,
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

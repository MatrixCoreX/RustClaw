use super::*;

pub(super) fn structured_scalar_candidate(
    state: Option<&AppState>,
    route: Option<&crate::IntentOutputContract>,
    request_text: Option<&str>,
    skill: &str,
    body: &str,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
    prefer_english: bool,
) -> Option<String> {
    if skill == "git_basic" {
        return git_basic_scalar_candidate(route, body);
    }
    if skill == "archive_basic" {
        if let Some(route) = route {
            if super::output_route_policy::route_contract_marker_is(
                route,
                crate::OutputSemanticKind::ArchivePack,
            ) {
                if let Some(path) = archive_basic_path_value_from_body(
                    body,
                    &["archive", "archive_path", "output_path", "path"],
                ) {
                    return Some(path);
                }
            } else if super::output_route_policy::route_contract_marker_is(
                route,
                crate::OutputSemanticKind::ArchiveUnpack,
            ) {
                if let Some(path) = archive_basic_path_value_from_body(
                    body,
                    &["dest", "dest_path", "destination", "path"],
                ) {
                    return Some(path);
                }
            }
        }
        let summary = archive_list_summary_from_body(body)?;
        if route.is_some_and(route_requests_scalar_count) {
            return Some(summary.entries.len().to_string());
        }
        return route
            .filter(|route| route_requests_scalar_existence(route))
            .and_then(|route| {
                archive_entry_existence_direct_answer(
                    state,
                    route,
                    request_text,
                    &summary,
                    auto_locator_path.or(locator_hint),
                    prefer_english,
                )
            });
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if let Some(answer) = selected_structured_scalar_candidate(route, &value) {
        return Some(answer);
    }
    if skill == "service_control" {
        let response_shape = route.map(|route| route.response_shape);
        let service_status_route = route.is_some_and(|route| {
            super::output_route_policy::route_contract_marker_is(
                route,
                crate::OutputSemanticKind::ServiceStatus,
            )
        });
        if route.is_some_and(|route| route.response_shape == crate::OutputResponseShape::Scalar) {
            return service_control_status_direct_answer_candidate(&value, response_shape);
        }
        if service_status_route {
            return None;
        }
        return service_control_summary_candidate(&value);
    }
    if skill == "fs_search" {
        let rooted_scalar = prefer_full_path
            .then(|| {
                fs_search_scalar_candidate(
                    state,
                    &value,
                    locator_hint,
                    auto_locator_path,
                    prefer_full_path,
                    prefer_english,
                )
            })
            .flatten();
        if let Some(answer) = rooted_scalar.or_else(|| {
            route
                .and_then(|route| {
                    fs_search_output_direct_answer_candidate(
                        state,
                        Some(route),
                        &value,
                        locator_hint,
                        prefer_english,
                        true,
                        prefer_full_path,
                    )
                })
                .or_else(|| {
                    fs_search_scalar_candidate(
                        state,
                        &value,
                        locator_hint,
                        auto_locator_path,
                        prefer_full_path,
                        prefer_english,
                    )
                })
        }) {
            return Some(answer);
        }
        return None;
    }
    if skill == "fs_basic"
        && value
            .get("action")
            .and_then(|v| v.as_str())
            .is_some_and(|action| {
                action.eq_ignore_ascii_case("find_ext") || action.eq_ignore_ascii_case("find_name")
            })
    {
        if let Some(answer) = route.and_then(|route| {
            fs_search_output_direct_answer_candidate(
                state,
                Some(route),
                &value,
                locator_hint,
                prefer_english,
                true,
                prefer_full_path,
            )
        }) {
            return Some(answer);
        }
    }
    if !matches!(skill, "system_basic" | "config_basic" | "fs_basic") {
        return None;
    }
    let value = structured_observed_body_value(&value);
    if skill == "system_basic"
        && route.is_some_and(route_requests_scalar_path_only)
        && system_basic_value_looks_like_info(value)
    {
        return system_basic_info_scalar_path_candidate(value);
    }
    let action = value.get("action").and_then(|v| v.as_str())?;
    match action {
        "validate_structured" => {
            validate_structured_direct_answer_candidate(state, value, prefer_english)
        }
        "read_range" => route
            .filter(|route| route_allows_scalar_read_range_direct_answer(route))
            .and_then(|_| {
                value
                    .get("excerpt")
                    .and_then(|v| v.as_str())
                    .and_then(|excerpt| {
                        normalize_read_range_excerpt_for_direct_answer(
                            state,
                            excerpt,
                            prefer_english,
                            read_range_preserve_blank_lines(&value),
                        )
                    })
            }),
        "inventory_dir" => {
            let hidden_count_route = route.is_some_and(|route| {
                route.response_shape == crate::OutputResponseShape::Scalar
                    && route_requests_hidden_entries_check(route)
            });
            if hidden_count_route {
                value
                    .get("counts")
                    .and_then(|v| v.get("hidden"))
                    .and_then(value_scalar_text)
                    .or_else(|| {
                        inventory_dir_names(&value).map(|names| {
                            names
                                .into_iter()
                                .filter(|name| is_user_hidden_entry(name))
                                .count()
                                .to_string()
                        })
                    })
            } else if route.is_some_and(route_requests_scalar_count) {
                value
                    .get("counts")
                    .and_then(|v| v.get("total"))
                    .and_then(value_scalar_text)
                    .or_else(|| inventory_dir_names(&value).map(|names| names.len().to_string()))
            } else if route.is_some_and(route_requests_scalar_path_only) {
                inventory_dir_scalar_path_candidate(&value, prefer_full_path)
            } else {
                None
            }
        }
        "tree_summary" => tree_summary_direct_answer_candidate(state, &value, prefer_english),
        "dir_compare" => dir_compare_direct_answer_candidate(state, &value, prefer_english),
        "extract_field" | "read_field" => {
            if route.is_some_and(|route| route.response_shape != crate::OutputResponseShape::Scalar)
            {
                return None;
            }
            if value
                .get("exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                if route.is_some() && extract_field_has_non_exact_resolution(&value) {
                    if let Some(field_path) = json_trimmed_str(&value, "resolved_field_path") {
                        let field_value = value.get("value").unwrap_or(&serde_json::Value::Null);
                        if matches!(
                            field_value,
                            serde_json::Value::Object(_) | serde_json::Value::Array(_)
                        ) {
                            let scalar_contract = route.is_some_and(|route| {
                                route.response_shape == crate::OutputResponseShape::Scalar
                            });
                            if !scalar_contract {
                                return None;
                            }
                            return Some(structured_field_display_line(
                                state,
                                field_path,
                                field_value,
                                value.get("value_text").and_then(|v| v.as_str()),
                                true,
                                prefer_english,
                            ));
                        }
                        if route.is_some_and(|route| {
                            route.response_shape == crate::OutputResponseShape::Scalar
                        }) && json_trimmed_str(&value, "match_strategy")
                            .is_some_and(|strategy| strategy == "array_item_key_path")
                        {
                            return value_structured_text(
                                field_value,
                                value.get("value_text").and_then(|v| v.as_str()),
                            );
                        }
                        return Some(structured_field_display_line(
                            state,
                            field_path,
                            field_value,
                            value.get("value_text").and_then(|v| v.as_str()),
                            true,
                            prefer_english,
                        ));
                    }
                }
                let field_value = value.get("value").unwrap_or(&serde_json::Value::Null);
                if matches!(
                    field_value,
                    serde_json::Value::Object(_) | serde_json::Value::Array(_)
                ) {
                    let scalar_contract = route.is_some_and(|route| {
                        route.response_shape == crate::OutputResponseShape::Scalar
                    });
                    if !scalar_contract {
                        return None;
                    }
                    let value_text = value.get("value_text").and_then(|v| v.as_str());
                    if route.is_some() && extract_field_has_non_exact_resolution(&value) {
                        if let Some(field_path) = json_trimmed_str(&value, "resolved_field_path") {
                            return Some(structured_field_display_line(
                                state,
                                field_path,
                                field_value,
                                value_text,
                                true,
                                prefer_english,
                            ));
                        }
                    }
                    return value_structured_text(field_value, value_text);
                }
                return value_structured_text(
                    value.get("value").unwrap_or(&serde_json::Value::Null),
                    value.get("value_text").and_then(|v| v.as_str()),
                );
            }
            let field_path = value
                .get("field_path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("requested field");
            Some(missing_extract_field_machine_answer(field_path))
        }
        "extract_fields" | "read_fields" => {
            if route.is_some_and(|route| route.response_shape != crate::OutputResponseShape::Scalar)
            {
                return None;
            }
            structured_scalar_observation_from_value(&value).map(|observation| observation.text)
        }
        "path_batch_facts" => {
            if route.is_some_and(route_requests_scalar_existence) {
                system_basic_scalar_existence_candidate(state, value, prefer_english)
            } else if route.is_some_and(route_requests_file_basename) {
                system_basic_path_batch_file_basename_candidate(value)
            } else if route.is_some_and(route_requests_scalar_path_only) {
                system_basic_path_batch_scalar_path_candidate(value)
            } else {
                None
            }
        }
        "runtime_status" => value
            .get("value")
            .or_else(|| value.get("field_value"))
            .or_else(|| value.get("command_output"))
            .and_then(value_scalar_text),
        "count_inventory" => count_inventory_direct_answer_candidate(
            state,
            value,
            route.map(|route| route.response_shape),
            prefer_english,
        ),
        "structured_keys" => structured_keys_direct_answer_candidate(
            state,
            value,
            request_text,
            route.map(|route| route.response_shape),
            prefer_english,
        ),
        _ => None,
    }
}

fn selected_structured_scalar_candidate(
    route: Option<&crate::IntentOutputContract>,
    value: &serde_json::Value,
) -> Option<String> {
    let route = route?;
    if route.response_shape != crate::OutputResponseShape::Scalar {
        return None;
    }
    let selector = route
        .selection
        .structured_field_selector
        .as_deref()
        .map(str::trim)
        .filter(|selector| {
            !selector.is_empty()
                && !selector.contains(',')
                && !selector.chars().any(char::is_whitespace)
                && !selector.chars().any(|ch| matches!(ch, '*' | '[' | ']'))
        })?;
    structured_value_at_path(value, selector)
        .or_else(|| {
            value
                .get("extra")
                .and_then(|extra| structured_value_at_path(extra, selector))
        })
        .and_then(value_scalar_text)
}

fn structured_value_at_path<'a>(
    value: &'a serde_json::Value,
    selector: &str,
) -> Option<&'a serde_json::Value> {
    selector
        .split('.')
        .try_fold(value, |current, segment| current.as_object()?.get(segment))
}

pub(super) fn selected_capability_result_scalar_candidate(
    route: Option<&crate::IntentOutputContract>,
    results: &[claw_core::capability_result::CapabilityResultEnvelope],
) -> Option<String> {
    let route = route?;
    if route.response_shape != crate::OutputResponseShape::Scalar {
        return None;
    }
    let fields = route
        .selection
        .structured_field_selector
        .as_deref()
        .and_then(crate::machine_kv_projection::exact_machine_field_selector)?;
    let [field] = fields.as_slice() else {
        return None;
    };
    results.iter().rev().find_map(|result| {
        if result.status != claw_core::capability_result::CapabilityResultStatus::Ok {
            return None;
        }
        selected_result_data_value(&result.data, field).and_then(value_scalar_text)
    })
}

pub(super) fn selected_capability_result_exact_candidate(
    route: Option<&crate::IntentOutputContract>,
    results: &[claw_core::capability_result::CapabilityResultEnvelope],
) -> Option<String> {
    let route = route?;
    if route.response_shape != crate::OutputResponseShape::Strict {
        return None;
    }
    let fields = route
        .selection
        .structured_field_selector
        .as_deref()
        .and_then(crate::machine_kv_projection::exact_machine_field_selector)?;
    let [field] = fields.as_slice() else {
        return None;
    };
    results.iter().rev().find_map(|result| {
        if result.status != claw_core::capability_result::CapabilityResultStatus::Ok {
            return None;
        }
        selected_result_data_value(&result.data, field).and_then(exact_result_value_text)
    })
}

fn exact_result_value_text(value: &serde_json::Value) -> Option<String> {
    value_scalar_text(value).or_else(|| match value {
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            serde_json::to_string(value).ok()
        }
        _ => None,
    })
}

fn selected_result_data_value<'a>(
    data: &'a serde_json::Value,
    selector: &str,
) -> Option<&'a serde_json::Value> {
    structured_value_at_path(data, selector)
        .or_else(|| {
            data.get("extra")
                .and_then(|extra| structured_value_at_path(extra, selector))
        })
        .or_else(|| {
            data.get("output")
                .and_then(|output| structured_value_at_path(output, selector))
        })
}

pub(super) fn git_basic_commit_subject_candidate(body: &str) -> Option<String> {
    static GIT_ONELINE_SUBJECT_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let regex = GIT_ONELINE_SUBJECT_RE.get_or_init(|| {
        regex::Regex::new(r"^[0-9a-fA-F]{7,40}\s+(.+)$").expect("valid git oneline regex")
    });
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .find_map(|line| regex.captures(line))
        .and_then(|captures| captures.get(1))
        .map(|subject| subject.as_str().trim().to_string())
        .filter(|subject| !subject.is_empty())
}

pub(super) fn git_basic_scalar_candidate(
    route: Option<&crate::IntentOutputContract>,
    body: &str,
) -> Option<String> {
    if route.is_some_and(|route| {
        super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::GitCommitSubject,
        )
    }) {
        return git_basic_commit_subject_candidate(body);
    }
    if let Some(branch) = git_basic_current_branch_scalar_candidate(route, body) {
        return Some(branch);
    }
    let scalar = normalized_scalar_candidate(body)?;
    static GIT_ONELINE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let regex = GIT_ONELINE_RE.get_or_init(|| {
        regex::Regex::new(r"^[0-9a-fA-F]{7,40}\s+.+$").expect("valid git oneline regex")
    });
    if regex.is_match(&scalar) {
        return None;
    }
    Some(scalar)
}

pub(super) fn git_basic_current_branch_scalar_candidate(
    route: Option<&crate::IntentOutputContract>,
    body: &str,
) -> Option<String> {
    if route.is_some_and(|route| {
        !super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::GitRepositoryState,
        )
    }) {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body.trim()).ok()?;
    let action = git_basic_json_action(&value)?;
    if action != "current_branch" {
        return None;
    }
    git_current_branch_from_json_value(&value)
}

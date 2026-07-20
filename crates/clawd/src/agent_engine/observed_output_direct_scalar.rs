use super::*;

pub(super) fn structured_scalar_candidate(
    state: Option<&AppState>,
    route: Option<&crate::IntentOutputContract>,
    skill: &str,
    body: &str,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
    prefer_english: bool,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if let Some(answer) = selected_structured_scalar_candidate(route, &value) {
        return Some(answer);
    }
    if skill == "service_control" {
        return None;
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
        && route.is_some_and(route_requests_exact_scalar_path)
        && system_basic_value_looks_like_info(value)
    {
        return system_basic_info_scalar_path_candidate(value);
    }
    let action = value.get("action").and_then(|v| v.as_str())?;
    match action {
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
            if route.is_some_and(route_requests_scalar_count) {
                value
                    .get("counts")
                    .and_then(|v| v.get("total"))
                    .and_then(value_scalar_text)
                    .or_else(|| inventory_dir_names(&value).map(|names| names.len().to_string()))
            } else if let Some(field) = route.and_then(exact_scalar_path_selector) {
                inventory_dir_scalar_path_candidate(&value, &field)
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
            } else if let Some(field) = route.and_then(exact_scalar_path_selector) {
                system_basic_path_batch_scalar_path_candidate(value, &field)
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

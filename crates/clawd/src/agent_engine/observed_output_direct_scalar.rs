use super::*;

pub(super) fn structured_scalar_candidate(
    state: Option<&AppState>,
    route: Option<&crate::RouteResult>,
    skill: &str,
    body: &str,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
    allow_localized_direct_template: bool,
    prefer_english: bool,
) -> Option<String> {
    if skill == "package_manager" {
        let response_shape = route.map(|route| route.output_contract.response_shape);
        return package_manager_summary_candidate(
            state,
            body,
            response_shape,
            allow_localized_direct_template,
            prefer_english,
        );
    }
    if skill == "git_basic" {
        return git_basic_scalar_candidate(route, body);
    }
    if skill == "archive_basic" {
        if let Some(route) = route {
            match route.output_contract.semantic_kind {
                crate::OutputSemanticKind::ArchivePack => {
                    if let Some(path) = archive_basic_path_value_from_body(
                        body,
                        &["archive", "archive_path", "output_path", "path"],
                    ) {
                        return Some(path);
                    }
                }
                crate::OutputSemanticKind::ArchiveUnpack => {
                    if let Some(path) = archive_basic_path_value_from_body(
                        body,
                        &["dest", "dest_path", "destination", "path"],
                    ) {
                        return Some(path);
                    }
                }
                _ => {}
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
                    Some(route.resolved_intent.as_str()),
                    &summary,
                    auto_locator_path.or(locator_hint),
                    prefer_english,
                )
            });
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if market_quote_skill_supports_scalar(state, skill) {
        if let Some(answer) = market_quote_scalar_candidate(route, &value) {
            return Some(answer);
        }
    }
    if skill == "db_basic" {
        if let Some(route) = route {
            return match route.output_contract.semantic_kind {
                crate::OutputSemanticKind::ScalarCount => db_basic_count_candidate(&value),
                crate::OutputSemanticKind::SqliteTableNamesOnly => {
                    db_basic_table_names(&value).map(|names| names.join("\n"))
                }
                crate::OutputSemanticKind::SqliteTableListing
                | crate::OutputSemanticKind::SqliteDatabaseKindJudgment => None,
                _ => db_basic_scalar_candidate(&value),
            };
        }
        return db_basic_scalar_candidate(&value);
    }
    if skill == "service_control" {
        let response_shape = route.map(|route| route.output_contract.response_shape);
        let service_status_route = route.is_some_and(|route| {
            route.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
        });
        if route.is_some_and(|route| {
            route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        }) {
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
    if skill == "system_basic"
        && route.is_some_and(route_requests_scalar_path_only)
        && system_basic_value_looks_like_info(&value)
    {
        return system_basic_info_scalar_path_candidate(&value);
    }
    let action = value.get("action").and_then(|v| v.as_str())?;
    match action {
        "validate_structured" => {
            validate_structured_direct_answer_candidate(state, &value, prefer_english)
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
                route.output_contract.response_shape == crate::OutputResponseShape::Scalar
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
            if route.is_some_and(|route| {
                route.output_contract.response_shape != crate::OutputResponseShape::Scalar
            }) {
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
                                route.output_contract.response_shape
                                    == crate::OutputResponseShape::Scalar
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
                            route.output_contract.response_shape
                                == crate::OutputResponseShape::Scalar
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
                        route.output_contract.response_shape == crate::OutputResponseShape::Scalar
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
            Some(observed_t_with_vars(
                state,
                "clawd.msg.extract_field_missing",
                "未找到 {field_path} 字段",
                "field not found: {field_path}",
                prefer_english,
                &[("field_path", field_path)],
            ))
        }
        "extract_fields" | "read_fields" => {
            if route.is_some_and(|route| {
                route.output_contract.response_shape != crate::OutputResponseShape::Scalar
            }) {
                return None;
            }
            structured_scalar_observation_from_value(&value).map(|observation| observation.text)
        }
        "path_batch_facts" => {
            if route.is_some_and(route_requests_scalar_existence) {
                system_basic_scalar_existence_candidate(state, &value, prefer_english)
            } else if route.is_some_and(route_requests_file_basename) {
                system_basic_path_batch_file_basename_candidate(&value)
            } else if route.is_some_and(route_requests_scalar_path_only) {
                system_basic_path_batch_scalar_path_candidate(&value)
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
            &value,
            route.map(|route| route.output_contract.response_shape),
            prefer_english,
        ),
        "structured_keys" => structured_keys_direct_answer_candidate(
            state,
            &value,
            route.map(|route| route.resolved_intent.as_str()),
            route.map(|route| route.output_contract.response_shape),
            prefer_english,
        ),
        _ => None,
    }
}

pub(super) fn market_quote_scalar_candidate(
    route: Option<&crate::RouteResult>,
    value: &serde_json::Value,
) -> Option<String> {
    let route = route?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::MarketQuote
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
    {
        return None;
    }
    let quote = value
        .get("quote")
        .or_else(|| value.pointer("/extra/quote"))
        .filter(|quote| quote.is_object())?;
    let price = quote
        .get("price_usd")
        .or_else(|| quote.get("price"))
        .or_else(|| quote.get("last"))
        .and_then(|value| value.as_f64())?;
    let symbol = quote
        .get("symbol")
        .or_else(|| value.get("symbol"))
        .or_else(|| value.pointer("/extra/symbol"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    Some(match symbol {
        Some(symbol) => format!("{symbol} ${}", format_market_price(price)),
        None => format!("${}", format_market_price(price)),
    })
}

pub(super) fn market_quote_output_has_scalar_price(
    state: Option<&AppState>,
    skill: &str,
    body: &str,
) -> bool {
    if !market_quote_skill_supports_scalar(state, skill) {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("quote")
                .or_else(|| value.pointer("/extra/quote"))
                .cloned()
        })
        .and_then(|quote| {
            quote
                .get("price_usd")
                .or_else(|| quote.get("price"))
                .or_else(|| quote.get("last"))
                .and_then(|value| value.as_f64())
        })
        .is_some()
}

pub(super) fn market_quote_skill_supports_scalar(state: Option<&AppState>, skill: &str) -> bool {
    state
        .and_then(|state| state.get_skills_registry())
        .is_some_and(|registry| registry.has_semantic_tag(skill, MARKET_QUOTE_SCALAR_SEMANTIC_TAG))
}

pub(super) fn format_market_price(price: f64) -> String {
    let raw = if price.abs() >= 1.0 {
        format!("{price:.2}")
    } else {
        format!("{price:.8}")
    };
    raw.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub(super) fn package_manager_summary_candidate(
    state: Option<&AppState>,
    body: &str,
    response_shape: Option<crate::OutputResponseShape>,
    allow_localized_direct_template: bool,
    prefer_english: bool,
) -> Option<String> {
    let manager = body
        .lines()
        .find_map(|line| line.trim().strip_prefix("package_manager="))
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    match response_shape {
        Some(crate::OutputResponseShape::Scalar) => Some(manager.to_string()),
        Some(
            crate::OutputResponseShape::OneSentence
            | crate::OutputResponseShape::Free
            | crate::OutputResponseShape::Strict,
        ) if allow_localized_direct_template => {
            Some(observed_t_with_vars(
                state,
                "clawd.msg.package_manager_detected",
                "检测到的包管理器是 {manager}，依据是 package_manager 返回了 package_manager={manager}。",
                "Detected package manager: {manager}. Basis: package_manager returned package_manager={manager}.",
                prefer_english,
                &[("manager", manager)],
            ))
        }
        _ => None,
    }
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
    route: Option<&crate::RouteResult>,
    body: &str,
) -> Option<String> {
    if route.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::GitCommitSubject
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
    route: Option<&crate::RouteResult>,
    body: &str,
) -> Option<String> {
    if route.is_some_and(|route| {
        route.output_contract.semantic_kind != crate::OutputSemanticKind::GitRepositoryState
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

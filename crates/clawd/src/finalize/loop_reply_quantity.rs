use std::path::Path;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{
    latest_contractual_synthesis_output, latest_plan_requested_synthesis,
    latest_successful_synthesis_output_matches, log_deterministic_delivery_record,
    looks_like_structured_machine_output, path_display_label,
    planned_delivery_is_publishable_model_language_answer,
    prefer_english_for_agent_contextual_user_text, route_allows_model_language_final_answer,
    structured_json_values_from_output,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SizeComparisonAnswerStyle {
    DeltaOnly,
    ExplainRatio,
}

fn size_comparison_answer_style(
    route: &crate::RouteResult,
    user_text: &str,
) -> SizeComparisonAnswerStyle {
    if crate::intent_router::contract_test_hint_value(user_text, "selector_answer_style")
        .as_deref()
        .is_some_and(|value| {
            matches!(
                value.trim(),
                "larger_with_sizes" | "comparison_with_sizes" | "explain_ratio"
            )
        })
    {
        return SizeComparisonAnswerStyle::ExplainRatio;
    }
    if crate::intent_router::contract_test_hint_value(user_text, "selector_answer_style")
        .as_deref()
        .is_some_and(|value| matches!(value.trim(), "delta_only" | "size_delta"))
    {
        return SizeComparisonAnswerStyle::DeltaOnly;
    }
    let _ = route;
    SizeComparisonAnswerStyle::ExplainRatio
}

fn compare_paths_size_ratio_answer_with_style(
    body: &str,
    prefer_english: bool,
    style: SizeComparisonAnswerStyle,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|value| value.as_str()) != Some("compare_paths") {
        return None;
    }
    let left = value.get("left")?;
    let right = value.get("right")?;
    let left_size = left.get("size_bytes").and_then(|value| value.as_u64())?;
    let right_size = right.get("size_bytes").and_then(|value| value.as_u64())?;
    let left_label = path_display_label(left, "left");
    let right_label = path_display_label(right, "right");
    if style == SizeComparisonAnswerStyle::DeltaOnly {
        if left_size == right_size {
            return Some(if prefer_english {
                format!("{left_label} and {right_label}: 0 bytes")
            } else {
                format!("{left_label} 和 {right_label}：0 字节")
            });
        }
        let (larger_label, delta) = if left_size > right_size {
            (left_label, left_size - right_size)
        } else {
            (right_label, right_size - left_size)
        };
        return Some(if prefer_english {
            format!("{larger_label}: {delta} bytes")
        } else {
            format!("{larger_label}：{delta} 字节")
        });
    }
    if left_size == right_size {
        return Some(if prefer_english {
            format!("They are the same size: {left_label} and {right_label} are both {left_size} bytes.")
        } else {
            format!("{left_label} 和 {right_label} 一样大，都是 {left_size} 字节。")
        });
    }
    let (larger_label, larger_size, smaller_label, smaller_size) = if left_size > right_size {
        (left_label, left_size, right_label, right_size)
    } else {
        (right_label, right_size, left_label, left_size)
    };
    let ratio = (smaller_size > 0).then(|| larger_size as f64 / smaller_size as f64);
    Some(match (prefer_english, ratio) {
        (true, Some(ratio)) => format!(
            "`{larger_label}` is larger: {larger_size} bytes, about {ratio:.2}x `{smaller_label}` ({smaller_size} bytes)."
        ),
        (true, None) => format!(
            "`{larger_label}` is larger: {larger_size} bytes; `{smaller_label}` is 0 bytes."
        ),
        (false, Some(ratio)) => format!(
            "`{larger_label}` 更大：{larger_size} 字节，大约是 `{smaller_label}`（{smaller_size} 字节）的 {ratio:.2} 倍。"
        ),
        (false, None) => format!(
            "`{larger_label}` 更大：{larger_size} 字节；`{smaller_label}` 为 0 字节。"
        )
    })
}

#[cfg(test)]
pub(super) fn compare_paths_size_ratio_answer(body: &str, prefer_english: bool) -> Option<String> {
    compare_paths_size_ratio_answer_with_style(
        body,
        prefer_english,
        SizeComparisonAnswerStyle::ExplainRatio,
    )
}

#[derive(Debug, Clone)]
pub(crate) struct PathSizeFact {
    pub(crate) label: String,
    pub(crate) size_bytes: u64,
}

pub(super) fn path_batch_size_facts(value: &serde_json::Value) -> Option<Vec<PathSizeFact>> {
    if value.get("action").and_then(|value| value.as_str()) != Some("path_batch_facts") {
        return None;
    }
    let facts = value.get("facts")?.as_array()?;
    let mut out = Vec::new();
    for entry in facts {
        if entry.get("exists").and_then(|value| value.as_bool()) != Some(true) {
            continue;
        }
        let fact = entry.get("fact").unwrap_or(entry);
        let size_bytes = fact
            .get("size_bytes")
            .and_then(|value| value.as_u64())
            .or_else(|| entry.get("size_bytes").and_then(|value| value.as_u64()))?;
        let label = path_display_label(fact, "path");
        out.push(PathSizeFact { label, size_bytes });
    }
    (out.len() >= 2).then_some(out)
}

fn compare_paths_size_facts(value: &serde_json::Value) -> Option<Vec<PathSizeFact>> {
    if value.get("action").and_then(|value| value.as_str()) != Some("compare_paths") {
        return None;
    }
    let left = value.get("left")?;
    let right = value.get("right")?;
    let left_size = left.get("size_bytes").and_then(|value| value.as_u64())?;
    let right_size = right.get("size_bytes").and_then(|value| value.as_u64())?;
    Some(vec![
        PathSizeFact {
            label: path_display_label(left, "left"),
            size_bytes: left_size,
        },
        PathSizeFact {
            label: path_display_label(right, "right"),
            size_bytes: right_size,
        },
    ])
}

pub(super) fn observed_quantity_size_facts(loop_state: &LoopState) -> Vec<PathSizeFact> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .find_map(|output| {
            structured_json_values_from_output(output)
                .iter()
                .find_map(|value| {
                    path_batch_size_facts(value).or_else(|| compare_paths_size_facts(value))
                })
        })
        .unwrap_or_default()
}

pub(super) fn latest_delivery_preserves_observed_quantity_size_facts(
    loop_state: &LoopState,
) -> Option<String> {
    let answer = loop_state
        .delivery_messages
        .iter()
        .rev()
        .find(|message| !crate::finalize::is_execution_summary_message(message))?
        .trim();
    let facts = observed_quantity_size_facts(loop_state);
    if facts.len() < 2 {
        return None;
    }
    if structured_quantity_json_preserves_observed_size_facts(answer, &facts) {
        return Some(answer.to_string());
    }
    if answer.is_empty()
        || crate::finalize::parse_delivery_token(answer).is_some()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || crate::finalize::is_execution_summary_message(answer)
        || looks_like_structured_machine_output(answer)
    {
        return None;
    }
    let matched = facts
        .iter()
        .filter(|fact| {
            answer.contains(&fact.label) && answer_contains_observed_size(answer, fact.size_bytes)
        })
        .count();
    (matched >= 2).then(|| answer.to_string())
}

fn answer_contains_observed_size(answer: &str, size_bytes: u64) -> bool {
    let needle = size_bytes.to_string();
    answer.contains(&needle)
        || decimal_digit_runs_allowing_group_separators(answer)
            .iter()
            .any(|run| run == &needle)
        || answer_contains_rounded_size_unit(answer, size_bytes)
}

fn answer_contains_rounded_size_unit(answer: &str, size_bytes: u64) -> bool {
    size_quantities(answer).into_iter().any(|quantity| {
        let displayed_bytes = quantity.value * quantity.multiplier;
        let rounding_tolerance =
            0.5 * 10f64.powi(-(quantity.decimal_places as i32)) * quantity.multiplier;
        (displayed_bytes - size_bytes as f64).abs() <= rounding_tolerance.max(1.0)
    })
}

#[derive(Debug, Clone, Copy)]
struct SizeQuantity {
    value: f64,
    decimal_places: usize,
    multiplier: f64,
}

fn size_quantities(text: &str) -> Vec<SizeQuantity> {
    let chars = text.char_indices().collect::<Vec<_>>();
    let mut quantities = Vec::new();
    let mut idx = 0;
    while idx < chars.len() {
        let (start_byte, ch) = chars[idx];
        if !ch.is_ascii_digit() {
            idx += 1;
            continue;
        }

        let mut cursor = idx;
        let mut number = String::new();
        let mut decimal_seen = false;
        let mut decimal_places = 0usize;
        while cursor < chars.len() {
            let (_, ch) = chars[cursor];
            if ch.is_ascii_digit() {
                number.push(ch);
                if decimal_seen {
                    decimal_places += 1;
                }
                cursor += 1;
                continue;
            }
            if ch == '.'
                && !decimal_seen
                && cursor + 1 < chars.len()
                && chars[cursor + 1].1.is_ascii_digit()
            {
                number.push(ch);
                decimal_seen = true;
                cursor += 1;
                continue;
            }
            break;
        }

        while cursor < chars.len() && is_size_unit_spacing(chars[cursor].1) {
            cursor += 1;
        }
        let unit_start = cursor;
        while cursor < chars.len() && chars[cursor].1.is_ascii_alphabetic() {
            cursor += 1;
        }
        let unit_end = if cursor < chars.len() {
            chars[cursor].0
        } else {
            text.len()
        };
        let unit = if unit_start < cursor {
            &text[chars[unit_start].0..unit_end]
        } else {
            ""
        };
        if let (Ok(value), Some(multiplier)) = (number.parse::<f64>(), size_unit_multiplier(unit)) {
            quantities.push(SizeQuantity {
                value,
                decimal_places,
                multiplier,
            });
        }
        idx = if cursor > idx { cursor } else { idx + 1 };
        if idx == unit_start && unit_start == cursor && start_byte >= text.len() {
            break;
        }
    }
    quantities
}

fn is_size_unit_spacing(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '\u{00a0}' | '\u{202f}')
}

fn size_unit_multiplier(unit: &str) -> Option<f64> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "b" => Some(1.0),
        "kb" => Some(1_000.0),
        "kib" => Some(1_024.0),
        "mb" => Some(1_000_000.0),
        "mib" => Some(1_048_576.0),
        "gb" => Some(1_000_000_000.0),
        "gib" => Some(1_073_741_824.0),
        _ => None,
    }
}

fn decimal_digit_runs_allowing_group_separators(text: &str) -> Vec<String> {
    let mut runs = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
            continue;
        }
        if !current.is_empty() && is_numeric_group_separator(ch) {
            continue;
        }
        if !current.is_empty() {
            runs.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs
}

fn is_numeric_group_separator(ch: char) -> bool {
    matches!(ch, ',' | '_' | '.' | '\'' | ' ' | '\u{00a0}' | '\u{202f}')
}

pub(super) fn structured_quantity_json_preserves_observed_size_facts(
    answer: &str,
    facts: &[PathSizeFact],
) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(answer.trim()) else {
        return false;
    };
    let Some(obj) = value.as_object() else {
        return false;
    };
    let Some(larger_file) = obj
        .get("larger_file")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let Some(size_delta_bytes) = obj.get("size_delta_bytes").and_then(json_value_u64) else {
        return false;
    };
    let mut sorted = facts.to_vec();
    sorted.sort_by(|a, b| {
        b.size_bytes
            .cmp(&a.size_bytes)
            .then_with(|| a.label.cmp(&b.label))
    });
    let Some(largest) = sorted.first() else {
        return false;
    };
    let Some(runner_up) = sorted.get(1) else {
        return false;
    };
    let expected_delta = largest.size_bytes.saturating_sub(runner_up.size_bytes);
    if size_delta_bytes != expected_delta {
        return false;
    }
    let largest_basename = Path::new(&largest.label)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(largest.label.as_str());
    larger_file == largest.label || larger_file == largest_basename
}

fn json_value_u64(value: &serde_json::Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        let value = value.as_i64()?;
        (value >= 0).then_some(value as u64)
    })
}

fn quantity_comparison_json_answer_from_facts(mut facts: Vec<PathSizeFact>) -> Option<String> {
    if facts.len() < 2 {
        return None;
    }
    facts.sort_by(|a, b| {
        b.size_bytes
            .cmp(&a.size_bytes)
            .then_with(|| a.label.cmp(&b.label))
    });
    let largest = facts.first()?;
    let runner_up = facts.get(1)?;
    if largest.size_bytes == runner_up.size_bytes {
        return None;
    }
    let delta = largest.size_bytes.saturating_sub(runner_up.size_bytes);
    Some(
        serde_json::json!({
            "larger_file": largest.label,
            "size_delta_bytes": delta,
        })
        .to_string(),
    )
}

fn strict_quantity_comparison_json_answer(body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    path_batch_size_facts(&value)
        .or_else(|| compare_paths_size_facts(&value))
        .and_then(quantity_comparison_json_answer_from_facts)
}

fn path_batch_size_comparison_answer_with_style(
    body: &str,
    prefer_english: bool,
    style: SizeComparisonAnswerStyle,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let mut facts = path_batch_size_facts(&value)?;
    facts.sort_by(|a, b| {
        b.size_bytes
            .cmp(&a.size_bytes)
            .then_with(|| a.label.cmp(&b.label))
    });
    let largest = facts.first()?;
    let runner_up = facts.get(1)?;
    if largest.size_bytes == runner_up.size_bytes {
        let tied = facts
            .iter()
            .filter(|fact| fact.size_bytes == largest.size_bytes)
            .map(|fact| fact.label.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Some(if prefer_english {
            if style == SizeComparisonAnswerStyle::DeltaOnly {
                format!("{tied}: 0 bytes")
            } else {
                format!(
                    "They are the same size: {tied} are all {} bytes.",
                    largest.size_bytes
                )
            }
        } else {
            if style == SizeComparisonAnswerStyle::DeltaOnly {
                format!("{tied}：0 字节")
            } else {
                format!("它们一样大：{tied} 都是 {} 字节。", largest.size_bytes)
            }
        });
    }
    if style == SizeComparisonAnswerStyle::DeltaOnly {
        let delta = largest.size_bytes.saturating_sub(runner_up.size_bytes);
        return Some(if prefer_english {
            format!("{}: {} bytes", largest.label, delta)
        } else {
            format!("{}：{} 字节", largest.label, delta)
        });
    }
    let ratio = if runner_up.size_bytes == 0 {
        None
    } else {
        Some(largest.size_bytes as f64 / runner_up.size_bytes as f64)
    };
    Some(match (prefer_english, ratio) {
        (true, Some(ratio)) => format!(
            "`{}` is larger: {} bytes, about {:.2}x `{}` ({} bytes).",
            largest.label, largest.size_bytes, ratio, runner_up.label, runner_up.size_bytes
        ),
        (true, None) => format!(
            "`{}` is larger: {} bytes; `{}` is 0 bytes.",
            largest.label, largest.size_bytes, runner_up.label
        ),
        (false, Some(ratio)) => format!(
            "`{}` 更大：{} 字节，大约是 `{}`（{} 字节）的 {:.2} 倍。",
            largest.label, largest.size_bytes, runner_up.label, runner_up.size_bytes, ratio
        ),
        (false, None) => format!(
            "`{}` 更大：{} 字节；`{}` 为 0 字节。",
            largest.label, largest.size_bytes, runner_up.label
        ),
    })
}

fn compact_binary_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes_f = bytes as f64;
    if bytes_f >= GIB {
        format!("{:.1} GiB", bytes_f / GIB)
    } else if bytes_f >= MIB {
        format!("{:.1} MiB", bytes_f / MIB)
    } else if bytes_f >= KIB {
        format!("{:.1} KiB", bytes_f / KIB)
    } else {
        format!("{bytes} bytes")
    }
}

fn count_inventory_size_answer_with_shape(
    body: &str,
    _prefer_english: bool,
    response_shape: crate::OutputResponseShape,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|value| value.as_str()) != Some("count_inventory") {
        return None;
    }
    let counts = value.get("counts")?;
    let total_size = counts
        .get("total_size_bytes")
        .and_then(|value| value.as_u64())?;
    if matches!(response_shape, crate::OutputResponseShape::Scalar) {
        return None;
    }
    let label = path_display_label(&value, "path");
    let compact = compact_binary_size(total_size);
    let total_entries = counts.get("total").and_then(|value| value.as_u64());
    if matches!(response_shape, crate::OutputResponseShape::OneSentence) {
        let mut parts = vec![
            format!("path={label}"),
            format!("size.bytes={total_size}"),
            format!("size.human={compact}"),
        ];
        if let Some(total) = total_entries {
            parts.push(format!("count.total={total}"));
        }
        return Some(parts.join(" "));
    }
    let mut lines = vec![
        format!("path={label}"),
        format!("size.bytes={total_size}"),
        format!("size.human={compact}"),
    ];
    if let Some(total) = total_entries {
        lines.push(format!("count.total={total}"));
    }
    Some(lines.join("\n"))
}

fn output_has_count_inventory_total(output: &str) -> bool {
    let output = crate::agent_engine::observed_output::normalized_success_body_for_direct_answer(
        output.trim(),
    );
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return false;
    };
    if value.get("action").and_then(|value| value.as_str()) != Some("count_inventory") {
        return false;
    }
    value
        .get("counts")
        .and_then(|counts| counts.get("total"))
        .and_then(|value| value.as_u64())
        .is_some()
}

fn count_inventory_total_observation_count(loop_state: &crate::agent_engine::LoopState) -> usize {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .filter(|output| output_has_count_inventory_total(output))
        .count()
}

pub(super) fn inventory_ranked_size_list_answer(
    body: &str,
    route: &crate::RouteResult,
) -> Option<String> {
    if route.output_contract.response_shape != crate::OutputResponseShape::Strict {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|value| value.as_str()) != Some("inventory_dir") {
        return None;
    }
    let sort_by = value.get("sort_by").and_then(|value| value.as_str())?;
    if !matches!(sort_by, "size_desc" | "size_asc") {
        return None;
    }
    let mut entries = value
        .get("entries")
        .and_then(|value| value.as_array())?
        .iter()
        .filter(|entry| {
            entry
                .get("kind")
                .and_then(|value| value.as_str())
                .is_none_or(|kind| kind == "file")
        })
        .filter_map(|entry| {
            let name = entry
                .get("name")
                .or_else(|| entry.get("path"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())?;
            let size_bytes = entry.get("size_bytes").and_then(|value| value.as_u64())?;
            Some((name.to_string(), size_bytes))
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    if sort_by == "size_desc" {
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    } else {
        entries.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    }
    if let Some(limit) = route_list_selector_limit(route) {
        entries.truncate(limit.min(entries.len()));
    }
    Some(
        entries
            .into_iter()
            .map(|(name, size_bytes)| format!("{name} {size_bytes}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn route_list_selector_limit(route: &crate::RouteResult) -> Option<usize> {
    route
        .output_contract
        .self_extension
        .list_selector
        .limit
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}

#[cfg(test)]
pub(super) fn path_batch_size_comparison_answer(
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    path_batch_size_comparison_answer_with_style(
        body,
        prefer_english,
        SizeComparisonAnswerStyle::ExplainRatio,
    )
}

pub(super) fn direct_quantity_comparison_from_compare_paths(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route.output_contract_marker_is(crate::OutputSemanticKind::QuantityComparison)
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    if crate::agent_engine::observed_output::route_quantity_comparison_requires_model_language_synthesis(route)
        && latest_contractual_synthesis_output(loop_state).is_none()
    {
        return None;
    }
    let prefer_english =
        prefer_english_for_agent_contextual_user_text(state, user_text, agent_run_context);
    let style = size_comparison_answer_style(route, user_text);
    if count_inventory_total_observation_count(loop_state) >= 2 {
        return None;
    }
    let answer = {
        loop_state
            .executed_step_results
            .iter()
            .rev()
            .find_map(|step| {
                if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                    return None;
                }
                let output = step.output.as_deref()?;
                let output =
                    crate::agent_engine::observed_output::normalized_success_body_for_direct_answer(
                        output,
                    );
                if let Some(answer) = compare_paths_existence_verdict_answer(&output, route) {
                    return Some(answer);
                }
                if strict_quantity_comparison_json_fallback_allowed(user_text, style)
                    && route.output_contract.response_shape == crate::OutputResponseShape::Strict
                {
                    if let Some(answer) = strict_quantity_comparison_json_answer(&output) {
                        return Some(answer);
                    }
                }
                inventory_ranked_size_list_answer(&output, route)
                    .or_else(|| {
                        count_inventory_size_answer_with_shape(
                            &output,
                            prefer_english,
                            route.output_contract.response_shape,
                        )
                    })
                    .or_else(|| {
                        compare_paths_size_ratio_answer_with_style(&output, prefer_english, style)
                    })
                    .or_else(|| {
                        path_batch_size_comparison_answer_with_style(&output, prefer_english, style)
                    })
            })
    }?;
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

pub(super) fn direct_quantity_compare_paths_required_metadata_from_compare_paths(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        state,
        user_text,
        loop_state,
        agent_run_context,
    )?;
    compare_paths_required_metadata_answer(&answer).then_some((answer, summary))
}

pub(super) fn direct_compare_paths_required_metadata_from_observed_output(
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_allows_compare_paths_required_metadata_projection(route) {
        return None;
    }
    let answer = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?;
            let output =
                crate::agent_engine::observed_output::normalized_success_body_for_direct_answer(
                    output,
                );
            compare_paths_existence_verdict_answer_from_body(&output)
        })?;
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: loop_state.executed_step_results.len(),
            ..Default::default()
        },
    ))
}

fn compare_paths_required_metadata_answer(answer: &str) -> bool {
    let mut has_same_path = false;
    let mut has_left_exists = false;
    let mut has_right_exists = false;
    for line in answer.lines().map(str::trim) {
        if line.starts_with("same_path=") {
            has_same_path = true;
        } else if line.starts_with("left_exists=") {
            has_left_exists = true;
        } else if line.starts_with("right_exists=") {
            has_right_exists = true;
        }
    }
    has_same_path && has_left_exists && has_right_exists
}

fn route_allows_compare_paths_required_metadata_projection(route: &crate::RouteResult) -> bool {
    !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && crate::evidence_policy::required_evidence_fields_for_output_contract(
            &route.output_contract,
        )
        .iter()
        .any(|field| matches!(field.as_str(), "exists" | "kind"))
}

fn strict_quantity_comparison_json_fallback_allowed(
    user_text: &str,
    style: SizeComparisonAnswerStyle,
) -> bool {
    style != SizeComparisonAnswerStyle::DeltaOnly
        && crate::intent_router::contract_test_hint_value(user_text, "selector_answer_style")
            .is_none()
}

fn compare_paths_existence_verdict_answer(
    body: &str,
    route: &crate::RouteResult,
) -> Option<String> {
    if !route_allows_compare_paths_required_metadata_projection(route) {
        return None;
    }
    compare_paths_existence_verdict_answer_from_body(body)
}

fn compare_paths_existence_verdict_answer_from_body(body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(serde_json::Value::as_str) != Some("compare_paths") {
        return None;
    }
    let field_value = value.get("field_value").filter(|value| value.is_object());
    let same_path = field_value
        .and_then(|item| item.get("same_path"))
        .or_else(|| {
            value
                .get("comparison")
                .and_then(|item| item.get("same_path"))
        })
        .and_then(serde_json::Value::as_bool)?;
    let left = value.get("left")?;
    let right = value.get("right")?;
    let left_exists = field_value
        .and_then(|item| item.get("left_exists"))
        .or_else(|| left.get("exists"))
        .and_then(serde_json::Value::as_bool)?;
    let right_exists = field_value
        .and_then(|item| item.get("right_exists"))
        .or_else(|| right.get("exists"))
        .and_then(serde_json::Value::as_bool)?;
    let left_kind = left
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .unwrap_or("-");
    let right_kind = right
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .unwrap_or("-");
    Some(format!(
        "same_path={same_path}\nleft_exists={left_exists}\nleft_kind={left_kind}\nright_exists={right_exists}\nright_kind={right_kind}"
    ))
}

pub(super) fn replace_delivery_with_deterministic_quantity_comparison_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some((answer, summary)) = direct_quantity_comparison_from_compare_paths(
        state,
        user_text,
        loop_state,
        agent_run_context,
    ) else {
        return false;
    };
    if let Some(existing_answer) =
        latest_delivery_preserves_observed_quantity_size_facts(loop_state)
    {
        let strict_response = agent_run_context
            .and_then(|ctx| ctx.route_result.as_ref())
            .is_some_and(|route| {
                route.output_contract.response_shape == crate::OutputResponseShape::Strict
            });
        if strict_response {
            let facts = observed_quantity_size_facts(loop_state);
            if !structured_quantity_json_preserves_observed_size_facts(&existing_answer, &facts) {
                let preserve_model_language = agent_run_context
                    .and_then(|ctx| ctx.route_result.as_ref())
                    .is_some_and(|route| {
                        strict_quantity_comparison_should_preserve_model_language_delivery(
                            route,
                            loop_state,
                            &existing_answer,
                        )
                    });
                if preserve_model_language {
                    loop_state.last_user_visible_respond = Some(existing_answer);
                    *finalizer_summary = Some(summary);
                    return true;
                }
                // A prose answer can be factually grounded but still violate a strict machine
                // contract, so allow the deterministic strict answer below to replace it.
            } else {
                loop_state.last_user_visible_respond = Some(existing_answer);
                *finalizer_summary = Some(summary);
                return true;
            }
        } else {
            loop_state.last_user_visible_respond = Some(existing_answer);
            *finalizer_summary = Some(summary);
            return true;
        }
    }
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|message| message.trim() == answer.trim())
    {
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return true;
    }
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    log_deterministic_delivery_record(
        &task.task_id,
        "replace_with_deterministic_quantity_comparison",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn strict_quantity_comparison_should_preserve_model_language_delivery(
    route: &crate::RouteResult,
    loop_state: &LoopState,
    answer: &str,
) -> bool {
    if !route_allows_model_language_final_answer(route)
        || !planned_delivery_is_publishable_model_language_answer(answer)
    {
        return false;
    }
    route.output_contract.exact_sentence_count.is_some()
        || latest_plan_requested_synthesis(loop_state)
        || loop_state
            .last_publishable_synthesis_output
            .as_deref()
            .is_some_and(|synthesis| synthesis.trim() == answer.trim())
        || latest_successful_synthesis_output_matches(loop_state, answer)
}

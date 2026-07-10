use super::*;

pub(super) fn system_basic_info_scalar_path_candidate(value: &serde_json::Value) -> Option<String> {
    value
        .get("cwd")
        .or_else(|| value.get("workspace_root"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

pub(super) fn system_basic_value_looks_like_info(value: &serde_json::Value) -> bool {
    value
        .get("hostname")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some()
        && value
            .get("os")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .is_some()
}

fn system_basic_info_payload_from_value(value: &serde_json::Value) -> Option<serde_json::Value> {
    if system_basic_value_looks_like_info(value) {
        return Some(value.clone());
    }
    if let Some(extra) = value
        .get("extra")
        .filter(|extra| system_basic_value_looks_like_info(extra))
    {
        return Some(extra.clone());
    }
    None
}

pub(super) fn system_basic_info_value(skill: &str, body: &str) -> Option<serde_json::Value> {
    if skill != "system_basic" {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    system_basic_info_payload_from_value(&value)
}

pub(super) fn system_basic_existence_with_path_value(
    skill: &str,
    body: &str,
) -> Option<serde_json::Value> {
    if !matches!(skill, "system_basic" | "fs_basic") {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    matches!(
        value.get("action").and_then(|v| v.as_str()),
        Some("find_path" | "path_batch_facts" | "find_name")
    )
    .then_some(value)
}

pub(super) fn system_basic_inventory_dir_value(
    skill: &str,
    body: &str,
) -> Option<serde_json::Value> {
    if skill != "system_basic" {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    (value.get("action").and_then(|v| v.as_str()) == Some("inventory_dir")).then_some(value)
}

pub(super) fn system_basic_structured_doc_value(
    skill: &str,
    body: &str,
) -> Option<serde_json::Value> {
    if !matches!(skill, "system_basic" | "config_basic") {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    matches!(
        value.get("action").and_then(|v| v.as_str()),
        Some("extract_field" | "extract_fields" | "read_field" | "read_fields" | "structured_keys")
    )
    .then_some(value)
}

pub(super) fn system_basic_structured_doc_observed_body(skill: &str, body: &str) -> Option<String> {
    let value = system_basic_structured_doc_value(skill, body)?;
    match value.get("action").and_then(|v| v.as_str()) {
        Some("extract_field" | "read_field") => {
            let field_path = value
                .get("resolved_field_path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .or_else(|| {
                    value
                        .get("field_path")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                })
                .unwrap_or("requested field");
            Some(structured_field_display_line(
                None,
                field_path,
                value.get("value").unwrap_or(&serde_json::Value::Null),
                value.get("value_text").and_then(|v| v.as_str()),
                value
                    .get("exists")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                true,
            ))
        }
        Some("extract_fields" | "read_fields") => extract_fields_direct_answer_candidate(
            None,
            &value,
            Some(crate::OutputResponseShape::Free),
            true,
            true,
        )
        .or_else(|| Some(body.to_string())),
        Some("structured_keys") => Some(body.to_string()),
        _ => None,
    }
}

pub(super) fn inventory_dir_names(value: &serde_json::Value) -> Option<Vec<String>> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if action != "inventory_dir" {
        return None;
    }
    value
        .get("names")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
}

fn inventory_dir_names_by_kind(value: &serde_json::Value, kind: &str) -> Vec<String> {
    value
        .get("names_by_kind")
        .and_then(|v| v.get(kind))
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn inventory_dir_grouped_names_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    let files = inventory_dir_names_by_kind(value, "files");
    let dirs = inventory_dir_names_by_kind(value, "dirs");
    let other = inventory_dir_names_by_kind(value, "other");
    if files.is_empty() && dirs.is_empty() && other.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    let mut push_group = |title: &str, items: Vec<String>| {
        if items.is_empty() {
            return;
        }
        lines.push(format!("{title}:"));
        lines.extend(items.into_iter().map(|name| format!("- {name}")));
    };
    let dirs_title = observed_t(
        state,
        "clawd.msg.directory_group_dirs",
        "目录",
        "Directories",
        prefer_english,
    );
    let files_title = observed_t(
        state,
        "clawd.msg.directory_group_files",
        "文件",
        "Files",
        prefer_english,
    );
    let other_title = observed_t(
        state,
        "clawd.msg.directory_group_other",
        "其它",
        "Other",
        prefer_english,
    );
    push_group(&dirs_title, dirs);
    push_group(&files_title, files);
    push_group(&other_title, other);
    normalized_listing_text(&lines.join("\n"))
}

pub(super) fn inventory_dir_direct_answer_candidate(
    state: Option<&AppState>,
    route: Option<&crate::RouteResult>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if route.is_some_and(|route| {
        super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::DirectoryEntryGroups,
        )
    }) {
        return inventory_dir_grouped_names_candidate(state, value, prefer_english);
    }
    if route.is_some_and(|route| {
        super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::FileNames,
        )
    }) {
        let files = inventory_dir_names_by_kind(value, "files");
        if !files.is_empty() {
            return normalized_listing_text(&files.join("\n"));
        }
    }
    if route.is_some_and(|route| {
        super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::DirectoryNames,
        )
    }) {
        let dirs = inventory_dir_names_by_kind(value, "dirs");
        if !dirs.is_empty() {
            return normalized_listing_text(&dirs.join("\n"));
        }
    }
    if value
        .get("names_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let names = inventory_dir_names(value)?;
        return normalized_listing_text(&names.join("\n"));
    }
    if let Some(entries) = value.get("entries").and_then(|v| v.as_array()) {
        let lines = entries
            .iter()
            .filter_map(|entry| {
                let name = entry
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())?;
                let size = entry.get("size_bytes").and_then(|v| v.as_u64())?;
                Some(format!("{name} {size}"))
            })
            .collect::<Vec<_>>();
        if !lines.is_empty() {
            return normalized_listing_text(&lines.join("\n"));
        }
    }
    let names = inventory_dir_names(value)?;
    normalized_listing_text(&names.join("\n"))
}

fn tree_summary_display_name(entry: &serde_json::Value) -> Option<String> {
    entry
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            entry
                .get("path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .and_then(|path| Path::new(path).file_name().and_then(|name| name.to_str()))
                .map(ToOwned::to_owned)
        })
}

pub(super) fn tree_summary_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("tree_summary") {
        return None;
    }
    if let Some(answer) = tree_summary_rows_machine_answer(value) {
        return Some(answer);
    }
    let tree = value.get("tree")?;
    let children = tree.get("children").and_then(|v| v.as_array())?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut other = Vec::new();
    for child in children {
        let mut name = tree_summary_display_name(child)?;
        let kind = child
            .get("kind")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("");
        match kind {
            "dir" => {
                if !name.ends_with('/') {
                    name.push('/');
                }
                dirs.push(name);
            }
            "file" => files.push(name),
            _ => other.push(name),
        }
    }
    if dirs.is_empty() && files.is_empty() && other.is_empty() {
        return Some(observed_t(
            state,
            "clawd.msg.tree_summary_empty",
            "顶层为空",
            "Top level is empty",
            prefer_english,
        ));
    }
    let mut parts = Vec::new();
    if !dirs.is_empty() {
        parts.push(format!(
            "{} {}",
            observed_t(
                state,
                "clawd.msg.tree_summary_dirs",
                "目录",
                "directories",
                prefer_english,
            ),
            dirs.join(", ")
        ));
    }
    if !files.is_empty() {
        parts.push(format!(
            "{} {}",
            observed_t(
                state,
                "clawd.msg.tree_summary_files",
                "文件",
                "files",
                prefer_english,
            ),
            files.join(", ")
        ));
    }
    if !other.is_empty() {
        parts.push(format!(
            "{} {}",
            observed_t(
                state,
                "clawd.msg.tree_summary_other",
                "其它",
                "other",
                prefer_english,
            ),
            other.join(", ")
        ));
    }
    let prefix = observed_t(
        state,
        "clawd.msg.tree_summary_top_level",
        "顶层结构：",
        "Top level: ",
        prefer_english,
    );
    let separator = if prefer_english { "; " } else { "；" };
    let mut answer = format!("{prefix}{}", parts.join(separator));
    let truncated_nodes = value
        .get("truncated_nodes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let root_omitted = tree
        .get("omitted_children")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if truncated_nodes > 0 || root_omitted > 0 {
        let count = truncated_nodes.max(root_omitted).to_string();
        answer.push_str(&observed_t_with_vars(
            state,
            "clawd.msg.tree_summary_partial",
            "（另有 {count} 项未显示）",
            " ({count} more not shown)",
            prefer_english,
            &[("count", &count)],
        ));
    }
    Some(answer)
}

fn tree_summary_rows_machine_answer(value: &serde_json::Value) -> Option<String> {
    let rows = value
        .get("summary_rows")
        .or_else(|| value.get("candidates"))
        .or_else(|| value.get("results"))
        .and_then(|rows| rows.as_array())?;
    let lines = rows
        .iter()
        .filter_map(tree_summary_row_machine_line)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

fn tree_summary_row_machine_line(row: &serde_json::Value) -> Option<String> {
    if row.get("kind").and_then(|value| value.as_str()) != Some("dir") {
        return None;
    }
    let name = row
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            row.get("path")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .and_then(|path| Path::new(path).file_name().and_then(|name| name.to_str()))
        })?;
    let file_count = row.get("file_count").and_then(|value| value.as_u64())?;
    let truncated = row
        .get("truncated")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    Some(format!(
        "name={name} file_count={file_count} truncated={truncated}"
    ))
}

pub(super) fn dir_compare_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("dir_compare") {
        return None;
    }
    let counts = value.get("counts").and_then(|v| v.as_object())?;
    let left_only = counts
        .get("left_only")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let right_only = counts
        .get("right_only")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let kind_mismatches = counts
        .get("kind_mismatches")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if left_only == 0 && right_only == 0 && kind_mismatches == 0 {
        return Some(observed_t(
            state,
            "clawd.msg.dir_compare_no_diff",
            "未发现差异。",
            "No differences found.",
            prefer_english,
        ));
    }
    Some(if prefer_english {
        format!(
            "Differences found: left-only {left_only}, right-only {right_only}, kind mismatches {kind_mismatches}."
        )
    } else {
        format!("发现差异：左侧独有 {left_only} 项，右侧独有 {right_only} 项，类型不一致 {kind_mismatches} 项。")
    })
}

pub(super) fn inventory_dir_scalar_path_candidate(
    value: &serde_json::Value,
    prefer_full_path: bool,
) -> Option<String> {
    let names = inventory_dir_names(value)?;
    if !prefer_full_path {
        return Some(names.join("\n"));
    }
    let root = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let paths = names
        .into_iter()
        .map(|name| {
            let name_path = Path::new(&name);
            if name_path.is_absolute() {
                canonical_existing_path(name_path)
            } else if let Some(root) = root {
                let candidate = Path::new(root).join(&name);
                if candidate.exists() {
                    canonical_existing_path(&candidate)
                } else {
                    candidate.display().to_string()
                }
            } else {
                name
            }
        })
        .collect::<Vec<_>>();
    (!paths.is_empty()).then(|| paths.join("\n"))
}

fn compact_inventory_dir_kind_lines(entries: &[serde_json::Value]) -> Option<Vec<String>> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut others = Vec::new();

    for entry in entries {
        let entry = entry.as_object()?;
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())?;
        let mut label = name.to_string();
        if let Some(size_bytes) = entry.get("size_bytes").and_then(|v| v.as_u64()) {
            label.push_str(&format!(":size_bytes={size_bytes}"));
        }
        match entry
            .get("kind")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("other")
        {
            "dir" => dirs.push(label),
            "file" => files.push(label),
            _ => others.push(label),
        }
    }

    let mut lines = Vec::new();
    if !dirs.is_empty() {
        lines.push(format!("dir_entries={}", dirs.join(",")));
    }
    if !files.is_empty() {
        lines.push(format!("file_entries={}", files.join(",")));
    }
    if !others.is_empty() {
        lines.push(format!("other_entries={}", others.join(",")));
    }
    (!lines.is_empty()).then_some(lines)
}

fn inventory_dir_size_summary_lines(value: &serde_json::Value) -> Vec<String> {
    let Some(summary) = value.get("size_summary").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut lines = Vec::new();
    for key in ["matched_file_count", "total_file_size_bytes"] {
        if let Some(value) = summary.get(key).and_then(value_scalar_text) {
            lines.push(format!("size_summary.{key}={value}"));
        }
    }
    for key in ["largest_file", "smallest_file"] {
        let Some(file) = summary.get(key).and_then(|v| v.as_object()) else {
            continue;
        };
        let mut fields = Vec::new();
        for field in ["name", "path", "kind", "size_bytes", "modified_ts"] {
            if let Some(value) = file.get(field).and_then(value_scalar_text) {
                fields.push(format!("{field}={value}"));
            }
        }
        if !fields.is_empty() {
            lines.push(format!("size_summary.{key} {}", fields.join(" ")));
        }
    }
    lines
}

pub(super) fn inventory_dir_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let mut header = format!("inventory_dir path={path}");
    if let Some(sort_by) = value
        .get("sort_by")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        header.push_str(&format!(" sort_by={sort_by}"));
    }
    if let Some(counts) = value.get("counts").and_then(|v| v.as_object()) {
        for key in ["total", "files", "dirs", "hidden"] {
            if let Some(count) = counts.get(key).and_then(value_scalar_text) {
                header.push_str(&format!(" {key}={count}"));
            }
        }
    }
    let size_summary_lines = inventory_dir_size_summary_lines(value);
    if let Some(entries) = value.get("entries").and_then(|v| v.as_array()) {
        if entries.len() > 16 {
            if let Some(lines) = compact_inventory_dir_kind_lines(entries) {
                let lines = size_summary_lines
                    .into_iter()
                    .chain(lines)
                    .collect::<Vec<_>>();
                return Some(format!("{header}\n{}", lines.join("\n")));
            }
        }
        let mut lines = size_summary_lines.clone();
        lines.extend(
            entries
                .iter()
                .filter_map(|entry| {
                    let entry = entry.as_object()?;
                    let name = entry
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|v| !v.is_empty())?;
                    let kind = entry
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .unwrap_or("-");
                    let size = entry
                        .get("size_bytes")
                        .and_then(|v| v.as_u64())
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let modified = entry
                        .get("modified_ts")
                        .and_then(|v| v.as_i64())
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    Some(format!(
                        "entry name={name} kind={kind} size_bytes={size} modified_ts={modified}"
                    ))
                })
                .collect::<Vec<_>>(),
        );
        if !lines.is_empty() {
            return Some(format!("{header}\n{}", lines.join("\n")));
        }
    }
    let names = inventory_dir_names(value)?;
    let mut lines = size_summary_lines;
    lines.extend(names.into_iter().map(|name| format!("entry name={name}")));
    Some(format!("{header}\n{}", lines.join("\n")))
}

fn count_inventory_count_value(value: &serde_json::Value) -> Option<(String, &'static str)> {
    let counts = value.get("counts")?;
    let kind_filter = value
        .get("kind_filter")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase());
    let count_key = if value
        .get("files_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || matches!(
            kind_filter.as_deref(),
            Some("file" | "files" | "regular_file")
        ) {
        "files"
    } else if value
        .get("dirs_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || matches!(
            kind_filter.as_deref(),
            Some("dir" | "dirs" | "directory" | "directories")
        )
    {
        "dirs"
    } else {
        "total"
    };
    counts
        .get(count_key)
        .or_else(|| counts.get("total"))
        .and_then(value_scalar_text)
        .map(|count| (count, count_key))
}

pub(super) fn count_inventory_breakdown_value(
    value: &serde_json::Value,
) -> Option<(String, String, Option<String>)> {
    let counts = value.get("counts")?;
    let files = counts.get("files").and_then(value_scalar_text)?;
    let dirs = counts.get("dirs").and_then(value_scalar_text)?;
    let total = counts.get("total").and_then(value_scalar_text);
    Some((files, dirs, total))
}

pub(super) fn count_inventory_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
) -> Option<String> {
    let (count, count_key) = count_inventory_count_value(value)?;
    let has_component_breakdown = count_inventory_breakdown_value(value).is_some();
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(count);
    }
    if response_shape.is_none() && count_key == "total" && has_component_breakdown {
        return None;
    }
    let noun = match count_key {
        "files" => observed_t(
            state,
            "clawd.msg.count_inventory_noun_files",
            "普通文件",
            "regular files",
            prefer_english,
        ),
        "dirs" => observed_t(
            state,
            "clawd.msg.count_inventory_noun_dirs",
            "目录",
            "directories",
            prefer_english,
        ),
        _ => observed_t(
            state,
            "clawd.msg.count_inventory_noun_items",
            "项目",
            "items",
            prefer_english,
        ),
    };
    Some(observed_t_with_vars(
        state,
        "clawd.msg.count_inventory_direct_answer",
        "{count}，当前范围内共有 {count} 个{noun}。",
        "{count}: there are {count} {noun} in the requested scope.",
        prefer_english,
        &[("count", &count), ("noun", &noun)],
    ))
}

fn plan_requests_count_inventory_file_dir_breakdown(loop_state: &LoopState) -> bool {
    loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|trace| trace.plan_result.as_ref())
        .any(|plan| {
            plan.steps.iter().any(|step| {
                step.action_type == "call_skill"
                    && step.skill == "system_basic"
                    && step.args.get("action").and_then(|v| v.as_str()) == Some("count_inventory")
                    && step
                        .args
                        .get("count_files")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    && step
                        .args
                        .get("count_dirs")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
            })
        })
}

fn latest_count_inventory_file_dir_breakdown(
    loop_state: &LoopState,
) -> Option<(String, String, Option<String>)> {
    let idx = latest_successful_step_index(loop_state, |step| {
        step.skill == "system_basic"
            && step
                .output
                .as_deref()
                .and_then(|body| serde_json::from_str::<serde_json::Value>(body).ok())
                .is_some_and(|value| {
                    value.get("action").and_then(|v| v.as_str()) == Some("count_inventory")
                })
    })?;
    let body = loop_state.executed_step_results[idx].output.as_deref()?;
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|v| v.as_str()) != Some("count_inventory") {
        return None;
    }
    count_inventory_breakdown_value(&value)
}

pub(super) fn count_inventory_planned_file_dir_breakdown_answer(
    state: Option<&AppState>,
    loop_state: &LoopState,
    prefer_english: bool,
) -> Option<String> {
    if !plan_requests_count_inventory_file_dir_breakdown(loop_state) {
        return None;
    }
    let (files, dirs, _total) = latest_count_inventory_file_dir_breakdown(loop_state)?;
    Some(observed_t_with_vars(
        state,
        "clawd.msg.count_inventory_file_dir_breakdown",
        "文件：{files} 个\n文件夹：{dirs} 个",
        "Files: {files}\nDirectories: {dirs}",
        prefer_english,
        &[("files", &files), ("dirs", &dirs)],
    ))
}

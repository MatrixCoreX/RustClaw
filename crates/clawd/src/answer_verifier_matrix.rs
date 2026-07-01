use super::*;

pub(super) fn matrix_strict_list_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let candidate_items = strict_list_answer_items(candidate_answer);
    if candidate_items.is_empty() {
        return false;
    }
    if let Some(limit) = strict_list_selector_limit(route) {
        if candidate_items.len() > limit {
            return false;
        }
    }
    let observed_items = observed_strict_list_items(route, journal);
    if observed_items.is_empty() {
        return false;
    }
    let observed_variants = observed_items
        .iter()
        .flat_map(|item| strict_list_item_variants_for_route(route, item, true))
        .collect::<BTreeSet<_>>();
    let candidate_variants = candidate_items
        .iter()
        .flat_map(|item| strict_list_item_variants_for_route(route, item, false))
        .collect::<BTreeSet<_>>();
    let candidate_is_observed = candidate_items.iter().all(|item| {
        strict_list_item_variants_for_route(route, item, false)
            .iter()
            .any(|item| observed_variants.contains(item))
    });
    if !candidate_is_observed {
        return false;
    }
    if strict_list_route_allows_observed_subset(route) {
        return true;
    }
    observed_items.iter().all(|item| {
        strict_list_item_variants_for_route(route, item, true)
            .iter()
            .any(|item| candidate_variants.contains(item))
            || candidate_items
                .iter()
                .any(|candidate| strict_list_candidate_annotates_observed_item(candidate, item))
    })
}

fn strict_list_selector_limit(route: &RouteResult) -> Option<usize> {
    route
        .output_contract
        .self_extension
        .list_selector
        .limit
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}

pub(super) fn strict_list_route_allows_observed_subset(route: &RouteResult) -> bool {
    route_contract_marker_is(route, crate::OutputSemanticKind::FilePaths)
        || route_contract_marker_is(route, crate::OutputSemanticKind::DirectoryNames)
}

fn route_contract_marker_is(route: &RouteResult, semantic_kind: crate::OutputSemanticKind) -> bool {
    route.output_contract_marker_is(semantic_kind)
}

pub(super) fn route_contract_marker_is_scalar_path_only(route: &RouteResult) -> bool {
    route_contract_marker_is(route, crate::OutputSemanticKind::ScalarPathOnly)
}

fn route_contract_marker_is_service_status(route: &RouteResult) -> bool {
    route_contract_marker_is(route, crate::OutputSemanticKind::ServiceStatus)
}

fn route_contract_marker_is_archive_unpack(route: &RouteResult) -> bool {
    route_contract_marker_is(route, crate::OutputSemanticKind::ArchiveUnpack)
}

fn route_contract_marker_is_git_repository_state(route: &RouteResult) -> bool {
    route_contract_marker_is(route, crate::OutputSemanticKind::GitRepositoryState)
}

fn route_contract_marker_is_hidden_entries_check(route: &RouteResult) -> bool {
    route_contract_marker_is(route, crate::OutputSemanticKind::HiddenEntriesCheck)
}

fn route_contract_marker_is_directory_names(route: &RouteResult) -> bool {
    route_contract_marker_is(route, crate::OutputSemanticKind::DirectoryNames)
}

pub(super) fn matrix_table_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let candidate_cells = markdown_table_data_cells(candidate_answer);
    if candidate_cells.is_empty() {
        return false;
    }
    let observed_cells = observed_table_cells(route, journal);
    if observed_cells.is_empty() {
        return false;
    }
    candidate_cells.is_subset(&observed_cells) && observed_cells.is_subset(&candidate_cells)
}

pub(super) fn matrix_single_path_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if let Some(candidate_path) = strict_single_path_answer(candidate_answer) {
        return observed_single_path_values(route, journal)
            .iter()
            .any(|observed_path| single_path_matches_observed(&candidate_path, observed_path));
    }
    if !route_contract_marker_is_scalar_path_only(route) {
        return false;
    }
    let candidate_items = strict_list_answer_items(candidate_answer);
    if candidate_items.len() <= 1 {
        return false;
    }
    let observed_variants = observed_strict_list_items(route, journal)
        .iter()
        .flat_map(|item| strict_list_item_variants(item))
        .collect::<BTreeSet<_>>();
    !observed_variants.is_empty()
        && candidate_items.iter().all(|item| {
            strict_list_item_variants(item)
                .iter()
                .any(|item| observed_variants.contains(item))
        })
}

pub(super) fn matrix_delivery_artifact_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    route_requires_single_file_delivery(route)
        && (candidate_answer_has_grounded_existing_file_token(journal, candidate_answer)
            || candidate_answer_has_grounded_existing_plain_path(journal, candidate_answer))
}

pub(super) fn candidate_answer_has_grounded_existing_plain_path(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let Some(candidate_path) = strict_single_path_answer(candidate_answer) else {
        return false;
    };
    let Ok(canonical_candidate_path) = std::path::Path::new(&candidate_path).canonicalize() else {
        return false;
    };
    file_token_path_is_grounded_in_observations(journal, &canonical_candidate_path)
}

pub(super) fn archive_unpack_summary_answer_is_grounded_in_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route_contract_marker_is_archive_unpack(route) {
        return false;
    }
    let candidate_answer = candidate_answer.trim();
    if candidate_answer.is_empty() || candidate_answer.lines().count() > 1 {
        return false;
    }
    let observed_paths = observed_archive_unpack_destination_paths(route, journal);
    !observed_paths.is_empty()
        && observed_paths
            .iter()
            .any(|path| answer_mentions_observed_path(candidate_answer, path))
}

pub(super) fn git_repository_state_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route_contract_marker_is_git_repository_state(route) {
        return false;
    }
    let candidate = candidate_answer.trim();
    if candidate.is_empty() {
        return false;
    }
    journal.step_results.iter().any(|step| {
        step_can_supply_verifier_observation_for_route(route, step)
            && step.skill == "git_basic"
            && step.output_excerpt.as_deref().is_some_and(|output| {
                let Some(observed) =
                    crate::agent_engine::observed_output::git_repository_state_observation_from_status_output(
                        output,
                        None,
                    )
                else {
                    return false;
                };
                let worktree = if observed.dirty { "dirty" } else { "clean" };
                if !candidate.contains(&format!("git.worktree={worktree}")) {
                    return false;
                }
                if let Some(branch) = observed
                    .branch
                    .as_deref()
                    .filter(|branch| !branch.is_empty())
                {
                    if !candidate.contains(&format!("git.branch={branch}")) {
                        return false;
                    }
                }
                if route.output_contract.response_shape == crate::OutputResponseShape::OneSentence
                    || route.output_contract.exact_sentence_count == Some(1)
                {
                    return true;
                }
                if schema_field_value(candidate, "git.changed.count=")
                    .and_then(|value| value.parse::<usize>().ok())
                    != Some(observed.changed_entries.len())
                {
                    return false;
                }
                observed
                    .changed_entries
                    .iter()
                    .all(|entry| candidate.contains(entry.path.as_str()))
            })
    })
}

pub(super) fn schema_field_value<'a>(candidate: &'a str, prefix: &str) -> Option<&'a str> {
    candidate
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(prefix).map(str::trim))
        .or_else(|| {
            candidate
                .split_whitespace()
                .find_map(|part| part.strip_prefix(prefix).map(str::trim))
        })
        .filter(|value| !value.is_empty())
}

pub(super) fn service_status_port_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route_contract_marker_is_service_status(route) {
        return false;
    }
    let candidate_answer = candidate_answer.trim();
    if candidate_answer.is_empty() {
        return false;
    }
    let observed_ports = observed_service_status_ports(route, journal);
    if observed_ports.is_empty() {
        return false;
    }
    let candidate_ports = candidate_service_status_ports(candidate_answer);
    if candidate_ports.is_empty() {
        return false;
    }
    candidate_ports.is_subset(&observed_ports) && observed_ports.is_subset(&candidate_ports)
}

pub(super) fn observed_service_status_ports(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<u16> {
    let mut ports = BTreeSet::new();
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation_for_route(route, step) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        collect_keyed_port_numbers(output, &mut ports);
        collect_socket_suffix_ports(output, &mut ports);
    }
    ports
}

pub(super) fn candidate_service_status_ports(candidate_answer: &str) -> BTreeSet<u16> {
    let mut ports = BTreeSet::new();
    collect_keyed_port_numbers(candidate_answer, &mut ports);
    collect_markdown_table_first_cell_ports(candidate_answer, &mut ports);
    collect_socket_suffix_ports(candidate_answer, &mut ports);
    ports
}

pub(super) fn health_check_diagnostic_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route_contract_marker_is_service_status(route) {
        return false;
    }
    let candidate = candidate_answer.trim();
    if candidate.is_empty() {
        return false;
    }
    journal.step_results.iter().any(|step| {
        step_can_supply_verifier_observation_for_route(route, step)
            && step.skill == "health_check"
            && step.output_excerpt.as_deref().is_some_and(|output| {
                let Some(value) = health_check_value_from_output(output) else {
                    return false;
                };
                let tokens = health_check_diagnostic_expected_tokens(&value);
                tokens.len() > 1 && tokens.iter().all(|token| candidate.contains(token))
            })
    })
}

pub(super) fn health_check_value_from_output(output: &str) -> Option<serde_json::Value> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    if health_check_json_has_primary_fields(&value) {
        return Some(value);
    }
    if let Some(extra) = value.get("extra") {
        if health_check_json_has_primary_fields(extra) {
            return Some(extra.clone());
        }
    }
    None
}

pub(super) fn health_check_json_has_primary_fields(value: &serde_json::Value) -> bool {
    value.get("clawd_process_count").is_some()
        || value.get("clawd_health_port_open").is_some()
        || value.get("system_health").is_some()
}

pub(super) fn health_check_diagnostic_expected_tokens(value: &serde_json::Value) -> Vec<String> {
    let mut tokens = Vec::new();
    let process_count = value
        .get("clawd_process_count")
        .and_then(|field| field.as_i64());
    let port_open = value
        .get("clawd_health_port_open")
        .and_then(|field| field.as_bool());
    let status = match (process_count, port_open) {
        (Some(count), Some(true)) if count > 0 => "clawd.status=running",
        (Some(count), _) if count <= 0 => "clawd.status=not_running",
        (Some(_), Some(false)) => "clawd.status=degraded",
        (None, Some(true)) => "clawd.status=reachable",
        _ => "clawd.status=unknown",
    };
    tokens.push(status.to_string());
    if let Some(count) = process_count {
        tokens.push(format!("clawd_process_count={count}"));
    }
    if let Some(open) = port_open {
        tokens.push(format!("clawd_health_port_open={open}"));
    }
    if let Some(count) = value
        .get("telegramd_process_count")
        .and_then(|field| field.as_i64())
    {
        tokens.push(format!("telegramd_process_count={count}"));
    }
    collect_health_check_log_tokens(value, "clawd_log", &mut tokens);
    collect_health_check_log_tokens(value, "nni_log", &mut tokens);
    collect_health_check_log_tokens(value, "nni_server_log", &mut tokens);
    collect_health_check_log_tokens(value, "telegramd_log", &mut tokens);
    collect_health_check_system_tokens(value, &mut tokens);
    tokens
}

pub(super) fn collect_health_check_log_tokens(
    value: &serde_json::Value,
    field_name: &str,
    tokens: &mut Vec<String>,
) {
    let Some(log) = value.get(field_name).and_then(|field| field.as_object()) else {
        return;
    };
    if let Some(exists) = log.get("exists").and_then(|field| field.as_bool()) {
        tokens.push(format!("{field_name}.exists={exists}"));
    }
    if let Some(count) = log
        .get("keyword_error_count")
        .and_then(|field| field.as_i64())
    {
        tokens.push(format!("{field_name}.keyword_error_count={count}"));
    }
}

pub(super) fn collect_health_check_system_tokens(
    value: &serde_json::Value,
    tokens: &mut Vec<String>,
) {
    let Some(system) = value
        .get("system_health")
        .and_then(|field| field.as_object())
    else {
        return;
    };
    for key in [
        "os_family",
        "service_manager",
        "cpu_count",
        "load_avg_1m",
        "load_avg_5m",
        "load_avg_15m",
        "memory_available_bytes",
        "memory_total_bytes",
        "disk_root_available_bytes",
        "disk_root_total_bytes",
        "uptime_seconds",
    ] {
        let Some(field) = system.get(key) else {
            continue;
        };
        if let Some(text) = health_check_scalar_token_value(field) {
            tokens.push(format!("system_health.{key}={text}"));
        }
    }
    if let Some(warnings) = system
        .get("warnings")
        .and_then(|field| field.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::trim))
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
    {
        tokens.push(format!("system_health.warnings={}", warnings.join(",")));
    }
}

pub(super) fn health_check_scalar_token_value(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .or_else(|| value.as_i64().map(|number| number.to_string()))
        .or_else(|| value.as_u64().map(|number| number.to_string()))
        .or_else(|| value.as_f64().map(|number| number.to_string()))
        .or_else(|| value.as_bool().map(|flag| flag.to_string()))
}

pub(super) fn collect_keyed_port_numbers(text: &str, ports: &mut BTreeSet<u16>) {
    for line in text.lines() {
        let line = line.trim();
        if !line.to_ascii_lowercase().contains("port") {
            continue;
        }
        for marker in [".number=", "number=", "port="] {
            let Some((_, tail)) = line.split_once(marker) else {
                continue;
            };
            if let Some(port) = parse_leading_port_number(tail) {
                ports.insert(port);
            }
        }
    }
}

pub(super) fn collect_markdown_table_first_cell_ports(text: &str, ports: &mut BTreeSet<u16>) {
    let rows = text
        .lines()
        .map(markdown_table_row_cells)
        .filter(|cells| !cells.is_empty())
        .collect::<Vec<_>>();
    if rows.len() < 3 || !markdown_table_separator_row(&rows[1]) {
        return;
    }
    for row in rows.iter().skip(2) {
        let Some(first_cell) = row.first() else {
            continue;
        };
        if let Some(port) = parse_port_cell(first_cell) {
            ports.insert(port);
        }
    }
}

pub(super) fn collect_socket_suffix_ports(text: &str, ports: &mut BTreeSet<u16>) {
    let bytes = text.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] != b':' {
            idx += 1;
            continue;
        }
        let digit_start = idx + 1;
        if digit_start >= bytes.len() || !bytes[digit_start].is_ascii_digit() {
            idx += 1;
            continue;
        }
        let mut digit_end = digit_start;
        while digit_end < bytes.len() && bytes[digit_end].is_ascii_digit() {
            digit_end += 1;
        }
        if digit_end - digit_start <= 5 && socket_port_boundary(bytes.get(digit_end).copied()) {
            if let Ok(port) = text[digit_start..digit_end].parse::<u16>() {
                if port > 0 {
                    ports.insert(port);
                }
            }
        }
        idx = digit_end;
    }
}

pub(super) fn socket_port_boundary(next: Option<u8>) -> bool {
    next.is_none_or(|ch| !ch.is_ascii_alphanumeric() && !matches!(ch, b'.' | b'_'))
}

pub(super) fn parse_leading_port_number(raw: &str) -> Option<u16> {
    let digits = raw
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    parse_port_number(&digits)
}

pub(super) fn parse_port_cell(raw: &str) -> Option<u16> {
    let normalized = raw
        .trim()
        .trim_matches('`')
        .trim_matches('*')
        .trim()
        .to_string();
    parse_port_number(&normalized)
}

pub(super) fn parse_port_number(raw: &str) -> Option<u16> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 5 || !raw.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    raw.parse::<u16>().ok().filter(|port| *port > 0)
}

pub(super) fn observed_archive_unpack_destination_paths(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut paths = observed_single_path_values_from_evidence_map_for_route(route, journal);
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation_for_route(route, step) {
            continue;
        }
        if !step_can_supply_strict_evidence_for_route(route, step) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        collect_archive_unpack_destination_paths_from_output(output, &mut paths);
    }
    paths
}

pub(super) fn collect_archive_unpack_destination_paths_from_output(
    output: &str,
    paths: &mut BTreeSet<String>,
) {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) {
        collect_archive_unpack_destination_paths_from_json(&value, paths);
        return;
    }
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if archive_unpack_destination_key(key.trim()) {
            let value = value.trim();
            if !value.is_empty() {
                paths.insert(value.to_string());
            }
        }
    }
}

pub(super) fn collect_archive_unpack_destination_paths_from_json(
    value: &serde_json::Value,
    paths: &mut BTreeSet<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if archive_unpack_destination_key(key) {
                    if let Some(path) = child
                        .as_str()
                        .map(str::trim)
                        .filter(|path| !path.is_empty())
                    {
                        paths.insert(path.to_string());
                    }
                }
                collect_archive_unpack_destination_paths_from_json(child, paths);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_archive_unpack_destination_paths_from_json(item, paths);
            }
        }
        _ => {}
    }
}

pub(super) fn archive_unpack_destination_key(key: &str) -> bool {
    matches!(
        key,
        "dest" | "dest_path" | "destination" | "destination_path" | "path"
    )
}

pub(super) fn answer_mentions_observed_path(answer: &str, observed_path: &str) -> bool {
    let observed_path = observed_path.trim();
    if observed_path.is_empty() {
        return false;
    }
    if answer.contains(observed_path) {
        return true;
    }
    answer.split_whitespace().any(|token| {
        let token = token
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '`' | '"'
                        | '\''
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | ','
                        | '，'
                        | '.'
                        | '。'
                        | ';'
                        | '；'
                        | ':'
                        | '：'
                )
            })
            .trim();
        !token.is_empty() && single_path_matches_observed(token, observed_path)
    })
}

pub(super) fn strict_single_path_answer(answer: &str) -> Option<String> {
    let answer = answer.trim();
    if answer.is_empty() || answer.lines().count() > 1 {
        return None;
    }
    let lower = answer.to_ascii_lowercase();
    if lower.starts_with("file:")
        || answer.contains(':')
        || answer.contains('：')
        || answer.ends_with('.')
        || answer.ends_with('。')
    {
        return None;
    }
    Some(answer.to_string())
}

pub(super) fn observed_single_path_values(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut paths = observed_single_path_values_from_evidence_map_for_route(route, journal);
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation_for_route(route, step) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) {
            collect_single_path_values_from_json(&value, &mut paths);
        } else if let Some(path) = strict_single_path_answer(output) {
            paths.insert(path);
        }
    }
    paths
}

pub(super) fn collect_single_path_values_from_json(
    value: &serde_json::Value,
    paths: &mut BTreeSet<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            collect_joined_path_values_from_json_object(map, paths);
            for (key, child) in map {
                if single_path_evidence_key(key) {
                    if let Some(path) = child
                        .as_str()
                        .map(str::trim)
                        .filter(|path| !path.is_empty())
                    {
                        paths.insert(path.to_string());
                    }
                }
                collect_single_path_values_from_json(child, paths);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_single_path_values_from_json(item, paths);
            }
        }
        _ => {}
    }
}

pub(super) fn collect_joined_path_values_from_json_object(
    map: &serde_json::Map<String, serde_json::Value>,
    paths: &mut BTreeSet<String>,
) {
    let Some(root) = map
        .get("resolved_path")
        .or_else(|| map.get("root"))
        .or_else(|| map.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    for key in ["results", "names", "paths", "files"] {
        let Some(items) = map.get(key).and_then(|value| value.as_array()) else {
            continue;
        };
        for item in items {
            let Some(child) = item
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let child_path = std::path::Path::new(child);
            if child_path.is_absolute() || joined_result_already_contains_root(root, child) {
                paths.insert(child.to_string());
            } else {
                paths.insert(
                    std::path::Path::new(root)
                        .join(child_path)
                        .display()
                        .to_string(),
                );
            }
        }
    }
}

pub(super) fn joined_result_already_contains_root(root: &str, child: &str) -> bool {
    let root = root.trim().trim_matches('/');
    if root.is_empty() || root == "." {
        return true;
    }
    let child = child.trim().trim_start_matches("./");
    child == root || child.starts_with(&format!("{root}/"))
}

pub(super) fn single_path_evidence_key(key: &str) -> bool {
    matches!(
        key,
        "path"
            | "resolved_path"
            | "cwd"
            | "current_dir"
            | "working_directory"
            | "workspace_root"
            | "root"
            | "archive_path"
            | "output_path"
            | "created_path"
            | "destination_path"
            | "target_path"
            | "file_path"
            | "result_path"
    )
}

pub(super) fn single_path_matches_observed(candidate_path: &str, observed_path: &str) -> bool {
    let candidate_path = candidate_path.trim();
    let observed_path = observed_path.trim();
    if candidate_path.is_empty() || observed_path.is_empty() {
        return false;
    }
    if candidate_path == observed_path {
        return true;
    }
    let candidate = std::path::Path::new(candidate_path);
    let observed = std::path::Path::new(observed_path);
    if candidate.canonicalize().is_ok_and(|candidate| {
        observed
            .canonicalize()
            .is_ok_and(|observed| candidate == observed)
    }) {
        return true;
    }
    std::env::current_dir().ok().is_some_and(|dir| {
        dir.join(candidate).canonicalize().is_ok_and(|candidate| {
            observed
                .canonicalize()
                .is_ok_and(|observed| candidate == observed)
        })
    })
}

pub(super) fn markdown_table_data_cells(answer: &str) -> BTreeSet<String> {
    let rows = answer
        .lines()
        .map(markdown_table_row_cells)
        .filter(|cells| !cells.is_empty())
        .collect::<Vec<_>>();
    if rows.len() < 3 || !markdown_table_separator_row(&rows[1]) {
        return BTreeSet::new();
    }
    let mut cells = BTreeSet::new();
    for row in rows.iter().skip(2) {
        for cell in row {
            let normalized = normalize_strict_list_item(cell);
            if !normalized.is_empty() {
                cells.insert(normalized);
            }
        }
    }
    cells
}

pub(super) fn markdown_table_row_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return Vec::new();
    }
    trimmed
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn markdown_table_separator_row(cells: &[String]) -> bool {
    cells.iter().all(|cell| {
        let value = cell.trim();
        value.len() >= 3
            && value.chars().all(|ch| matches!(ch, '-' | ':' | ' ' | '\t'))
            && value.chars().any(|ch| ch == '-')
    })
}

pub(super) fn observed_table_cells(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut cells = observed_table_cells_from_evidence_map_for_route(route, journal);
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation_for_route(route, step) {
            continue;
        }
        if !step_can_supply_strict_evidence_for_route(route, step) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
            continue;
        };
        collect_observed_table_cells_from_value(&value, &mut cells);
    }
    cells
}

pub(super) fn collect_observed_table_cells_from_value(
    value: &serde_json::Value,
    cells: &mut BTreeSet<String>,
) {
    if let Some(rows) = value.get("rows").and_then(|value| value.as_array()) {
        collect_observed_table_cells_from_rows(rows, cells);
    }
    if let Some(rows) = value
        .pointer("/result/rows")
        .and_then(|value| value.as_array())
    {
        collect_observed_table_cells_from_rows(rows, cells);
    }
}

pub(super) fn collect_observed_table_cells_from_rows(
    rows: &[serde_json::Value],
    cells: &mut BTreeSet<String>,
) {
    for row in rows {
        match row {
            serde_json::Value::Object(map) => {
                for value in map.values() {
                    push_observed_table_cell(value, cells);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    push_observed_table_cell(value, cells);
                }
            }
            value => push_observed_table_cell(value, cells),
        }
    }
}

pub(super) fn push_observed_table_cell(value: &serde_json::Value, cells: &mut BTreeSet<String>) {
    let text = match value {
        serde_json::Value::String(value) => value.trim().to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        _ => String::new(),
    };
    let normalized = normalize_strict_list_item(&text);
    if !normalized.is_empty() {
        cells.insert(normalized);
    }
}

pub(super) fn strict_list_answer_items(answer: &str) -> Vec<String> {
    let mut items = Vec::new();
    for line in answer.lines() {
        let line = strip_list_marker(line);
        if line.is_empty() || line.ends_with(':') || line.ends_with('：') {
            continue;
        }
        let segments = line
            .split([',', '，'])
            .map(strip_list_marker)
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>();
        items.extend(segments);
    }
    items.sort_by_key(|item| item.to_ascii_lowercase());
    items.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    items
}

pub(super) fn strip_list_marker(raw: &str) -> String {
    let mut value = raw
        .trim()
        .trim_matches('`')
        .trim()
        .trim_start_matches(['-', '*', '•'])
        .trim()
        .to_string();
    if let Some((prefix, rest)) = value.split_once('.') {
        if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
            value = rest.trim().to_string();
        }
    }
    value.trim_matches('`').trim().to_string()
}

pub(super) fn strict_list_item_variants_for_route(
    route: &RouteResult,
    item: &str,
    observed_item: bool,
) -> Vec<String> {
    let mut variants = strict_list_item_variants(item);
    if observed_item && route_contract_marker_is_directory_names(route) {
        variants.extend(strict_list_parent_directory_variants(item));
    }
    variants.sort();
    variants.dedup();
    variants
}

pub(super) fn strict_list_item_variants(item: &str) -> Vec<String> {
    let normalized = normalize_strict_list_item(item);
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut variants = vec![normalized.clone()];
    if let Some(file_name) = std::path::Path::new(&normalized)
        .file_name()
        .and_then(|value| value.to_str())
        .map(normalize_strict_list_item)
        .filter(|value| !value.is_empty() && value != &normalized)
    {
        variants.push(file_name);
    }
    variants.sort();
    variants.dedup();
    variants
}

pub(super) fn strict_list_parent_directory_variants(item: &str) -> Vec<String> {
    let normalized = normalize_strict_list_item(item);
    if normalized.is_empty() {
        return Vec::new();
    }
    let path = std::path::Path::new(&normalized);
    let parent = path
        .parent()
        .map(|value| {
            let text = value.to_string_lossy();
            if text.is_empty() {
                ".".to_string()
            } else {
                text.to_string()
            }
        })
        .unwrap_or_else(|| ".".to_string());
    vec![normalize_strict_list_item(&parent)]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect()
}

pub(super) fn strict_list_candidate_annotates_observed_item(
    candidate: &str,
    observed: &str,
) -> bool {
    let candidate = normalize_strict_list_item(candidate);
    let observed = normalize_strict_list_item(observed);
    !candidate.is_empty()
        && !observed.is_empty()
        && candidate.len() > observed.len()
        && candidate.starts_with(&observed)
        && candidate[observed.len()..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
}

pub(super) fn normalize_strict_list_item(item: &str) -> String {
    item.trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_ascii_lowercase()
}

pub(super) fn observed_strict_list_items(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> Vec<String> {
    let mut items = observed_strict_list_items_from_evidence_map_for_route(route, journal);
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation_for_route(route, step) {
            continue;
        }
        if !step_can_supply_strict_evidence_for_route(route, step) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        if route_contract_marker_is_hidden_entries_check(route) {
            collect_observed_hidden_entries_from_output(output, &mut items);
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
            continue;
        };
        collect_observed_strict_list_items_from_value(&value, &mut items);
    }
    items.into_iter().collect()
}

pub(super) fn collect_observed_hidden_entries_from_output(
    output: &str,
    items: &mut BTreeSet<String>,
) {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) {
        collect_observed_hidden_entries_from_value(&value, items);
        return;
    }
    for line in output.lines() {
        if let Some(entry) = observed_hidden_entry_name(line) {
            push_observed_list_item(&entry, items);
        }
    }
}

pub(super) fn collect_observed_hidden_entries_from_value(
    value: &serde_json::Value,
    items: &mut BTreeSet<String>,
) {
    if let Some(entries) = value.get("entries").and_then(|value| value.as_array()) {
        for entry in entries {
            let Some(map) = entry.as_object() else {
                continue;
            };
            if !map
                .get("hidden")
                .and_then(|value| value.as_bool())
                .unwrap_or_else(|| {
                    map.get("name")
                        .or_else(|| map.get("path"))
                        .and_then(|value| value.as_str())
                        .and_then(observed_hidden_entry_name)
                        .is_some()
                })
            {
                continue;
            }
            for key in ["name", "path", "resolved_path"] {
                if let Some(entry) = map
                    .get(key)
                    .and_then(|value| value.as_str())
                    .and_then(observed_hidden_entry_name)
                {
                    push_observed_list_item(&entry, items);
                    break;
                }
            }
        }
    }
    for key in ["names", "paths", "files", "dirs", "directories", "results"] {
        if let Some(values) = value.get(key).and_then(|value| value.as_array()) {
            for item in values {
                if let Some(entry) = item.as_str().and_then(observed_hidden_entry_name) {
                    push_observed_list_item(&entry, items);
                } else {
                    collect_observed_hidden_entries_from_value(item, items);
                }
            }
        }
    }
}

pub(super) fn observed_hidden_entry_name(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() || value == "." || value == ".." {
        return None;
    }
    let name = std::path::Path::new(value)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(value);
    (name.starts_with('.') && name != "." && name != "..").then(|| value.to_string())
}

pub(super) fn collect_observed_strict_list_items_from_value(
    value: &serde_json::Value,
    items: &mut BTreeSet<String>,
) {
    push_string_array_values(
        value,
        items,
        &[
            "keys",
            "identity_values",
            "names",
            "paths",
            "files",
            "dirs",
            "directories",
            "results",
            "tables",
        ],
    );
    if let Some(names_by_kind) = value
        .get("names_by_kind")
        .and_then(|value| value.as_object())
    {
        for child in names_by_kind.values() {
            push_array_strings(child, items);
        }
    }
    for key in ["entries", "items", "facts", "matches", "rows"] {
        if let Some(array) = value.get(key).and_then(|value| value.as_array()) {
            for item in array {
                collect_observed_list_item_object_fields(item, items);
            }
        }
    }
}

pub(super) fn push_string_array_values(
    value: &serde_json::Value,
    items: &mut BTreeSet<String>,
    keys: &[&str],
) {
    for key in keys {
        if let Some(child) = value.get(*key) {
            push_array_strings(child, items);
        }
    }
}

pub(super) fn push_array_strings(value: &serde_json::Value, items: &mut BTreeSet<String>) {
    let Some(array) = value.as_array() else {
        return;
    };
    for item in array {
        if let Some(text) = item.as_str() {
            push_observed_list_item(text, items);
        } else {
            collect_observed_list_item_object_fields(item, items);
        }
    }
}

pub(super) fn collect_observed_list_item_object_fields(
    value: &serde_json::Value,
    items: &mut BTreeSet<String>,
) {
    let Some(map) = value.as_object() else {
        return;
    };
    if let Some(with_size) = observed_name_size_item(map) {
        push_observed_list_item(&with_size, items);
    }
    for key in [
        "name",
        "path",
        "resolved_path",
        "table",
        "table_name",
        "identity_value",
    ] {
        if let Some(text) = map.get(key).and_then(|value| value.as_str()) {
            push_observed_list_item(text, items);
        }
    }
}

pub(super) fn observed_name_size_item(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let name = map
        .get("name")
        .or_else(|| map.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let size = map
        .get("size_bytes")
        .or_else(|| map.get("size"))
        .and_then(|value| match value {
            serde_json::Value::Number(value) => Some(value.to_string()),
            serde_json::Value::String(value) => Some(value.trim().to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty())?;
    Some(format!("{name} {size}"))
}

pub(super) fn push_observed_list_item(text: &str, items: &mut BTreeSet<String>) {
    let item = normalize_strict_list_item(text);
    if !item.is_empty() {
        items.insert(item);
    }
}

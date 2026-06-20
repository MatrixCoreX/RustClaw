use std::path::Path;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{
    current_delivery_is_latest_publishable_synthesis, log_deterministic_delivery_record,
    path_batch_size_facts, step_output_is_read_range, structured_json_values_from_output,
    truncate_with_ellipsis,
};

pub(super) fn direct_directory_purpose_summary_from_size_facts(
    _state: &AppState,
    _user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryPurposeSummary
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let mut facts = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?;
            let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
            path_batch_size_facts(&value)
        })?;
    facts.sort_by(|a, b| {
        b.size_bytes
            .cmp(&a.size_bytes)
            .then_with(|| a.label.cmp(&b.label))
    });
    let largest = facts.first()?;
    let dir_label = route
        .output_contract
        .locator_hint
        .trim()
        .trim_end_matches(['/', '\\'])
        .trim()
        .to_string();
    let dir_label = if dir_label.is_empty() {
        ".".to_string()
    } else {
        dir_label
    };
    let subject = schema_subject_from_path_label(&largest.label);
    let file_count = facts.len() as u64;
    let mut fields = vec![
        format!("directory={dir_label}; file.count={file_count}"),
        format!(
            "largest.path={}; largest.size_bytes={}",
            largest.label, largest.size_bytes
        ),
        format!("schema.subject={subject}"),
    ];
    if let Some(summary) = latest_directory_purpose_synthesis_excerpt(
        loop_state,
        largest.label.as_str(),
        Some(largest.size_bytes),
    ) {
        fields.push(format!("directory_purpose_summary={summary}"));
    }
    let answer = fields.join("\n");
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

#[derive(Clone, Debug)]
struct InventoryDocumentFile {
    name: String,
    path: String,
    size_bytes: Option<u64>,
}

#[derive(Clone, Debug)]
struct ReadRangeObservation {
    path: String,
    excerpt: String,
}

#[derive(Clone, Debug)]
struct RecentArtifactEntry {
    name: String,
    path: String,
    kind: String,
    modified_ts: Option<i64>,
    size_bytes: Option<u64>,
}

#[derive(Clone, Debug)]
struct RecentArtifactInventory {
    root: String,
    sort_by: Option<String>,
    entries: Vec<RecentArtifactEntry>,
    auxiliary_paths: Vec<String>,
}

pub(super) fn direct_recent_artifacts_judgment_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::RecentArtifactsJudgment
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let mut inventory = latest_recent_artifact_inventory(loop_state)?;
    apply_recent_artifacts_selector_target_kind(route, &mut inventory);
    if inventory.entries.is_empty() {
        return None;
    }
    if let Some(limit) = recent_artifacts_selector_limit(route) {
        inventory.entries.truncate(limit);
    }
    let classification = classify_recent_artifact_inventory(&inventory);
    let mut fields = vec![
        format!("recent_entries.count={}", inventory.entries.len()),
        format!("recent_entries.root={}", inventory.root),
    ];
    if let Some(sort_by) = inventory.sort_by.as_deref() {
        fields.push(format!("recent_entries.sort_by={sort_by}"));
    }
    for (index, entry) in inventory.entries.iter().enumerate() {
        let entry_classification = classify_recent_artifact_entry(&inventory, entry);
        let prefix = format!("recent_entries[{index}]");
        fields.push(format!("{prefix}.name={}", entry.name));
        fields.push(format!("{prefix}.kind={}", entry.kind));
        fields.push(format!("{prefix}.path={}", entry.path));
        if let Some(modified_ts) = entry.modified_ts {
            fields.push(format!("{prefix}.modified_ts={modified_ts}"));
        }
        if let Some(size_bytes) = entry.size_bytes {
            fields.push(format!("{prefix}.size_bytes={size_bytes}"));
        }
        fields.push(format!(
            "{prefix}.classification={}",
            entry_classification.label
        ));
        fields.push(format!(
            "{prefix}.business_data={}",
            entry_classification.business_data
        ));
        if !entry_classification.basis_tokens.is_empty() {
            fields.push(format!(
                "{prefix}.classification_basis_tokens={}",
                entry_classification.basis_tokens.join(",")
            ));
        }
    }
    fields.push("classification.output_format=per_entry".to_string());
    fields.push(format!("classification={}", classification.label));
    fields.push(format!(
        "classification.formal_config={}",
        classification.formal_config
    ));
    fields.push(format!(
        "classification.business_data={}",
        classification.business_data
    ));
    if !classification.basis_tokens.is_empty() {
        fields.push(format!(
            "classification.basis_tokens={}",
            classification.basis_tokens.join(",")
        ));
    }
    Some((
        fields.join("\n"),
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

fn latest_recent_artifact_inventory(loop_state: &LoopState) -> Option<RecentArtifactInventory> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .flat_map(structured_json_values_from_output)
        .find_map(|value| recent_artifact_inventory_from_value(&value))
}

fn recent_artifacts_selector_limit(route: &crate::RouteResult) -> Option<usize> {
    route
        .output_contract
        .self_extension
        .list_selector
        .limit
        .and_then(|limit| usize::try_from(limit).ok())
        .filter(|limit| *limit > 0)
        .or_else(|| selector_limit_machine_token(route.resolved_intent.as_str()))
        .or_else(|| selector_limit_machine_token(route.route_reason.as_str()))
}

fn apply_recent_artifacts_selector_target_kind(
    route: &crate::RouteResult,
    inventory: &mut RecentArtifactInventory,
) {
    match recent_artifacts_selector_target_kind(route) {
        crate::OutputScalarCountTargetKind::Any => {}
        crate::OutputScalarCountTargetKind::File => {
            inventory.entries.retain(|entry| entry.kind == "file");
        }
        crate::OutputScalarCountTargetKind::Dir => {
            inventory
                .entries
                .retain(|entry| matches!(entry.kind.as_str(), "dir" | "directory"));
        }
    }
}

fn recent_artifacts_selector_target_kind(
    route: &crate::RouteResult,
) -> crate::OutputScalarCountTargetKind {
    let selector = &route.output_contract.self_extension.list_selector;
    if selector.target_kind_specified {
        return selector.target_kind;
    }
    selector_target_kind_machine_token(route.resolved_intent.as_str())
        .or_else(|| selector_target_kind_machine_token(route.route_reason.as_str()))
        .unwrap_or_default()
}

fn selector_target_kind_machine_token(text: &str) -> Option<crate::OutputScalarCountTargetKind> {
    text.split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | ')' | '('))
        .filter_map(|part| part.trim().strip_prefix("selector_target_kind="))
        .find_map(|raw| match raw.trim() {
            "file" => Some(crate::OutputScalarCountTargetKind::File),
            "dir" => Some(crate::OutputScalarCountTargetKind::Dir),
            "any" => Some(crate::OutputScalarCountTargetKind::Any),
            _ => None,
        })
}

fn selector_limit_machine_token(text: &str) -> Option<usize> {
    text.split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | ')' | '('))
        .filter_map(|part| part.trim().strip_prefix("selector_limit="))
        .filter_map(|raw| raw.trim().parse::<usize>().ok())
        .find(|limit| *limit > 0)
}

fn recent_artifact_inventory_from_value(
    value: &serde_json::Value,
) -> Option<RecentArtifactInventory> {
    if value.get("action").and_then(serde_json::Value::as_str) != Some("inventory_dir") {
        return None;
    }
    let entries = value
        .get("entries")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .filter_map(recent_artifact_entry_from_value)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    let root = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or(".")
        .to_string();
    let sort_by = value
        .get("sort_by")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|sort_by| !sort_by.is_empty())
        .map(ToOwned::to_owned);
    let mut auxiliary_paths = Vec::new();
    for pointer in [
        "/size_summary/largest_file/path",
        "/size_summary/smallest_file/path",
    ] {
        if let Some(path) = value
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            push_unique_recent_artifact_token(path, &mut auxiliary_paths);
        }
    }
    Some(RecentArtifactInventory {
        root,
        sort_by,
        entries,
        auxiliary_paths,
    })
}

fn recent_artifact_entry_from_value(value: &serde_json::Value) -> Option<RecentArtifactEntry> {
    let name = value
        .get("name")
        .or_else(|| value.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())?
        .to_string();
    let path = value
        .get("path")
        .or_else(|| value.get("resolved_path"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or(name.as_str())
        .to_string();
    let kind = value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .unwrap_or("unknown")
        .to_string();
    Some(RecentArtifactEntry {
        name,
        path,
        kind,
        modified_ts: value
            .get("modified_ts")
            .or_else(|| value.get("mtime"))
            .and_then(serde_json::Value::as_i64),
        size_bytes: value
            .get("size_bytes")
            .or_else(|| value.get("size"))
            .and_then(serde_json::Value::as_u64),
    })
}

#[derive(Clone, Debug)]
struct RecentArtifactClassification {
    label: &'static str,
    formal_config: bool,
    business_data: bool,
    basis_tokens: Vec<String>,
}

fn classify_recent_artifact_inventory(
    inventory: &RecentArtifactInventory,
) -> RecentArtifactClassification {
    let tokens = recent_artifact_machine_tokens(inventory);
    classify_recent_artifact_tokens(tokens)
}

fn classify_recent_artifact_entry(
    inventory: &RecentArtifactInventory,
    entry: &RecentArtifactEntry,
) -> RecentArtifactClassification {
    let mut tokens = Vec::new();
    push_recent_artifact_path_tokens(inventory.root.as_str(), &mut tokens);
    push_recent_artifact_path_tokens(entry.name.as_str(), &mut tokens);
    push_recent_artifact_path_tokens(entry.path.as_str(), &mut tokens);
    push_recent_artifact_path_tokens(entry.kind.as_str(), &mut tokens);
    classify_recent_artifact_tokens(tokens)
}

fn classify_recent_artifact_tokens(tokens: Vec<String>) -> RecentArtifactClassification {
    let has = |needle: &str| tokens.iter().any(|token| token == needle);
    let mut basis_tokens = Vec::new();
    let mut push_basis = |token: &str| push_unique_recent_artifact_token(token, &mut basis_tokens);
    if has("generated") && has("skills") && has("md") {
        for token in ["generated", "skills", "md"] {
            if has(token) {
                push_basis(token);
            }
        }
        return RecentArtifactClassification {
            label: "generated_skill_prompt",
            formal_config: false,
            business_data: false,
            basis_tokens,
        };
    }
    if has("prompts") && has("layers") && has("overlays") {
        for token in ["prompts", "layers", "overlays"] {
            if has(token) {
                push_basis(token);
            }
        }
        return RecentArtifactClassification {
            label: "prompt_layer_file",
            formal_config: false,
            business_data: false,
            basis_tokens,
        };
    }
    if has("scripts") && (has("py") || has("sh") || has("bash")) {
        for token in ["scripts", "py", "sh", "bash"] {
            if has(token) {
                push_basis(token);
            }
        }
        return RecentArtifactClassification {
            label: "maintenance_script",
            formal_config: false,
            business_data: false,
            basis_tokens,
        };
    }
    if has("docs") && has("md") {
        for token in ["docs", "md"] {
            if has(token) {
                push_basis(token);
            }
        }
        return RecentArtifactClassification {
            label: "project_documentation",
            formal_config: false,
            business_data: false,
            basis_tokens,
        };
    }
    if has("tmp") || has("temp") || has("unpack") || has("bundle") || has("zip") {
        for token in ["tmp", "temp", "unpack", "bundle", "zip"] {
            if has(token) {
                push_basis(token);
            }
        }
        return RecentArtifactClassification {
            label: "temporary_bundle_artifact",
            formal_config: false,
            business_data: false,
            basis_tokens,
        };
    }
    if has("fixtures") || has("fixture") || has("nl_tests") {
        for token in ["fixtures", "fixture", "nl_tests"] {
            if has(token) {
                push_basis(token);
            }
        }
        return RecentArtifactClassification {
            label: "test_fixture_artifact",
            formal_config: false,
            business_data: false,
            basis_tokens,
        };
    }
    if has("logs") || has("log") {
        for token in ["logs", "log"] {
            if has(token) {
                push_basis(token);
            }
        }
        return RecentArtifactClassification {
            label: "runtime_log",
            formal_config: false,
            business_data: false,
            basis_tokens,
        };
    }
    if has("configs") || has("config") || has("toml") || has("yaml") || has("yml") {
        for token in ["configs", "config", "toml", "yaml", "yml"] {
            if has(token) {
                push_basis(token);
            }
        }
        return RecentArtifactClassification {
            label: "formal_config",
            formal_config: true,
            business_data: false,
            basis_tokens,
        };
    }
    if has("data")
        || has("csv")
        || has("tsv")
        || has("parquet")
        || has("sqlite")
        || has("db")
        || has("xlsx")
    {
        for token in ["data", "csv", "tsv", "parquet", "sqlite", "db", "xlsx"] {
            if has(token) {
                push_basis(token);
            }
        }
        return RecentArtifactClassification {
            label: "business_data",
            formal_config: false,
            business_data: true,
            basis_tokens,
        };
    }
    RecentArtifactClassification {
        label: "unknown",
        formal_config: false,
        business_data: false,
        basis_tokens,
    }
}

fn recent_artifact_machine_tokens(inventory: &RecentArtifactInventory) -> Vec<String> {
    let mut tokens = Vec::new();
    push_recent_artifact_path_tokens(inventory.root.as_str(), &mut tokens);
    for entry in &inventory.entries {
        push_recent_artifact_path_tokens(entry.name.as_str(), &mut tokens);
        push_recent_artifact_path_tokens(entry.path.as_str(), &mut tokens);
        push_recent_artifact_path_tokens(entry.kind.as_str(), &mut tokens);
    }
    for path in &inventory.auxiliary_paths {
        push_recent_artifact_path_tokens(path, &mut tokens);
    }
    tokens
}

fn push_recent_artifact_path_tokens(raw: &str, tokens: &mut Vec<String>) {
    for token in raw
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .flat_map(|part| part.split('_'))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
    {
        push_unique_recent_artifact_token(token.as_str(), tokens);
    }
}

fn push_unique_recent_artifact_token(raw: &str, tokens: &mut Vec<String>) {
    let token = raw.trim();
    if token.is_empty() || tokens.iter().any(|existing| existing == token) {
        return;
    }
    tokens.push(token.to_string());
}

pub(super) fn replace_delivery_with_deterministic_recent_artifacts_judgment_answer(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some((answer, summary)) =
        direct_recent_artifacts_judgment_answer(loop_state, agent_run_context)
    else {
        return false;
    };
    let current_delivery = loop_state
        .delivery_messages
        .last()
        .map(String::as_str)
        .unwrap_or_default();
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let current_is_publishable_synthesis =
        current_delivery_is_latest_publishable_synthesis(loop_state, current_delivery);
    let one_sentence_synthesis = route.is_some_and(|route| {
        route.output_contract.response_shape == crate::OutputResponseShape::OneSentence
    });
    let current_is_recent_artifact_machine_fields =
        recent_artifacts_delivery_is_machine_field_dump(current_delivery);
    if current_is_publishable_synthesis
        && !current_is_recent_artifact_machine_fields
        && (one_sentence_synthesis
            || recent_artifacts_delivery_mentions_all_entries(current_delivery, answer.as_str()))
    {
        loop_state.last_user_visible_respond = Some(current_delivery.trim().to_string());
        return true;
    }
    loop_state
        .delivery_messages
        .retain(|message| crate::finalize::is_execution_summary_message(message));
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    log_deterministic_delivery_record(
        &task.task_id,
        "replace_with_deterministic_recent_artifacts_judgment",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) async fn compose_recent_artifacts_machine_field_delivery(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    delivery: &str,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::RecentArtifactsJudgment
        || !recent_artifacts_delivery_is_machine_field_dump(delivery)
    {
        return None;
    }
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let observed_facts = delivery
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(80)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if observed_facts.is_empty() {
        return None;
    }
    let contract = crate::fallback::UserResponseContract {
        kind: crate::fallback::UserResponseKind::FinalAnswer,
        reason_code: "recent_artifacts_machine_field_render".to_string(),
        missing_slots: Vec::new(),
        observed_facts,
        policy_boundary: vec![
            "Use only observed_facts for entry names, paths, kinds, sizes, and classifications."
                .to_string(),
            "Do not output raw key=value machine fields, JSON, trace names, or schema names."
                .to_string(),
            "Cover the selected recent entries and the grounded judgment requested by the user."
                .to_string(),
        ],
        original_user_request: user_text.trim().to_string(),
        resolved_user_intent: route.resolved_intent.trim().to_string(),
        response_shape: route.output_contract.response_shape.as_str().to_string(),
        language_hint,
    };
    let rendered = crate::fallback::compose_user_response_from_contract(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty,
    )
    .await;
    (!recent_artifacts_delivery_is_machine_field_dump(&rendered)).then_some(rendered)
}

pub(super) fn recent_artifacts_delivery_is_machine_field_dump(delivery: &str) -> bool {
    let mut nonempty_lines = 0usize;
    let mut machine_field_lines = 0usize;
    for line in delivery
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        nonempty_lines += 1;
        if line
            .split_once('=')
            .is_some_and(|(key, _)| recent_artifacts_machine_field_key(key.trim()))
        {
            machine_field_lines += 1;
        }
    }
    nonempty_lines > 0 && machine_field_lines == nonempty_lines
}

fn recent_artifacts_machine_field_key(key: &str) -> bool {
    key == "classification"
        || key.starts_with("classification.")
        || key.starts_with("recent_entries.")
        || (key.starts_with("recent_entries[") && key.contains("]."))
}

fn recent_artifacts_delivery_mentions_all_entries(
    delivery: &str,
    deterministic_answer: &str,
) -> bool {
    let delivery = delivery.trim();
    if delivery.is_empty() {
        return false;
    }
    deterministic_answer
        .lines()
        .filter_map(|line| line.strip_prefix("recent_entries["))
        .filter_map(|line| line.split_once("].name="))
        .map(|(_, name)| name.trim())
        .filter(|name| !name.is_empty())
        .all(|name| delivery.contains(name))
}

fn direct_directory_purpose_summary_from_listing_content(
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryPurposeSummary
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let files = latest_inventory_document_files(loop_state)?;
    if files.len() < 2 {
        return None;
    }
    let reads = read_range_observations(loop_state);
    let relevant = select_relevant_inventory_document_file(&files, &reads)?;
    let listed_files = document_files_matching_relevant_extension(&files, &relevant);
    let excerpt = reads
        .iter()
        .find(|read| inventory_read_path_matches_file(read.path.as_str(), &relevant))
        .map(|read| compact_directory_purpose_excerpt(read.excerpt.as_str()));
    let summary = latest_directory_purpose_synthesis_excerpt(
        loop_state,
        relevant.path.as_str(),
        relevant.size_bytes,
    )
    .or_else(|| {
        latest_directory_purpose_synthesis_excerpt(
            loop_state,
            relevant.name.as_str(),
            relevant.size_bytes,
        )
    });
    if excerpt.is_none() && summary.is_none() {
        return None;
    }
    let file_count = listed_files.len();
    let file_names = listed_files
        .iter()
        .map(|file| file.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let relevant_is_largest = relevant.size_bytes.is_some_and(|size_bytes| {
        listed_files
            .iter()
            .all(|file| file.size_bytes.unwrap_or(0) <= size_bytes)
    });
    let mut fields = vec![
        format!("documentation.files.count={file_count}; documentation.files={file_names}"),
        format!(
            "relevant.name={}; relevant.path={}; relevant.subject={}",
            relevant.name,
            relevant.path,
            schema_subject_from_path_label(relevant.name.as_str())
        ),
    ];
    if let Some(size_bytes) = relevant.size_bytes {
        if relevant_is_largest {
            fields.push(format!(
                "largest.name={}; largest.path={}; largest.size_bytes={size_bytes}",
                relevant.name, relevant.path
            ));
        } else {
            fields.push(format!("relevant.size_bytes={size_bytes}"));
        }
    }
    if let Some(excerpt) = excerpt.as_ref().filter(|excerpt| !excerpt.is_empty()) {
        fields.push(format!("content_excerpt={excerpt}"));
    }
    if let Some(summary) = summary.or_else(|| excerpt.clone()) {
        fields.push(format!("directory_purpose_summary={summary}"));
    }
    Some((
        fields.join("\n"),
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: if reads.is_empty() { 1 } else { 2 },
            ..Default::default()
        },
    ))
}

fn latest_inventory_document_files(
    loop_state: &crate::agent_engine::LoopState,
) -> Option<Vec<InventoryDocumentFile>> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .flat_map(structured_json_values_from_output)
        .find_map(|value| inventory_document_files_from_value(&value))
}

fn inventory_document_files_from_value(
    value: &serde_json::Value,
) -> Option<Vec<InventoryDocumentFile>> {
    if value.get("action").and_then(|value| value.as_str()) != Some("inventory_dir") {
        return None;
    }
    let entries = value.get("entries").and_then(|value| value.as_array())?;
    let files = entries
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
            if !document_file_extension_is_supported(name) {
                return None;
            }
            let path = entry
                .get("path")
                .or_else(|| entry.get("resolved_path"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .unwrap_or(name);
            Some(InventoryDocumentFile {
                name: name.to_string(),
                path: path.to_string(),
                size_bytes: entry.get("size_bytes").and_then(|value| value.as_u64()),
            })
        })
        .collect::<Vec<_>>();
    (!files.is_empty()).then_some(files)
}

fn read_range_observations(
    loop_state: &crate::agent_engine::LoopState,
) -> Vec<ReadRangeObservation> {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .flat_map(structured_json_values_from_output)
        .filter_map(read_range_observation_from_value)
        .collect()
}

fn read_range_observation_from_value(value: serde_json::Value) -> Option<ReadRangeObservation> {
    if !matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("read_range" | "read_text_range")
    ) {
        return None;
    }
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())?
        .to_string();
    let excerpt = value
        .get("excerpt")
        .or_else(|| value.get("content"))
        .and_then(|value| value.as_str())
        .map(normalize_directory_purpose_read_excerpt)
        .filter(|excerpt| !excerpt.trim().is_empty())?;
    Some(ReadRangeObservation { path, excerpt })
}

fn select_relevant_inventory_document_file(
    files: &[InventoryDocumentFile],
    reads: &[ReadRangeObservation],
) -> Option<InventoryDocumentFile> {
    files
        .iter()
        .filter(|file| {
            reads
                .iter()
                .any(|read| inventory_read_path_matches_file(read.path.as_str(), file))
        })
        .max_by_key(|file| file.size_bytes.unwrap_or(0))
        .cloned()
        .or_else(|| {
            files
                .iter()
                .max_by_key(|file| file.size_bytes.unwrap_or(0))
                .cloned()
        })
}

fn inventory_read_path_matches_file(read_path: &str, file: &InventoryDocumentFile) -> bool {
    let read_path = normalize_path_for_directory_purpose(read_path);
    let file_path = normalize_path_for_directory_purpose(file.path.as_str());
    let file_name = normalize_path_for_directory_purpose(file.name.as_str());
    !read_path.is_empty()
        && (!file_path.is_empty() && read_path.ends_with(file_path.as_str())
            || !file_name.is_empty() && read_path.ends_with(file_name.as_str()))
}

fn normalize_path_for_directory_purpose(path: &str) -> String {
    path.trim()
        .trim_matches('`')
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

fn document_files_matching_relevant_extension(
    files: &[InventoryDocumentFile],
    relevant: &InventoryDocumentFile,
) -> Vec<InventoryDocumentFile> {
    let Some(extension) = document_file_extension(relevant.name.as_str())
        .or_else(|| document_file_extension(relevant.path.as_str()))
    else {
        return files.to_vec();
    };
    let filtered = files
        .iter()
        .filter(|file| {
            document_file_extension(file.name.as_str())
                .or_else(|| document_file_extension(file.path.as_str()))
                .is_some_and(|file_extension| file_extension == extension)
        })
        .cloned()
        .collect::<Vec<_>>();
    if filtered.len() >= 2 {
        filtered
    } else {
        files.to_vec()
    }
}

fn document_file_extension(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|extension| !extension.is_empty())
        .map(str::to_ascii_lowercase)
}

fn latest_directory_purpose_synthesis_excerpt(
    loop_state: &crate::agent_engine::LoopState,
    preferred_label: &str,
    preferred_size_bytes: Option<u64>,
) -> Option<String> {
    let preferred_tokens = directory_purpose_synthesis_preferred_tokens(preferred_label);
    let preferred_size_token = preferred_size_bytes.map(|size_bytes| size_bytes.to_string());
    let mut fallback = None;
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || step.skill != "synthesize_answer" {
            continue;
        }
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        let compact = compact_directory_purpose_synthesis_excerpt(output);
        if compact.is_empty() {
            continue;
        }
        if preferred_size_token.is_none() {
            fallback.get_or_insert_with(|| compact.clone());
        }
        let match_surface = output.replace('\\', "/").to_ascii_lowercase();
        let label_matches = preferred_tokens
            .iter()
            .any(|token| match_surface.contains(token.as_str()));
        let size_matches = preferred_size_token
            .as_ref()
            .is_none_or(|token| match_surface.contains(token.as_str()));
        if label_matches && size_matches {
            return Some(compact);
        }
    }
    fallback
}

fn directory_purpose_synthesis_preferred_tokens(label: &str) -> Vec<String> {
    let normalized = normalize_path_for_directory_purpose(label).to_ascii_lowercase();
    let mut tokens = Vec::new();
    push_unique_directory_purpose_synthesis_token(normalized.as_str(), &mut tokens);
    if let Some(file_name) = Path::new(normalized.as_str())
        .file_name()
        .and_then(|value| value.to_str())
    {
        push_unique_directory_purpose_synthesis_token(file_name, &mut tokens);
        if let Some(stem) = Path::new(file_name)
            .file_stem()
            .and_then(|value| value.to_str())
        {
            push_unique_directory_purpose_synthesis_token(stem, &mut tokens);
        }
    }
    tokens
}

fn push_unique_directory_purpose_synthesis_token(raw: &str, tokens: &mut Vec<String>) {
    let token = raw.trim();
    if token.len() < 3 || tokens.iter().any(|existing| existing == token) {
        return;
    }
    tokens.push(token.to_string());
}

fn compact_directory_purpose_synthesis_excerpt(excerpt: &str) -> String {
    truncate_with_ellipsis(
        &excerpt
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace('`', "'"),
        360,
    )
}

fn normalize_directory_purpose_read_excerpt(excerpt: &str) -> String {
    excerpt
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            trimmed
                .split_once('|')
                .filter(|(prefix, _)| prefix.chars().all(|ch| ch.is_ascii_digit()))
                .map(|(_, tail)| tail.trim())
                .unwrap_or(trimmed)
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn compact_directory_purpose_excerpt(excerpt: &str) -> String {
    const MAX_CHARS: usize = 480;
    let mut compact = excerpt
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if compact.chars().count() > MAX_CHARS {
        compact = compact.chars().take(MAX_CHARS).collect::<String>();
        compact.push_str("...");
    }
    compact
}

fn document_file_extension_is_supported(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|extension| extension.trim().to_ascii_lowercase())
        .is_some_and(|extension| {
            matches!(
                extension.as_str(),
                "md" | "markdown"
                    | "txt"
                    | "json"
                    | "toml"
                    | "yaml"
                    | "yml"
                    | "pdf"
                    | "doc"
                    | "docx"
                    | "rtf"
                    | "html"
                    | "htm"
            )
        })
}

fn delivery_mentions_unobserved_document_file(
    loop_state: &crate::agent_engine::LoopState,
    delivery: &str,
) -> bool {
    let Some(files) = latest_inventory_document_files(loop_state) else {
        return false;
    };
    let observed = files
        .iter()
        .flat_map(|file| {
            [
                normalize_path_for_directory_purpose(file.name.as_str()),
                normalize_path_for_directory_purpose(file.path.as_str()),
            ]
        })
        .collect::<Vec<_>>();
    candidate_document_file_tokens(delivery)
        .into_iter()
        .any(|token| {
            !observed.iter().any(|item| {
                !item.is_empty() && (token == *item || token.ends_with(&format!("/{item}")))
            })
        })
}

fn delivery_omits_observed_document_file(
    loop_state: &crate::agent_engine::LoopState,
    delivery: &str,
) -> bool {
    let Some(files) = latest_inventory_document_files(loop_state) else {
        return false;
    };
    if files.len() < 2 || delivery.trim().is_empty() {
        return false;
    }
    let mentioned = files
        .iter()
        .filter(|file| delivery_mentions_inventory_document_file(delivery, file))
        .count();
    mentioned > 0 && mentioned < files.len()
}

fn delivery_mentions_inventory_document_file(delivery: &str, file: &InventoryDocumentFile) -> bool {
    let exact_surface = normalize_path_for_directory_purpose(delivery);
    let spaced_surface = normalize_document_identity_surface(delivery);
    inventory_document_file_identity_variants(file)
        .into_iter()
        .any(|variant| {
            let exact = normalize_path_for_directory_purpose(&variant);
            let spaced = normalize_document_identity_surface(&variant);
            (!exact.is_empty() && exact_surface.contains(&exact))
                || (!spaced.is_empty() && spaced_surface.contains(&spaced))
        })
}

fn inventory_document_file_identity_variants(file: &InventoryDocumentFile) -> Vec<String> {
    let mut variants = Vec::new();
    for raw in [file.name.as_str(), file.path.as_str()] {
        push_unique_document_identity_variant(raw, &mut variants);
        if let Some(stem) = Path::new(raw).file_stem().and_then(|value| value.to_str()) {
            push_unique_document_identity_variant(stem, &mut variants);
        }
    }
    push_unique_document_identity_variant(
        &schema_subject_from_path_label(file.name.as_str()),
        &mut variants,
    );
    variants
}

fn push_unique_document_identity_variant(raw: &str, variants: &mut Vec<String>) {
    let value = raw.trim();
    if value.is_empty()
        || variants
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(value))
    {
        return;
    }
    variants.push(value.to_string());
}

fn normalize_document_identity_surface(text: &str) -> String {
    text.to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn candidate_document_file_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '(' | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '"'
                    | '\''
                    | '`'
                    | ','
                    | '，'
                    | ';'
                    | '；'
                    | ':'
                    | '：'
                    | '。'
                    | '、'
                    | '!'
                    | '?'
                    | '？'
                    | '！'
            )
    })
    .map(normalize_path_for_directory_purpose)
    .filter(|token| !token.is_empty() && document_file_extension_is_supported(token))
    .collect()
}

fn inventory_top_level_dirs_from_value(value: &serde_json::Value) -> Option<Vec<String>> {
    if value.get("action").and_then(|value| value.as_str()) != Some("inventory_dir") {
        return None;
    }
    let mut dirs = Vec::<String>::new();
    if let Some(names) = value
        .get("names_by_kind")
        .and_then(serde_json::Value::as_object)
        .and_then(|names_by_kind| names_by_kind.get("dirs"))
        .and_then(serde_json::Value::as_array)
    {
        for name in names {
            if let Some(name) = name.as_str() {
                push_unique_inventory_dir_name(name, &mut dirs);
            }
        }
    }
    if let Some(entries) = value.get("entries").and_then(serde_json::Value::as_array) {
        for entry in entries {
            if !entry
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("dir"))
            {
                continue;
            }
            if let Some(name) = entry.get("name").and_then(serde_json::Value::as_str) {
                push_unique_inventory_dir_name(name, &mut dirs);
            }
        }
    }
    (!dirs.is_empty()).then_some(dirs)
}

fn push_unique_inventory_dir_name(raw: &str, dirs: &mut Vec<String>) {
    let name = raw.trim().trim_matches('`').trim();
    if name.is_empty() || matches!(name, "." | "..") {
        return;
    }
    if dirs
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(name))
    {
        return;
    }
    dirs.push(name.to_string());
}

fn observed_current_workspace_top_level_dirs(loop_state: &LoopState) -> Option<Vec<String>> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?;
            structured_json_values_from_output(output)
                .iter()
                .find_map(inventory_top_level_dirs_from_value)
        })
}

pub(super) fn direct_current_workspace_top_level_dirs_overview_answer(
    _state: &AppState,
    _user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free
                | crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::FileToken
                | crate::OutputResponseShape::OneSentence
        )
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::WorkspaceProjectSummary
        )
        || loop_state
            .executed_step_results
            .iter()
            .any(step_output_is_read_range)
    {
        return None;
    }
    let dirs = observed_current_workspace_top_level_dirs(loop_state)?;
    let answer = format!(
        "workspace.top_level_dirs.count={}\nworkspace.top_level_dirs={}\nworkspace.overview.kind=repository_sections_by_purpose\nworkspace.overview.section_hints=docs,config,code,scripts,runtime_data,build_output",
        dirs.len(),
        dirs.join(",")
    );
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

pub(super) fn replace_delivery_with_deterministic_current_workspace_dirs_overview_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some((answer, summary)) = direct_current_workspace_top_level_dirs_overview_answer(
        state,
        user_text,
        loop_state,
        agent_run_context,
    ) else {
        return false;
    };
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|message| message.trim() == answer.trim())
    {
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return true;
    }
    loop_state
        .delivery_messages
        .retain(|message| crate::finalize::is_execution_summary_message(message));
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    log_deterministic_delivery_record(
        &task.task_id,
        "replace_with_deterministic_current_workspace_dirs_overview",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn schema_subject_from_path_label(label: &str) -> String {
    let file_name = Path::new(label)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(label)
        .trim();
    let subject = file_name
        .strip_suffix(".schema.json")
        .or_else(|| file_name.strip_suffix(".json"))
        .unwrap_or(file_name)
        .replace(['_', '-'], " ");
    let subject = subject.trim();
    if subject.is_empty() {
        file_name.to_string()
    } else {
        subject.to_string()
    }
}

pub(super) fn replace_delivery_with_deterministic_directory_purpose_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let current_delivery = loop_state
        .delivery_messages
        .last()
        .map(String::as_str)
        .unwrap_or_default();
    if agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            route.output_contract.semantic_kind
                == crate::OutputSemanticKind::DirectoryPurposeSummary
        })
        && current_delivery_is_latest_publishable_synthesis(loop_state, current_delivery)
    {
        loop_state.last_user_visible_respond = Some(current_delivery.trim().to_string());
        log_deterministic_delivery_record(
            &task.task_id,
            "directory_purpose_keep_publishable_synthesis",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    let listing_content_answer = (current_delivery.trim().is_empty()
        || delivery_mentions_unobserved_document_file(loop_state, current_delivery)
        || delivery_omits_observed_document_file(loop_state, current_delivery))
    .then(|| direct_directory_purpose_summary_from_listing_content(loop_state, agent_run_context))
    .flatten();
    let Some((answer, summary)) = listing_content_answer.or_else(|| {
        direct_directory_purpose_summary_from_size_facts(
            state,
            user_text,
            loop_state,
            agent_run_context,
        )
    }) else {
        return false;
    };
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|message| message.trim() == answer.trim())
    {
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return true;
    }
    loop_state
        .delivery_messages
        .retain(|message| crate::finalize::is_execution_summary_message(message));
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    log_deterministic_delivery_record(
        &task.task_id,
        "replace_with_deterministic_directory_purpose",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

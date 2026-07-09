use serde_json::Value;
use std::path::Path;

use crate::pipeline_types::OutputContractRef;

use super::{
    execution_finalize_style_for_contract, state_patch_deictic_reference_requires_clarify,
    ActFinalizeStyle, AppState, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
    OutputResponseShape, OutputSemanticKind, RouteDecision, ScheduleKind,
};

pub(super) fn resolved_existing_directory_from_current_request(
    state: &AppState,
    req: &str,
) -> Option<String> {
    match crate::worker::try_resolve_implicit_locator_path(
        state,
        req,
        "",
        OutputLocatorKind::Path,
        None,
    ) {
        Some(crate::worker::LocatorAutoResolution::Direct(path)) if Path::new(&path).is_dir() => {
            return Some(path);
        }
        Some(crate::worker::LocatorAutoResolution::Direct(_))
        | Some(crate::worker::LocatorAutoResolution::Fuzzy(_))
        | None => {}
    }
    resolve_unique_direct_child_directory_token(state, req)
}

pub(super) fn resolved_directory_pair_from_current_request(
    state: &AppState,
    req: &str,
) -> Option<(String, String)> {
    let mut out = Vec::new();
    for token in current_request_locator_tokens(req) {
        if !strong_structural_locator_token(&token) {
            continue;
        }
        let Some(path) = resolve_unique_directory_token_under_workspace(state, &token) else {
            continue;
        };
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&path))
        {
            out.push(path);
        }
        if out.len() >= 2 {
            break;
        }
    }
    (out.len() == 2).then(|| (out.remove(0), out.remove(0)))
}

fn strong_structural_locator_token(token: &str) -> bool {
    token.contains(['_', '-', '.']) || token.chars().any(|ch| ch.is_ascii_digit())
}

fn resolve_unique_directory_token_under_workspace(state: &AppState, token: &str) -> Option<String> {
    let workspace_root = state.skill_rt.workspace_root.as_path();
    if !workspace_root.is_dir() || token.trim().is_empty() {
        return None;
    }
    let mut stack = vec![workspace_root.to_path_buf()];
    let mut matches = Vec::new();
    let mut visits = 0usize;
    let max_visits = state.skill_rt.locator_scan_max_files.max(50_000);
    while let Some(dir) = stack.pop() {
        visits = visits.saturating_add(1);
        if visits > max_visits {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                file_type.is_dir().then(|| entry.path())
            })
            .collect::<Vec<_>>();
        children.sort();
        for child in children.into_iter().rev() {
            if child
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(token))
            {
                let canonical = child.canonicalize().unwrap_or(child.clone());
                matches.push(canonical.display().to_string());
                if matches.len() > 1 {
                    return None;
                }
            }
            stack.push(child);
        }
    }
    matches.pop()
}

fn resolve_unique_direct_child_directory_token(state: &AppState, req: &str) -> Option<String> {
    let mut matches = Vec::new();
    for token in current_request_locator_tokens(req) {
        for root in [
            state.skill_rt.workspace_root.as_path(),
            state.skill_rt.default_locator_search_dir.as_path(),
        ] {
            collect_direct_child_directory_token_matches(root, &token, &mut matches);
        }
    }
    matches.sort();
    matches.dedup();
    (matches.len() == 1).then(|| matches.remove(0))
}

fn current_request_locator_tokens(req: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in req.split_whitespace() {
        for token in raw.split(|ch: char| {
            matches!(
                ch,
                ',' | '，'
                    | '。'
                    | ';'
                    | '；'
                    | ':'
                    | '：'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '《'
                    | '》'
            )
        }) {
            let token = token
                .trim_matches(|ch: char| {
                    !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
                })
                .trim();
            if token.chars().count() < 2
                || token.contains('/')
                || token.contains('\\')
                || token.starts_with('.')
                || token.chars().all(|ch| ch.is_ascii_digit())
                || !token
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
            {
                continue;
            }
            if !out.iter().any(|existing: &String| existing == token) {
                out.push(token.to_string());
            }
        }
    }
    out
}

fn collect_direct_child_directory_token_matches(
    root: &Path,
    token: &str,
    matches: &mut Vec<String>,
) {
    if !root.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.eq_ignore_ascii_case(token) {
            continue;
        }
        let canonical = path.canonicalize().unwrap_or(path);
        matches.push(canonical.display().to_string());
    }
}

pub(super) fn apply_resolved_directory_observation_clarify_repair(
    state: &AppState,
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    state_patch: Option<&Value>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !*needs_clarify
        || req_surface.is_structural_locator_only_reply()
        || req_surface.token_count <= 2
        || output_contract.delivery_required
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
    {
        return None;
    }
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return None;
    }
    let recover_empty_listing_contract =
        empty_directory_listing_contract_can_bind_directory(output_contract, req);
    if !output_contract.requires_content_evidence && !recover_empty_listing_contract {
        return None;
    }
    let directory = resolved_existing_directory_from_current_request(state, req)?;
    output_contract.requires_content_evidence = true;
    output_contract.delivery_required = false;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.locator_kind = OutputLocatorKind::Path;
    output_contract.locator_hint = directory;
    if recover_empty_listing_contract {
        let output_contract_ref = if request_has_extension_filter_token(req) {
            OutputSemanticKind::FileNames
        } else {
            OutputSemanticKind::DirectoryEntryGroups
        };
        output_contract.apply_output_contract_ref(OutputContractRef::new(output_contract_ref));
        output_contract.response_shape = OutputResponseShape::Strict;
    }
    *needs_clarify = false;
    clarify_question.clear();
    *execution_finalize_style =
        crate::post_route_policy::content_evidence_execution_finalize_style(output_contract, false)
            .unwrap_or_else(|| execution_finalize_style_for_contract(output_contract));
    Some("resolved_directory_observation_clarify_repair")
}

fn empty_directory_listing_contract_can_bind_directory(
    output_contract: &IntentOutputContract,
    req: &str,
) -> bool {
    output_contract.semantic_kind_is_unclassified()
        && output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Strict | OutputResponseShape::Free
        )
        && (matches!(output_contract.response_shape, OutputResponseShape::Strict)
            || request_has_extension_filter_token(req))
}

fn request_has_extension_filter_token(req: &str) -> bool {
    req.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                ',' | '，'
                    | '。'
                    | ';'
                    | '；'
                    | ':'
                    | '：'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '《'
                    | '》'
                    | '、'
            )
    })
    .map(str::trim)
    .any(|token| {
        let Some(ext) = token.strip_prefix('.') else {
            return false;
        };
        !ext.is_empty()
            && ext.len() <= 16
            && ext
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    })
}

pub(super) fn directory_pair_fallback_decision(
    state: &AppState,
    req: &str,
) -> Option<RouteDecision> {
    let enabled_skills = state.get_skills_list();
    if !enabled_skills.is_empty() && !enabled_skills.contains("system_basic") {
        return None;
    }
    let (left, right) = resolved_directory_pair_from_current_request(state, req)?;
    if left.eq_ignore_ascii_case(&right) {
        return None;
    }

    Some(RouteDecision {
        resolved_user_intent: req.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer_unavailable_directory_pair".to_string(),
        confidence: Some(0.62),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: format!("{left} | {right}"),
            ..Default::default()
        },
    })
}

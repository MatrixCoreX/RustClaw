use std::collections::BTreeSet;
use std::path::Path;

use super::{
    locator_hint_points_to_workspace_root, IntentOutputContract, OutputDeliveryIntent,
    OutputLocatorKind, OutputResponseShape, OutputSemanticKind,
};

pub(super) fn current_workspace_generic_summary_needs_workspace_summary_contract(
    output_contract: &IntentOutputContract,
) -> bool {
    output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && output_contract.semantic_kind_is_unclassified()
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Free | OutputResponseShape::OneSentence
        )
        && matches!(
            output_contract.locator_kind,
            OutputLocatorKind::CurrentWorkspace
        )
}

pub(super) fn current_turn_extension_inventory_file_paths_repair_applies(
    output_contract: &IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    if output_contract.delivery_required
        || !output_contract.requires_content_evidence
        || !output_contract.semantic_kind_is_unclassified()
        || req_surface.has_filename_candidates()
        || req_surface.has_structured_target_refinement()
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::CurrentWorkspace | OutputLocatorKind::Path
        )
    {
        return false;
    }
    current_turn_structural_extension_filter(req)
        .or_else(|| current_turn_structural_extension_filter(&output_contract.locator_hint))
        .is_some()
}

pub(super) fn extension_inventory_locator_hint_should_use_workspace(
    hint: &str,
    workspace_root: &Path,
) -> bool {
    let hint = hint.trim();
    if hint.is_empty() || locator_hint_points_to_workspace_root(hint, workspace_root) {
        return true;
    }
    let path = Path::new(hint);
    if path.is_dir() || hint.contains(['/', '\\']) {
        return false;
    }
    current_turn_structural_extension_filter(hint).is_some()
}

fn current_turn_structural_extension_filter(text: &str) -> Option<String> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '*')))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .find_map(|token| {
            structural_extension_from_globish_token(token)
                .or_else(|| structural_extension_from_bare_token(token))
        })
}

fn structural_extension_from_globish_token(text: &str) -> Option<String> {
    let cleaned = text
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_ascii_lowercase();
    let (_prefix, ext) = cleaned.rsplit_once('.')?;
    if ext.is_empty()
        || ext.contains(['*', '?', '/', '\\'])
        || !ext
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return None;
    }
    cleaned
        .contains('*')
        .then(|| ext.to_string())
        .or_else(|| cleaned.strip_prefix('.').map(ToString::to_string))
}

fn structural_extension_from_bare_token(text: &str) -> Option<String> {
    let cleaned = text
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if cleaned.is_empty()
        || cleaned.contains(['*', '?', '/', '\\', '.'])
        || !cleaned
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return None;
    }
    // Language-neutral file extension tokens, not user-phrase routing.
    const STRUCTURAL_EXTENSION_TOKENS: &[&str] = &[
        "bash", "cfg", "conf", "css", "csv", "env", "html", "ini", "js", "json", "jsonl", "jsx",
        "lock", "log", "md", "mjs", "py", "rs", "scss", "sh", "sql", "toml", "ts", "tsx", "txt",
        "xml", "yaml", "yml", "zsh",
    ];
    STRUCTURAL_EXTENSION_TOKENS
        .contains(&cleaned.as_str())
        .then_some(cleaned)
}

pub(super) fn file_paths_missing_file_locator_parent_dir(
    output_contract: &IntentOutputContract,
    workspace_root: &Path,
) -> Option<String> {
    if output_contract.delivery_required
        || !output_contract.semantic_kind_is(OutputSemanticKind::FilePaths)
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        )
    {
        return None;
    }
    let raw = output_contract.locator_hint.trim();
    if raw.is_empty() || raw.contains('|') {
        return None;
    }
    let raw_path = Path::new(raw);
    let absolute_path = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        workspace_root.join(raw_path)
    };
    if absolute_path.exists() {
        return None;
    }
    let absolute_parent = absolute_path.parent()?;
    if !absolute_parent.is_dir() {
        return None;
    }
    if raw_path.is_absolute() {
        return Some(absolute_parent.display().to_string());
    }
    Some(
        raw_path
            .parent()
            .and_then(|parent| parent.to_str())
            .filter(|parent| !parent.trim().is_empty())
            .unwrap_or(".")
            .to_string(),
    )
}

pub(super) fn existence_with_path_mixed_locator_summary_repair(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    if output_contract.delivery_required
        || !output_contract.semantic_kind_is(OutputSemanticKind::ExistenceWithPath)
        || !output_contract.requires_content_evidence
        || !matches!(
            output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
        || !req_surface.has_concrete_locator_hint()
    {
        return false;
    }
    let locator_hint = output_contract.locator_hint.trim();
    if locator_hint.is_empty() {
        return false;
    }
    let locator_hint = locator_hint.replace('\\', "/").to_ascii_lowercase();
    req_surface.filename_candidates.iter().any(|candidate| {
        let candidate = candidate.trim();
        !candidate.is_empty()
            && Path::new(candidate).extension().is_some()
            && !locator_hint.contains(&candidate.replace('\\', "/").to_ascii_lowercase())
    })
}

pub(super) fn quoted_literal_content_presence_contract_repair(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    if output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !output_contract.semantic_kind_is(OutputSemanticKind::ExistenceWithPath)
        || req_surface.single_quoted_literal().is_none()
        || req_surface.has_structured_target_refinement()
    {
        return false;
    }
    if !matches!(
        output_contract.locator_kind,
        OutputLocatorKind::Path | OutputLocatorKind::Filename | OutputLocatorKind::CurrentWorkspace
    ) {
        return false;
    }
    !output_contract.locator_hint.trim().is_empty()
        || req_surface.has_explicit_path_or_url()
        || req_surface.has_single_filename_candidate()
}

pub(super) fn structured_config_keys_contract_from_surface(
    output_contract: &IntentOutputContract,
    req: &str,
) -> Option<String> {
    if output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !output_contract.semantic_kind_is(OutputSemanticKind::FileNames)
    {
        return None;
    }
    output_contract_structured_config_path(output_contract).or_else(|| {
        crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req).and_then(
            |locator| {
                path_has_structured_config_extension(&locator.locator_hint)
                    .then_some(locator.locator_hint)
            },
        )
    })
}

pub(super) fn surface_has_directory_scoped_filename_lookup(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
) -> bool {
    if req_surface.filename_candidates.is_empty() {
        return false;
    }
    crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(req)
        .into_iter()
        .filter(|locator| matches!(locator.locator_kind, OutputLocatorKind::Path))
        .any(|locator| {
            let raw = locator.locator_hint.trim();
            if raw.is_empty() {
                return false;
            }
            let path = Path::new(raw);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                workspace_root.join(path)
            };
            path.is_dir()
        })
}

pub(super) fn config_mutation_contract_from_surface(
    output_contract: &IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Option<String> {
    if output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::None
                | OutputSemanticKind::ContentExcerptSummary
                | OutputSemanticKind::ScalarPathOnly
                | OutputSemanticKind::StructuredKeys
                | OutputSemanticKind::ConfigValidation
                | OutputSemanticKind::ConfigRiskAssessment
                | OutputSemanticKind::ConfigMutation
                | OutputSemanticKind::FilesystemMutationResult
                | OutputSemanticKind::ExecutionFailedStep
        )
    {
        return None;
    }
    let field_path = req_surface.dotted_field_selector.as_deref()?;
    if !structural_config_value_after_field(req, field_path) {
        return None;
    }
    output_contract_structured_config_path(output_contract)
        .or_else(|| explicit_structured_config_path_from_request(req))
}

pub(super) fn structured_field_value_contract_from_quantity_comparison(
    output_contract: &IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Option<String> {
    if output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::QuantityComparison
        )
    {
        return None;
    }
    let field_path = req_surface.dotted_field_selector.as_deref()?;
    if structural_config_value_after_field(req, field_path) {
        return None;
    }
    let locator_hint = output_contract_structured_config_path(output_contract).or_else(|| {
        crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req).and_then(
            |locator| {
                path_has_structured_config_extension(&locator.locator_hint)
                    .then_some(locator.locator_hint)
            },
        )
    })?;
    if split_structural_locator_targets(&locator_hint).len() != 1 {
        return None;
    }
    Some(locator_hint)
}

pub(super) fn structured_field_pair_contract_from_quantity_comparison(
    output_contract: &IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Option<String> {
    if output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::QuantityComparison
        )
        || !req_surface.has_structured_target_refinement()
    {
        return None;
    }

    let mut locators = Vec::new();
    for target in split_structural_locator_targets(&output_contract.locator_hint) {
        if path_has_structured_config_extension(target)
            && !locators
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(target))
        {
            locators.push(target.to_string());
        }
    }
    for locator in
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(req)
    {
        let target = locator.locator_hint.trim();
        if path_has_structured_config_extension(target)
            && !locators
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(target))
        {
            locators.push(target.to_string());
        }
    }

    (locators.len() >= 2).then(|| locators.join(" | "))
}

fn split_structural_locator_targets(locator_hint: &str) -> Vec<&str> {
    locator_hint
        .split(['|', '\n', ';', ',', '、'])
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .collect()
}

pub(super) fn structured_identifier_presence_contract_from_surface(
    output_contract: &IntentOutputContract,
    req: &str,
    workspace_root: &Path,
) -> Option<String> {
    if output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::ContentPresenceCheck
                | OutputSemanticKind::ExistenceWithPath
                | OutputSemanticKind::ConfigValidation
        )
    {
        return None;
    }
    let locator_hint = output_contract_structured_config_path(output_contract).or_else(|| {
        crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req).and_then(
            |locator| {
                path_has_structured_config_extension(&locator.locator_hint)
                    .then_some(locator.locator_hint)
            },
        )
    })?;
    if !request_has_code_identifier_outside_locator(req, &locator_hint) {
        return None;
    }
    let path = Path::new(&locator_hint);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    if !resolved.is_file() {
        return None;
    }
    Some(locator_hint)
}

fn request_has_code_identifier_outside_locator(req: &str, locator_hint: &str) -> bool {
    let locator_parts = identifier_parts_from_locator(locator_hint);
    req.split(|ch: char| !ch.is_ascii_alphanumeric() && !matches!(ch, '_' | '-' | '$'))
        .map(str::trim)
        .filter(|token| {
            token.chars().any(|ch| matches!(ch, '_' | '-' | '$'))
                && token
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '$'))
        })
        .any(|token| !locator_parts.contains(&token.to_ascii_lowercase()))
}

fn identifier_parts_from_locator(locator_hint: &str) -> BTreeSet<String> {
    locator_hint
        .split(|ch: char| !ch.is_ascii_alphanumeric() && !matches!(ch, '_' | '-' | '$'))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

pub(super) fn output_contract_structured_config_path(
    output_contract: &IntentOutputContract,
) -> Option<String> {
    let hint = output_contract.locator_hint.trim();
    if hint.is_empty() || !path_has_structured_config_extension(hint) {
        return None;
    }
    matches!(
        output_contract.locator_kind,
        OutputLocatorKind::Path | OutputLocatorKind::Filename
    )
    .then(|| hint.to_string())
}

fn path_has_structured_config_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| matches!(ext.as_str(), "json" | "toml" | "yaml" | "yml"))
}

fn explicit_structured_config_path_from_request(req: &str) -> Option<String> {
    crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(req)
        .into_iter()
        .filter(|locator| matches!(locator.locator_kind, OutputLocatorKind::Path))
        .find_map(|locator| {
            path_has_structured_config_extension(&locator.locator_hint)
                .then_some(locator.locator_hint)
        })
}

pub(super) fn structural_config_value_after_field(req: &str, field_path: &str) -> bool {
    let req_lower = req.to_ascii_lowercase();
    let field_lower = field_path.to_ascii_lowercase();
    let Some(field_idx) = req_lower.find(&field_lower) else {
        return false;
    };
    let Some(suffix) = req.get(field_idx + field_path.len()..) else {
        return false;
    };
    structural_config_value_candidate_tokens(suffix).any(|token| {
        token.eq_ignore_ascii_case("true")
            || token.eq_ignore_ascii_case("false")
            || token.eq_ignore_ascii_case("null")
            || token.parse::<i64>().is_ok()
            || token.parse::<f64>().is_ok()
    })
}

fn structural_config_value_candidate_tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|ch: char| {
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
            )
    })
    .map(|token| {
        token
            .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '=' | '>' | '-' | '→'))
            .trim()
            .to_string()
    })
    .filter(|token| !token.is_empty())
}

pub(super) fn inline_structured_payload_contract_context(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    output_contract: &IntentOutputContract,
) -> bool {
    req_surface.inline_json_shape.is_some()
        && !crate::intent::surface_signals::inline_json_transform_request(req)
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && output_contract.locator_hint.trim().is_empty()
        && output_contract.semantic_kind_is_unclassified()
        && !req_surface.has_explicit_path_or_url()
        && !req_surface.has_delivery_token_reference()
        && req_surface.locator_target_pair.is_none()
}

pub(super) fn inline_structured_transform_contract_context(
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    output_contract: &IntentOutputContract,
) -> bool {
    req_surface.inline_json_shape.is_some()
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && output_contract.locator_hint.trim().is_empty()
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Strict | OutputResponseShape::Scalar
        )
        && matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::ContentExcerptWithSummary
                | OutputSemanticKind::ContentExcerptSummary
                | OutputSemanticKind::StructuredKeys
        )
        && !req_surface.has_explicit_path_or_url()
        && !req_surface.has_delivery_token_reference()
        && req_surface.locator_target_pair.is_none()
}

pub(super) fn generated_file_delivery_filename_only_existing_target_repair(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
) -> Option<String> {
    if !output_contract.semantic_kind_is(OutputSemanticKind::GeneratedFileDelivery)
        || !output_contract.delivery_required
        || output_contract.delivery_intent != OutputDeliveryIntent::FileSingle
        || output_contract.response_shape != OutputResponseShape::FileToken
        || req_surface.has_explicit_path_or_url()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_delivery_token_reference()
    {
        return None;
    }
    let filename = req_surface
        .single_filename_candidate()
        .map(str::trim)
        .filter(|filename| !filename.is_empty())
        .map(ToString::to_string)?;
    workspace_root.join(&filename).is_file().then_some(filename)
}

pub(super) fn generated_file_delivery_existing_content_summary_repair(
    output_contract: &IntentOutputContract,
    workspace_root: &Path,
) -> Option<String> {
    if !output_contract.semantic_kind_is(OutputSemanticKind::GeneratedFileDelivery)
        || !output_contract.delivery_required
        || output_contract.delivery_intent != OutputDeliveryIntent::FileSingle
        || output_contract.response_shape != OutputResponseShape::FileToken
        || output_contract.exact_sentence_count.is_none()
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        )
    {
        return None;
    }
    let raw_hint = output_contract.locator_hint.trim();
    if raw_hint.is_empty() || raw_hint.contains('|') {
        return None;
    }
    let candidate = Path::new(raw_hint);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };
    if !resolved.is_file() {
        return None;
    }
    Some(
        resolved
            .canonicalize()
            .unwrap_or(resolved)
            .display()
            .to_string(),
    )
}

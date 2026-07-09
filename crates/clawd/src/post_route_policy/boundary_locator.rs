use crate::{OutputLocatorKind, RouteResult};
use std::path::Path;

pub(super) fn locator_kind_is_current_workspace(kind: OutputLocatorKind) -> bool {
    matches!(kind, OutputLocatorKind::CurrentWorkspace)
}

pub(super) fn locator_kind_requires_path_binding(kind: OutputLocatorKind) -> bool {
    matches!(
        kind,
        OutputLocatorKind::Path | OutputLocatorKind::CurrentWorkspace | OutputLocatorKind::Filename
    )
}

pub(super) fn service_status_locator_hint_satisfies_non_path_binding(
    route_result: &RouteResult,
) -> bool {
    route_reason_has_marker(route_result, "service_status")
        && !route_result.output_contract.locator_hint.trim().is_empty()
}

pub(super) fn path_is_existing_directory(path: &str) -> bool {
    let trimmed = path.trim();
    !trimmed.is_empty() && Path::new(trimmed).is_dir()
}

fn route_requires_database_file_locator(route_result: &RouteResult) -> bool {
    route_reason_has_marker(route_result, "sqlite_table_listing")
        || route_reason_has_marker(route_result, "sqlite_table_names_only")
        || route_reason_has_marker(route_result, "sqlite_database_kind_judgment")
        || route_reason_has_marker(route_result, "sqlite_schema_version")
}

pub(super) fn direct_locator_path_is_unsuitable_for_contract(
    route_result: &RouteResult,
    path: &str,
) -> bool {
    route_requires_database_file_locator(route_result) && path_is_existing_directory(path)
}

pub(super) fn current_workspace_content_summary_requires_concrete_locator(
    route_result: &RouteResult,
) -> bool {
    route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && route_result.output_contract.locator_kind == OutputLocatorKind::CurrentWorkspace
        && (route_reason_has_marker(route_result, "content_excerpt_summary")
            || route_reason_has_marker(route_result, "content_excerpt_with_summary")
            || route_reason_has_marker(route_result, "excerpt_kind_judgment"))
}

pub(super) fn route_reason_has_marker(route_result: &RouteResult, marker: &str) -> bool {
    route_result
        .route_reason
        .split(';')
        .any(|part| part.trim() == marker)
}

fn path_without_parent_components(path: &Path) -> bool {
    !path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
}

fn relative_locator_hint_is_specific_path(path: &Path) -> bool {
    path_without_parent_components(path)
        && path
            .components()
            .filter(|component| {
                matches!(
                    component,
                    std::path::Component::Normal(_) | std::path::Component::Prefix(_)
                )
            })
            .count()
            >= 2
}

fn normalize_path_for_identity(path: &Path) -> std::path::PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub(super) fn locator_hint_matches_direct_locator(
    route_result: &RouteResult,
    direct_locator_path: Option<&str>,
) -> bool {
    let hint = route_result.output_contract.locator_hint.trim();
    let Some(direct_locator_path) = direct_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    if hint.is_empty() || hint.contains('\n') {
        return false;
    }
    let hint_path = Path::new(hint);
    let direct_path = Path::new(direct_locator_path);
    if !path_without_parent_components(hint_path) {
        return false;
    }
    if hint_path.is_absolute() {
        return normalize_path_for_identity(hint_path) == normalize_path_for_identity(direct_path);
    }
    relative_locator_hint_is_specific_path(hint_path)
        && (direct_path.ends_with(hint_path)
            || hint_path
                .canonicalize()
                .is_ok_and(|hint| hint == normalize_path_for_identity(direct_path)))
}

pub(super) fn direct_auto_locator_can_satisfy_background_clarify(
    route_result: &RouteResult,
    direct_locator_path: Option<&str>,
) -> bool {
    if !route_reason_has_marker(route_result, "clarify_reason_code:missing_read_target") {
        return true;
    }
    if route_reason_has_marker(route_result, "filesystem_mutation_result") {
        return filesystem_mutation_locator_can_satisfy_missing_read_target(
            route_result,
            direct_locator_path,
        );
    }
    if route_reason_has_marker(route_result, "archive_unpack") {
        return archive_locator_can_satisfy_missing_read_target(route_result, direct_locator_path);
    }
    if route_reason_has_marker(route_result, "content_excerpt_summary")
        || route_reason_has_marker(route_result, "content_excerpt_with_summary")
        || route_reason_has_marker(route_result, "content_presence_check")
        || route_reason_has_marker(route_result, "document_heading")
        || route_reason_has_marker(route_result, "excerpt_kind_judgment")
    {
        return locator_hint_matches_direct_locator(route_result, direct_locator_path);
    }
    true
}

fn filesystem_mutation_locator_can_satisfy_missing_read_target(
    route_result: &RouteResult,
    direct_locator_path: Option<&str>,
) -> bool {
    let Some(path) = direct_locator_path else {
        return false;
    };
    if !locator_hint_matches_direct_locator(route_result, Some(path)) {
        return false;
    }
    if route_result
        .output_contract
        .self_extension
        .structured_field_selector
        .is_some()
    {
        return true;
    }
    !path_is_existing_directory(path)
}

fn archive_locator_can_satisfy_missing_read_target(
    route_result: &RouteResult,
    direct_locator_path: Option<&str>,
) -> bool {
    let Some(path) = direct_locator_path else {
        return false;
    };
    locator_hint_matches_direct_locator(route_result, Some(path)) && path_looks_like_archive(path)
}

fn path_looks_like_archive(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    lower.ends_with(".zip")
        || lower.ends_with(".tar")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".tar.bz2")
        || lower.ends_with(".tbz2")
        || lower.ends_with(".tar.xz")
        || lower.ends_with(".txz")
        || lower.ends_with(".gz")
        || lower.ends_with(".bz2")
        || lower.ends_with(".xz")
}

pub(super) fn legacy_semantic_repair_requested(
    force_clarify: bool,
    missing_locator_for_path_scoped_content: bool,
    fuzzy_locator_requires_clarify: bool,
    background_locator_clarify: bool,
    direct_auto_locator_satisfies_background_clarify: bool,
) -> bool {
    force_clarify
        && !missing_locator_for_path_scoped_content
        && !fuzzy_locator_requires_clarify
        && !(background_locator_clarify && direct_auto_locator_satisfies_background_clarify)
}

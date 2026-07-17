use super::{bounded_context_segment, format_last_turn_full_context};

#[test]
fn bounded_context_segment_marks_truncation_within_budget() {
    let segment =
        bounded_context_segment("constraint:no_external_publish constraint:read_only", 32);

    assert!(segment.ends_with("...(truncated)"));
    assert!(segment.len() <= 32);
}

#[test]
fn last_turn_total_truncation_is_explicit_and_closed() {
    let context = format_last_turn_full_context(
        "goal:atlas_release constraint:no_external_publish",
        "fact:build_green decision:minimax_primary",
        128,
        64,
    );

    assert!(context.contains("...(truncated)"));
    assert!(context.ends_with("[/LAST_TURN_FULL]"));
    assert!(context.len() <= 64);
}

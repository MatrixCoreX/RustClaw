use super::*;

#[test]
fn admission_timestamp_has_a_bounded_freshness_window() {
    assert!(timestamp_is_fresh(1_000, 1_000));
    assert!(timestamp_is_fresh(
        1_000,
        1_000 - CHANNEL_EVENT_ADMISSION_SIGNATURE_TOLERANCE_SECS
    ));
    assert!(!timestamp_is_fresh(1_000, 0));
    assert!(!timestamp_is_fresh(
        1_000,
        999 - CHANNEL_EVENT_ADMISSION_SIGNATURE_TOLERANCE_SECS
    ));
}

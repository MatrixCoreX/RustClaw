use super::detect_with;

#[test]
fn machine_redirected_and_minimal_modes_disable_visual_effects() {
    for capabilities in [
        detect_with(true, true, true, false, Some("xterm"), Some("120")),
        detect_with(false, true, false, false, Some("xterm"), Some("120")),
        detect_with(true, true, false, false, Some("dumb"), Some("120")),
        detect_with(true, true, false, true, Some("xterm"), Some("120")),
    ] {
        assert!(!capabilities.animation);
        assert!(!capabilities.color);
    }
}

#[test]
fn width_is_bounded_and_machine_readable() {
    assert_eq!(
        detect_with(true, true, false, false, Some("xterm"), Some("120")).width,
        Some(120)
    );
    assert_eq!(
        detect_with(true, true, false, false, Some("xterm"), Some("8")).width,
        None
    );
    assert_eq!(
        detect_with(true, true, false, false, Some("xterm"), Some("wide")).width,
        None
    );
}

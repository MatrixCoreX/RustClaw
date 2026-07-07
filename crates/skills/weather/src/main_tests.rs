use super::*;

#[test]
fn location_display_prefers_user_supplied_place() {
    assert_eq!(
        weather_location_display(
            Some("北京"),
            Some("Beijing"),
            "Beijing, Beijing Municipality, China"
        ),
        "北京"
    );
}

#[test]
fn location_display_uses_city_when_display_location_missing() {
    assert_eq!(
        weather_location_display(
            None,
            Some("Beijing"),
            "Beijing, Beijing Municipality, China"
        ),
        "Beijing"
    );
}

#[test]
fn location_display_falls_back_to_resolved_place() {
    assert_eq!(
        weather_location_display(None, None, "Shanghai, Shanghai Municipality, China"),
        "Shanghai, Shanghai Municipality, China"
    );
}

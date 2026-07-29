use super::*;

#[test]
fn preview_query_normalizes_forecast_without_external_calls() {
    let cat = TextCatalog {
        current: default_embedded_strings(),
    };
    let args = json!({
        "action": "preview_query",
        "city": "Nanjing",
        "display_location": "Nanjing",
        "days": 30
    });

    let (_, extra) = execute(&args, &cat, "en-US").unwrap();

    assert_eq!(extra["action"], "preview_query");
    assert_eq!(extra["mode"], "daily");
    assert_eq!(extra["forecast_days_requested"], 30);
    assert_eq!(extra["forecast_days_applied"], MAX_FORECAST_DAYS);
    assert_eq!(extra["forecast_days_capped"], true);
    assert_eq!(extra["geocode_required"], true);
    assert_eq!(extra["would_execute"], false);
    assert_eq!(extra["external_call_count"], 0);
}

#[test]
fn preview_query_rejects_invalid_coordinates_before_external_calls() {
    let cat = TextCatalog {
        current: default_embedded_strings(),
    };
    let args = json!({
        "action": "preview_query",
        "latitude": 91.0,
        "longitude": 0.0
    });

    assert_eq!(
        execute(&args, &cat, "en-US").unwrap_err(),
        "code=invalid_coordinates"
    );
}

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_code"], "execution_failed");
    assert_eq!(extra["message_key"], "skill.weather.execution_failed");
    assert_eq!(extra["retryable"], false);
}

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

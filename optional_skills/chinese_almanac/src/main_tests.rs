use super::*;

fn request(args: Value) -> String {
    json!({
        "request_id": "test-1",
        "args": args,
        "context": null,
        "user_id": 1,
        "chat_id": 1
    })
    .to_string()
}

#[test]
fn spring_festival_2024_has_expected_lunar_date() {
    let response = handle_line(&request(json!({"date": "2024-02-10"})));
    assert_eq!(response.status, "ok");
    assert_eq!(response.extra["lunar"]["year"], 2024);
    assert_eq!(response.extra["lunar"]["month"], 1);
    assert_eq!(response.extra["lunar"]["day"], 1);
    assert_eq!(response.extra["lunar"]["is_leap_month"], false);
    assert_eq!(response.extra["ganzhi"]["year"], "甲辰");
    assert_eq!(response.extra["zodiac"], "龙");
    assert!(response.extra["festivals"]["lunar"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "春节"));
}

#[test]
fn offset_days_resolves_relative_date() {
    let response = handle_line(&request(json!({
        "date": "2024-02-09",
        "offset_days": 1,
        "detail": "summary"
    })));
    assert_eq!(response.status, "ok");
    assert_eq!(response.extra["date"], "2024-02-10");
    assert_eq!(response.extra["detail"], "summary");
}

#[test]
fn full_result_contains_structured_almanac_and_disclaimer() {
    let response = handle_line(&request(json!({"date": "2026-08-01"})));
    assert_eq!(response.status, "ok");
    assert!(response.extra["almanac"]["yi"].is_array());
    assert!(response.extra["almanac"]["ji"].is_array());
    assert_eq!(response.extra["basis"]["offline"], true);
    assert!(response.text.contains("传统民俗信息"));
}

#[test]
fn invalid_date_returns_canonical_error_contract() {
    let response = handle_line(&request(json!({"date": "2024-02-30"})));
    assert_eq!(response.status, "error");
    assert_eq!(response.extra["status"], "error");
    assert_eq!(response.extra["error_code"], "invalid_date");
    assert_eq!(
        response.extra["message_key"],
        "skill.chinese_almanac.invalid_date"
    );
    assert_eq!(response.extra["retryable"], false);
}

#[test]
fn component_date_requires_all_three_fields() {
    let response = handle_line(&request(json!({"year": 2024, "month": 2})));
    assert_eq!(response.status, "error");
    assert_eq!(response.extra["error_code"], "incomplete_date");
}

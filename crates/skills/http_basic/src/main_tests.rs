use super::http_observation;

#[test]
fn http_non_success_response_is_structured_observation() {
    let (text, extra) = http_observation(
        "get",
        "http://127.0.0.1:8787/missing",
        404,
        false,
        "not found",
    );

    assert_eq!(text, "status=404\nnot found");
    assert_eq!(extra.get("status_code").and_then(|v| v.as_u64()), Some(404));
    assert_eq!(
        extra.get("success_status").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        extra.get("body_preview").and_then(|v| v.as_str()),
        Some("not found")
    );
}

use super::*;

#[test]
fn late_tools_and_usage_survive_a_full_raw_prefix() {
    let mut capture = StreamCapture::default();
    for _ in 0..6000 {
        capture.observe(&json!({"choices":[{"delta":{}}]}));
    }
    let calls = json!([{"index":0,"id":"tool-1","function":{"name":"fixture","arguments":"{\"value\":\"文字\"}"}}]);
    capture.observe(&json!({"choices":[{"index":0,"delta":{"tool_calls":calls}}]}));
    capture.observe(&json!({"choices":[{"index":0,"finish_reason":"tool_calls"}]}));
    capture.observe(
        &json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":8,"total_tokens":11}}),
    );
    let record = capture.record(100, true);
    assert_eq!(
        record["tool_frames"][0]["choices"][0]["delta"]["tool_calls"],
        calls
    );
    assert_eq!(
        record["terminal"]["choices"][0]["finish_reason"],
        "tool_calls"
    );
    assert_eq!(record["terminal"]["usage"]["total_tokens"], 11);
    assert_eq!(record["raw_prefix_truncated"], true);
    assert_eq!(record["tool_frames_complete"], true);
    assert!(record.to_string().len() < 2000);
}

#[test]
fn oversized_tools_are_explicitly_incomplete_without_losing_terminal() {
    let mut capture = StreamCapture::default();
    capture.observe(&json!({"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"x".repeat(TOOL_FRAME_BYTES)}}]}}]}));
    capture.observe(&json!({"choices":[{"finish_reason":"length"}]}));
    let record = capture.record(1, true);
    assert_eq!(record["omitted_tool_frames"], 1);
    assert_eq!(record["tool_frames_complete"], false);
    assert_eq!(record["terminal"]["choices"][0]["finish_reason"], "length");
    assert!(record.to_string().len() < TOOL_FRAME_BYTES + TERMINAL_BYTES + 2048);
}

#[test]
fn partial_and_oversized_terminal_data_are_not_reported_complete() {
    let mut capture = StreamCapture::default();
    capture.observe(&json!({"usage":{"oversized":"x".repeat(TERMINAL_BYTES)}}));
    let record = capture.record(1, false);
    assert_eq!(record["stream_complete"], false);
    assert_eq!(record["tool_frames_complete"], false);
    assert_eq!(record["terminal_truncated"], true);
}

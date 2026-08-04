use super::*;

#[test]
fn findings_are_closed_machine_fields_and_block_only_on_error() {
    let result = json!({"result":{"review_findings":[{
        "severity":"error",
        "file":"src/main.rs",
        "line_range":{"start":4,"end":8},
        "finding_code":"unchecked_result",
        "message_key":"review.unchecked_result",
        "suggestion_ref":"task:t:evidence:1",
        "prose":"discard me"
    }]}});
    let observation = review_observation("task-1", "review", &result, true);
    assert_eq!(
        observation["policy_decision"],
        PolicyDecision::RequireConfirmation.as_token()
    );
    assert!(observation["review_findings"][0].get("prose").is_none());
}

#[test]
fn nonblocking_review_never_blocks_delivery() {
    let result = json!({"review_findings":[{
        "severity":"error","file":"src/lib.rs",
        "line_range":{"start":1,"end":1},
        "finding_code":"test","message_key":"review.test"
    }]});
    assert_eq!(
        review_observation("task-1", "review", &result, false)["policy_decision"],
        PolicyDecision::Allow.as_token()
    );
}

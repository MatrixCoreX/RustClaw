use super::super::execution_evidence_prompt_block;
use serde_json::{json, Value};

#[test]
fn planner_repair_evidence_retains_machine_history_not_arbitrary_observation_text() {
    let mut journal = crate::task_journal::TaskJournal::default();
    journal.task_observations.push(json!({
        "owner_layer": "tool", "reason_code": "native_plan_contract_repair_progress",
        "repair_attempt": 999
    }));
    journal.task_observations.push(json!({
        "owner_layer": "planner", "reason_code": "unrelated_event", "text": "ignored"
    }));
    for attempt in 1..=35 {
        journal.task_observations.push(json!({
            "owner_layer": "planner", "state": "progress",
            "reason_code": "native_plan_contract_repair_progress",
            "source_error_code": "native_plan_required_args_missing",
            "repair_attempt": attempt,
            "prompt": "not projected", "arguments": {"content": "not projected"}
        }));
    }
    let block: Value = serde_json::from_str(&execution_evidence_prompt_block(&journal)).unwrap();
    let repairs = &block["planner_repair_evidence"];
    assert_eq!(repairs["observed_events"], 35);
    assert_eq!(repairs["truncated"], true);
    let events = repairs["events"].as_array().unwrap();
    assert_eq!(events.len(), 32);
    assert_eq!(events.first().unwrap()["repair_attempt"], 4);
    assert_eq!(events.last().unwrap()["repair_attempt"], 35);
    assert_eq!(
        events[0]["source_error_code"],
        "native_plan_required_args_missing"
    );
    assert!(events
        .iter()
        .all(|event| event.get("prompt").is_none() && event.get("arguments").is_none()));
}

use serde_json::{json, Value};

use super::{TaskJournal, TaskJournalFinalStatus};

fn task_event_json(seq: &mut u64, event_type: &'static str, payload: Value) -> Value {
    *seq += 1;
    json!({
        "seq": *seq,
        "event_type": event_type,
        "owner_layer": "task_journal",
        "payload": payload,
    })
}

pub(super) fn task_event_stream_json(journal: &TaskJournal) -> Vec<Value> {
    let mut seq = 0;
    let mut events = Vec::new();
    if let Some(lifecycle) = journal.task_lifecycle.as_ref() {
        events.push(task_event_json(
            &mut seq,
            "task_lifecycle",
            lifecycle.clone(),
        ));
    }
    for round in &journal.rounds {
        events.push(task_event_json(
            &mut seq,
            "agent_round",
            json!({
                "round_no": round.round_no,
                "has_plan": round.plan_result.is_some(),
                "has_verify": round.verify_result.is_some(),
                "execution_recipe_summary_present": round.execution_recipe_summary.is_some(),
            }),
        ));
    }
    for (index, step) in journal.step_results.iter().enumerate() {
        events.push(task_event_json(
            &mut seq,
            "tool_step",
            json!({
                "index": index,
                "step_id": step.step_id,
                "skill": step.skill,
                "status": step.status.as_str(),
                "started_at": step.started_at,
                "finished_at": step.finished_at,
            }),
        ));
    }
    for (index, observation) in journal.task_observations.iter().enumerate() {
        events.push(task_event_json(
            &mut seq,
            "task_observation",
            json!({
                "index": index,
                "observation": observation,
            }),
        ));
    }
    if journal.final_status.is_some() || journal.final_stop_signal.is_some() {
        events.push(task_event_json(
            &mut seq,
            "task_final",
            json!({
                "final_status": journal.final_status.map(TaskJournalFinalStatus::as_str),
                "final_stop_signal": journal.final_stop_signal.as_deref(),
                "final_failure_attribution": journal.final_failure_attribution.as_deref(),
            }),
        ));
    }
    events
}

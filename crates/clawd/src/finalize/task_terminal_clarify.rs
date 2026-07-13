use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalClarifyIntent {
    content: Option<String>,
    clarify_reason_code: Option<String>,
    missing_slot: Option<String>,
    message_key: Option<String>,
    field_path: Option<String>,
    locator_kind: Option<String>,
}

pub(super) fn preserve_terminal_clarify_from_journal(
    journal: &crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
) -> bool {
    if answer_has_terminal_clarify_machine_fields(answer_text, answer_messages) {
        return false;
    }
    let Some(intent) = latest_terminal_clarify_intent(journal) else {
        return false;
    };
    let base = answer_text
        .trim()
        .is_empty()
        .then(|| intent.content.as_deref().unwrap_or(""))
        .unwrap_or_else(|| answer_text.trim())
        .trim();
    if base.is_empty() {
        return false;
    }
    *answer_text = base.to_string();
    answer_messages.clear();
    answer_messages.push(answer_text.clone());
    true
}

fn latest_terminal_clarify_intent(
    journal: &crate::task_journal::TaskJournal,
) -> Option<TerminalClarifyIntent> {
    for plan in journal
        .rounds
        .iter()
        .rev()
        .filter_map(|round| round.plan_result.as_ref())
    {
        match terminal_respond_class_from_plan(plan) {
            TerminalRespondClass::Clarify(intent) => return Some(intent),
            TerminalRespondClass::Answer => return None,
            TerminalRespondClass::None => {}
        }
    }
    journal
        .plan_result
        .as_ref()
        .and_then(|plan| match terminal_respond_class_from_plan(plan) {
            TerminalRespondClass::Clarify(intent) => Some(intent),
            TerminalRespondClass::Answer | TerminalRespondClass::None => None,
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TerminalRespondClass {
    Clarify(TerminalClarifyIntent),
    Answer,
    None,
}

fn terminal_respond_class_from_plan(plan: &crate::PlanResult) -> TerminalRespondClass {
    let raw_steps = raw_plan_steps(&plan.raw_plan_text);
    if let Some(intent) = plan
        .steps
        .iter()
        .find_map(terminal_clarify_intent_from_plan_step)
        .or_else(|| {
            raw_steps
                .iter()
                .find_map(terminal_clarify_intent_from_raw_step)
        })
    {
        return TerminalRespondClass::Clarify(intent);
    }
    if plan.steps.iter().any(plan_step_is_non_clarify_respond)
        || raw_steps.iter().any(raw_step_is_non_clarify_respond)
    {
        return TerminalRespondClass::Answer;
    }
    TerminalRespondClass::None
}

fn terminal_clarify_intent_from_plan_step(step: &crate::PlanStep) -> Option<TerminalClarifyIntent> {
    if !plan_step_is_respond(step) {
        return None;
    }
    terminal_clarify_intent_from_object(&step.args)
}

fn terminal_clarify_intent_from_raw_step(step: &Value) -> Option<TerminalClarifyIntent> {
    let raw_type = string_field(step, &["type", "action_type", "action"])?.to_ascii_lowercase();
    (raw_type == "respond").then(|| terminal_clarify_intent_from_object(step))?
}

fn plan_step_is_respond(step: &crate::PlanStep) -> bool {
    step.action_type == "respond"
}

fn plan_step_is_non_clarify_respond(step: &crate::PlanStep) -> bool {
    plan_step_is_respond(step) && !object_terminal_intent_is_clarify(&step.args)
}

fn raw_step_is_non_clarify_respond(step: &Value) -> bool {
    let Some(raw_type) = string_field(step, &["type", "action_type", "action"]) else {
        return false;
    };
    raw_type.eq_ignore_ascii_case("respond") && !object_terminal_intent_is_clarify(step)
}

fn object_terminal_intent_is_clarify(value: &Value) -> bool {
    string_field(value, &["terminal_intent"])
        .is_some_and(|terminal_intent| terminal_intent.eq_ignore_ascii_case("clarify"))
}

fn terminal_clarify_intent_from_object(value: &Value) -> Option<TerminalClarifyIntent> {
    let terminal_intent = string_field(value, &["terminal_intent"])?.to_ascii_lowercase();
    (terminal_intent == "clarify").then(|| TerminalClarifyIntent {
        content: string_field(value, &["content"]).map(str::to_string),
        clarify_reason_code: string_field(value, &["clarify_reason_code"]).map(str::to_string),
        missing_slot: string_field(value, &["missing_slot"]).map(str::to_string),
        message_key: string_field(value, &["message_key"]).map(str::to_string),
        field_path: string_field(value, &["field_path"]).map(str::to_string),
        locator_kind: string_field(value, &["locator_kind"]).map(str::to_string),
    })
}

fn raw_plan_steps(raw_plan_text: &str) -> Vec<Value> {
    let Some(value) =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw_plan_text)
    else {
        return Vec::new();
    };
    if let Some(steps) = value.get("steps").and_then(Value::as_array) {
        return steps.clone();
    }
    if let Some(actions) = value.get("actions").and_then(Value::as_array) {
        return actions.clone();
    }
    value.as_array().cloned().unwrap_or_default()
}

fn string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn answer_has_terminal_clarify_machine_fields(text: &str, messages: &[String]) -> bool {
    std::iter::once(text)
        .chain(messages.iter().map(String::as_str))
        .any(has_terminal_clarify_machine_fields)
}

fn has_terminal_clarify_machine_fields(raw: &str) -> bool {
    let trimmed = raw.trim();
    if let Ok(payload) = serde_json::from_str::<Value>(trimmed) {
        if payload
            .get("terminal_intent")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "clarify")
        {
            return true;
        }
    }
    let markers = crate::RouteReasonMarkers::new(trimmed);
    markers.machine_value("terminal_intent") == Some("clarify")
        || markers.machine_value("agent_loop.terminal_intent") == Some("clarify")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plan_with_raw(raw_plan_text: &str) -> crate::PlanResult {
        crate::PlanResult {
            goal: "missing locator".to_string(),
            missing_slots: Vec::new(),
            needs_confirmation: false,
            steps: vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "respond".to_string(),
                skill: "respond".to_string(),
                args: json!({"content": "Which file do you want me to read?"}),
                depends_on: Vec::new(),
                why: "respond".to_string(),
            }],
            planner_notes: String::new(),
            plan_kind: crate::PlanKind::Incremental,
            raw_plan_text: raw_plan_text.to_string(),
        }
    }

    #[test]
    fn preserves_latest_raw_plan_terminal_clarify_user_text_only() {
        let raw_plan = r#"{"steps":[{"type":"respond","terminal_intent":"clarify","clarify_reason_code":"missing_locator","missing_slot":"locator","locator_kind":"path","content":"Which file do you want me to read?"}]}"#;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-1", "ask", "missing locator");
        journal
            .rounds
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 2,
                goal: "missing locator".to_string(),
                plan_result: Some(plan_with_raw(raw_plan)),
                ..Default::default()
            });
        let mut answer_text = "Which file do you want me to read?".to_string();
        let mut answer_messages = vec![answer_text.clone()];

        assert!(preserve_terminal_clarify_from_journal(
            &journal,
            &mut answer_text,
            &mut answer_messages
        ));

        assert!(!answer_text.trim().is_empty());
        assert!(!answer_text.contains("terminal_intent=clarify"));
        assert!(!answer_text.contains("clarify_reason_code=missing_locator"));
        assert!(!answer_text.contains("missing_slot=locator"));
        assert!(!answer_text.contains("locator_kind=path"));
        assert_eq!(answer_messages, vec![answer_text]);
    }

    #[test]
    fn newer_non_clarify_answer_stops_old_clarify_recovery() {
        let clarify_raw = r#"{"steps":[{"type":"respond","terminal_intent":"clarify","clarify_reason_code":"missing_topic","missing_slot":"topic","content":"Need topic"}]}"#;
        let answer_raw = r#"{"steps":[{"type":"respond","content":"Draft answer"}]}"#;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-2", "ask", "draft request");
        journal
            .rounds
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "draft request".to_string(),
                plan_result: Some(plan_with_raw(clarify_raw)),
                ..Default::default()
            });
        journal
            .rounds
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 2,
                goal: "draft request".to_string(),
                plan_result: Some(plan_with_raw(answer_raw)),
                ..Default::default()
            });
        let mut answer_text = "Draft answer".to_string();
        let mut answer_messages = vec![answer_text.clone()];

        assert!(!preserve_terminal_clarify_from_journal(
            &journal,
            &mut answer_text,
            &mut answer_messages
        ));
        assert_eq!(answer_text, "Draft answer");
        assert_eq!(answer_messages, vec!["Draft answer".to_string()]);
    }
}

use crate::PlanStep;

use super::{VerifyIssue, VerifyIssueKind};

pub(super) fn session_alias_reference_issue(
    request_text: Option<&str>,
    context_bundle_summary: Option<&str>,
    step: &PlanStep,
    normalized_skill: &str,
) -> Option<VerifyIssue> {
    if normalized_skill != "task_control"
        || step.args.get("action").and_then(serde_json::Value::as_str) != Some("bind_session_alias")
    {
        return None;
    }

    let request_text = request_text?.trim();
    let context_bundle_summary = context_bundle_summary?;
    let requested_alias = step.args.get("alias")?.as_str()?.trim();
    let requested_target = step.args.get("target")?.as_str()?.trim();
    let requested_target_kind = step
        .args
        .get("target_kind")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if request_text.is_empty() || requested_alias.is_empty() || requested_target.is_empty() {
        return None;
    }

    if requested_alias == requested_target {
        return Some(invalid_alias_issue(
            &step.step_id,
            "error_code=session_alias_alias_equals_target field=target constraint=distinct_machine_target",
            "target",
        ));
    }
    if !crate::conversation_state::session_alias_target_shape_is_valid(
        requested_target,
        requested_target_kind,
    ) {
        return Some(invalid_alias_issue(
            &step.step_id,
            "error_code=session_alias_target_invalid field=target_kind constraint=typed_machine_target",
            "target_kind",
        ));
    }

    let bindings =
        crate::agent_engine::session_alias_bindings_from_context_summary(context_bundle_summary);
    let mentioned =
        crate::conversation_state::alias_bindings_mentioned_in_prompt(&bindings, request_text);
    if mentioned.is_empty()
        || mentioned
            .iter()
            .all(|binding| binding.target.trim() == requested_target)
        || mentioned
            .iter()
            .any(|binding| binding.alias.trim() == requested_alias)
    {
        return None;
    }

    Some(invalid_alias_issue(
        &step.step_id,
        "error_code=session_alias_rebind_key_mismatch field=alias constraint=existing_alias_exact",
        "alias",
    ))
}

fn invalid_alias_issue(step_id: &str, detail: &str, field: &str) -> VerifyIssue {
    VerifyIssue {
        step_id: step_id.to_string(),
        kind: VerifyIssueKind::InvalidArgumentValue,
        detail: detail.to_string(),
        missing_fields: vec![field.to_string()],
    }
}

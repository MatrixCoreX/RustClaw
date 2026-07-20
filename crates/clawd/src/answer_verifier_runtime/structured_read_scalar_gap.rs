use super::*;

pub(in crate::answer_verifier) fn scalar_field_value_gap_is_grounded_in_structured_read(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
    gap: &AnswerVerifierOut,
) -> bool {
    if gap.missing_evidence_fields.len() != 1
        || gap.missing_evidence_fields.first().map(String::as_str) != Some("field_value")
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || !route.output_contract_is_unclassified()
    {
        return false;
    }
    let Some(shape) =
        crate::evidence_policy::final_answer_shape_for_output_contract(&route.output_contract)
    else {
        return false;
    };
    if shape.class() != crate::evidence_policy::FinalAnswerShapeClass::ScalarValue
        || !scalar_answer_is_strict(candidate_answer)
    {
        return false;
    }
    journal.step_results.iter().any(|step| {
        step_can_supply_verifier_observation_for_route(route, step)
            && step_can_supply_strict_evidence_for_route(route, step)
            && step.output_excerpt.as_deref().is_some_and(|output| {
                structured_read_output_contains_scalar_answer(output, candidate_answer)
            })
    })
}

pub(in crate::answer_verifier) fn structured_read_output_contains_scalar_answer(
    output: &str,
    candidate_answer: &str,
) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return false;
    };
    structured_read_json_contains_scalar_answer(&value, candidate_answer)
}

pub(in crate::answer_verifier) fn structured_read_json_contains_scalar_answer(
    value: &serde_json::Value,
    candidate_answer: &str,
) -> bool {
    let action = value
        .get("action")
        .or_else(|| value.pointer("/extra/action"))
        .and_then(|value| value.as_str())
        .map(str::trim);
    if !matches!(action, Some("read_range" | "read_text_range")) {
        return false;
    }
    [
        value.get("excerpt").and_then(|value| value.as_str()),
        value
            .pointer("/extra/excerpt")
            .and_then(|value| value.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(read_range_excerpt_without_line_prefixes)
    .filter_map(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
    .any(|document| json_value_contains_scalar_answer(&document, candidate_answer.trim()))
}

pub(in crate::answer_verifier) fn read_range_excerpt_without_line_prefixes(
    excerpt: &str,
) -> String {
    excerpt
        .lines()
        .map(strip_read_range_line_prefix)
        .collect::<Vec<_>>()
        .join("\n")
}

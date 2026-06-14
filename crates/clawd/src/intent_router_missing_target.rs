use crate::{
    ActFinalizeStyle, FirstLayerDecision, IntentOutputContract, OutputDeliveryIntent,
    OutputLocatorKind, OutputSemanticKind,
};

fn reason_text_has_marker(reason: &str, marker: &str) -> bool {
    reason.split(';').map(str::trim).any(|part| {
        part == marker
            || part
                .rsplit_once(':')
                .is_some_and(|(_, suffix)| suffix.trim() == marker)
    })
}

pub(super) fn apply_missing_read_target_mutation_clarify(
    reason: &str,
    output_contract: &mut IntentOutputContract,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if *needs_clarify
        || !reason_text_has_marker(reason, "clarify_reason_code:missing_read_target")
        || !output_contract.requires_content_evidence
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::FilesystemMutationResult | OutputSemanticKind::ArchiveUnpack
        )
    {
        return None;
    }
    *needs_clarify = true;
    clarify_question.clear();
    *first_layer_decision = FirstLayerDecision::Clarify;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    Some("missing_read_target_mutation_clarify")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OutputResponseShape;

    #[test]
    fn missing_read_target_mutation_contract_forces_clarify() {
        let mut contract = IntentOutputContract {
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            delivery_intent: OutputDeliveryIntent::None,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "/tmp/unpack_dest".to_string(),
            semantic_kind: OutputSemanticKind::FilesystemMutationResult,
            ..IntentOutputContract::default()
        };
        let mut needs_clarify = false;
        let mut clarify_question = String::new();
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize_style = ActFinalizeStyle::ChatWrapped;

        let reason = apply_missing_read_target_mutation_clarify(
            "semantic_contract_requires_evidence; clarify_reason_code:missing_read_target",
            &mut contract,
            &mut needs_clarify,
            &mut clarify_question,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, Some("missing_read_target_mutation_clarify"));
        assert!(needs_clarify);
        assert_eq!(decision, FirstLayerDecision::Clarify);
        assert_eq!(finalize_style, ActFinalizeStyle::Plain);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
        assert!(contract.locator_hint.is_empty());
        assert!(contract.requires_content_evidence);
    }
}

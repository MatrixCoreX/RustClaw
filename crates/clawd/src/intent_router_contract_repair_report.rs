use std::collections::BTreeSet;

#[derive(Clone, Debug, Default)]
pub(super) struct ContractRepairReport {
    pub(super) sources: BTreeSet<&'static str>,
    pub(super) details: BTreeSet<&'static str>,
}

impl ContractRepairReport {
    pub(super) fn add(&mut self, source: &'static str, detail: &'static str) {
        self.sources.insert(source);
        self.details.insert(detail);
    }

    pub(super) fn source_csv(&self) -> String {
        if self.sources.is_empty() {
            "none".to_string()
        } else {
            self.sources.iter().copied().collect::<Vec<_>>().join(",")
        }
    }

    pub(super) fn detail_csv(&self) -> String {
        if self.details.is_empty() {
            "none".to_string()
        } else {
            self.details.iter().copied().collect::<Vec<_>>().join(",")
        }
    }

    pub(super) fn has_detail(&self, detail: &'static str) -> bool {
        self.details.contains(detail)
    }

    pub(super) fn needs_llm_contract_integrity_repair(&self) -> bool {
        if self.sources.contains("tool_payload") {
            return true;
        }
        if !self.sources.contains("conservative_none") {
            return self
                .details
                .iter()
                .copied()
                .any(contract_repair_detail_requires_llm_integrity_repair);
        }
        self.details
            .iter()
            .copied()
            .any(|detail| detail != "execution_recipe_untrusted_text_ignored")
    }

    pub(super) fn merge(&mut self, other: &Self) {
        self.sources.extend(other.sources.iter().copied());
        self.details.extend(other.details.iter().copied());
    }
}

fn contract_repair_detail_requires_llm_integrity_repair(detail: &str) -> bool {
    matches!(
        detail,
        "executable_route_unknown_scalar_output_contract"
            | "answer_candidate_memory_only_binding"
            | "active_task_answer_candidate_conflict"
            | "active_task_invalid_turn_binding"
            | "active_ordered_scalar_path_missing_ordered_entry_ref"
            | "chat_route_with_file_delivery_request"
            | "chat_route_requires_content_evidence"
            | "chat_route_requires_delivery"
            | "chat_route_has_observable_semantic_kind"
            | "chat_route_has_observable_locator"
            | "raw_command_output_locator_needs_semantic_review"
            | "locatorless_generic_evidence_contract_needs_semantic_shape_review"
    )
}

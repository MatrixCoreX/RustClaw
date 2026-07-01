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

    pub(super) fn class_csv(&self) -> String {
        let mut classes = BTreeSet::new();
        for source in &self.sources {
            classes.insert(contract_repair_source_class(source));
        }
        for detail in &self.details {
            classes.insert(contract_repair_detail_class(detail));
        }
        if classes.is_empty() {
            "none".to_string()
        } else {
            classes.into_iter().collect::<Vec<_>>().join(",")
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

fn contract_repair_source_class(source: &str) -> &'static str {
    match source {
        "command_payload"
        | "enum_alias"
        | "structured_contract"
        | "structured_recipe"
        | "tool_payload" => "schema_normalization",
        "structural_cleanup" => "boundary_safety_repair",
        "conservative_none" => "boundary_safety_repair",
        "semantic_suspect" => "contract_integrity_repair",
        _ => "machine_repair_unclassified",
    }
}

fn contract_repair_detail_class(detail: &str) -> &'static str {
    match detail {
        "execution_recipe_command_payload"
        | "execution_signal_derived_from_output_contract"
        | "execution_recipe_enum"
        | "execution_recipe_fields_normalized"
        | "execution_recipe_health_check_observation"
        | "execution_recipe_package_detect_manager_capability"
        | "execution_recipe_scalar_runtime_tool_observation"
        | "execution_recipe_service_status_observation"
        | "execution_recipe_structured_read_observation"
        | "output_contract_delivery_intent_normalized"
        | "output_contract_response_shape_normalized"
        | "output_contract_semantic_kind_normalized"
        | "turn_type_enum_normalized"
        | "target_task_policy_enum_normalized" => "schema_normalization",
        "output_contract_requires_evidence_repaired" => "schema_normalization",
        "output_contract_locator_kind_normalized" => "machine_locator_repair",
        "execution_recipe_untrusted_text_ignored"
        | "non_object_output_safe_chat_schema"
        | "output_contract_unknown_semantic_ignored"
        | "raw_parse_failed_safe_chat_schema" => "boundary_safety_repair",
        _ => "machine_repair_unclassified",
    }
}

fn contract_repair_detail_requires_llm_integrity_repair(detail: &str) -> bool {
    matches!(
        detail,
        "executable_route_unknown_scalar_output_contract"
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

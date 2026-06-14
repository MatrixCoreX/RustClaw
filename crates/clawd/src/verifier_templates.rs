use std::collections::HashSet;

use crate::PlanStep;

#[derive(Debug, Clone, Default)]
pub(super) struct TemplatePlaceholderScope {
    exact_refs: HashSet<String>,
    indexable_refs: HashSet<String>,
}

impl TemplatePlaceholderScope {
    pub(super) fn register_step_output(&mut self, step: &PlanStep, step_number: usize) {
        self.exact_refs.insert("last_output".to_string());
        self.exact_refs.insert(format!("s{step_number}.output"));
        self.exact_refs
            .insert(format!("{}.last_output", step.step_id.trim()));
        self.indexable_refs.insert("last_output".to_string());
        self.indexable_refs.insert(format!("s{step_number}"));
        self.indexable_refs.insert(step.step_id.trim().to_string());
    }

    fn allows(&self, raw_ref: &str) -> bool {
        let reference = raw_ref.trim();
        if reference.is_empty() {
            return false;
        }
        if self.exact_refs.contains(reference) {
            return true;
        }
        placeholder_indexable_base(reference).is_some_and(|base| self.indexable_refs.contains(base))
    }
}

fn placeholder_indexable_base(reference: &str) -> Option<&str> {
    let dot = reference.find('.');
    let bracket = reference.find('[');
    let split_at = match (dot, bracket) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return None,
    };
    let base = reference[..split_at].trim();
    (!base.is_empty()).then_some(base)
}

fn extract_template_refs(text: &str) -> Option<Vec<String>> {
    let mut refs = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            return None;
        };
        let reference = after_start[..end].trim();
        if reference.is_empty() {
            return None;
        }
        refs.push(reference.to_string());
        rest = &after_start[end + 2..];
    }
    Some(refs)
}

pub(super) fn value_contains_unresolved_template(
    value: &serde_json::Value,
    template_scope: &TemplatePlaceholderScope,
) -> bool {
    match value {
        serde_json::Value::String(text) => {
            let text = text.trim();
            if !(text.contains("{{") || text.contains("}}")) {
                return false;
            }
            let Some(refs) = extract_template_refs(text) else {
                return true;
            };
            refs.is_empty()
                || refs
                    .iter()
                    .any(|reference| !template_scope.allows(reference))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| value_contains_unresolved_template(item, template_scope)),
        serde_json::Value::Object(map) => map
            .values()
            .any(|item| value_contains_unresolved_template(item, template_scope)),
        _ => false,
    }
}

pub(super) fn step_can_produce_output_for_template_scope(step: &PlanStep) -> bool {
    matches!(
        step.action_type.as_str(),
        "call_skill" | "call_tool" | "synthesize_answer"
    )
}

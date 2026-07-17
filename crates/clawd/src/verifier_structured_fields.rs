use crate::{AppState, PlanResult, PlanStep};

use super::{VerifyIssue, VerifyIssueKind};

fn structured_field_target_path(
    state: &AppState,
    output_contract: &crate::IntentOutputContract,
) -> Option<String> {
    if !matches!(
        output_contract.locator_kind,
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
    ) {
        return None;
    }
    let locator = output_contract.locator_hint.trim();
    if locator.is_empty() || locator.contains('\n') {
        return None;
    }
    let path = std::path::Path::new(locator);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(path)
    };
    path.is_file().then(|| path.display().to_string())
}

fn step_field_paths_match_selector(step: &PlanStep, selector: &str) -> bool {
    let Some(obj) = step.args.as_object() else {
        return false;
    };
    if obj
        .get("field_path")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value == selector)
    {
        return true;
    }
    match obj.get("field_paths") {
        Some(serde_json::Value::String(value)) => value == selector,
        Some(serde_json::Value::Array(values)) => {
            values.len() == 1
                && values
                    .first()
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| value == selector)
        }
        _ => false,
    }
}

fn step_path_matches_target(step: &PlanStep, target_path: &str) -> bool {
    step.args
        .as_object()
        .and_then(|obj| obj.get("path"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value == target_path)
}

fn step_has_explicit_path(step: &PlanStep) -> bool {
    step.args
        .as_object()
        .and_then(|obj| obj.get("path"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn step_is_structured_field_read(state: &AppState, step: &PlanStep) -> bool {
    if state.resolve_canonical_skill_name(&step.skill) != "config_basic" {
        return false;
    }
    step.args
        .as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|action| matches!(action, "read_field" | "read_fields"))
}

fn plan_has_multiple_explicit_structured_field_reads(
    state: &AppState,
    plan_result: &PlanResult,
) -> bool {
    plan_result
        .steps
        .iter()
        .filter(|step| {
            step_is_structured_field_read(state, step)
                && step_has_explicit_path(step)
                && !step_field_paths(step).is_empty()
        })
        .take(2)
        .count()
        > 1
}

fn output_contract_allows_structured_field_selector_repair(
    output_contract: &crate::IntentOutputContract,
) -> bool {
    output_contract.requires_content_evidence
        && matches!(
            output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
}

fn normalize_machine_field_selector_token(raw: &str) -> Option<String> {
    let token = raw.trim_matches('.');
    if token.is_empty()
        || token.chars().count() > 256
        || token.contains('/')
        || token.contains('\\')
        || !token.contains('.')
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
    {
        return None;
    }
    for segment in token.split('.') {
        if segment.is_empty()
            || !segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '$'))
        {
            return None;
        }
    }
    Some(token.to_string())
}

fn push_unique_case_insensitive(out: &mut Vec<String>, candidate: String) {
    if !out
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&candidate))
    {
        out.push(candidate);
    }
}

fn dotted_machine_field_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in
        text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '$' | '.')))
    {
        if let Some(candidate) = normalize_machine_field_selector_token(raw) {
            push_unique_case_insensitive(&mut out, candidate);
        }
    }
    out.sort_by_key(|candidate| std::cmp::Reverse(candidate.len()));
    out
}

fn step_field_paths(step: &PlanStep) -> Vec<String> {
    let Some(obj) = step.args.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(field) = obj
        .get("field_path")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        out.push(field.to_string());
    }
    match obj.get("field_paths") {
        Some(serde_json::Value::String(value)) if !value.trim().is_empty() => {
            out.push(value.trim().to_string());
        }
        Some(serde_json::Value::Array(values)) => {
            for value in values {
                if let Some(field) = value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    out.push(field.to_string());
                }
            }
        }
        _ => {}
    }
    out
}

fn selector_refines_planned_field(selector: &str, planned_fields: &[String]) -> bool {
    planned_fields.iter().any(|field| {
        let field = field.trim();
        !field.is_empty()
            && selector.len() > field.len()
            && selector
                .get(..field.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(field))
            && selector
                .as_bytes()
                .get(field.len())
                .is_some_and(|byte| *byte == b'.')
    })
}

fn inferred_selector_from_planned_field_prefix(
    output_contract: &crate::IntentOutputContract,
    request_text: Option<&str>,
    plan_result: &PlanResult,
) -> Option<String> {
    if !output_contract_allows_structured_field_selector_repair(output_contract) {
        return None;
    }
    let candidates = request_text
        .into_iter()
        .flat_map(dotted_machine_field_tokens)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    for step in &plan_result.steps {
        if step.skill != "config_basic" {
            continue;
        }
        let Some(action) = step
            .args
            .as_object()
            .and_then(|obj| obj.get("action"))
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        if !matches!(action, "read_field" | "read_fields") {
            continue;
        }
        let planned_fields = step_field_paths(step);
        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| selector_refines_planned_field(candidate, &planned_fields))
        {
            return Some(candidate.clone());
        }
    }
    None
}

pub(super) fn apply_structured_field_selector_repair(
    state: &AppState,
    output_contract: Option<&crate::IntentOutputContract>,
    request_text: Option<&str>,
    plan_result: &mut PlanResult,
    issues: &mut Vec<VerifyIssue>,
) {
    let Some(output_contract) = output_contract else {
        return;
    };
    let Some(selector) = output_contract
        .selection
        .structured_field_selector
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            inferred_selector_from_planned_field_prefix(output_contract, request_text, plan_result)
        })
    else {
        return;
    };
    let target_path = structured_field_target_path(state, output_contract);
    let preserve_explicit_multi_target_reads =
        plan_has_multiple_explicit_structured_field_reads(state, plan_result);
    for step in &mut plan_result.steps {
        if !step_is_structured_field_read(state, step) {
            continue;
        }
        let Some(action) = step
            .args
            .as_object()
            .and_then(|obj| obj.get("action"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        if !matches!(action.as_str(), "read_field" | "read_fields") {
            continue;
        }
        let field_paths_match = step_field_paths_match_selector(step, &selector);
        let target_path_matches = target_path
            .as_deref()
            .is_some_and(|target_path| step_path_matches_target(step, target_path));
        let explicit_step_would_be_overwritten = !field_paths_match
            || target_path
                .as_deref()
                .is_some_and(|target_path| !step_path_matches_target(step, target_path));
        if preserve_explicit_multi_target_reads
            && step_has_explicit_path(step)
            && !step_field_paths(step).is_empty()
            && explicit_step_would_be_overwritten
        {
            continue;
        }
        let step_id = step.step_id.clone();
        let Some(obj) = step.args.as_object_mut() else {
            continue;
        };
        if !field_paths_match {
            if action == "read_field" {
                obj.insert(
                    "field_path".to_string(),
                    serde_json::Value::String(selector.to_string()),
                );
            } else {
                obj.insert(
                    "field_paths".to_string(),
                    serde_json::json!([selector.clone()]),
                );
            }
            issues.push(VerifyIssue {
                step_id: step_id.clone(),
                kind: VerifyIssueKind::ContractPolicyViolation,
                detail: format!(
                    "structured_field_selector `{selector}` was enforced on config_basic.{action}"
                ),
                missing_fields: Vec::new(),
            });
        }
        if let Some(target_path) = target_path.as_deref() {
            if target_path_matches {
                continue;
            }
            obj.insert(
                "path".to_string(),
                serde_json::Value::String(target_path.to_string()),
            );
            issues.push(VerifyIssue {
                step_id,
                kind: VerifyIssueKind::ContractPolicyViolation,
                detail: format!(
                    "structured_field_target_path was enforced on config_basic.{action}"
                ),
                missing_fields: Vec::new(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::PlanKind;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempFileGuard {
        path: PathBuf,
    }

    impl TempFileGuard {
        fn new(name: &str) -> Self {
            let mut path = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time before unix epoch")
                .as_nanos();
            path.push(format!(
                "clawd_structured_field_{name}_{}_{}",
                std::process::id(),
                nanos
            ));
            fs::write(&path, "package = { version = \"0.1.0\" }\n").expect("write temp file");
            Self { path }
        }
    }

    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn structured_scalar_contract() -> crate::IntentOutputContract {
        let mut output_contract = crate::IntentOutputContract::default();
        output_contract.response_shape = crate::OutputResponseShape::Scalar;
        output_contract.requires_content_evidence = true;
        output_contract.locator_kind = crate::OutputLocatorKind::Path;
        output_contract
    }

    fn contract_with_selector(selector: &str) -> crate::IntentOutputContract {
        let mut contract = structured_scalar_contract();
        contract.selection.structured_field_selector = Some(selector.to_string());
        contract
    }

    fn contract_with_selector_and_locator(
        selector: &str,
        locator: &str,
    ) -> crate::IntentOutputContract {
        let mut contract = contract_with_selector(selector);
        contract.locator_kind = crate::OutputLocatorKind::Path;
        contract.locator_hint = locator.to_string();
        contract
    }

    fn plan_result(steps: Vec<PlanStep>) -> PlanResult {
        PlanResult {
            goal: "test".to_string(),
            missing_slots: Vec::new(),
            needs_confirmation: false,
            output_contract: None,
            steps,
            planner_notes: String::new(),
            plan_kind: PlanKind::Single,
            raw_plan_text: String::new(),
        }
    }

    #[test]
    fn repairs_config_read_fields_to_structured_selector() {
        let state = AppState::test_default_with_fixture_provider();
        let contract = contract_with_selector("workspace.dependencies.toml");
        let mut plan = plan_result(vec![PlanStep {
            step_id: "s1".to_string(),
            action_type: "call_tool".to_string(),
            skill: "config_basic".to_string(),
            args: json!({
                "action": "read_fields",
                "path": "/home/guagua/rustclaw/Cargo.toml",
                "field_paths": ["workspace", "workspace.dependencies"],
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }]);
        let mut issues = Vec::new();

        apply_structured_field_selector_repair(
            &state,
            Some(&contract),
            None,
            &mut plan,
            &mut issues,
        );

        assert_eq!(
            plan.steps[0].args.get("field_paths"),
            Some(&json!(["workspace.dependencies.toml"]))
        );
        assert!(issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractPolicyViolation)));
    }

    #[test]
    fn infers_structured_selector_from_machine_token_that_refines_planned_parent_field() {
        let state = AppState::test_default_with_fixture_provider();
        let contract = structured_scalar_contract();
        let mut plan = plan_result(vec![PlanStep {
            step_id: "s1".to_string(),
            action_type: "call_tool".to_string(),
            skill: "config_basic".to_string(),
            args: json!({
                "action": "read_fields",
                "path": "./Cargo.toml",
                "field_paths": ["workspace", "workspace.dependencies"],
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }]);
        let mut issues = Vec::new();

        apply_structured_field_selector_repair(
            &state,
            Some(&contract),
            Some("Read workspace.dependencies.toml from ./Cargo.toml and output only the value."),
            &mut plan,
            &mut issues,
        );

        assert_eq!(
            plan.steps[0].args.get("field_paths"),
            Some(&json!(["workspace.dependencies.toml"]))
        );
        assert!(issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractPolicyViolation)));
    }

    #[test]
    fn does_not_infer_filename_token_without_planned_field_prefix() {
        let state = AppState::test_default_with_fixture_provider();
        let contract = structured_scalar_contract();
        let mut plan = plan_result(vec![PlanStep {
            step_id: "s1".to_string(),
            action_type: "call_tool".to_string(),
            skill: "config_basic".to_string(),
            args: json!({
                "action": "read_fields",
                "path": "./Cargo.toml",
                "field_paths": ["workspace.dependencies"],
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }]);
        let mut issues = Vec::new();

        apply_structured_field_selector_repair(
            &state,
            Some(&contract),
            Some("Read ./Cargo.toml and output only the value."),
            &mut plan,
            &mut issues,
        );

        assert_eq!(
            plan.steps[0].args.get("field_paths"),
            Some(&json!(["workspace.dependencies"]))
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn keeps_config_read_field_when_selector_already_matches() {
        let state = AppState::test_default_with_fixture_provider();
        let contract = contract_with_selector("workspace.dependencies.toml");
        let mut plan = plan_result(vec![PlanStep {
            step_id: "s1".to_string(),
            action_type: "call_tool".to_string(),
            skill: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": "/home/guagua/rustclaw/Cargo.toml",
                "field_path": "workspace.dependencies.toml",
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }]);
        let mut issues = Vec::new();

        apply_structured_field_selector_repair(
            &state,
            Some(&contract),
            None,
            &mut plan,
            &mut issues,
        );

        assert_eq!(
            plan.steps[0].args.get("field_path"),
            Some(&json!("workspace.dependencies.toml"))
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn repairs_config_read_field_to_structured_target_path() {
        let state = AppState::test_default_with_fixture_provider();
        let target = TempFileGuard::new("target_path");
        let wrong = TempFileGuard::new("wrong_path");
        let target_path = target.path.display().to_string();
        let wrong_path = wrong.path.display().to_string();
        let contract = contract_with_selector_and_locator("package.version", &target_path);
        let mut plan = plan_result(vec![PlanStep {
            step_id: "s1".to_string(),
            action_type: "call_tool".to_string(),
            skill: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": wrong_path,
                "field_path": "workspace.package.version",
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }]);
        let mut issues = Vec::new();

        apply_structured_field_selector_repair(
            &state,
            Some(&contract),
            None,
            &mut plan,
            &mut issues,
        );

        assert_eq!(plan.steps[0].args.get("path"), Some(&json!(target_path)));
        assert_eq!(
            plan.steps[0].args.get("field_path"),
            Some(&json!("package.version"))
        );
        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn keeps_explicit_multi_target_structured_reads() {
        let state = AppState::test_default_with_fixture_provider();
        let first = TempFileGuard::new("first_target");
        let second = TempFileGuard::new("second_target");
        let first_path = first.path.display().to_string();
        let second_path = second.path.display().to_string();
        let contract = contract_with_selector_and_locator("package.name", &second_path);
        let mut plan = plan_result(vec![
            PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "config_basic".to_string(),
                args: json!({
                    "action": "read_field",
                    "path": first_path,
                    "field_path": "name",
                }),
                depends_on: Vec::new(),
                why: String::new(),
            },
            PlanStep {
                step_id: "s2".to_string(),
                action_type: "call_tool".to_string(),
                skill: "config_basic".to_string(),
                args: json!({
                    "action": "read_field",
                    "path": second_path,
                    "field_path": "package.name",
                }),
                depends_on: vec!["s1".to_string()],
                why: String::new(),
            },
        ]);
        let mut issues = Vec::new();

        apply_structured_field_selector_repair(
            &state,
            Some(&contract),
            None,
            &mut plan,
            &mut issues,
        );

        assert_eq!(plan.steps[0].args.get("path"), Some(&json!(first_path)));
        assert_eq!(plan.steps[0].args.get("field_path"), Some(&json!("name")));
        assert_eq!(plan.steps[1].args.get("path"), Some(&json!(second_path)));
        assert_eq!(
            plan.steps[1].args.get("field_path"),
            Some(&json!("package.name"))
        );
        assert!(issues.is_empty());
    }
}

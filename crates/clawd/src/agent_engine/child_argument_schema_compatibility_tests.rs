use super::*;
#[test]
fn numeric_strings_are_rejected_without_coercing_the_constraint() {
    let host =
        json!({"type":"object","properties":{"line":{"type":"integer"},"code":{"type":"string"}}});
    let proposal = json!({"properties":{"line":{"enum":["12"]},"code":{"enum":["12"]}}});
    let original = proposal.clone();
    let mut errors = Vec::new();
    compare(&proposal, &host, "", 0, &mut errors);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["path"], "/properties/line/enum");
    assert_eq!(errors[0]["expected_type"], "integer");
    assert_eq!(proposal, original);
    errors.clear();
    compare(
        &json!({"properties":{"line":{"enum":[12]}}}),
        &host,
        "",
        0,
        &mut errors,
    );
    assert!(errors.is_empty());
}
#[test]
fn nullable_and_nested_literal_types_are_checked_with_bounded_diagnostics() {
    let host =
        json!({"properties":{"entries":{"type":"array","items":{"type":["integer","null"]}}}});
    for (values, valid) in [
        (json!([12, null]), true),
        (json!(["12"]), false),
        (json!([true]), false),
    ] {
        let mut errors = Vec::new();
        compare(
            &json!({"properties":{"entries":{"items":{"enum":values}}}}),
            &host,
            "",
            0,
            &mut errors,
        );
        assert_eq!(errors.is_empty(), valid);
    }
    let host = json!({"properties":(0..64).map(|i|(format!("p{i}"),json!({"type":"integer"}))).collect::<serde_json::Map<_,_>>()});
    let constraint = json!({"properties":(0..64).map(|i|(format!("p{i}"),json!({"enum":["1"]}))).collect::<serde_json::Map<_,_>>()});
    let mut errors = Vec::new();
    compare(&constraint, &host, "", 0, &mut errors);
    assert_eq!(errors.len(), 16);
}

#[test]
fn injected_action_conflicts_are_reported_without_widening_closed_grants() {
    for proposal in [
        json!({"type":"object","additionalProperties":false,"properties":{"path":{"enum":["doc.txt"]}}}),
        json!({"properties":{"action":{"enum":["other"]}}}),
        json!({"properties":{"action":{"type":"integer"}}}),
        json!({"properties":{"action":{"maxLength":2}}}),
        json!({"enum":[{"path":"doc.txt"}]}),
    ] {
        let original = proposal.clone();
        let mut issues = Vec::new();
        compare_injected_action(&proposal, "write_text", "/0/allowed_args/0", &mut issues);
        assert_eq!(issues.len(), 1, "{proposal}");
        assert_eq!(
            issues[0]["reason"],
            "constraint_excludes_registry_injected_argument"
        );
        assert_eq!(issues[0]["expected_value"], "write_text");
        assert_eq!(issues[0]["path"], "/0/allowed_args/0/properties/action");
        assert_eq!(proposal, original);
    }
}

#[test]
fn compatible_actions_and_open_argument_constraints_remain_unchanged() {
    for proposal in [
        json!({"properties":{"path":{"enum":["doc.txt"]}}}),
        json!({"additionalProperties":false,"properties":{"action":{"enum":["write_text"]}}}),
        json!({"properties":{"action":{"type":"string"}}}),
        json!({"enum":[{"path":"doc.txt","action":"write_text"}]}),
    ] {
        let original = proposal.clone();
        let mut issues = Vec::new();
        compare_injected_action(&proposal, "write_text", "", &mut issues);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(proposal, original);
    }
}

#[test]
fn registry_action_conflict_diagnostics_keep_the_existing_bound() {
    let mut issues = (0..16).map(|i| json!({"fixture":i})).collect::<Vec<_>>();
    compare_injected_action(
        &json!({"additionalProperties":false}),
        "fixture_action",
        "",
        &mut issues,
    );
    assert_eq!(issues.len(), 16);
}

#[test]
fn registry_selected_action_is_checked_for_canonical_and_alias_constraints() {
    let state = crate::AppState::test_default_with_fixture_provider();
    crate::agent_engine::skill_execution::tests::install_test_registry(
        &state,
        r#"
[[skills]]
name = "fixture_storage"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "low"
side_effect = false
input_schema = { type = "object", properties = { path = { type = "string" } } }
planner_capability_aliases = { "fixture.legacy_read" = "fixture.read" }
planner_capabilities = [
 { name = "fixture.read", action = "read_range", required = ["path"], effect = "observe", risk_level = "low", isolation_profile = "read_only" },
 { name = "fixture.legacy_read", action = "read_range", required = ["path"], effect = "observe", risk_level = "low", isolation_profile = "read_only" },
]
"#,
        &["fixture_storage"],
    );
    for capability in ["fixture.read", "fixture.legacy_read"] {
        let mut grant = json!([{"capability":capability,"allowed_args":[{"type":"object","required":["path"],"additionalProperties":false,"properties":{"path":{"enum":["doc.txt"]}}}]}]);
        let original = grant.clone();
        let errors = issues(&state, &grant);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["expected_value"], "read_range");
        assert_eq!(grant, original);
        grant[0]["allowed_args"][0]["properties"]["action"] = json!({"enum":["read_range"]});
        assert!(issues(&state, &grant).is_empty());
    }
}

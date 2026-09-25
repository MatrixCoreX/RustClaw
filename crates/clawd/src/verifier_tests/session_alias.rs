use serde_json::json;

use crate::PlanStep;

use super::super::session_alias::session_alias_reference_issue;
use super::super::VerifyIssueKind;

fn alias_context(alias: &str, target: &str) -> String {
    format!("### SESSION_ALIAS_BINDINGS\n- alias: {alias}\n  target: {target}\n")
}

fn bind_step(alias: &str, target: &str) -> PlanStep {
    PlanStep {
        step_id: "bind_alias".to_string(),
        action_type: "call_tool".to_string(),
        skill: "task_control".to_string(),
        args: json!({
            "action": "bind_session_alias",
            "alias": alias,
            "target": target,
            "target_kind": "path",
        }),
        depends_on: Vec::new(),
        why: String::new(),
    }
}

#[test]
fn scalar_recall_marker_cannot_be_persisted_as_a_session_alias() {
    let context = alias_context("old file", "docs/old.md");
    let mut step = bind_step("RC-CONT-0428", "RC-CONT-0428");
    step.args["target_kind"] = json!("resource");
    let issue = session_alias_reference_issue(
        Some("retain this marker for the next turn"),
        Some(&context),
        &step,
        "task_control",
    )
    .expect("a scalar recall value is not an alias mapping");

    assert_eq!(issue.kind, VerifyIssueKind::InvalidArgumentValue);
    assert!(issue.detail.contains("session_alias_alias_equals_target"));
    assert_eq!(issue.missing_fields, vec!["target"]);
}

#[test]
fn typed_resource_alias_requires_a_machine_address() {
    let context = alias_context("old file", "docs/old.md");
    let mut invalid = bind_step("current service", "local-agent");
    invalid.args["target_kind"] = json!("resource");
    let issue = session_alias_reference_issue(
        Some("assign a structured resource reference"),
        Some(&context),
        &invalid,
        "task_control",
    )
    .expect("untyped resource value must be rejected");
    assert!(issue.detail.contains("session_alias_target_invalid"));

    invalid.args["target"] = json!("service:local-agent");
    assert!(session_alias_reference_issue(
        Some("assign a structured resource reference"),
        Some(&context),
        &invalid,
        "task_control",
    )
    .is_none());
}

#[test]
fn existing_alias_rebind_accepts_the_exact_structured_key() {
    let context = alias_context("甲文件", "docs/old.md");
    let issue = session_alias_reference_issue(
        Some("甲文件现在指向 docs/new.md"),
        Some(&context),
        &bind_step("甲文件", "docs/new.md"),
        "task_control",
    );

    assert!(issue.is_none());
}

#[test]
fn existing_alias_rebind_rejects_a_shortened_key_before_dispatch() {
    let context = alias_context("甲文件", "docs/old.md");
    let issue = session_alias_reference_issue(
        Some("甲文件现在指向 docs/new.md"),
        Some(&context),
        &bind_step("甲", "docs/new.md"),
        "task_control",
    )
    .expect("shortened alias must be rejected");

    assert_eq!(issue.kind, VerifyIssueKind::InvalidArgumentValue);
    assert!(issue.detail.contains("session_alias_rebind_key_mismatch"));
    assert_eq!(issue.missing_fields, vec!["alias"]);
}

#[test]
fn new_alias_binding_is_not_inferred_or_rejected_by_existing_state() {
    let context = alias_context("旧文件", "docs/old.md");
    let issue = session_alias_reference_issue(
        Some("建立一个新的会话引用"),
        Some(&context),
        &bind_step("新文件", "docs/new.md"),
        "task_control",
    );

    assert!(issue.is_none());
}

#[test]
fn alias_to_the_same_target_can_create_an_additional_reference() {
    let context = alias_context("旧文件", "docs/shared.md");
    let issue = session_alias_reference_issue(
        Some("保留旧文件，同时增加另一个引用"),
        Some(&context),
        &bind_step("新文件", "docs/shared.md"),
        "task_control",
    );

    assert!(issue.is_none());
}

use super::*;

#[test]
fn removes_only_scaffold_and_preserves_authored_language_rules_and_version() {
    let source = format!("<!-- version: 7 -->\n## Multilingual Reinforcement\n{EDITOR_SCAFFOLD}\n### zh-CN\n保留真实规则\n### en\nKeep the authored policy.\n");
    let expected = source.replacen(EDITOR_SCAFFOLD, "", 1);
    let projected = project_template(&source);
    assert_eq!(projected, expected);
    assert_eq!(project_template(&projected), projected);
}

#[test]
fn preserves_fenced_examples_with_both_delimiters_and_longer_fences() {
    for fence in ["```", "````", "~~~", "~~~~"] {
        let source = format!("{fence}markdown\n{EDITOR_SCAFFOLD}\n{fence}\n{EDITOR_SCAFFOLD}\n");
        assert_eq!(
            project_template(&source),
            format!("{fence}markdown\n{EDITOR_SCAFFOLD}\n{fence}\n\n")
        );
    }
}

#[test]
fn unknown_comments_inline_literals_and_unclosed_fences_remain_verbatim() {
    for source in [
        EDITOR_SCAFFOLD.replace("optional", "required"),
        format!("quoted {EDITOR_SCAFFOLD}\n"),
        format!("{EDITOR_SCAFFOLD} appended user text\n"),
        format!("```text\n{EDITOR_SCAFFOLD}\n"),
        EDITOR_SCAFFOLD.replace('\n', "\r\n"),
    ] {
        assert_eq!(project_template(&source), source);
    }
}

#[test]
fn repeated_source_fragments_do_not_change_policy_or_placeholders() {
    let part = format!("policy\n{EDITOR_SCAFFOLD}\n__INPUT__\n");
    let source = part.repeat(6);
    assert_eq!(project_template(&source), "policy\n\n__INPUT__\n".repeat(6));
}

#[test]
fn interpolation_preserves_user_and_evidence_literals_identical_to_scaffold() {
    let template = format!("policy\n{EDITOR_SCAFFOLD}\n__USER__\n__EVIDENCE__");
    let payload = format!("用户原文\n{EDITOR_SCAFFOLD}\n");
    let rendered = crate::render_prompt_template(
        &template,
        &[("__USER__", &payload), ("__EVIDENCE__", &payload)],
    );
    assert_eq!(rendered.matches(EDITOR_SCAFFOLD).count(), 2);
    assert_eq!(rendered, format!("policy\n\n{payload}\n{payload}"));
}

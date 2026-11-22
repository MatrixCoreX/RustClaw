use super::*;

const BODY: &str = "shared policy\n<!--a1:read-->\nread contract\n<!--/a1-->\n<!--a1:write,append-->\nwrite contract\n<!--/a1-->\n## Multilingual Reinforcement\nshared language policy";

#[test]
fn selected_action_retains_shared_policy_and_excludes_other_action_details() {
    let selected = BTreeSet::from(["read".to_string()]);
    let out = render(BODY, &selected).unwrap();
    assert!(out.contains("read contract"));
    assert!(out.contains("shared policy") && out.contains("shared language policy"));
    assert!(!out.contains("write contract") && !out.contains("<!--"));
}

#[test]
fn loading_group_without_selected_action_retains_shared_discovery_and_policy() {
    let out = render(BODY, &BTreeSet::new()).unwrap();
    assert!(out.contains("shared policy"));
    assert!(!out.contains("read contract") && !out.contains("write contract"));
    assert!(render(BODY, &BTreeSet::from(["append".to_string()]))
        .unwrap()
        .contains("write contract"));
}

#[test]
fn unknown_actions_or_malformed_package_sections_fall_back_without_hiding_rules() {
    assert!(render(BODY, &BTreeSet::from(["not_documented".to_string()])).is_none());
    for body in [
        "legacy body",
        "<!--a1:read-->\nunterminated",
        "<!--/a1-->",
        "<!--a7:read-->",
        "<!--a1:arbitrary prose-->\n<!--/a1-->",
    ] {
        assert!(render(body, &BTreeSet::new()).is_none());
    }
}

#[test]
fn filesystem_read_playbook_retains_scope_and_byte_proof_without_edit_instructions() {
    let body = include_str!("../../../../prompts/layers/generated/skills/fs_basic.md");
    let selected = BTreeSet::from(["read_text_range".to_string()]);
    let read = render(body, &selected).unwrap();
    for rule in [
        "inclusive line/byte bounds",
        "confirmation",
        "final newline",
        "ends_with_newline",
        "do not authorize reading outside that scope",
        "field_selector",
        "Multilingual Reinforcement",
    ] {
        assert!(read.contains(rule), "missing shared/read policy: {rule}");
    }
    assert!(!read.contains("### Edit action matrix"));
    assert!(!read.contains("| `replace_text` |"));
    assert!(read.len() * 2 < body.len());
    assert!(render(body, &BTreeSet::from(["unknown_action".to_string()])).is_none());
    let write = render(body, &BTreeSet::from(["write_text".to_string()])).unwrap();
    assert!(write.contains("Requires confirmation"));
    assert!(write.contains("Literal writes/appends preserve all requested whitespace"));
}

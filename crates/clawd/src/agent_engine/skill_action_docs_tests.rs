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

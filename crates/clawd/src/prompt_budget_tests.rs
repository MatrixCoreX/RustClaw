use super::*;

#[test]
fn prompt_section_report_records_tokens_cacheability_provenance_and_omission() {
    let report = prompt_section_budget_report(
        "planner",
        &[
            PromptSection {
                name: "stable_protocol",
                text: "protocol",
                cacheability: "stable_prefix",
                provenance: "prompt_registry",
                omission_reason: None,
            },
            PromptSection {
                name: "skill_playbook",
                text: "",
                cacheability: "task_scoped",
                provenance: "skill_registry",
                omission_reason: Some("not_selected"),
            },
        ],
    );

    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["section_count"], 2);
    assert_eq!(report["included_section_count"], 1);
    assert_eq!(report["sections"][0]["cacheability"], "stable_prefix");
    assert_eq!(report["sections"][0]["provenance"], "prompt_registry");
    assert_eq!(report["sections"][1]["omission_reason"], "not_selected");
    assert!(report["token_safety_estimate"].as_u64().unwrap_or(0) > 0);
}

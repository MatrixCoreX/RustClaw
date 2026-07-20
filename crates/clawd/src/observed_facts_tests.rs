use super::{derive_observed_facts_from_ask_outcome, ObservedFacts};

fn dummy_route_result() -> crate::IntentOutputContract {
    crate::IntentOutputContract::default()
}

#[test]
fn derives_ordered_entries_from_numbered_answer_text() {
    let journal = crate::task_journal::TaskJournal::new("list");
    let mut route = dummy_route_result();
    route.requires_content_evidence = true;
    let facts = derive_observed_facts_from_ask_outcome(
        "1. README.md\n2. Cargo.toml\n3. configs",
        &[],
        &journal,
        &route,
    );
    assert_eq!(
        facts.ordered_entries,
        vec![
            "README.md".to_string(),
            "Cargo.toml".to_string(),
            "configs".to_string()
        ]
    );
    assert_eq!(facts.selected_entry_index, None);
}

#[test]
fn ignores_plain_chat_numbered_text_as_ordered_entries() {
    let journal = crate::task_journal::TaskJournal::new("chat");
    let facts = derive_observed_facts_from_ask_outcome(
        "1. Keep the intro short\n2. Use concrete examples\n3. End with next steps",
        &[],
        &journal,
        &dummy_route_result(),
    );
    assert!(
        facts.ordered_entries.is_empty(),
        "plain generated prose should not become follow-up list state"
    );
}

#[test]
fn ignores_plain_chat_bullet_text_as_ordered_entries() {
    let journal = crate::task_journal::TaskJournal::new("chat");
    let facts = derive_observed_facts_from_ask_outcome(
        "- Keep the intro short\n- Use concrete examples\n- End with next steps",
        &[],
        &journal,
        &dummy_route_result(),
    );
    assert!(
        facts.ordered_entries.is_empty(),
        "plain generated bullet prose should not become follow-up list state"
    );
}

#[test]
fn derives_structural_bullet_entries_from_generic_visible_candidate_answer() {
    let journal = crate::task_journal::TaskJournal::new("find");
    let mut route = dummy_route_result();
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3".to_string();
    let facts = derive_observed_facts_from_ask_outcome(
        "在 `fuzzy_top3` 目录下找到4个文件名包含 \"abcd\" 的文件：\n- `abcd_report.md`\n- `my_abcd.txt`\n- `x_abcd_log.txt`\n- `zz_abcd_backup.log`\n这些都是模糊匹配测试的 fixture 文件。",
        &[],
        &journal,
        &route,
    );
    assert_eq!(
        facts.ordered_entries,
        vec![
            "abcd_report.md",
            "my_abcd.txt",
            "x_abcd_log.txt",
            "zz_abcd_backup.log"
        ]
    );
    assert_eq!(facts.observed_entry_count, Some(4));
}

#[test]
fn derives_delivery_targets_from_file_tokens() {
    let journal = crate::task_journal::TaskJournal::new("send");
    let facts = derive_observed_facts_from_ask_outcome(
        "FILE:/tmp/a.log",
        &["FILE:/tmp/b.log".to_string()],
        &journal,
        &dummy_route_result(),
    );
    assert_eq!(
        facts.delivery_targets,
        vec!["/tmp/a.log".to_string(), "/tmp/b.log".to_string()]
    );
}

#[test]
fn ignores_execution_summary_messages_for_observed_facts() {
    let journal = crate::task_journal::TaskJournal::new("send");
    let facts = derive_observed_facts_from_ask_outcome(
        "1. real.log\nFILE:/tmp/real.log",
        &[
            "**执行过程**\n1. wrong.log\nFILE:/tmp/wrong.log".to_string(),
            "2. final.log".to_string(),
        ],
        &journal,
        &dummy_route_result(),
    );

    assert_eq!(facts.delivery_targets, vec!["/tmp/real.log".to_string()]);
    assert!(facts.ordered_entries.contains(&"real.log".to_string()));
    assert!(facts.ordered_entries.contains(&"final.log".to_string()));
    assert!(!facts.ordered_entries.contains(&"wrong.log".to_string()));
    assert!(!facts
        .delivery_targets
        .contains(&"/tmp/wrong.log".to_string()));
}

#[test]
fn derives_selected_entry_index_from_bound_target_and_ordered_entries() {
    let mut journal = crate::task_journal::TaskJournal::new("read");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "s1".to_string(),
            skill: "system_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                serde_json::json!({
                    "action": "read_range",
                    "resolved_path": "logs/clawd.log",
                    "mode": "tail",
                    "n": 2,
                    "excerpt": "1|a\n2|b"
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let mut route = dummy_route_result();
    route.requires_content_evidence = true;
    let facts = derive_observed_facts_from_ask_outcome(
        "1. act_plan.log\n2. clawd.log\n3. clawd.run.log",
        &[],
        &journal,
        &route,
    );
    assert_eq!(facts.selected_entry_index, Some(1));
}

#[test]
fn derives_slice_spec_from_requested_n_when_range_output_omits_n() {
    let mut journal = crate::task_journal::TaskJournal::new("read");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "s1".to_string(),
            skill: "system_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                serde_json::json!({
                    "action": "read_range",
                    "resolved_path": "logs/model_io.log",
                    "mode": "tail",
                    "requested_n": 5,
                    "excerpt": "6|a\n7|b"
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let facts = derive_observed_facts_from_ask_outcome("", &[], &journal, &dummy_route_result());
    assert_eq!(
        facts.slice_spec,
        Some(crate::followup_frame::FollowupSliceSpec {
            kind: crate::followup_frame::FollowupSliceKind::Tail,
            n: Some(5),
            start_line: None,
            end_line: None,
        })
    );
}

#[test]
fn does_not_infer_slice_spec_from_request_text_when_journal_has_no_range_step() {
    let journal = crate::task_journal::TaskJournal::new("clarify_rewrite");
    let mut route = dummy_route_result();
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "/tmp/model_io.log".to_string();

    let facts = derive_observed_facts_from_ask_outcome(
        "line1\nline2\nline3\nline4\nline5",
        &[],
        &journal,
        &route,
    );

    assert_eq!(facts.slice_spec, None);
    assert_eq!(facts.bound_target.as_deref(), Some("/tmp/model_io.log"));
}

#[test]
fn uses_route_locator_hint_and_observed_entry_count_when_journal_lacks_scope() {
    let journal = crate::task_journal::TaskJournal::new("list");
    let mut route = dummy_route_result();
    route.locator_hint = "logs".to_string();
    route.requires_content_evidence = true;
    let facts = derive_observed_facts_from_ask_outcome(
        "1. act_plan.log\n2. clawd.log\n3. clawd.run.log",
        &[],
        &journal,
        &route,
    );
    assert_eq!(facts.bound_target.as_deref(), Some("logs"));
    assert_eq!(facts.observed_entry_count, Some(3));
}

#[test]
fn derives_bound_target_from_scalar_path_answer_contract() {
    let journal = crate::task_journal::TaskJournal::new("find_path");
    let mut route = dummy_route_result();
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.selection.structured_field_selector = Some("path".to_string());
    let target =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD";

    let facts = derive_observed_facts_from_ask_outcome(target, &[], &journal, &route);

    assert_eq!(facts.bound_target.as_deref(), Some(target));
}

#[test]
fn ignores_plain_scalar_answer_as_bound_target_without_path_contract() {
    let journal = crate::task_journal::TaskJournal::new("chat");
    let mut route = dummy_route_result();
    route.response_shape = crate::OutputResponseShape::Scalar;
    let facts = derive_observed_facts_from_ask_outcome(
        "/home/guagua/rustclaw/README.md",
        &[],
        &journal,
        &route,
    );

    assert_eq!(facts.bound_target, None);
}

#[test]
fn generic_workspace_evidence_does_not_bind_evidence_file_path() {
    let mut journal = crate::task_journal::TaskJournal::new("workspace_summary");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "s1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                serde_json::json!({
                    "extra": {
                        "action": "read_text_range",
                        "resolved_path": "/workspace/plan/current.md",
                        "excerpt": "current project evidence"
                    }
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let mut route = dummy_route_result();
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

    let facts = derive_observed_facts_from_ask_outcome(
        "RustClaw release note draft.",
        &[],
        &journal,
        &route,
    );

    assert_eq!(facts.bound_target, None);
}

#[test]
fn derives_output_shape_hint_from_output_contract() {
    let journal = crate::task_journal::TaskJournal::new("send");
    let mut route = dummy_route_result();
    route.response_shape = crate::OutputResponseShape::FileToken;
    let facts = derive_observed_facts_from_ask_outcome("FILE:/tmp/a.log", &[], &journal, &route);
    assert_eq!(facts.output_shape.as_deref(), Some("file_token"));
}

#[test]
fn empty_observed_facts_reports_empty() {
    assert!(ObservedFacts::default().is_empty());
    assert!(!ObservedFacts {
        bound_target: Some("README.md".to_string()),
        ..ObservedFacts::default()
    }
    .is_empty());
}

#[test]
fn observed_entry_count_is_derived_from_visible_entries_not_request_text() {
    let journal = crate::task_journal::TaskJournal::new("list");
    let mut route = dummy_route_result();
    route.requires_content_evidence = true;
    let facts = derive_observed_facts_from_ask_outcome(
        "1. act_plan.log\n2. clawd.log\n3. clawd.run.log",
        &[],
        &journal,
        &route,
    );
    assert_eq!(facts.observed_entry_count, Some(3));
}

use super::*;
use crate::verifier::{VerifyIssueKind as Kind, VerifyMode};

fn summary(kinds: &[Kind], mode: VerifyMode) -> TaskJournalVerifySummary {
    let blocked = kinds
        .iter()
        .any(|kind| crate::verifier::issue_blocks_in_enforce(*kind));
    TaskJournalVerifySummary {
        mode,
        approved: mode == VerifyMode::ObserveOnly || !blocked,
        blocked_reason: (mode == VerifyMode::Enforce && blocked).then(|| "fixture denial".into()),
        shadow_blocked_reason: blocked.then(|| "fixture denial".into()),
        needs_confirmation: kinds.contains(&Kind::ConfirmationRequired),
        issues: kinds
            .iter()
            .map(|kind| TaskJournalVerifyIssue {
                step_id: "fixture_step".into(),
                kind: *kind,
                detail: "fixture machine detail".into(),
                missing_fields: Vec::new(),
            })
            .collect(),
        ..Default::default()
    }
}

fn assert_blocked_codes(value: &Value, kind: Option<Kind>) {
    assert_eq!(
        value["blocked_reason_code"],
        json!(kind.map(Kind::reason_code))
    );
    assert_eq!(
        value["blocked_status_code"],
        json!(kind.map(Kind::status_code))
    );
    assert_eq!(
        value["blocked_message_key"],
        json!(kind.map(Kind::message_key))
    );
}

#[test]
fn informational_notes_do_not_fabricate_blocking_or_repair_evidence() {
    for kind in [
        Kind::DefaultCreationTargetApplied,
        Kind::ContractPreferredActionAvailable,
    ] {
        let verify = summary(&[kind], VerifyMode::Enforce);
        assert!(verify.approved);
        assert_blocked_codes(&verify_summary_json(&verify), None);
        let trace = verify_trace_json(&verify, None);
        assert_blocked_codes(&trace, None);
        assert_eq!(trace["issues"].as_array().unwrap().len(), 1);
        let issue = &trace["issues"][0];
        assert_eq!(issue["kind"], kind.as_str());
        assert_eq!(issue["status_code"], kind.status_code());
        assert_eq!(issue["severity"], "info");
        assert_eq!(issue["retryable"], false);
        for field in [
            "failure_attribution",
            "repair_envelope",
            "rejected_action",
            "forbidden_repeat_fingerprint",
        ] {
            assert!(
                issue.get(field).is_none_or(Value::is_null),
                "unexpected {field}"
            );
        }
        assert!(verify_repair_signals_json(Some(&verify), None).is_empty());
    }
}

#[test]
fn informational_notes_cannot_mask_a_real_blocking_issue() {
    for mode in [VerifyMode::Enforce, VerifyMode::ObserveOnly] {
        for kind in [
            Kind::MissingRequiredArg,
            Kind::SandboxPolicyDenied,
            Kind::ContractPolicyViolation,
        ] {
            let verify = summary(
                &[
                    Kind::DefaultCreationTargetApplied,
                    Kind::ContractPreferredActionAvailable,
                    kind,
                ],
                mode,
            );
            assert_blocked_codes(&verify_summary_json(&verify), Some(kind));
            let trace = verify_trace_json(&verify, None);
            assert_blocked_codes(&trace, Some(kind));
            assert_eq!(trace["issues"].as_array().unwrap().len(), 3);
            assert_eq!(trace["issues"][0]["kind"], "DefaultCreationTargetApplied");
            let signals = verify_repair_signals_json(Some(&verify), None);
            assert_eq!(signals.len(), 1);
            assert_eq!(signals[0]["kind"], kind.as_str());
            assert_eq!(
                signals[0]["failure_attribution"],
                kind.failure_attribution().as_str()
            );
            assert!(signals[0]["repair_envelope"].is_object());
        }
    }
}

#[test]
fn pending_confirmation_remains_actionable_but_is_not_a_blocking_failure() {
    let verify = summary(
        &[
            Kind::DefaultCreationTargetApplied,
            Kind::ConfirmationRequired,
        ],
        VerifyMode::Enforce,
    );
    assert!(verify.approved && verify.needs_confirmation);
    assert_blocked_codes(&verify_summary_json(&verify), None);
    assert_blocked_codes(&verify_trace_json(&verify, None), None);
    let signals = verify_repair_signals_json(Some(&verify), None);
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0]["kind"], "ConfirmationRequired");
    assert_eq!(signals[0]["failure_attribution"], "permission_denied");
    assert_eq!(signals[0]["retryable"], false);
}

#[test]
fn journal_round_projection_preserves_policy_errors_and_information_separately() {
    let verify = summary(
        &[
            Kind::DefaultCreationTargetApplied,
            Kind::ConfirmationRequired,
            Kind::SandboxPolicyDenied,
        ],
        VerifyMode::Enforce,
    );
    let mut journal = TaskJournal::for_task("fixture_projection", "ask", "fixture request");
    journal.verify_result = Some(verify.clone());
    journal.rounds.push(TaskJournalRoundTrace {
        round_no: 1,
        verify_result: Some(verify),
        ..Default::default()
    });
    assert_blocked_codes(
        &journal.to_summary_json()["verify_result"],
        Some(Kind::SandboxPolicyDenied),
    );
    let trace = journal.to_trace_json();
    let round = &trace["rounds"][0];
    assert_blocked_codes(&round["verify_result"], Some(Kind::SandboxPolicyDenied));
    assert_eq!(
        round["verify_result"]["issues"].as_array().unwrap().len(),
        3
    );
    let signals = round["repair_signals"].as_array().unwrap();
    assert_eq!(signals.len(), 2);
    assert_eq!(signals[0]["kind"], "ConfirmationRequired");
    assert_eq!(signals[1]["kind"], "SandboxPolicyDenied");
}

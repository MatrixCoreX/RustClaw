use super::*;

#[test]
fn only_explicitly_accepted_outcomes_accept_rewritten_answers() {
    let passed = AnswerVerifierOut {
        pass: true,
        ..Default::default()
    };
    let failed = AnswerVerifierOut {
        pass: false,
        ..Default::default()
    };
    assert!(AnswerVerificationOutcome::NotRequired.accepts_candidate());
    assert!(AnswerVerificationOutcome::Passed(None).accepts_candidate());
    assert!(AnswerVerificationOutcome::from_verdict(passed).accepts_candidate());
    assert!(!AnswerVerificationOutcome::from_verdict(failed).accepts_candidate());
    assert!(!AnswerVerificationOutcome::Unavailable {
        error_code: "answer_verifier_provider_unavailable",
        retryable: true,
    }
    .accepts_candidate());
}

#[test]
fn all_four_outcomes_are_distinct_machine_states() {
    let outcomes = [
        (AnswerVerificationOutcome::NotRequired, "not_required"),
        (AnswerVerificationOutcome::Passed(None), "passed"),
        (
            AnswerVerificationOutcome::Failed(AnswerVerifierOut::default()),
            "failed",
        ),
        (
            AnswerVerificationOutcome::Unavailable {
                error_code: "provider_unavailable",
                retryable: true,
            },
            "unavailable",
        ),
    ];
    for (outcome, expected) in outcomes {
        let record = outcome.record("opaque-candidate");
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["status"], expected);
        assert!(record.matches("opaque-candidate"));
        assert!(!record.matches("changed-candidate"));
        assert_eq!(value["schema_version"], 1);
    }
}

#[test]
fn unavailable_is_persisted_without_fabricating_a_pass_or_evidence_gap() {
    let mut journal = crate::task_journal::TaskJournal::new("opaque");
    journal.record_answer_verifier_summary(AnswerVerifierOut {
        pass: true,
        ..Default::default()
    });
    journal.record_answer_verification(
        AnswerVerificationOutcome::Unavailable {
            error_code: "answer_verifier_prompt_unavailable",
            retryable: false,
        },
        "opaque-answer",
    );
    assert!(journal.answer_verifier_summary.is_none());
    let summary = journal.to_summary_json();
    assert_eq!(summary["answer_verification"]["status"], "unavailable");
    assert_eq!(summary["answer_verification"]["retryable"], false);
    assert!(journal.has_answer_verification_for("opaque-answer"));
    assert!(!journal.has_answer_verification_for("changed-answer"));
}

#[test]
fn failed_answer_verification_does_not_relabel_successful_execution_as_failed() {
    let mut journal = crate::task_journal::TaskJournal::new("fixture");
    journal
        .capability_results
        .push(claw_core::capability_result::CapabilityResultEnvelope::ok(
            "fixture.write",
            None,
            serde_json::json!({}),
        ));
    journal.record_answer_verification(
        AnswerVerificationOutcome::Failed(AnswerVerifierOut::default()),
        "candidate",
    );
    let summary = journal.to_summary_json();
    assert_eq!(summary["execution_status"], "succeeded");
    assert_eq!(summary["answer_verification"]["status"], "failed");
}

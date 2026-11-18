use super::support::*;
use super::*;

#[test]
fn incorrect_without_content_does_not_commit_feedback() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    reject_without_change(
        &db,
        &fixture.principal,
        MEMORY_ID,
        &request(MemoryFeedbackKind::Incorrect, None),
        "memory_correction_content_required",
    );
}

#[test]
fn blank_correction_does_not_commit_feedback() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    reject_without_change(
        &db,
        &fixture.principal,
        MEMORY_ID,
        &request(MemoryFeedbackKind::Incorrect, Some(" \n\t ")),
        "memory_correction_content_required",
    );
}

#[test]
fn unsupported_correction_does_not_commit_feedback() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "feedback-recent",
        "Original turn",
    );
    reject_without_change(
        &db,
        &fixture.principal,
        "feedback-recent",
        &request(MemoryFeedbackKind::Incorrect, Some("Replacement turn")),
        "memory_correction_kind_unsupported",
    );
}

#[test]
fn replacement_insert_failure_rolls_back_feedback_and_supersession() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_feedback_replacement BEFORE INSERT ON memory_facts
         BEGIN SELECT RAISE(ABORT,'feedback_fixture_replacement_failed'); END;",
    )
    .unwrap();
    reject_without_change(
        &db,
        &fixture.principal,
        MEMORY_ID,
        &request(MemoryFeedbackKind::Incorrect, Some("Replacement fact")),
        "feedback_fixture_replacement_failed",
    );
}

#[test]
fn revision_insert_failure_rolls_back_feedback_and_replacement() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_feedback_revision BEFORE INSERT ON memory_revisions
         BEGIN SELECT RAISE(ABORT,'feedback_fixture_revision_failed'); END;",
    )
    .unwrap();
    reject_without_change(
        &db,
        &fixture.principal,
        MEMORY_ID,
        &request(MemoryFeedbackKind::Incorrect, Some("Replacement fact")),
        "feedback_fixture_revision_failed",
    );
}

#[test]
fn recall_disable_failure_rolls_back_feedback() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_feedback_disable BEFORE UPDATE ON memory_facts
         BEGIN SELECT RAISE(ABORT,'feedback_fixture_disable_failed'); END;",
    )
    .unwrap();
    reject_without_change(
        &db,
        &fixture.principal,
        MEMORY_ID,
        &request(MemoryFeedbackKind::DoNotUse, None),
        "feedback_fixture_disable_failed",
    );
}

#[test]
fn feedback_insert_failure_cannot_publish_a_correction() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_feedback_insert BEFORE INSERT ON memory_retrieval_feedback
         BEGIN SELECT RAISE(ABORT,'feedback_fixture_insert_failed'); END;",
    )
    .unwrap();
    reject_without_change(
        &db,
        &fixture.principal,
        MEMORY_ID,
        &request(MemoryFeedbackKind::Incorrect, Some("Replacement fact")),
        "feedback_fixture_insert_failed",
    );
}

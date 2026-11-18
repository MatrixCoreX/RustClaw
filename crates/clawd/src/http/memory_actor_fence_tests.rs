use super::*;

#[path = "memory_actor_fence_fixtures.rs"]
mod fixtures;
#[path = "memory_actor_fence_http.rs"]
mod http;
#[path = "memory_actor_fence_nested.rs"]
mod nested;

#[test]
fn revoked_original_key_cannot_correct_through_another_live_account_key() {
    http::scenario(fixtures::Action::Correct, http::Change::Key);
}
#[test]
fn revoked_original_key_cannot_record_feedback_through_another_live_account_key() {
    http::scenario(fixtures::Action::Feedback, http::Change::Key);
}
#[test]
fn revoked_original_key_cannot_delete_through_another_live_account_key() {
    http::scenario(fixtures::Action::Delete, http::Change::Key);
}
#[test]
fn revoked_original_key_cannot_undo_through_another_live_account_key() {
    http::scenario(fixtures::Action::Undo, http::Change::Key);
}
#[test]
fn revoked_original_key_cannot_persist_an_import_preview() {
    http::scenario(fixtures::Action::Preview, http::Change::Key);
}
#[test]
fn revoked_original_key_cannot_confirm_an_existing_import_preview() {
    http::scenario(fixtures::Action::Confirm, http::Change::Key);
}
#[test]
fn revoked_original_key_cannot_clear_owned_memory() {
    http::scenario(fixtures::Action::Clear, http::Change::Key);
}
#[test]
fn revoked_original_binding_cannot_borrow_a_surviving_credential() {
    http::scenario(fixtures::Action::Correct, http::Change::Binding);
}
#[test]
fn changed_original_role_is_rejected_at_the_actual_writer() {
    http::scenario(fixtures::Action::Correct, http::Change::Role);
}
#[test]
fn reassigned_original_credential_cannot_mutate_either_account() {
    http::scenario(fixtures::Action::Correct, http::Change::Reassign);
}
#[test]
fn unchanged_original_identity_still_commits_the_real_correction() {
    http::scenario(fixtures::Action::Correct, http::Change::None);
}
#[test]
fn final_identity_loss_rolls_back_canonical_projection_audit_and_identity_changes() {
    http::scenario(fixtures::Action::Correct, http::Change::InTransaction);
}
#[test]
fn nested_correction_failure_preserves_private_rows_and_the_callers_transaction() {
    nested::correction_rollback();
}
#[test]
fn nested_import_failure_preserves_preview_and_the_callers_transaction() {
    nested::import_rollback();
}

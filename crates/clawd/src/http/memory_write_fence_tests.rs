use super::*;
#[path = "memory_actor_fence_fixtures.rs"]
mod base;
#[path = "memory_write_fence_http.rs"]
mod http;
#[path = "memory_write_fence_support.rs"]
mod support;

#[test]
fn revoked_key_cannot_commit_queued_settings() {
    http::scenario(support::Action::Settings, http::Change::Key);
}

#[test]
fn unchanged_identity_can_commit_real_settings() {
    http::scenario(support::Action::Settings, http::Change::None);
}

#[test]
fn revoked_key_cannot_commit_queued_reindex() {
    http::scenario(support::Action::Reindex, http::Change::Key);
}

#[test]
fn unchanged_identity_can_commit_real_reindex() {
    http::scenario(support::Action::Reindex, http::Change::None);
}

#[test]
fn revoked_key_cannot_commit_queued_pause() {
    http::scenario(support::Action::Pause, http::Change::Key);
}

#[test]
fn unchanged_identity_can_commit_real_pause() {
    http::scenario(support::Action::Pause, http::Change::None);
}

#[test]
fn revoked_key_cannot_commit_queued_resume() {
    http::scenario(support::Action::Resume, http::Change::Key);
}

#[test]
fn unchanged_identity_can_commit_real_resume() {
    http::scenario(support::Action::Resume, http::Change::None);
}

#[test]
fn revoked_key_cannot_commit_queued_cancel() {
    http::scenario(support::Action::Cancel, http::Change::Key);
}

#[test]
fn unchanged_identity_can_commit_real_cancel() {
    http::scenario(support::Action::Cancel, http::Change::None);
}

#[test]
fn revoked_key_cannot_commit_queued_legacy_delete() {
    http::scenario(support::Action::LegacyDelete, http::Change::Key);
}

#[test]
fn unchanged_identity_can_commit_real_legacy_delete() {
    http::scenario(support::Action::LegacyDelete, http::Change::None);
}

#[test]
fn revoked_key_cannot_commit_queued_legacy_expire() {
    http::scenario(support::Action::LegacyExpire, http::Change::Key);
}

#[test]
fn unchanged_identity_can_commit_real_legacy_expire() {
    http::scenario(support::Action::LegacyExpire, http::Change::None);
}

#[test]
fn revoked_key_cannot_commit_queued_legacy_clear() {
    http::scenario(support::Action::LegacyClear, http::Change::Key);
}

#[test]
fn unchanged_identity_can_commit_real_legacy_clear() {
    http::scenario(support::Action::LegacyClear, http::Change::None);
}

#[test]
fn revoked_key_cannot_commit_queued_project_link() {
    http::scenario(support::Action::Link, http::Change::Key);
}

#[test]
fn unchanged_identity_can_commit_real_project_link() {
    http::scenario(support::Action::Link, http::Change::None);
}

#[test]
fn revoked_key_cannot_commit_queued_project_unlink() {
    http::scenario(support::Action::Unlink, http::Change::Key);
}

#[test]
fn unchanged_identity_can_commit_real_project_unlink() {
    http::scenario(support::Action::Unlink, http::Change::None);
}

#[test]
fn changed_original_binding_cannot_commit_settings() {
    http::scenario(support::Action::Settings, http::Change::Binding);
}

#[test]
fn changed_original_role_cannot_commit_settings() {
    http::scenario(support::Action::Settings, http::Change::Role);
}

#[test]
fn changed_original_principal_cannot_commit_settings() {
    http::scenario(support::Action::Settings, http::Change::Reassign);
}

#[test]
fn stale_policy_at_writer_rejects_settings_without_partial_effects() {
    http::scenario(support::Action::Settings, http::Change::StalePolicy);
}

#[test]
fn stale_policy_at_writer_rejects_reindex_without_partial_effects() {
    http::scenario(support::Action::Reindex, http::Change::StalePolicy);
}

#[test]
fn stale_policy_at_writer_rejects_pause_without_partial_effects() {
    http::scenario(support::Action::Pause, http::Change::StalePolicy);
}

#[test]
fn stale_policy_at_writer_rejects_resume_without_partial_effects() {
    http::scenario(support::Action::Resume, http::Change::StalePolicy);
}

#[test]
fn stale_policy_at_writer_rejects_cancel_without_partial_effects() {
    http::scenario(support::Action::Cancel, http::Change::StalePolicy);
}

#[test]
fn final_key_revocation_rolls_back_settings_and_avoids_false_audit() {
    http::scenario(support::Action::Settings, http::Change::LateRevoke);
}

#[test]
fn late_sql_failure_rolls_back_reindex_effects() {
    http::scenario(support::Action::Reindex, http::Change::SqlFault);
}

#[test]
fn late_sql_failure_rolls_back_cancel_effects() {
    http::scenario(support::Action::Cancel, http::Change::SqlFault);
}

#[test]
fn late_sql_failure_rolls_back_project_link_effects() {
    http::scenario(support::Action::Link, http::Change::SqlFault);
}

#[test]
fn empty_reindex_can_activate_inside_a_parent_transaction_without_committing_it() {
    http::empty_reindex_nested();
}

use claw_core::types::AuthIdentity;
use rusqlite::{params, Connection};

use super::{
    apply_revocation_fence, resolve_memory_settings, resolve_principal_memory_settings,
    update_memory_settings, ExternalContextPolicy, MemorySettingMode, MemorySettingScope,
    MemorySettingsUpdateRequest,
};

fn setup() -> (Connection, AuthIdentity) {
    let db = Connection::open_in_memory().expect("memory settings db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("auth schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES ('settings-key', 'user', 1, '1')",
        [],
    )
    .expect("auth key");
    crate::repo::auth::ensure_principal_identity_schema(&db).expect("principal schema");
    let principal_id: String = db
        .query_row(
            "SELECT principal_id FROM auth_keys WHERE user_key = 'settings-key'",
            [],
            |row| row.get(0),
        )
        .expect("principal id");
    (
        db,
        AuthIdentity {
            user_key: "settings-key".to_string(),
            principal_id,
            role: "user".to_string(),
            user_id: 7,
            chat_id: 11,
        },
    )
}

#[test]
fn empty_install_starts_disabled_until_an_authenticated_choice() {
    let db = Connection::open_in_memory().expect("new install settings db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("auth schema");
    crate::repo::auth::ensure_principal_identity_schema(&db).expect("new install migration");
    let (installation_class, onboarding_status): (String, String) = db
        .query_row(
            "SELECT installation_class, status FROM memory_onboarding_state WHERE singleton_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("onboarding row");
    assert_eq!(installation_class, "new_install");
    assert_eq!(onboarding_status, "pending_choice");

    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES ('new-install-key', 'admin', 1, '1')",
        [],
    )
    .expect("bootstrap key");
    crate::repo::auth::ensure_principal_identity_schema(&db).expect("bootstrap principal");
    let principal_id = crate::repo::auth::principal_id_for_user_key(&db, "new-install-key")
        .expect("resolve bootstrap principal")
        .expect("bootstrap principal");
    let effective = resolve_principal_memory_settings(&db, &principal_id, true)
        .expect("new install effective settings");
    assert!(!effective.use_memory);
    assert!(!effective.generate_memory);
}

#[test]
fn upgrade_preserves_release_memory_behavior() {
    let db = Connection::open_in_memory().expect("upgrade settings db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("auth schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES ('upgrade-key', 'user', 1, '1')",
        [],
    )
    .expect("existing auth key");
    crate::repo::auth::ensure_principal_identity_schema(&db).expect("upgrade migration");
    let principal_id = crate::repo::auth::principal_id_for_user_key(&db, "upgrade-key")
        .expect("resolve upgraded principal")
        .expect("upgraded principal");
    let effective = resolve_principal_memory_settings(&db, &principal_id, true)
        .expect("upgrade effective settings");
    let (installation_class, onboarding_status): (String, String) = db
        .query_row(
            "SELECT installation_class, status FROM memory_onboarding_state WHERE singleton_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("upgrade onboarding row");
    assert_eq!(installation_class, "upgrade");
    assert_eq!(onboarding_status, "upgrade_preserved");
    assert!(effective.use_memory);
    assert!(effective.generate_memory);
}

#[test]
fn turn_snapshot_only_accepts_monotonic_revocation_changes() {
    let (db, actor) = setup();
    let mut initial = update_request(MemorySettingScope::Principal, 0);
    initial.use_mode = Some(MemorySettingMode::Enabled);
    initial.generate_mode = Some(MemorySettingMode::Enabled);
    initial.external_context_policy = Some(ExternalContextPolicy::EvidenceOnly);
    let pinned = update_memory_settings(&db, &actor, &initial, false).expect("pinned settings");

    let mut broader = pinned.clone();
    broader.external_context_policy = ExternalContextPolicy::Allow;
    broader.revision += 1;
    broader.policy_digest = "broader".to_string();
    let mut fenced = pinned.clone();
    apply_revocation_fence(&mut fenced, &broader);
    assert_eq!(fenced, pinned, "broader changes wait for the next turn");

    let mut narrower = broader;
    narrower.use_memory = false;
    narrower.generate_memory = false;
    narrower.external_context_policy = ExternalContextPolicy::Exclude;
    narrower.managed_deny_reason = Some("fixture_deny".to_string());
    apply_revocation_fence(&mut fenced, &narrower);
    assert!(!fenced.use_memory);
    assert!(!fenced.generate_memory);
    assert_eq!(
        fenced.external_context_policy,
        ExternalContextPolicy::Exclude
    );
    assert_eq!(fenced.policy_digest, pinned.policy_digest);
    assert_eq!(fenced.revision, pinned.revision);
}

fn update_request(
    scope: MemorySettingScope,
    expected_revision: i64,
) -> MemorySettingsUpdateRequest {
    MemorySettingsUpdateRequest {
        scope,
        target_principal_id: None,
        conversation_id: None,
        use_mode: None,
        generate_mode: None,
        external_context_policy: None,
        expected_revision: Some(expected_revision),
        long_term_enabled: None,
    }
}

#[test]
fn principal_and_conversation_modes_resolve_immediately_without_restart() {
    let (db, actor) = setup();
    let mut principal = update_request(MemorySettingScope::Principal, 0);
    principal.use_mode = Some(MemorySettingMode::Disabled);
    principal.generate_mode = Some(MemorySettingMode::Enabled);
    let principal_result =
        update_memory_settings(&db, &actor, &principal, true).expect("write principal setting");
    assert!(!principal_result.use_memory);
    assert!(principal_result.generate_memory);
    assert!(!principal_result.restart_required);
    assert_eq!(principal_result.revision, 1);

    let mut conversation = update_request(MemorySettingScope::Conversation, 0);
    conversation.conversation_id = Some("conversation-a".to_string());
    conversation.use_mode = Some(MemorySettingMode::Enabled);
    conversation.generate_mode = Some(MemorySettingMode::Disabled);
    conversation.external_context_policy = Some(ExternalContextPolicy::EvidenceOnly);
    let conversation_result = update_memory_settings(&db, &actor, &conversation, true)
        .expect("write conversation setting");
    assert!(conversation_result.use_memory);
    assert!(!conversation_result.generate_memory);
    assert_eq!(
        conversation_result.external_context_policy,
        ExternalContextPolicy::EvidenceOnly
    );
    assert_eq!(conversation_result.use_source, "conversation_override");

    let other = resolve_memory_settings(&db, &actor.principal_id, Some("conversation-b"), true)
        .expect("resolve other conversation");
    assert!(!other.use_memory);
    assert!(other.generate_memory);
}

#[test]
fn stale_revision_is_rejected_and_admin_managed_deny_wins() {
    let (db, actor) = setup();
    let mut first = update_request(MemorySettingScope::Principal, 0);
    first.use_mode = Some(MemorySettingMode::Enabled);
    update_memory_settings(&db, &actor, &first, false).expect("first update");
    let error =
        update_memory_settings(&db, &actor, &first, false).expect_err("stale update must fail");
    assert!(error
        .to_string()
        .contains("memory_settings_revision_conflict"));

    db.execute(
        "INSERT INTO memory_runtime_settings (
            setting_key, setting_scope, use_mode, generate_mode,
            external_context_policy, managed_deny_use, managed_deny_generate,
            revision, policy_digest, updated_at
         ) VALUES (
            'admin:default', 'admin', 'enabled', 'enabled', 'inherit', 1, 1,
            1, 'fixture', '1'
         )",
        [],
    )
    .expect("managed deny row");
    let effective = resolve_principal_memory_settings(&db, &actor.principal_id, true)
        .expect("resolve managed deny");
    assert!(!effective.use_memory);
    assert!(!effective.generate_memory);
    assert_eq!(effective.use_source, "admin_managed_deny");
    assert_eq!(
        effective.managed_deny_reason.as_deref(),
        Some("memory_use_managed_denied,memory_generate_managed_denied")
    );
}

#[test]
fn ordinary_user_cannot_target_another_principal_or_admin_defaults() {
    let (db, actor) = setup();
    let mut cross_target = update_request(MemorySettingScope::Principal, 0);
    cross_target.target_principal_id = Some("principal-other".to_string());
    assert!(update_memory_settings(&db, &actor, &cross_target, true)
        .expect_err("cross target must fail")
        .to_string()
        .contains("memory_settings_admin_required"));

    let admin = update_request(MemorySettingScope::Admin, 0);
    assert!(update_memory_settings(&db, &actor, &admin, true)
        .expect_err("admin setting must fail")
        .to_string()
        .contains("memory_settings_admin_required"));

    let row_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_runtime_settings",
            params![],
            |row| row.get(0),
        )
        .expect("settings row count");
    assert_eq!(row_count, 0);
}

#[test]
fn runtime_settings_update_never_mutates_tracked_release_config() {
    let (db, actor) = setup();
    let config_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/memory.toml");
    let before = std::fs::read(&config_path).expect("read release memory config before");
    let mut request = update_request(MemorySettingScope::Principal, 0);
    request.use_mode = Some(MemorySettingMode::Disabled);
    request.generate_mode = Some(MemorySettingMode::Disabled);
    update_memory_settings(&db, &actor, &request, true).expect("runtime settings update");
    let after = std::fs::read(&config_path).expect("read release memory config after");
    assert_eq!(before, after);
}

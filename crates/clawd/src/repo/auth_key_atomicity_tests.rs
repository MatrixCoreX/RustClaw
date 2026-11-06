use super::*;

#[test]
fn auth_key_creation_failure_does_not_leave_an_orphan_auth_row_or_identity() {
    let state = auth_lifecycle_test_state();
    let existing = create_auth_key(&state, "user").unwrap();
    let before = resolve_auth_identity_by_key(&state, &existing)
        .unwrap()
        .unwrap();
    let counts = || {
        let db = state.core.db.get().unwrap();
        ["auth_keys", "principals", "credential_bindings"].map(|table| {
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
        })
    };
    let original_counts = counts();
    state
        .core
        .db
        .get()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fixture_new_binding_failure BEFORE INSERT ON credential_bindings
         BEGIN SELECT RAISE(ABORT, 'synthetic new key binding failure'); END;",
        )
        .unwrap();
    create_auth_key(&state, "user").expect_err("injected binding failure");
    assert_eq!(counts(), original_counts);
    let after = resolve_auth_identity_by_key(&state, &existing)
        .unwrap()
        .unwrap();
    assert_eq!(after.user_key, before.user_key);
    assert_eq!(after.principal_id, before.principal_id);
    assert_eq!(after.role, before.role);
    assert_eq!(after.user_id, before.user_id);
    assert_eq!(after.chat_id, before.chat_id);
}

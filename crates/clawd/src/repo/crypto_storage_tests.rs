use super::*;

#[test]
fn credentials_live_only_in_the_crypto_pool_and_can_be_restored() {
    let state = AppState::test_default_with_fixture_provider();
    upsert_for_user_key(
        &state,
        "rk-user",
        "okx",
        "api-key",
        "api-secret",
        Some("phrase"),
    )
    .expect("upsert");
    let context = credential_context_for_user_key(&state, "rk-user").expect("context");
    assert_eq!(context["okx"]["api_key"], "api-key");

    let main = state.core.db.get().expect("main");
    let main_has_table: i64 = main
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table' AND name='exchange_api_credentials'",
            [],
            |row| row.get(0),
        )
        .expect("main table check");
    assert_eq!(main_has_table, 0);

    let removed = take_for_user_key(&state, "rk-user").expect("take");
    assert_eq!(removed.len(), 1);
    assert_eq!(
        credential_context_for_user_key(&state, "rk-user").expect("empty context"),
        serde_json::json!({})
    );
    restore(&state, &removed).expect("restore");
    assert_eq!(
        credential_context_for_user_key(&state, "rk-user").expect("restored context")["okx"]
            ["api_secret"],
        "api-secret"
    );
}

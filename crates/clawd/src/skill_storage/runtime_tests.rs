use super::*;

#[test]
fn runtime_keeps_crypto_and_kb_in_separate_pools() {
    let runtime = SkillStorageRuntime::test_default();
    let crypto = runtime.crypto_pool().get().expect("crypto db");
    let kb = runtime.kb_pool().get().expect("kb db");
    let crypto_has_credentials: i64 = crypto
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='exchange_api_credentials'",
            [],
            |row| row.get(0),
        )
        .expect("crypto schema");
    let kb_has_credentials: i64 = kb
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='exchange_api_credentials'",
            [],
            |row| row.get(0),
        )
        .expect("kb schema");
    assert_eq!(crypto_has_credentials, 1);
    assert_eq!(kb_has_credentials, 0);
}

#[test]
fn clearing_one_skill_never_removes_another_skills_rows() {
    let runtime = SkillStorageRuntime::test_default();
    runtime
        .crypto_pool()
        .get()
        .expect("crypto db")
        .execute(
            "INSERT INTO exchange_api_credentials
                (user_key, exchange, api_key, api_secret, enabled, updated_at)
             VALUES ('rk-user', 'okx', 'key', 'secret', 1, '1')",
            [],
        )
        .expect("seed crypto");
    runtime
        .kb_pool()
        .get()
        .expect("KB db")
        .execute(
            "INSERT INTO kb_namespaces
                (owner_user_key, namespace, payload_json, updated_at_epoch)
             VALUES ('rk-user', 'docs', '{}', 1)",
            [],
        )
        .expect("seed KB");

    let removed = runtime.clear_skill_data("crypto").expect("clear crypto");

    assert!(removed.data_present_before);
    assert_eq!(removed.rows_deleted, 1);
    assert_eq!(runtime.data_state("crypto").expect("crypto state"), "empty");
    assert_eq!(runtime.data_state("kb").expect("KB state"), "present");
}

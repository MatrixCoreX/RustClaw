#[cfg(any(target_os = "windows", target_os = "macos"))]
#[test]
#[ignore = "Requires an unlocked native user credential store; run on disposable CI"]
fn native_vault_round_trip_and_profile_isolation() {
    use agent_desktop::credentials::{self, LoginSecret};
    use uuid::Uuid;
    let profile = Uuid::new_v4();
    let other = Uuid::new_v4();
    struct Cleanup(Uuid);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = credentials::forget(self.0);
        }
    }
    let _cleanup = Cleanup(profile);
    credentials::save(
        profile,
        &LoginSecret {
            mode: "password".into(),
            username: "native-test".into(),
            secret: "ephemeral-fixture-only".into(),
        },
    )
    .unwrap();
    assert_eq!(
        credentials::load(profile).unwrap().secret,
        "ephemeral-fixture-only"
    );
    assert!(credentials::load(other).is_err());
    credentials::forget(profile).unwrap();
    assert!(credentials::load(profile).is_err());
}

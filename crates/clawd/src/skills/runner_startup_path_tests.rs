use super::tests::StartupFixture;
use super::*;

async fn renamed_directory_case(replace: bool) {
    let fixture = StartupFixture::new();
    let source = fixture.root.join("source");
    let retained = fixture.root.join("retained");
    std::fs::create_dir(&source).unwrap();
    let command = StartupFixture::command_in(
        "cat >/dev/null; printf owned > payload-marker; printf ready > \"$APP_DURABLE_JOB_DIR/lease_ready\"",
        &source,
    );
    std::fs::rename(&source, &retained).unwrap();
    if replace {
        std::fs::create_dir(&source).unwrap();
    }
    let result = fixture
        .start_command(command, "{}", &RunnerAdmission::new(None))
        .await;
    assert!(
        result.is_ok(),
        "startup lost its admitted working directory: {result:?}"
    );
    assert_eq!(
        std::fs::read(retained.join("payload-marker")).ok(),
        Some(b"owned".to_vec()),
        "payload did not use the pinned directory"
    );
    assert!(!source.join("payload-marker").exists());
}

#[tokio::test]
async fn durable_launch_does_not_follow_a_replaced_working_directory() {
    renamed_directory_case(true).await;
}

#[tokio::test]
async fn durable_launch_retains_its_directory_when_the_original_path_disappears() {
    renamed_directory_case(false).await;
}

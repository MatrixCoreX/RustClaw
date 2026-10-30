use std::future::Future;
use std::task::{Context, Poll, Wake, Waker};

use super::*;

struct Fixture {
    state: AppState,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "channel-configuration-operation-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let mut state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        state.skill_rt.workspace_root = root.clone();
        state.seed_test_auth_identity("fixture-operation-admin", "admin");
        state.seed_test_auth_identity("fixture-operation-user", "user");
        Self { state, root }
    }

    fn write(&self, platform: AgentAppChannel, enabled: bool) {
        let raw = format!("[{}]\nenabled = {enabled}\napp_id = \"fixture-app\"\napp_secret = \"fixture-secret\"\n", platform.channel());
        for path in channel_configuration_paths(&self.state, platform) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, &raw).unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn channel_configuration_clones_and_aliases_share_a_lock_but_platforms_do_not() {
    let fixture = Fixture::new();
    let mut alias = fixture.state.clone();
    alias.skill_rt.workspace_root = fixture.root.join(".");
    let first = channel_configuration_slot(&fixture.state, AgentAppChannel::Feishu).unwrap();
    let second = channel_configuration_slot(&alias, AgentAppChannel::Feishu).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let held = lock_channel_configuration(&fixture.state, AgentAppChannel::Feishu)
        .await
        .unwrap();
    assert!(second.try_lock().is_err());
    let independent = channel_configuration_slot(&fixture.state, AgentAppChannel::Lark).unwrap();
    assert!(independent.try_lock().is_ok());
    drop(held);
    assert!(second.try_lock().is_ok());
    let weak = Arc::downgrade(&first);
    drop(first);
    drop(second);
    assert!(weak.upgrade().is_none());
}

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

#[cfg(unix)]
#[test]
fn channel_configuration_fifo_is_rejected_without_waiting_for_a_writer() {
    use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};
    let fixture = Fixture::new();
    let path = fixture.root.join("fixture-config.pipe");
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: a valid owned fixture path and restrictive FIFO mode are supplied.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    let worker_path = path.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result =
            read_registration_configuration(&worker_path).map_err(|error| error.to_string());
        let _ = sender.send(result);
    });
    let result = receiver.recv_timeout(Duration::from_secs(10));
    if result.is_err() {
        let writer = fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path);
        if writer.is_ok() {
            let _ = receiver.recv_timeout(Duration::from_secs(10));
        }
        drop(writer);
        if worker.is_finished() {
            worker.join().unwrap();
        }
        panic!("channel_configuration_reader_waited_for_fifo_writer");
    }
    assert_eq!(
        result.unwrap().unwrap_err(),
        "channel_configuration_unavailable"
    );
    worker.join().unwrap();
}

#[tokio::test]
async fn channel_configuration_waiting_mutation_rechecks_revoked_actor_before_revision() {
    let fixture = Fixture::new();
    let held = lock_channel_configuration(&fixture.state, AgentAppChannel::Feishu)
        .await
        .unwrap();
    let mut future = std::pin::pin!(begin_channel_configuration_mutation(
        &fixture.state,
        AgentAppChannel::Feishu,
        "fixture-operation-admin"
    ));
    let waker = Waker::from(Arc::new(NoopWake));
    assert!(matches!(
        future.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    fixture
        .state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE auth_keys SET enabled = 0 WHERE user_key = 'fixture-operation-admin'",
            [],
        )
        .unwrap();
    drop(held);
    let error = future.await.unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ChannelRegistrationRejection>(),
        Some(ChannelRegistrationRejection::OwnerUnavailable)
    ));
    let tables: i64 = fixture
        .state
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'channel_configuration_revisions'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
}

#[tokio::test]
async fn channel_configuration_mutation_advances_only_authorized_platform_revision() {
    let fixture = Fixture::new();
    assert!(begin_channel_configuration_mutation(
        &fixture.state,
        AgentAppChannel::Feishu,
        "fixture-operation-user"
    )
    .await
    .is_err());
    let first = begin_channel_configuration_mutation(
        &fixture.state,
        AgentAppChannel::Feishu,
        "fixture-operation-admin",
    )
    .await
    .unwrap();
    let revision: i64 = fixture
        .state
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT revision FROM channel_configuration_revisions WHERE channel = 'feishu'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(revision, 1);
    drop(first);
    let second = begin_channel_configuration_mutation(
        &fixture.state,
        AgentAppChannel::Lark,
        "fixture-operation-admin",
    )
    .await
    .unwrap();
    let revision: i64 = fixture
        .state
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT revision FROM channel_configuration_revisions WHERE channel = 'feishu'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(revision, 1);
    drop(second);
}

#[test]
fn channel_configuration_fingerprint_pins_both_copies_and_missing_state() {
    let fixture = Fixture::new();
    let platform = AgentAppChannel::Feishu;
    let empty = channel_configuration_fingerprint(&fixture.state, platform).unwrap();
    fixture.write(platform, true);
    let initial = channel_configuration_fingerprint(&fixture.state, platform).unwrap();
    assert_ne!(empty, initial);
    assert_eq!(initial.len(), 64);
    let paths = channel_configuration_paths(&fixture.state, platform);
    let original = fs::read(&paths[0]).unwrap();
    for path in &paths {
        fs::write(path, b"[feishu]\napp_id='fixture-modified'\n").unwrap();
        assert_ne!(
            channel_configuration_fingerprint(&fixture.state, platform).unwrap(),
            initial
        );
        fs::write(path, &original).unwrap();
    }
    fs::remove_file(&paths[1]).unwrap();
    assert_ne!(
        channel_configuration_fingerprint(&fixture.state, platform).unwrap(),
        initial
    );
}

#[test]
fn channel_configuration_unreadable_and_oversized_input_returns_only_a_machine_error() {
    let fixture = Fixture::new();
    let path = fixture.root.join("fixture-secret-path.toml");
    fs::create_dir(&path).unwrap();
    let error = read_registration_configuration(&path)
        .unwrap_err()
        .to_string();
    assert_eq!(error, "channel_configuration_unavailable");
    fs::remove_dir(&path).unwrap();
    let file = fs::File::create(&path).unwrap();
    file.set_len(1024 * 1024 + 1).unwrap();
    assert_eq!(
        read_registration_configuration(&path)
            .unwrap_err()
            .to_string(),
        error
    );
}

#[test]
fn channel_configuration_ready_and_applied_checks_preserve_disabled_configuration() {
    let fixture = Fixture::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        fixture.write(platform, false);
        for path in channel_configuration_paths(&fixture.state, platform) {
            fs::write(
                path,
                format!(
                    "[{}]\nenabled=false\napp_id=' fixture-app '\napp_secret=' fixture-secret '\n",
                    platform.channel()
                ),
            )
            .unwrap();
        }
        assert!(channel_configuration_copies_match(
            &fixture.state,
            platform,
            "fixture-app",
            "fixture-secret"
        ));
        assert!(verify_applied_channel_configuration(
            &fixture.state,
            platform,
            "fixture-app",
            "fixture-secret"
        )
        .is_err());
        fixture.write(platform, true);
        verify_applied_channel_configuration(
            &fixture.state,
            platform,
            "fixture-app",
            "fixture-secret",
        )
        .unwrap();
        let mounted = &channel_configuration_paths(&fixture.state, platform)[1];
        fs::write(mounted, "app_secret = \"fixture-parse-secret\n").unwrap();
        assert!(!channel_configuration_copies_match(
            &fixture.state,
            platform,
            "fixture-app",
            "fixture-secret"
        ));
        let error = verify_applied_channel_configuration(
            &fixture.state,
            platform,
            "fixture-app",
            "fixture-secret",
        )
        .unwrap_err()
        .to_string();
        assert_eq!(error, "channel_configuration_mismatch");
        fixture.write(platform, true);
        fs::remove_file(mounted).unwrap();
        assert!(!channel_configuration_copies_match(
            &fixture.state,
            platform,
            "fixture-app",
            "fixture-secret"
        ));
        assert!(verify_applied_channel_configuration(
            &fixture.state,
            platform,
            "fixture-app",
            "fixture-secret"
        )
        .is_err());
    }
}

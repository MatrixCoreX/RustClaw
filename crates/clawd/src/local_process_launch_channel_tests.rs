use std::fs;
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd};
use std::time::Duration;

use super::*;

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("lc-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn credential() -> OwnerCredential {
    OwnerRuntime::prepare_credential(uuid::Uuid::new_v4(), uuid::Uuid::new_v4()).unwrap()
}

fn frame(credential: OwnerCredential) -> LaunchFrame {
    LaunchFrame {
        schema_version: 1,
        credential,
        program: b"/bin/sh".to_vec(),
        arguments: vec![],
        environment: vec![],
        stdio_mask: 0,
    }
}

fn channel_pair() -> (LaunchChannel, LaunchChannel) {
    let (first, second) = UnixStream::pair().unwrap();
    (
        LaunchChannel::new(first).unwrap(),
        LaunchChannel::new(second).unwrap(),
    )
}

#[test]
fn bootstrap_transfers_exact_cloexec_handles_and_retains_the_channel() {
    let root = Directory::new();
    let (sender, receiver) = channel_pair();
    let sender = sender
        .send_bootstrap(
            b"{}",
            [
                File::open(&root.0).unwrap().into(),
                std::os::unix::net::UnixDatagram::unbound().unwrap().into(),
            ],
            deadline(),
        )
        .unwrap();
    let (body, handles, receiver) = receiver.receive_bootstrap(deadline()).unwrap();
    assert_eq!(body, b"{}");
    assert!(File::from(handles[0].try_clone().unwrap())
        .metadata()
        .unwrap()
        .is_dir());
    for handle in &handles {
        assert!(fcntl_getfd(handle).unwrap().contains(FdFlags::CLOEXEC));
    }
    sender.send_control(b"after_bootstrap", deadline()).unwrap();
    assert_eq!(
        receiver.receive_control(deadline()).unwrap(),
        b"after_bootstrap"
    );
}

#[test]
fn bootstrap_rejects_wrong_handle_count_and_closes_every_received_duplicate() {
    let (sender, receiver) = UnixStream::pair().unwrap();
    let receiver = LaunchChannel::new(receiver).unwrap();
    let (input, output) = nonblocking_pipe();
    let mut input = File::from(input);
    let output = File::from(output);
    let mut header = BOOTSTRAP_MAGIC.to_vec();
    header.extend_from_slice(&2_u32.to_be_bytes());
    raw_send(&sender, &header, &[&output, &output, &output]);
    drop(output);
    drop(sender);
    assert!(matches!(
        receiver.receive_bootstrap(deadline()),
        Err(LaunchChannelFailure::InvalidDescriptor)
    ));
    let mut byte = [0];
    assert_eq!(input.read(&mut byte).unwrap(), 0);
}

#[test]
fn bootstrap_body_truncation_closes_received_private_handles() {
    let (sender, receiver) = UnixStream::pair().unwrap();
    let receiver = LaunchChannel::new(receiver).unwrap();
    let (input, output) = nonblocking_pipe();
    let mut input = File::from(input);
    let output = File::from(output);
    let mut header = BOOTSTRAP_MAGIC.to_vec();
    header.extend_from_slice(&1024_u32.to_be_bytes());
    raw_send(&sender, &header, &[&output, &output]);
    drop(output);
    drop(sender);
    assert!(matches!(
        receiver.receive_bootstrap(deadline()),
        Err(LaunchChannelFailure::TransportUnavailable)
    ));
    assert_eq!(input.read(&mut [0]).unwrap(), 0);
}

pub(crate) fn raw_send(socket: &UnixStream, bytes: &[u8], files: &[&File]) {
    let fds = files.iter().map(|fd| fd.as_fd()).collect::<Vec<_>>();
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(32))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    if !fds.is_empty() {
        assert!(ancillary.push(SendAncillaryMessage::ScmRights(&fds)));
    }
    #[cfg(target_os = "linux")]
    let flags = SendFlags::NOSIGNAL;
    #[cfg(not(target_os = "linux"))]
    let flags = SendFlags::empty();
    assert_eq!(
        sendmsg(socket, &[IoSlice::new(bytes)], &mut ancillary, flags).unwrap(),
        bytes.len()
    );
}

fn encoded(value: &LaunchFrame) -> Vec<u8> {
    let body = serde_json::to_vec(value).unwrap();
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
    bytes.extend(body);
    bytes
}

fn null_io() -> PayloadIo {
    PayloadIo {
        stdin: None,
        stdout: None,
        stderr: None,
    }
}

fn nonblocking_pipe() -> (OwnedFd, OwnedFd) {
    let (input, output) = rustix::pipe::pipe().unwrap();
    for fd in [&input, &output] {
        fcntl_setfd(fd, FdFlags::CLOEXEC).unwrap();
        rustix::io::ioctl_fionbio(fd, true).unwrap();
    }
    (input, output)
}

#[test]
fn owner_launch_channel_preserves_raw_arguments_and_effective_environment() {
    let root = Directory::new();
    let expected = credential();
    let mut invocation = ProcessInvocation::new(OsString::from_vec(b"/bin/sh".to_vec()));
    invocation.arg(OsString::from_vec(b"a b;\xff".to_vec()));
    let launch = ProcessLaunch::new(
        invocation,
        root.0.clone(),
        vec![(
            OsString::from_vec(b"RAW\xfe".to_vec()),
            OsString::from_vec(b"v\xff".to_vec()),
        )],
        null_io(),
    )
    .unwrap();
    let (sender, receiver) = channel_pair();
    sender.send(launch, expected.clone(), deadline()).unwrap();
    let command = receiver
        .receive(&expected, deadline())
        .unwrap()
        .0
        .into_command();
    assert_eq!(command.get_program(), "/bin/sh");
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [OsString::from_vec(b"a b;\xff".to_vec())]
    );
    assert_eq!(
        command.get_envs().collect::<Vec<_>>(),
        [(
            OsString::from_vec(b"RAW\xfe".to_vec()).as_os_str(),
            Some(OsString::from_vec(b"v\xff".to_vec()).as_os_str()),
        )]
    );
    assert!(!root.0.join("unexpected").exists());
}

#[test]
fn owner_launch_channel_accepts_fragmented_header_without_repeating_rights() {
    let root = Directory::new();
    let expected = credential();
    let bytes = encoded(&frame(expected.clone()));
    let (sender, receiver) = channel_pair();
    let cwd = File::open(&root.0).unwrap();
    raw_send(&sender.socket, &bytes[..1], &[&cwd]);
    raw_send(&sender.socket, &bytes[1..], &[]);
    drop(sender);
    assert!(receiver.receive(&expected, deadline()).is_ok());
}

#[test]
fn owner_launch_channel_rejects_wrong_job_nonce_and_owner_binding() {
    for change in 0..5 {
        let root = Directory::new();
        let expected = credential();
        let mut wrong = expected.clone();
        match change {
            0 => wrong.job_id = uuid::Uuid::new_v4(),
            1 => wrong.nonce = uuid::Uuid::new_v4(),
            2 => wrong.owner.pid = if wrong.owner.pid == 2 { 3 } else { 2 },
            3 => wrong.owner.boot_id = uuid::Uuid::new_v4(),
            _ => match &mut wrong.owner.started {
                super::super::instance::ProcessStart::LinuxTicks { ticks } => *ticks += 1,
                super::super::instance::ProcessStart::MacOsMicroseconds { seconds, .. } => {
                    *seconds += 1
                }
            },
        }
        let (sender, receiver) = channel_pair();
        raw_send(
            &sender.socket,
            &encoded(&frame(wrong)),
            &[&File::open(&root.0).unwrap()],
        );
        drop(sender);
        assert!(matches!(
            receiver.receive(&expected, deadline()),
            Err(LaunchChannelFailure::CredentialMismatch)
        ));
    }
}

#[test]
fn owner_launch_channel_refuses_a_foreign_receiver_birth_without_waiting_for_input() {
    let mut expected = credential();
    expected.owner.pid = if expected.owner.pid == 2 { 3 } else { 2 };
    let (_sender, receiver) = channel_pair();
    assert!(matches!(
        receiver.receive(&expected, deadline()),
        Err(LaunchChannelFailure::CredentialMismatch)
    ));
}

#[test]
fn owner_launch_channel_rejects_schema_fields_lengths_and_descriptor_roles() {
    for case in 0..10 {
        let root = Directory::new();
        let expected = credential();
        let mut value = frame(expected.clone());
        let cwd = File::open(&root.0).unwrap();
        let mut files = vec![&cwd];
        match case {
            0 => value.schema_version = 2,
            1 => value.stdio_mask = 8,
            2 => value.stdio_mask = 1,
            3 => files.clear(),
            4 => files.push(&cwd),
            5 => value.program.clear(),
            6 => value.arguments.push(b"nul\0arg".to_vec()),
            7 => {
                value.environment = vec![
                    [b"K".to_vec(), b"1".to_vec()],
                    [b"K".to_vec(), b"2".to_vec()],
                ]
            }
            _ => {}
        }
        let mut bytes = encoded(&value);
        if case == 8 {
            bytes[4..8].copy_from_slice(&((MAX_LAUNCH_BYTES + 1) as u32).to_be_bytes());
        }
        if case == 9 {
            bytes[0] = b'?';
        }
        let (sender, receiver) = channel_pair();
        raw_send(&sender.socket, &bytes, &files);
        drop(sender);
        assert!(
            receiver.receive(&expected, deadline()).is_err(),
            "case={case}"
        );
    }
}

#[test]
fn owner_launch_channel_rejects_unknown_fields_without_a_payload_effect() {
    let root = Directory::new();
    let expected = credential();
    let mut value = serde_json::to_value(frame(expected.clone())).unwrap();
    value["command"] = serde_json::json!(["/bin/sh", "-c", "unexpected"]);
    let body = serde_json::to_vec(&value).unwrap();
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
    bytes.extend(body);
    let (sender, receiver) = channel_pair();
    raw_send(&sender.socket, &bytes, &[&File::open(&root.0).unwrap()]);
    drop(sender);
    assert!(matches!(
        receiver.receive(&expected, deadline()),
        Err(LaunchChannelFailure::InvalidFrame)
    ));
}

#[test]
fn owner_launch_channel_rejects_control_socket_as_a_data_descriptor() {
    for sender_side in [false, true] {
        let root = Directory::new();
        let expected = credential();
        let (sender, receiver) = channel_pair();
        let socket = if sender_side {
            &sender.socket
        } else {
            &receiver.socket
        };
        let duplicate = File::from(OwnedFd::from(socket.try_clone().unwrap()));
        let launch = ProcessLaunch::new(
            ProcessInvocation::new("/bin/sh"),
            root.0.clone(),
            vec![],
            PayloadIo {
                stdin: Some(duplicate),
                stdout: None,
                stderr: None,
            },
        )
        .unwrap();
        assert!(matches!(
            sender.send(launch, expected, deadline()),
            Err(LaunchChannelFailure::InvalidDescriptor)
        ));
    }
    // A malicious sender bypassing the ordinary sender validation is rejected too.
    let root = Directory::new();
    let expected = credential();
    let (sender, receiver) = channel_pair();
    let duplicate = File::from(OwnedFd::from(sender.socket.try_clone().unwrap()));
    let mut value = frame(expected.clone());
    value.stdio_mask = 1;
    raw_send(
        &sender.socket,
        &encoded(&value),
        &[&File::open(&root.0).unwrap(), &duplicate],
    );
    drop(sender);
    assert!(matches!(
        receiver.receive(&expected, deadline()),
        Err(LaunchChannelFailure::InvalidDescriptor)
    ));
}

#[test]
fn owner_launch_channel_received_handles_are_cloexec_and_roles_keep_their_access_modes() {
    for invalid in [false, true] {
        let root = Directory::new();
        let expected = credential();
        let cwd = File::open(&root.0).unwrap();
        let output = File::create(root.0.join("output")).unwrap();
        let mut value = frame(expected.clone());
        value.stdio_mask = if invalid { 1 } else { 2 };
        let (sender, receiver) = channel_pair();
        raw_send(&sender.socket, &encoded(&value), &[&cwd, &output]);
        drop(sender);
        let received = receiver.receive(&expected, deadline());
        if invalid {
            assert!(matches!(
                received,
                Err(LaunchChannelFailure::InvalidLaunch(
                    LaunchFailure::InvalidStdio
                ))
            ));
        } else {
            let (_, cwd, _, io) = received.unwrap().0.into_parts();
            for fd in [&cwd, io.stdout.as_ref().unwrap()] {
                assert!(fcntl_getfd(fd).unwrap().contains(FdFlags::CLOEXEC));
            }
            assert!(io.stdin.is_none());
            assert!(io.stderr.is_none());
        }
    }
}

#[test]
fn owner_launch_channel_closes_all_rights_on_bad_frames_and_ancillary_truncation() {
    for case in 0..4 {
        let root = Directory::new();
        let expected = credential();
        let (input, output) = nonblocking_pipe();
        let mut input = File::from(input);
        let output = File::from(output);
        let cwd = File::open(&root.0).unwrap();
        let mut value = frame(expected.clone());
        value.stdio_mask = 2;
        let mut bytes = encoded(&value);
        let mut files = vec![&cwd, &output];
        match case {
            0 => bytes[0] = b'?',
            1 => {
                files = vec![&output; 32];
            }
            2 => {
                bytes.truncate(9);
            }
            _ => {
                bytes[8] = b'?';
            }
        }
        let (sender, receiver) = channel_pair();
        raw_send(&sender.socket, &bytes, &files);
        drop(files);
        drop(output);
        drop(sender);
        assert!(receiver.receive(&expected, deadline()).is_err());
        assert_eq!(
            input.read(&mut [0; 1]).unwrap(),
            0,
            "leaked writer in case={case}"
        );
    }
}

#[test]
fn owner_launch_channel_rejects_rights_attached_to_the_body() {
    let root = Directory::new();
    let expected = credential();
    let bytes = encoded(&frame(expected.clone()));
    let cwd = File::open(&root.0).unwrap();
    let (sender, receiver) = channel_pair();
    raw_send(&sender.socket, &bytes[..8], &[&cwd]);
    raw_send(&sender.socket, &bytes[8..], &[&cwd]);
    drop(sender);
    assert!(matches!(
        receiver.receive(&expected, deadline()),
        Err(LaunchChannelFailure::InvalidDescriptor)
    ));
}

#[test]
fn owner_launch_channel_startup_budget_is_absolute_even_with_slow_partial_input() {
    let expected = credential();
    let bytes = encoded(&frame(expected.clone()));
    let (sender, receiver) = channel_pair();
    let started = Instant::now();
    let writer = std::thread::spawn(move || {
        let mut socket = sender.socket;
        for byte in bytes.into_iter().take(8) {
            if socket.write_all(&[byte]).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    assert!(matches!(
        receiver.receive(&expected, started + Duration::from_millis(55)),
        Err(LaunchChannelFailure::StartupTimeout)
    ));
    assert!(started.elapsed() < Duration::from_millis(500));
    writer.join().unwrap();
}

#[test]
fn owner_launch_channel_expired_or_disconnected_peer_never_launches() {
    for expired in [false, true] {
        let root = Directory::new();
        let launch = ProcessLaunch::new(
            ProcessInvocation::new("/bin/sh"),
            root.0.clone(),
            vec![],
            null_io(),
        )
        .unwrap();
        let (sender, receiver) = channel_pair();
        drop(receiver);
        let end = if expired { Instant::now() } else { deadline() };
        assert!(matches!(
            sender.send(launch, credential(), end),
            Err(LaunchChannelFailure::StartupTimeout | LaunchChannelFailure::TransportUnavailable)
        ));
    }
}

#[test]
fn owner_launch_channel_stalled_sender_has_a_startup_budget_not_an_unbounded_write() {
    let root = Directory::new();
    let mut invocation = ProcessInvocation::new("/bin/sh");
    invocation.arg(OsString::from_vec(vec![b'x'; 64 * 1024]));
    let launch = ProcessLaunch::new(invocation, root.0.clone(), vec![], null_io()).unwrap();
    let (sender, _receiver) = channel_pair();
    rustix::net::sockopt::set_socket_send_buffer_size(&sender.socket, 4096).unwrap();
    let started = Instant::now();
    assert!(matches!(
        sender.send(launch, credential(), started + Duration::from_millis(100)),
        Err(LaunchChannelFailure::StartupTimeout)
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn owner_launch_channel_bounded_encoder_never_exceeds_its_frame_budget() {
    let mut body = BoundedBody(Vec::new());
    body.write_all(&vec![b'x'; MAX_LAUNCH_BYTES - 1]).unwrap();
    assert!(body.write_all(b"xy").is_err());
    assert_eq!(body.0.len(), MAX_LAUNCH_BYTES - 1);
    body.write_all(b"x").unwrap();
    assert!(body.write_all(b"x").is_err());
    assert_eq!(body.0.len(), MAX_LAUNCH_BYTES);
}

#[test]
fn owner_launch_channel_bounds_sequence_allocation_before_materializing_a_launch() {
    for count in [MAX_LAUNCH_ITEMS, MAX_LAUNCH_ITEMS + 1] {
        let mut value = frame(credential());
        value.arguments = std::iter::repeat_with(Vec::new).take(count).collect();
        let bytes = encoded(&value);
        drop(value);
        assert!(bytes.len() < MAX_LAUNCH_BYTES);
        let result = serde_json::from_slice::<LaunchFrame>(&bytes[8..]);
        if count == MAX_LAUNCH_ITEMS {
            assert_eq!(result.unwrap().arguments.len(), count);
        } else {
            assert!(result.is_err());
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn owner_launch_channel_real_cross_process_handoff_preserves_data_handles_and_single_commit() {
    let root = Directory::new();
    let original = root.0.join("cwd");
    fs::create_dir(&original).unwrap();
    let (input, output) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).unwrap();
    let mut writer = File::from(output);
    let stdout = root.0.join("stdout");
    let stderr = root.0.join("stderr");
    let mut invocation = ProcessInvocation::new("/bin/sh");
    invocation.arg("-c").arg(
        "printf x >> marker; read -r value; printf '%s|%s' \"$VALUE\" \"$value\"; printf err >&2",
    );
    let launch = ProcessLaunch::new(
        invocation,
        original.clone(),
        vec![("VALUE".into(), "exact".into())],
        PayloadIo {
            stdin: Some(File::from(input)),
            stdout: Some(File::create(&stdout).unwrap()),
            stderr: Some(File::create(&stderr).unwrap()),
        },
    )
    .unwrap();
    fs::rename(&original, root.0.join("pinned")).unwrap();
    fs::create_dir(&original).unwrap();
    let (child, parent) = spawn_fixture("launch_handoff", &root.0, uuid::Uuid::from_u128(1), None);
    let expected = super::super::owner_store::tests::key()
        .credential_for(
            uuid::Uuid::from_u128(1),
            super::super::instance::observe_process(child.id())
                .unwrap()
                .birth,
            super::super::owner_store::tests::binding(),
        )
        .unwrap();
    super::super::owner_store::tests::store(&root.0)
        .publish(&expected, super::super::owner_store::tests::binding())
        .unwrap();
    LaunchChannel::new(parent)
        .unwrap()
        .send(launch, expected, deadline())
        .unwrap();
    writer.write_all(b"done\n").unwrap();
    drop(writer);
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    println!("{}", String::from_utf8_lossy(&result.stdout));
    assert_eq!(fs::read(root.0.join("pinned/marker")).unwrap(), b"x");
    assert!(!original.join("marker").exists());
    assert_eq!(fs::read(stdout).unwrap(), b"exact|done");
    assert_eq!(fs::read(stderr).unwrap(), b"err");
}

#[cfg(target_os = "linux")]
pub(crate) fn run_handoff() {
    use super::super::owner_protocol::OwnerPhase;
    use super::super::owner_start::OwnerStart;
    // Only this isolated fixture's launcher supplies this private descriptor.
    let socket = unsafe { UnixStream::from_raw_fd(198) };
    let expected = super::super::owner_store::tests::key()
        .credential_for(
            uuid::Uuid::from_u128(1),
            super::super::instance::observe_process(std::process::id())
                .unwrap()
                .birth,
            super::super::owner_store::tests::binding(),
        )
        .unwrap();
    let launch = LaunchChannel::new(socket)
        .unwrap()
        .receive(&expected, deadline())
        .unwrap()
        .0;
    let root = std::env::var_os("APP_OWNER_REFERENCE_TEST_ROOT").unwrap();
    let reference = super::super::owner_store::tests::store(std::path::Path::new(&root))
        .load(expected.job_id)
        .unwrap()
        .unwrap();
    let mut start = OwnerStart::prepare(
        launch,
        expected.job_id,
        expected.nonce,
        super::super::owner_store::tests::binding(),
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(unsafe { start.commit(&reference) }.unwrap() == expected);
    assert!(unsafe { start.commit(&reference) }.unwrap() == expected);
    let end = deadline();
    loop {
        let status = start.runtime().unwrap().tick().unwrap();
        if status.phase == OwnerPhase::Terminal {
            assert_eq!(status.root_exit.unwrap().code, Some(0));
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn spawn_fixture(
    case: &str,
    root: &std::path::Path,
    job_id: uuid::Uuid,
    authority: Option<super::super::owner_reference::AuthorityFingerprint>,
) -> (std::process::Child, UnixStream) {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    let (parent, child_socket) = UnixStream::pair().unwrap();
    let mut command = super::super::owner_scope::tests::fixture_command(case, None);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("APP_OWNER_REFERENCE_TEST_ROOT", root)
        .env("APP_OWNER_REFERENCE_TEST_JOB", job_id.to_string());
    if let Some(authority) = authority {
        command.env(
            "APP_OWNER_AUTHORITY_TEST_FINGERPRINT",
            serde_json::to_string(authority.as_bytes()).unwrap(),
        );
    }
    unsafe {
        command.pre_exec(move || {
            if child_socket.as_raw_fd() == 198 {
                if libc::fcntl(198, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
            } else if libc::dup2(child_socket.as_raw_fd(), 198) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().unwrap();
    drop(command);
    (child, parent)
}

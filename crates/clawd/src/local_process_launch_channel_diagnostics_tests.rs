use super::*;

pub(crate) fn assert_failure<T>(
    actual: Result<T, LaunchChannelFailure>,
    expected: LaunchChannelFailure,
) {
    assert_eq!(actual.err(), Some(expected));
}

pub(super) fn receive_error(phase: &str, requested: usize, errno: Option<i32>) {
    eprintln!(
        "LAUNCH_RECEIVE_DIAGNOSTIC {}",
        serde_json::json!({
            "phase": phase, "requested_bytes": requested, "errno": errno,
            "test": std::thread::current().name(),
        })
    );
}

pub(super) fn receive_flags(requested: usize, flags: u64) {
    eprintln!(
        "LAUNCH_RECEIVE_DIAGNOSTIC {}",
        serde_json::json!({
            "phase": "truncated", "requested_bytes": requested, "flags": flags,
            "test": std::thread::current().name(),
        })
    );
}

pub(super) fn receive_zero(requested: usize) {
    eprintln!(
        "LAUNCH_RECEIVE_DIAGNOSTIC {}",
        serde_json::json!({
            "phase": "eof", "requested_bytes": requested,
            "test": std::thread::current().name(),
        })
    );
}

#[test]
fn native_ancillary_framing_matches_when_sources_and_senders_close() {
    let root = tempfile::tempdir().unwrap();
    let body = b"owned-framing-body";
    let mut header = *b"LCH1\0\0\0\0";
    header[4..].copy_from_slice(&(body.len() as u32).to_be_bytes());
    for one_send in [false, true] {
        for close_source in [false, true] {
            for close_sender in [false, true] {
                let (sender, receiver) = UnixStream::pair().unwrap();
                let receiver = LaunchChannel::new(receiver).unwrap();
                receiver
                    .socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let source = File::open(root.path()).unwrap();
                let bytes = if one_send {
                    [header.as_slice(), body].concat()
                } else {
                    header.to_vec()
                };
                super::tests::raw_send(&sender, &bytes, &[&source]);
                if !one_send {
                    super::tests::raw_send(&sender, body, &[]);
                }
                let source = (!close_source).then_some(source);
                let sender = (!close_sender).then_some(sender);
                let mut actual_header = [0; 8];
                let mut space =
                    [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_HANDLES))];
                let mut ancillary = RecvAncillaryBuffer::new(&mut space);
                #[cfg(target_os = "linux")]
                let flags = RecvFlags::CMSG_CLOEXEC;
                #[cfg(not(target_os = "linux"))]
                let flags = RecvFlags::empty();
                let message = recvmsg(&receiver.socket, &mut [IoSliceMut::new(&mut actual_header)], &mut ancillary, flags)
                    .unwrap_or_else(|error| panic!("native header errno={} one_send={one_send} close_source={close_source} close_sender={close_sender}", error.raw_os_error()));
                assert_eq!(message.bytes, header.len());
                assert_eq!(actual_header, header);
                assert!(!message
                    .flags
                    .intersects(ReturnFlags::CTRUNC | ReturnFlags::TRUNC));
                let mut received = Vec::new();
                for message in ancillary.drain() {
                    match message {
                        RecvAncillaryMessage::ScmRights(rights) => {
                            for fd in rights {
                                let current = fcntl_getfd(&fd).unwrap();
                                fcntl_setfd(&fd, current | FdFlags::CLOEXEC).unwrap();
                                received.push(fd);
                            }
                        }
                        _ => panic!("unexpected ancillary type"),
                    }
                }
                assert_eq!(received.len(), 1);
                assert!(File::from(received[0].try_clone().unwrap())
                    .metadata()
                    .unwrap()
                    .is_dir());
                let mut actual_body = vec![0; body.len()];
                let message = recvmsg(&receiver.socket, &mut [IoSliceMut::new(&mut actual_body)], &mut ancillary, flags)
                    .unwrap_or_else(|error| panic!("native body errno={} one_send={one_send} close_source={close_source} close_sender={close_sender}", error.raw_os_error()));
                assert_eq!(message.bytes, body.len());
                assert!(!message
                    .flags
                    .intersects(ReturnFlags::CTRUNC | ReturnFlags::TRUNC));
                assert_eq!(actual_body, body);
                assert!(ancillary.drain().next().is_none());
                println!("LAUNCH_RAW_FRAMING_ACCEPTANCE one_send={one_send} source_closed={close_source} sender_closed={close_sender}");
                drop((sender, source, received));
            }
        }
    }
}

use std::io::ErrorKind;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixDatagram;

use super::owner_protocol::{
    decode_request, encode_reply, OwnerFailure, OwnerOutcome, OwnerReply, OwnerSnapshot,
    MAX_CONTROL_BYTES,
};
use super::owner_runtime::OwnerRuntime;

pub(super) const MAX_PACKETS_PER_PUMP: usize = 16;

pub(crate) struct OwnerDatagram {
    socket: UnixDatagram,
}

#[derive(Debug)]
pub(crate) struct OwnerIpcStep {
    pub(crate) received: usize,
    pub(crate) rejected: usize,
    pub(crate) replies_failed: usize,
    pub(crate) transport_unavailable: bool,
    pub(crate) observation: Result<OwnerSnapshot, OwnerFailure>,
}

impl OwnerDatagram {
    // Construct before starting the payload. These checks are a local endpoint
    // baseline, not a replacement for host-owned directories/sandbox fencing.
    pub(crate) fn new(socket: UnixDatagram) -> Result<Self, OwnerFailure> {
        Self::validate_endpoint(&socket)?;
        socket
            .set_nonblocking(true)
            .map_err(|_| OwnerFailure::TransportUnavailable)?;
        Ok(Self { socket })
    }

    pub(crate) fn validate_endpoint(socket: &UnixDatagram) -> Result<(), OwnerFailure> {
        let address = socket
            .local_addr()
            .map_err(|_| OwnerFailure::TransportUnavailable)?;
        let path = address
            .as_pathname()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or(OwnerFailure::TransportUnavailable)?;
        let endpoint =
            std::fs::symlink_metadata(path).map_err(|_| OwnerFailure::TransportUnavailable)?;
        let parent =
            std::fs::symlink_metadata(path.parent().ok_or(OwnerFailure::TransportUnavailable)?)
                .map_err(|_| OwnerFailure::TransportUnavailable)?;
        let uid = unsafe { libc::geteuid() };
        if !endpoint.file_type().is_socket()
            || endpoint.uid() != uid
            || endpoint.mode() & 0o777 != 0o600
            || !parent.is_dir()
            || parent.uid() != uid
            || parent.mode() & 0o777 != 0o700
        {
            return Err(OwnerFailure::TransportUnavailable);
        }
        Ok(())
    }

    pub(crate) fn pump(&self, owner: &mut OwnerRuntime) -> OwnerIpcStep {
        let (mut received, mut rejected, mut replies_failed) = (0, 0, 0);
        let mut transport_unavailable = false;
        // One extra byte detects truncation without allocating from packet data.
        let mut bytes = [0_u8; MAX_CONTROL_BYTES + 1];
        for _ in 0..MAX_PACKETS_PER_PUMP {
            let (length, sender) = match self.socket.recv_from(&mut bytes) {
                Ok(packet) => packet,
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    transport_unavailable = true;
                    break;
                }
            };
            received += 1;
            let Some(path) = sender
                .as_pathname()
                .filter(|path| !path.as_os_str().is_empty())
            else {
                rejected += 1;
                continue;
            };
            let request = match decode_request(&bytes[..length]) {
                Ok(request) => request,
                Err(_) => {
                    rejected += 1;
                    continue;
                }
            };
            let reply = match owner.handle(&request) {
                Ok(reply) => reply,
                Err(
                    OwnerFailure::CredentialMismatch
                    | OwnerFailure::InvalidCredential
                    | OwnerFailure::UnsupportedSchema,
                ) => {
                    rejected += 1;
                    continue;
                }
                Err(error_code) => OwnerReply {
                    schema_version: 1,
                    request_id: request.request_id,
                    outcome: OwnerOutcome::Error { error_code },
                },
            };
            match encode_reply(&reply) {
                Ok(reply) => {
                    if self.socket.send_to(&reply, path).is_err() {
                        replies_failed += 1;
                    }
                }
                Err(_) => replies_failed += 1,
            }
        }
        // A lost peer/reply or a packet flood cannot skip scope observation.
        OwnerIpcStep {
            received,
            rejected,
            replies_failed,
            transport_unavailable,
            observation: owner.tick(),
        }
    }
}

#[cfg(test)]
#[path = "local_process_owner_transport_tests.rs"]
pub(super) mod tests;

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Write};
use std::io::{IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::AsFd;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::time::Instant;

use rustix::io::{fcntl_getfd, fcntl_setfd, FdFlags};
use rustix::net::SendAncillaryMessage;
use rustix::net::{
    recvmsg, sendmsg, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags,
    SendAncillaryBuffer, SendFlags,
};
use serde::{Deserialize, Serialize};

use super::owner_protocol::{OwnerCredential, MAX_CONTROL_BYTES};
use super::owner_runtime::OwnerRuntime;
use super::process_launch::{LaunchFailure, PayloadIo, ProcessLaunch};
use crate::process_invocation::ProcessInvocation;

const MAX_LAUNCH_BYTES: usize = 32 * 1024 * 1024;
const MAX_LAUNCH_ITEMS: usize = MAX_LAUNCH_BYTES / 32;
const MAX_HANDLES: usize = 4;
const MAGIC: &[u8; 4] = b"LCH1";
const STARTUP_MAGIC: &[u8; 4] = b"OST1";
const BOOTSTRAP_MAGIC: &[u8; 4] = b"OBT1";

#[path = "local_process_owner_package_lease.rs"]
mod package_lease;
pub(crate) use package_lease::package_custody_binding;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchChannelFailure {
    InvalidFrame,
    UnsupportedSchema,
    InvalidCredential,
    CredentialMismatch,
    InvalidDescriptor,
    InvalidLaunch(LaunchFailure),
    LimitExceeded,
    StartupTimeout,
    TransportUnavailable,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Unsupported,
}

// Never persist or log this private frame: it contains the effective environment.
// Descriptor roles are cwd, then the present stdin/stdout/stderr bits, in order.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchFrame {
    schema_version: u32,
    credential: OwnerCredential,
    program: Vec<u8>,
    #[serde(deserialize_with = "bounded_items")]
    arguments: Vec<Vec<u8>>,
    #[serde(deserialize_with = "bounded_items")]
    environment: Vec<[Vec<u8>; 2]>,
    stdio_mask: u8,
}

// Transfer consumes its phase even on partial I/O, invalid input, or a lost peer.
// A successful transfer retains the private endpoint for the startup handshake.
// Only a trusted private endpoint may be supplied. Receiving belongs in the
// exclusive pre-spawn helper, particularly on systems without CMSG_CLOEXEC.
pub(crate) struct LaunchChannel {
    socket: UnixStream,
}

impl LaunchChannel {
    pub(crate) fn new(socket: UnixStream) -> Result<Self, LaunchChannelFailure> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return Err(LaunchChannelFailure::Unsupported);
        #[cfg(target_os = "macos")]
        rustix::net::sockopt::set_socket_nosigpipe(&socket, true)
            .map_err(|_| LaunchChannelFailure::TransportUnavailable)?;
        fcntl_setfd(&socket, FdFlags::CLOEXEC)
            .map_err(|_| LaunchChannelFailure::TransportUnavailable)?;
        socket
            .set_nonblocking(false)
            .map_err(|_| LaunchChannelFailure::TransportUnavailable)?;
        Ok(Self { socket })
    }

    pub(super) fn send_bootstrap(
        self,
        bytes: &[u8],
        handles: [OwnedFd; 2],
        deadline: Instant,
    ) -> Result<Self, LaunchChannelFailure> {
        if bytes.is_empty() || bytes.len() > MAX_CONTROL_BYTES {
            return Err(LaunchChannelFailure::LimitExceeded);
        }
        let mut header = [0; 8];
        header[..4].copy_from_slice(BOOTSTRAP_MAGIC);
        header[4..].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
        let borrowed = handles.iter().map(AsFd::as_fd).collect::<Vec<_>>();
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        if !ancillary.push(SendAncillaryMessage::ScmRights(&borrowed)) {
            return Err(LaunchChannelFailure::InvalidDescriptor);
        }
        let sent = self.send_piece(&header, &mut ancillary, deadline)?;
        self.send_bytes(&header[sent..], deadline)?;
        self.send_bytes(bytes, deadline)?;
        Ok(self)
    }

    pub(super) fn receive_bootstrap(
        self,
        deadline: Instant,
    ) -> Result<(Vec<u8>, [OwnedFd; 2], Self), LaunchChannelFailure> {
        let mut header = [0; 8];
        let mut handles = Vec::new();
        let count = self.receive_piece(&mut header, Some(&mut handles), deadline)?;
        self.receive_bytes(&mut header[count..], deadline)?;
        if &header[..4] != BOOTSTRAP_MAGIC {
            return Err(LaunchChannelFailure::InvalidFrame);
        }
        let count = u32::from_be_bytes(header[4..].try_into().unwrap()) as usize;
        if count == 0 || count > MAX_CONTROL_BYTES {
            return Err(LaunchChannelFailure::LimitExceeded);
        }
        let handles = handles
            .try_into()
            .map_err(|_| LaunchChannelFailure::InvalidDescriptor)?;
        let mut bytes = vec![0; count];
        self.receive_bytes(&mut bytes, deadline)?;
        remaining(deadline)?;
        Ok((bytes, handles, self))
    }

    pub(crate) fn send(
        self,
        launch: ProcessLaunch,
        credential: OwnerCredential,
        deadline: Instant,
    ) -> Result<Self, LaunchChannelFailure> {
        credential
            .validate()
            .map_err(|_| LaunchChannelFailure::InvalidCredential)?;
        let (invocation, cwd, environment, io) = launch.into_parts();
        let mut handles = vec![cwd];
        let mut mask = 0;
        for (bit, file) in [(1, io.stdin), (2, io.stdout), (4, io.stderr)] {
            if let Some(file) = file {
                validate_data_handle(&file)?;
                mask |= bit;
                handles.push(file);
            }
        }
        let (program, arguments) = invocation.into_parts();
        if arguments.len() > MAX_LAUNCH_ITEMS || environment.len() > MAX_LAUNCH_ITEMS {
            return Err(LaunchChannelFailure::LimitExceeded);
        }
        let frame = LaunchFrame {
            schema_version: 1,
            credential,
            program: program.into_vec(),
            arguments: arguments.into_iter().map(OsString::into_vec).collect(),
            environment: environment
                .into_iter()
                .map(|(key, value)| [key.into_vec(), value.into_vec()])
                .collect(),
            stdio_mask: mask,
        };
        let mut body = BoundedBody(Vec::new());
        serde_json::to_writer(&mut body, &frame)
            .map_err(|_| LaunchChannelFailure::LimitExceeded)?;
        let mut header = [0_u8; 8];
        header[..4].copy_from_slice(MAGIC);
        header[4..].copy_from_slice(&(body.0.len() as u32).to_be_bytes());
        let borrowed = handles.iter().map(AsFd::as_fd).collect::<Vec<_>>();
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_HANDLES))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        if !ancillary.push(SendAncillaryMessage::ScmRights(&borrowed)) {
            return Err(LaunchChannelFailure::InvalidDescriptor);
        }
        let sent = self.send_piece(&header, &mut ancillary, deadline)?;
        self.send_bytes(&header[sent..], deadline)?;
        self.send_bytes(&body.0, deadline)?;
        Ok(self)
    }

    pub(crate) fn receive(
        self,
        expected: &OwnerCredential,
        deadline: Instant,
    ) -> Result<(ProcessLaunch, Self), LaunchChannelFailure> {
        expected
            .validate()
            .map_err(|_| LaunchChannelFailure::InvalidCredential)?;
        let current = OwnerRuntime::prepare_credential(expected.job_id, expected.nonce)
            .map_err(|_| LaunchChannelFailure::InvalidCredential)?;
        if &current != expected {
            return Err(LaunchChannelFailure::CredentialMismatch);
        }
        let mut header = [0_u8; 8];
        let mut handles = Vec::new();
        let count = self.receive_piece(&mut header, Some(&mut handles), deadline)?;
        self.receive_bytes(&mut header[count..], deadline)?;
        if &header[..4] != MAGIC {
            return Err(LaunchChannelFailure::InvalidFrame);
        }
        let length = u32::from_be_bytes(header[4..].try_into().unwrap()) as usize;
        if length == 0 || length > MAX_LAUNCH_BYTES {
            return Err(LaunchChannelFailure::LimitExceeded);
        }
        let mut body = vec![0; length];
        self.receive_bytes(&mut body, deadline)?;
        let frame: LaunchFrame =
            serde_json::from_slice(&body).map_err(|_| LaunchChannelFailure::InvalidFrame)?;
        if frame.schema_version != 1 {
            return Err(LaunchChannelFailure::UnsupportedSchema);
        }
        frame
            .credential
            .validate()
            .map_err(|_| LaunchChannelFailure::InvalidCredential)?;
        if &frame.credential != expected {
            return Err(LaunchChannelFailure::CredentialMismatch);
        }
        if frame.stdio_mask & !7 != 0 || handles.len() != 1 + frame.stdio_mask.count_ones() as usize
        {
            return Err(LaunchChannelFailure::InvalidDescriptor);
        }
        let mut handles = handles.into_iter().map(File::from);
        let cwd = handles
            .next()
            .ok_or(LaunchChannelFailure::InvalidDescriptor)?;
        let mut data = |bit| -> Result<Option<File>, LaunchChannelFailure> {
            if frame.stdio_mask & bit == 0 {
                return Ok(None);
            }
            let file = handles
                .next()
                .ok_or(LaunchChannelFailure::InvalidDescriptor)?;
            validate_data_handle(&file)?;
            Ok(Some(file))
        };
        let io = PayloadIo {
            stdin: data(1)?,
            stdout: data(2)?,
            stderr: data(4)?,
        };
        let mut invocation = ProcessInvocation::new(OsString::from_vec(frame.program));
        for arg in frame.arguments {
            invocation.arg(OsString::from_vec(arg));
        }
        let environment = frame
            .environment
            .into_iter()
            .map(|[key, value]| (OsString::from_vec(key), OsString::from_vec(value)))
            .collect();
        let launch = ProcessLaunch::with_working_directory(invocation, cwd, environment, io)
            .map_err(LaunchChannelFailure::InvalidLaunch)?;
        remaining(deadline)?;
        Ok((launch, self))
    }

    pub(super) fn send_control(
        &self,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<(), LaunchChannelFailure> {
        if bytes.is_empty() || bytes.len() > MAX_CONTROL_BYTES {
            return Err(LaunchChannelFailure::LimitExceeded);
        }
        let mut header = [0; 8];
        header[..4].copy_from_slice(STARTUP_MAGIC);
        header[4..].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
        self.send_bytes(&header, deadline)?;
        self.send_bytes(bytes, deadline)
    }

    pub(super) fn receive_control(
        &self,
        deadline: Instant,
    ) -> Result<Vec<u8>, LaunchChannelFailure> {
        let mut header = [0; 8];
        self.receive_bytes(&mut header, deadline)?;
        if &header[..4] != STARTUP_MAGIC {
            return Err(LaunchChannelFailure::InvalidFrame);
        }
        let count = u32::from_be_bytes(header[4..].try_into().unwrap()) as usize;
        if count == 0 || count > MAX_CONTROL_BYTES {
            return Err(LaunchChannelFailure::LimitExceeded);
        }
        let mut bytes = vec![0; count];
        self.receive_bytes(&mut bytes, deadline)?;
        remaining(deadline)?;
        Ok(bytes)
    }

    fn send_piece(
        &self,
        bytes: &[u8],
        ancillary: &mut SendAncillaryBuffer<'_, '_, '_>,
        deadline: Instant,
    ) -> Result<usize, LaunchChannelFailure> {
        loop {
            self.socket
                .set_write_timeout(Some(remaining(deadline)?))
                .map_err(|_| LaunchChannelFailure::TransportUnavailable)?;
            #[cfg(target_os = "linux")]
            let flags = SendFlags::NOSIGNAL;
            #[cfg(not(target_os = "linux"))]
            let flags = SendFlags::empty();
            match sendmsg(&self.socket, &[IoSlice::new(bytes)], ancillary, flags) {
                Ok(0) => return Err(LaunchChannelFailure::TransportUnavailable),
                Ok(count) => return Ok(count),
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(io_failure(error)),
            }
        }
    }

    fn send_bytes(&self, mut bytes: &[u8], deadline: Instant) -> Result<(), LaunchChannelFailure> {
        while !bytes.is_empty() {
            let count = self.send_piece(bytes, &mut SendAncillaryBuffer::default(), deadline)?;
            bytes = &bytes[count..];
        }
        Ok(())
    }

    fn receive_piece(
        &self,
        bytes: &mut [u8],
        handles: Option<&mut Vec<OwnedFd>>,
        deadline: Instant,
    ) -> Result<usize, LaunchChannelFailure> {
        loop {
            self.socket
                .set_read_timeout(Some(remaining(deadline)?))
                .map_err(|_error| {
                    #[cfg(test)]
                    diagnostics_tests::receive_error(
                        "set_read_timeout",
                        bytes.len(),
                        _error.raw_os_error(),
                    );
                    LaunchChannelFailure::TransportUnavailable
                })?;
            let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_HANDLES))];
            let mut ancillary = RecvAncillaryBuffer::new(&mut space);
            #[cfg(target_os = "linux")]
            let flags = RecvFlags::CMSG_CLOEXEC;
            #[cfg(not(target_os = "linux"))]
            let flags = RecvFlags::empty();
            let message = match recvmsg(
                &self.socket,
                &mut [IoSliceMut::new(bytes)],
                &mut ancillary,
                flags,
            ) {
                Ok(message) => message,
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => {
                    #[cfg(test)]
                    diagnostics_tests::receive_error(
                        "recvmsg",
                        bytes.len(),
                        Some(error.raw_os_error()),
                    );
                    return Err(io_failure(error));
                }
            };
            if message
                .flags
                .intersects(ReturnFlags::CTRUNC | ReturnFlags::TRUNC)
            {
                #[cfg(test)]
                diagnostics_tests::receive_flags(bytes.len(), message.flags.bits() as u64);
                return Err(LaunchChannelFailure::InvalidDescriptor);
            }
            let mut handles = handles;
            for message in ancillary.drain() {
                match message {
                    RecvAncillaryMessage::ScmRights(rights) => {
                        for fd in rights {
                            let destination = handles
                                .as_mut()
                                .ok_or(LaunchChannelFailure::InvalidDescriptor)?;
                            if destination.len() >= MAX_HANDLES {
                                return Err(LaunchChannelFailure::InvalidDescriptor);
                            }
                            // Linux obtains CLOEXEC atomically. The dedicated Mac
                            // helper sets it before it may create any child.
                            let flags = fcntl_getfd(&fd)
                                .map_err(|_| LaunchChannelFailure::InvalidDescriptor)?;
                            fcntl_setfd(&fd, flags | FdFlags::CLOEXEC)
                                .map_err(|_| LaunchChannelFailure::InvalidDescriptor)?;
                            destination.push(fd);
                        }
                    }
                    _ => return Err(LaunchChannelFailure::InvalidDescriptor),
                }
            }
            return if message.bytes == 0 {
                #[cfg(test)]
                diagnostics_tests::receive_zero(bytes.len());
                Err(LaunchChannelFailure::TransportUnavailable)
            } else {
                Ok(message.bytes)
            };
        }
    }

    fn receive_bytes(
        &self,
        mut bytes: &mut [u8],
        deadline: Instant,
    ) -> Result<(), LaunchChannelFailure> {
        while !bytes.is_empty() {
            let count = self.receive_piece(bytes, None, deadline)?;
            bytes = &mut bytes[count..];
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "local_process_launch_channel_diagnostics_tests.rs"]
pub(super) mod diagnostics_tests;

#[cfg(test)]
#[path = "local_process_launch_channel_truncation_tests.rs"]
mod truncation_tests;

fn validate_data_handle(file: &File) -> Result<(), LaunchChannelFailure> {
    let kind = file
        .metadata()
        .map_err(|_| LaunchChannelFailure::InvalidDescriptor)?
        .file_type();
    // Socketpair endpoint inodes differ: comparing only the local endpoint
    // cannot exclude the peer. This wire contract permits files/pipes/TTYs,
    // not any socket, including a control endpoint disguised as payload I/O.
    if kind.is_file() || kind.is_fifo() || kind.is_char_device() {
        Ok(())
    } else {
        Err(LaunchChannelFailure::InvalidDescriptor)
    }
}

fn remaining(deadline: Instant) -> Result<std::time::Duration, LaunchChannelFailure> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|value| !value.is_zero())
        .ok_or(LaunchChannelFailure::StartupTimeout)
}

fn io_failure(error: rustix::io::Errno) -> LaunchChannelFailure {
    if error == rustix::io::Errno::AGAIN || error == rustix::io::Errno::TIMEDOUT {
        LaunchChannelFailure::StartupTimeout
    } else {
        LaunchChannelFailure::TransportUnavailable
    }
}

struct BoundedBody(Vec<u8>);
impl Write for BoundedBody {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_LAUNCH_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("launch_frame_limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn bounded_items<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Items<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Items<T> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("bounded launch sequence")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Vec<T>, A::Error> {
            // Do not trust size_hint or allocate an unbounded array of empty
            // strings just because its serialized body fits the byte budget.
            let mut items = Vec::new();
            while let Some(item) = sequence.next_element()? {
                if items.len() == MAX_LAUNCH_ITEMS {
                    return Err(serde::de::Error::custom("launch_entry_limit"));
                }
                items.push(item);
            }
            Ok(items)
        }
    }
    deserializer.deserialize_seq(Items(std::marker::PhantomData))
}

#[cfg(test)]
#[path = "local_process_launch_channel_tests.rs"]
pub(super) mod tests;

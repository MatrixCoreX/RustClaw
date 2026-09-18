use super::protocol::MAX_FRAME;
use crate::Result;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[cfg(unix)]
pub fn nonblocking(fd: i32) -> Result<()> {
    // SAFETY: fcntl changes only the passed, owned pipe endpoint's flags.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err("wallet_worker_unavailable".into());
        }
    }
    Ok(())
}
#[cfg(unix)]
pub fn ready(fd: i32, write: bool, timeout: Duration) -> Result<bool> {
    let mut p = libc::pollfd {
        fd,
        events: if write { libc::POLLOUT } else { libc::POLLIN },
        revents: 0,
    };
    // SAFETY: p is a valid one-element pollfd array.
    let n = unsafe { libc::poll(&mut p, 1, timeout.as_millis().min(i32::MAX as u128) as i32) };
    if n < 0 {
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err("wallet_worker_unavailable".into());
    }
    Ok(n > 0)
}
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or("wallet_worker_timeout".into())
}
fn read_exact(
    reader: &mut (impl Read + Channel),
    mut buf: &mut [u8],
    deadline: Instant,
) -> Result<()> {
    while !buf.is_empty() {
        if !ready(raw(reader), false, remaining(deadline)?)? {
            continue;
        }
        match reader.read(buf) {
            Ok(0) => return Err("wallet_worker_unavailable".into()),
            Ok(n) => buf = &mut buf[n..],
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return Err("wallet_worker_unavailable".into()),
        }
    }
    Ok(())
}
pub fn read(reader: &mut (impl Read + Channel), timeout: Duration) -> Result<Zeroizing<Vec<u8>>> {
    let deadline = Instant::now() + timeout;
    let mut header = [0; 4];
    read_exact(reader, &mut header, deadline)?;
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err("wallet_worker_protocol_invalid".into());
    }
    let mut bytes = Zeroizing::new(vec![0; len]);
    read_exact(reader, &mut bytes, deadline)?;
    Ok(bytes)
}
pub fn write(writer: &mut (impl Write + Channel), bytes: &[u8], timeout: Duration) -> Result<()> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err("wallet_worker_protocol_invalid".into());
    }
    let deadline = Instant::now() + timeout;
    let header = (bytes.len() as u32).to_be_bytes();
    for mut buf in [&header[..], bytes] {
        while !buf.is_empty() {
            if !ready(raw(writer), true, remaining(deadline)?)? {
                continue;
            }
            match writer.write(buf) {
                Ok(0) => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(n) => buf = &buf[n..],
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err("wallet_worker_unavailable".into()),
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
pub trait Channel: AsRawFd {}
#[cfg(unix)]
impl<T: AsRawFd> Channel for T {}
#[cfg(unix)]
pub fn raw(channel: &impl Channel) -> i32 {
    channel.as_raw_fd()
}
#[cfg(windows)]
pub trait Channel: std::os::windows::io::AsRawHandle {}
#[cfg(windows)]
impl<T: std::os::windows::io::AsRawHandle> Channel for T {}
#[cfg(windows)]
pub fn raw(channel: &impl Channel) -> windows_sys::Win32::Foundation::HANDLE {
    channel.as_raw_handle()
}
#[cfg(windows)]
pub use super::windows_pipe::{nonblocking, ready};

use super::{files, secure_memory::LockedBytes, windows_security};
use std::{io::Write, mem::{size_of, zeroed}, os::windows::io::AsRawHandle, time::{Duration, Instant}};
use windows_sys::Win32::{Security::Authorization::SE_FILE_OBJECT, System::Memory::*};

#[test]
fn windows_guard_pages_and_locked_buffer_lifetime() {
    let mut bytes = LockedBytes::new(32).unwrap();
    bytes.copy_from_slice(&[0x5a;32]);
    let ptr = bytes.as_ptr();
    unsafe {
        let mut info: MEMORY_BASIC_INFORMATION = zeroed();
        assert_ne!(VirtualQuery(ptr.cast(), &mut info, size_of::<MEMORY_BASIC_INFORMATION>()), 0);
        assert_eq!(info.Protect, PAGE_READWRITE);
        assert_eq!(info.State, MEM_COMMIT);
        let mut guard: MEMORY_BASIC_INFORMATION = zeroed();
        assert_ne!(VirtualQuery(ptr.sub(1).cast(), &mut guard, size_of::<MEMORY_BASIC_INFORMATION>()), 0);
        assert_eq!(guard.State, MEM_RESERVE);
        assert_ne!(VirtualUnlock(ptr.cast(), 32), 0, "allocation was not locked");
        assert_ne!(VirtualLock(ptr.cast(), 32), 0);
        drop(bytes);
        assert_ne!(VirtualQuery(ptr.cast(), &mut info, size_of::<MEMORY_BASIC_INFORMATION>()), 0);
        assert_eq!(info.State, MEM_FREE);
    }
}

#[test]
fn windows_private_acl_repairs_and_hardlinks_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("vault");
    files::private_directory(&directory).unwrap();
    let file = directory.join("vault.json");
    files::write(&file, b"encrypted fixture").unwrap();
    assert_eq!(files::read_private(&file).unwrap(), b"encrypted fixture");
    let mut options = std::fs::OpenOptions::new();
    use std::os::windows::fs::OpenOptionsExt;
    options.access_mode(0x00020000 | 0x00040000);
    let handle = options.open(&file).unwrap();
    windows_security::apply(handle.as_raw_handle(), SE_FILE_OBJECT,
        &windows_security::descriptor("D:P(A;;FA;;;WD)").unwrap()).unwrap();
    assert_eq!(files::read_private(&file).unwrap(), b"encrypted fixture");
    drop(handle);
    std::fs::hard_link(&file, root.path().join("alias.json")).unwrap();
    assert_eq!(files::read_private(&file).unwrap_err(), "wallet_storage_invalid");
}

#[test]
fn windows_pipe_frames_timeouts_backpressure_and_eof_are_bounded() {
    use super::worker::{transport, windows_pipe};
    let (mut a, mut b) = windows_pipe::pair().unwrap();
    transport::write(&mut a, b"bounded fixture", Duration::from_secs(1)).unwrap();
    assert_eq!(&*transport::read(&mut b, Duration::from_secs(1)).unwrap(), b"bounded fixture");
    a.write_all(&10u32.to_be_bytes()).unwrap(); a.write_all(b"abc").unwrap();
    assert_eq!(transport::read(&mut b, Duration::from_millis(30)).unwrap_err(), "wallet_worker_timeout");
    let start = Instant::now();
    let full = vec![0x5a;131072];
    let mut denied = false;
    for _ in 0..16 {
        if transport::write(&mut a, &full, Duration::from_millis(30)).is_err() { denied = true; break; }
    }
    assert!(denied && start.elapsed() < Duration::from_secs(2));
    drop(a);
    // Separate empty connection to verify EOF independently of buffered frames.
    let (a, mut b) = windows_pipe::pair().unwrap(); drop(a);
    assert!(transport::read(&mut b, Duration::from_millis(30)).is_err());
}

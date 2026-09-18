//! Test-only same-user probe; administrator debug privileges are excluded.
pub fn check(pid: u32) {
    unsafe {
        use windows_sys::Win32::{Foundation::*, Security::*, System::Threading::*};
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        // Hosted CI can have SeDebugPrivilege enabled. That administrator power
        // intentionally bypasses process DACLs and is outside the peer model.
        // Impersonate the same account without privileges for this access check.
        let mut source = std::ptr::null_mut();
        assert_ne!(OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY | TOKEN_DUPLICATE, &mut source), 0);
        let source = OwnedHandle::from_raw_handle(source);
        let mut restricted = std::ptr::null_mut();
        assert_ne!(CreateRestrictedToken(source.as_raw_handle(), DISABLE_MAX_PRIVILEGE,
            0, std::ptr::null(), 0, std::ptr::null(), 0, std::ptr::null(), &mut restricted), 0);
        let restricted = OwnedHandle::from_raw_handle(restricted);
        assert_ne!(ImpersonateLoggedOnUser(restricted.as_raw_handle()), 0);
        struct Revert;
        impl Drop for Revert {
            fn drop(&mut self) { unsafe { assert_ne!(RevertToSelf(), 0); } }
        }
        let _revert = Revert;
        let read = OpenProcess(
            PROCESS_VM_READ | PROCESS_VM_WRITE | PROCESS_VM_OPERATION,
            0,
            pid,
        );
        if !read.is_null() {
            CloseHandle(read);
            panic!("peer memory handle must be denied");
        }
        assert_eq!(GetLastError(), ERROR_ACCESS_DENIED);
    }
}

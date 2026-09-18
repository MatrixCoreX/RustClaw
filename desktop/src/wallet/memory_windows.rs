use super::LockedBytes;
use crate::Result;
use std::{
    mem::zeroed,
    ptr::{null, NonNull},
};
use windows_sys::Win32::System::{Memory::*, SystemInformation::*};
use zeroize::Zeroize;

pub fn allocate(len: usize) -> Result<LockedBytes> {
    // SAFETY: a private reservation owns the entire range, including both
    // inaccessible guard pages. No borrowed pointers survive an error.
    unsafe {
        let mut info: SYSTEM_INFO = zeroed();
        GetSystemInfo(&mut info);
        let page = info.dwPageSize as usize;
        if page == 0 {
            return Err("wallet_memory_protection_unavailable".into());
        }
        let pages = len.max(1).div_ceil(page) * page;
        let mapped = pages + 2 * page;
        let raw = VirtualAlloc(null(), mapped, MEM_RESERVE, PAGE_NOACCESS);
        if raw.is_null() {
            return Err("wallet_memory_protection_unavailable".into());
        }
        let ptr = raw.cast::<u8>().add(page);
        if VirtualAlloc(ptr.cast(), pages, MEM_COMMIT, PAGE_READWRITE).is_null()
            || VirtualLock(ptr.cast(), pages) == 0
        {
            VirtualFree(raw, 0, MEM_RELEASE);
            return Err("wallet_memory_protection_unavailable".into());
        }
        Ok(LockedBytes {
            allocation: NonNull::new_unchecked(raw),
            pointer: NonNull::new_unchecked(ptr),
            pages,
            mapped,
            len,
        })
    }
}

pub fn release(bytes: &mut LockedBytes) {
    // SAFETY: exclusively owned, committed pages; wipe before unlocking/freeing.
    unsafe {
        std::slice::from_raw_parts_mut(bytes.pointer.as_ptr(), bytes.pages).zeroize();
        VirtualUnlock(bytes.pointer.as_ptr().cast(), bytes.pages);
        VirtualFree(bytes.allocation.as_ptr(), 0, MEM_RELEASE);
    }
}

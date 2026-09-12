//! Locked, non-dumpable buffers for long-lived keys and decrypted vault records.
//! This does not claim to protect compiler temporaries, CPU registers or hibernation.
use crate::Result;
use std::ops::{Deref, DerefMut};
use zeroize::Zeroize;

pub struct LockedBytes {
    allocation: std::ptr::NonNull<std::ffi::c_void>,
    pointer: std::ptr::NonNull<u8>,
    mapped: usize,
    pages: usize,
    len: usize,
}
// SAFETY: the allocation is exclusively owned; moving it does not move its pages.
unsafe impl Send for LockedBytes {}
impl LockedBytes {
    pub fn new(len: usize) -> Result<Self> {
        if len > 1_000_000 {
            return Err("wallet_data_invalid".into());
        }
        #[cfg(unix)]
        {
            // SAFETY: sysconf has no pointer arguments.
            let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
            if page <= 0 {
                return Err("wallet_memory_protection_unavailable".into());
            }
            let page = page as usize;
            let pages = len.max(1).div_ceil(page) * page;
            let mapped = pages + 2 * page;
            // SAFETY: an anonymous private mapping has no external backing. Guard pages
            // remain inaccessible; only the interior becomes readable and writable.
            unsafe {
                let raw = libc::mmap(
                    std::ptr::null_mut(),
                    mapped,
                    libc::PROT_NONE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                );
                if raw == libc::MAP_FAILED {
                    return Err("wallet_memory_protection_unavailable".into());
                }
                let ptr = raw.cast::<u8>().add(page);
                if libc::mprotect(ptr.cast(), pages, libc::PROT_READ | libc::PROT_WRITE) != 0
                    || libc::mlock(ptr.cast(), pages) != 0
                {
                    libc::munmap(raw, mapped);
                    return Err("wallet_memory_protection_unavailable".into());
                }
                #[cfg(target_os = "linux")]
                if libc::madvise(ptr.cast(), pages, libc::MADV_DONTDUMP) != 0 {
                    libc::munlock(ptr.cast(), pages);
                    libc::munmap(raw, mapped);
                    return Err("wallet_memory_protection_unavailable".into());
                }
                Ok(Self {
                    allocation: std::ptr::NonNull::new_unchecked(raw),
                    pointer: std::ptr::NonNull::new_unchecked(ptr),
                    mapped,
                    pages,
                    len,
                })
            }
        }
        #[cfg(windows)]
        {
            windows::allocate(len)
        }
    }
}
impl Deref for LockedBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        // SAFETY: the initialized, mapped region outlives the returned borrow.
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.len) }
    }
}
impl DerefMut for LockedBytes {
    fn deref_mut(&mut self) -> &mut [u8] {
        // SAFETY: &mut self guarantees exclusive access to this owned allocation.
        unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.len) }
    }
}
impl AsRef<[u8]> for LockedBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}
impl AsMut<[u8]> for LockedBytes {
    fn as_mut(&mut self) -> &mut [u8] {
        self
    }
}
impl Drop for LockedBytes {
    fn drop(&mut self) {
        #[cfg(unix)]
        // SAFETY: these exact pages are owned by self and no borrow survives drop.
        unsafe {
            std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.pages).zeroize();
            libc::munlock(self.pointer.as_ptr().cast(), self.pages);
            libc::munmap(self.allocation.as_ptr(), self.mapped);
        }
        #[cfg(windows)]
        windows::release(self);
    }
}
pub struct LockedKey(LockedBytes);
impl LockedKey {
    pub fn zeroed() -> Result<Self> {
        Ok(Self(LockedBytes::new(32)?))
    }
    pub fn from_slice(value: &[u8]) -> Result<Self> {
        if value.len() != 32 {
            return Err("wallet_data_invalid".into());
        }
        let mut key = Self::zeroed()?;
        key.copy_from_slice(value);
        Ok(key)
    }
    pub fn random() -> Result<Self> {
        let mut key = Self::zeroed()?;
        getrandom::getrandom(key.as_mut()).map_err(|_| "wallet_random_unavailable")?;
        Ok(key)
    }
}
impl Deref for LockedKey {
    type Target = [u8; 32];
    fn deref(&self) -> &[u8; 32] {
        self.0.as_ref().try_into().expect("fixed key length")
    }
}
impl DerefMut for LockedKey {
    fn deref_mut(&mut self) -> &mut [u8; 32] {
        self.0.as_mut().try_into().expect("fixed key length")
    }
}
impl AsRef<[u8]> for LockedKey {
    fn as_ref(&self) -> &[u8] {
        &self[..]
    }
}
impl AsMut<[u8]> for LockedKey {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self[..]
    }
}

#[cfg(windows)]
#[path = "memory_windows.rs"]
mod windows;

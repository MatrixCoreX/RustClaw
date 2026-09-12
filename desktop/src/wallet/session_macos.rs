//! Read the current WindowServer session without subprocesses or credentials.
use std::{
    ffi::{c_char, c_void},
    ptr::null,
};
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGSessionCopyCurrentDictionary() -> *const c_void;
    static kCGSessionOnConsoleKey: *const c_void;
    static kCGSessionLoginDoneKey: *const c_void;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        text: *const c_char,
        encoding: u32,
    ) -> *const c_void;
    fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
    fn CFGetTypeID(value: *const c_void) -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: *const c_void) -> u8;
    fn CFRelease(value: *const c_void);
}
pub fn locked() -> bool {
    unsafe {
        let session = CGSessionCopyCurrentDictionary();
        if session.is_null() {
            return true;
        }
        let mut locked = false;
        // OnConsole is public. ScreenIsLocked is an additional OS observation;
        // focus loss and wall-clock suspension checks remain independent layers.
        let lock_key =
            CFStringCreateWithCString(null(), c"CGSSessionScreenIsLocked".as_ptr(), 0x08000100);
        if lock_key.is_null() {
            CFRelease(session);
            return true;
        }
        for (name, expected) in [
            (kCGSessionOnConsoleKey, true),
            (kCGSessionLoginDoneKey, true),
            (lock_key, false),
        ] {
            let value = CFDictionaryGetValue(session, name);
            if !value.is_null() && CFGetTypeID(value) == CFBooleanGetTypeID() {
                locked |= (CFBooleanGetValue(value) != 0) != expected;
            } else if expected {
                locked = true;
            }
        }
        CFRelease(lock_key);
        CFRelease(session);
        locked
    }
}

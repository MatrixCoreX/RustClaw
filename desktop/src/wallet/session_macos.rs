//! Read the current WindowServer session without subprocesses or credentials.
use std::{
    ffi::{c_char, c_void},
    ptr::null,
};
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGSessionCopyCurrentDictionary() -> *const c_void;
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
        // CGSession.h defines these as CFSTR macros, not exported symbols.
        for (key, expected) in [
            (c"kCGSSessionOnConsoleKey", true),
            (c"kCGSessionLoginDoneKey", true),
            (c"CGSSessionScreenIsLocked", false),
        ] {
            let name = CFStringCreateWithCString(null(), key.as_ptr(), 0x08000100);
            if name.is_null() {
                locked = true;
                break;
            }
            let value = CFDictionaryGetValue(session, name);
            if !value.is_null() && CFGetTypeID(value) == CFBooleanGetTypeID() {
                locked |= (CFBooleanGetValue(value) != 0) != expected;
            } else if expected {
                locked = true;
            }
            CFRelease(name);
        }
        CFRelease(session);
        locked
    }
}

pub fn locked() -> bool {
    use std::{mem::size_of, ptr::null_mut};
    use windows_sys::Win32::System::RemoteDesktop::*;
    // Windows 10+ uses the documented non-inverted lock flags. Missing/unknown
    // session information revokes the wallet instead of assuming an active user.
    unsafe {
        let mut buffer = null_mut();
        let mut bytes = 0;
        if WTSQuerySessionInformationW(
            null_mut(),
            WTS_CURRENT_SESSION,
            WTSSessionInfoEx,
            &mut buffer,
            &mut bytes,
        ) == 0
        {
            return true;
        }
        let locked = if bytes < size_of::<WTSINFOEXW>() as u32 {
            true
        } else {
            let info = &*buffer.cast::<WTSINFOEXW>();
            info.Level != 1
                || info.Data.WTSInfoExLevel1.SessionState != WTSActive
                || info.Data.WTSInfoExLevel1.SessionFlags as u32 != WTS_SESSIONSTATE_UNLOCK
        };
        WTSFreeMemory(buffer.cast());
        locked
    }
}

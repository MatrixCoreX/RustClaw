//! Handle-based Windows ACLs. Never operate on a second, unverified path.
use crate::Result;
use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    os::windows::io::{AsRawHandle, FromRawHandle},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    System::Threading::*,
};

fn storage_error(stage: &str, code: u32) -> String {
    #[cfg(test)]
    eprintln!("wallet_windows_storage stage={stage} code={code}");
    #[cfg(not(test))]
    let _ = (stage, code);
    "wallet_storage_unavailable".into()
}
pub struct Descriptor(pub PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
pub fn user_sid() -> Result<String> {
    token_sid(TokenUser)
}
fn token_sid(kind: TOKEN_INFORMATION_CLASS) -> Result<String> {
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err("wallet_process_protection_unavailable".into());
        }
        let mut size = 0;
        GetTokenInformation(token, kind, null_mut(), 0, &mut size);
        let mut storage = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        let ok = GetTokenInformation(token, kind, storage.as_mut_ptr().cast(), size, &mut size);
        CloseHandle(token);
        if ok == 0 {
            return Err("wallet_process_protection_unavailable".into());
        }
        let sid = if kind == TokenOwner {
            (*storage.as_ptr().cast::<TOKEN_OWNER>()).Owner
        } else {
            (*storage.as_ptr().cast::<TOKEN_USER>()).User.Sid
        };
        let mut text = null_mut();
        if ConvertSidToStringSidW(sid, &mut text) == 0 {
            return Err("wallet_process_protection_unavailable".into());
        }
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        let result = String::from_utf16(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        result.map_err(|_| "wallet_process_protection_unavailable".into())
    }
}
pub fn descriptor(sddl: &str) -> Result<Descriptor> {
    let text: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut out = null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            SDDL_REVISION_1,
            &mut out,
            null_mut(),
        )
    } == 0
    {
        return Err(storage_error("descriptor", unsafe { GetLastError() }));
    }
    Ok(Descriptor(out))
}
pub fn private_descriptor(directory: bool) -> Result<Descriptor> {
    let user = user_sid()?;
    let inherit = if directory { "OICI" } else { "" };
    descriptor(&format!(
        "O:{user}D:P(A;{inherit};FA;;;{user})(A;{inherit};FA;;;SY)"
    ))
}
pub fn apply(handle: HANDLE, kind: SE_OBJECT_TYPE, descriptor: &Descriptor) -> Result<()> {
    unsafe {
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = null_mut();
        if GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted) == 0
            || present == 0
            || acl.is_null()
        {
            return Err("wallet_storage_unavailable".into());
        }
        let status = SetSecurityInfo(
            handle,
            kind,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        );
        if status != ERROR_SUCCESS {
            return Err(storage_error("set_dacl", status));
        }
        Ok(())
    }
}
pub fn private_handle(file: &std::fs::File, directory: bool) -> Result<()> {
    unsafe {
        use windows_sys::Win32::Storage::FileSystem::*;
        // A tempfile may not have WRITE_DAC. Reopen the same object, never a path,
        // to obtain exactly the metadata/ACL rights required for hardening.
        // Directory handles are opened with READ_CONTROL | WRITE_DAC by files.rs.
        // ReOpenFile fails with ACCESS_DENIED for these reparse-safe directory
        // handles on Windows; retain the already verified original object.
        let reopened = if directory {
            None
        } else {
            let handle = ReOpenFile(
                file.as_raw_handle(),
                FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                0,
            );
            if handle == INVALID_HANDLE_VALUE {
                return Err(storage_error("reopen_file", GetLastError()));
            }
            Some(std::fs::File::from_raw_handle(handle))
        };
        let handle = reopened.as_ref().unwrap_or(file).as_raw_handle();
        let mut info: windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION =
            zeroed();
        if windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle(handle, &mut info)
            == 0
            || (!directory && info.nNumberOfLinks != 1)
        {
            return Err("wallet_storage_invalid".into());
        }
        let mut owner = null_mut();
        let mut raw: *mut c_void = null_mut();
        let status = GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut raw,
        );
        if status != ERROR_SUCCESS {
            return Err(storage_error("get_owner", status));
        }
        let _actual = Descriptor(raw);
        let expected = private_descriptor(directory)?;
        let mut expected_owner = null_mut();
        let mut defaulted = 0;
        let default_owner = descriptor(&format!("O:{}", token_sid(TokenOwner)?))?;
        let mut default_sid = null_mut();
        if GetSecurityDescriptorOwner(expected.0, &mut expected_owner, &mut defaulted) == 0
            || GetSecurityDescriptorOwner(default_owner.0, &mut default_sid, &mut defaulted) == 0
            || (EqualSid(owner, expected_owner) == 0 && EqualSid(owner, default_sid) == 0)
        {
            return Err("wallet_storage_invalid".into());
        }
        apply(handle, SE_FILE_OBJECT, &expected)
    }
}

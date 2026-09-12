//! Windows native process hardening. Job + mitigations are required; this is
//! intentionally not described as an AppContainer/network sandbox.
use crate::{wallet::windows_security, Result};
use std::{
    mem::{size_of, zeroed},
    ptr::null_mut,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::SE_KERNEL_OBJECT, *},
    System::{ErrorReporting::*, JobObjects::*, Threading::*},
};

pub fn enter() -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        let ok = unsafe {
            QueryInformationJobObject(
                null_mut(),
                JobObjectExtendedLimitInformation,
                (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                null_mut(),
            )
        };
        let required = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        if ok != 0
            && limits.BasicLimitInformation.ActiveProcessLimit == 1
            && limits.BasicLimitInformation.LimitFlags & required == required
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(protection_error("job_limits"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    remove_privileges().map_err(|_| protection_error("privileges"))?;
    for (policy, flags, stage) in [
        (ProcessChildProcessPolicy, 1u32, "child_policy"),
        (ProcessDynamicCodePolicy, 1, "dynamic_code_policy"),
        (ProcessExtensionPointDisablePolicy, 1, "extension_policy"),
        (ProcessImageLoadPolicy, 7, "image_policy"),
        (ProcessSystemCallDisablePolicy, 1, "win32k_policy"),
    ] {
        let mut observed = 0u32;
        unsafe {
            if SetProcessMitigationPolicy(policy, (&flags as *const u32).cast(), size_of::<u32>())
                == 0
                || GetProcessMitigationPolicy(
                    GetCurrentProcess(),
                    policy,
                    (&mut observed as *mut u32).cast(),
                    size_of::<u32>(),
                ) == 0
                || observed & flags != flags
            {
                return Err(protection_error(stage));
            }
        }
    }
    // Exclude heap from ordinary WER reports. This is not a guarantee against
    // administrator-created dumps or a compromised process owner changing ACLs.
    if unsafe { WerSetFlags(WER_FAULT_REPORTING_FLAG_NOHEAP) } < 0 {
        return Err(protection_error("wer_policy"));
    }
    let user = windows_security::user_sid().map_err(|_| protection_error("user_sid"))?;
    // New peer handles cannot read/write memory, inject threads or duplicate
    // handles. Keep parent termination, synchronization and limited status query.
    let acl = windows_security::descriptor(&format!(
        "D:P(D;;0x0000087A;;;WD)(A;;0x00101001;;;{user})(A;;GA;;;SY)"
    ))
    .map_err(|_| protection_error("process_descriptor"))?;
    windows_security::apply(unsafe { GetCurrentProcess() }, SE_KERNEL_OBJECT, &acl)
        .map_err(|_| protection_error("process_dacl"))
}

fn protection_error(stage: &str) -> String {
    // Static stage and OS status only; this runs before any wallet is opened.
    eprintln!("wallet_windows_protection stage={stage} code={}", unsafe {
        GetLastError()
    });
    "wallet_process_protection_unavailable".into()
}

fn remove_privileges() -> Result<()> {
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES,
            &mut token,
        ) == 0
        {
            return Err("wallet_process_protection_unavailable".into());
        }
        let result = (|| {
            let mut bytes = 0;
            GetTokenInformation(token, TokenPrivileges, null_mut(), 0, &mut bytes);
            let mut storage = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
            if GetTokenInformation(
                token,
                TokenPrivileges,
                storage.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            ) == 0
            {
                return Err("wallet_process_protection_unavailable".into());
            }
            let privileges = storage.as_mut_ptr().cast::<TOKEN_PRIVILEGES>();
            let count = (*privileges).PrivilegeCount as usize;
            let values =
                std::slice::from_raw_parts_mut((*privileges).Privileges.as_mut_ptr(), count);
            for value in values {
                value.Attributes = SE_PRIVILEGE_REMOVED;
            }
            if AdjustTokenPrivileges(token, 0, privileges, 0, null_mut(), null_mut()) == 0
                || GetLastError() == ERROR_NOT_ALL_ASSIGNED
            {
                return Err("wallet_process_protection_unavailable".into());
            }
            Ok(())
        })();
        CloseHandle(token);
        result
    }
}

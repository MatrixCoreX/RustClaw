use super::owner_scope::ScopeFailure;

// Process-only protection, not directory admission or a host-admin boundary.
// The real helper must enter before receiving private control/key material;
// scope admission only rechecks protection, never repairs a lost early guard.
pub(crate) struct OwnerProcessPrivacy {
    #[cfg(target_os = "linux")]
    pid: u32,
    _private: (),
}

impl OwnerProcessPrivacy {
    /// # Safety
    /// The caller is an independent control helper entered before private input,
    /// never the daemon or an ordinary test parent. This changes process-wide
    /// state and never restores it. Exit after its one operation or owned scope;
    /// spawning payloads additionally requires exclusive one-job scope admission.
    pub(crate) unsafe fn protect() -> Result<Self, ScopeFailure> {
        #[cfg(target_os = "linux")]
        {
            if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0 as libc::c_ulong, 0, 0, 0) } != 0 {
                return Err(ScopeFailure::Unsupported);
            }
        }
        Self::admit_current()
    }

    // Admission never repairs a missing/lost early protection. The helper
    // enters before private input; subsequent boundaries only validate it.
    pub(crate) fn admit_current() -> Result<Self, ScopeFailure> {
        let privacy = Self {
            #[cfg(target_os = "linux")]
            pid: std::process::id(),
            _private: (),
        };
        privacy.validate()?;
        Ok(privacy)
    }

    pub(crate) fn validate(&self) -> Result<(), ScopeFailure> {
        #[cfg(target_os = "linux")]
        {
            if self.pid != std::process::id() {
                return Err(ScopeFailure::OwnershipLost);
            }
            match unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) } {
                0 => Ok(()),
                -1 => Err(ScopeFailure::ProbeUnavailable),
                _ => Err(ScopeFailure::OwnershipLost),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(ScopeFailure::Unsupported)
        }
    }
}

#[cfg(test)]
#[path = "local_process_owner_privacy_tests.rs"]
pub(super) mod tests;

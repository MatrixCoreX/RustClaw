use serde::{Deserialize, Serialize};
use std::fs::{File, Metadata, Permissions};
use std::io;
use std::os::unix::fs::PermissionsExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub(crate) struct HostAccessMode(u32);

impl TryFrom<u32> for HostAccessMode {
    type Error = &'static str;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value & !0o777 != 0 {
            return Err("host_access_mode_invalid");
        }
        Ok(Self(value))
    }
}

impl From<HostAccessMode> for u32 {
    fn from(mode: HostAccessMode) -> Self {
        mode.0
    }
}

impl HostAccessMode {
    pub(crate) fn capture(metadata: &Metadata) -> io::Result<Self> {
        Self::try_from(metadata.permissions().mode() & 0o7777)
            .map_err(|code| io::Error::new(io::ErrorKind::Unsupported, code))
    }

    pub(super) fn apply(self, file: &File) -> io::Result<()> {
        file.set_permissions(Permissions::from_mode(self.0))
    }
}

#[cfg(test)]
#[path = "host_access_mode_tests.rs"]
mod tests;

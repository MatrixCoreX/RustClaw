//! Regular-file reads for host-owned metadata, not arbitrary ancestor confinement.
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, BufReader, Read};
use std::path::Path;

use serde::de::DeserializeOwned;

use crate::{SkillSdkError, SkillSdkResult};

#[derive(Clone, Copy)]
pub(crate) enum LeafLinks {
    Reject,
    Follow,
}

#[derive(Debug)]
enum OpenFailure {
    Io(io::Error),
    InvalidTarget,
    #[cfg(not(any(unix, windows)))]
    UnsupportedPlatform,
}

impl From<io::Error> for OpenFailure {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl OpenFailure {
    fn sdk(self, path: &Path) -> SkillSdkError {
        match self {
            Self::Io(error) => error.into(),
            Self::InvalidTarget => {
                SkillSdkError::new("metadata_read_target_invalid", path.display().to_string())
            }
            #[cfg(not(any(unix, windows)))]
            Self::UnsupportedPlatform => {
                SkillSdkError::new("metadata_read_platform_unsupported", std::env::consts::OS)
            }
        }
    }
}

fn regular(metadata: &Metadata, links: LeafLinks) -> bool {
    if !metadata.is_file() {
        return false;
    }
    #[cfg(windows)]
    if matches!(links, LeafLinks::Reject) {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0;
    }
    let _ = links;
    true
}

fn open_descriptor(path: &Path, links: LeafLinks) -> Result<File, OpenFailure> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let flags = libc::O_NONBLOCK
            | if matches!(links, LeafLinks::Reject) {
                libc::O_NOFOLLOW
            } else {
                0
            };
        options.custom_flags(flags);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Win32 CreateFileW: open the reparse point itself, never its target.
        // https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-createfilew
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        if matches!(links, LeafLinks::Reject) {
            options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        }
        // Rust's Windows documentation recommends explicit SQOS on arbitrary
        // paths; anonymous identification does not delegate caller credentials.
        options.security_qos_flags(0);
    }
    #[cfg(not(any(unix, windows)))]
    return Err(OpenFailure::UnsupportedPlatform);
    let file = options.open(path).map_err(|error| {
        #[cfg(unix)]
        if error.raw_os_error() == Some(libc::ELOOP) {
            return OpenFailure::InvalidTarget;
        }
        OpenFailure::Io(error)
    })?;
    if !regular(&file.metadata()?, links) {
        return Err(OpenFailure::InvalidTarget);
    }
    Ok(file)
}

fn checked_open(path: &Path, links: LeafLinks) -> Result<File, OpenFailure> {
    let metadata = match links {
        LeafLinks::Reject => fs::symlink_metadata(path)?,
        LeafLinks::Follow => fs::metadata(path)?,
    };
    if !regular(&metadata, links) {
        return Err(OpenFailure::InvalidTarget);
    }
    // The descriptor repeats the type check and Unix nonblocking/no-follow
    // flags close the check/open FIFO and leaf-link races.
    open_descriptor(path, links)
}

pub(crate) fn open(path: &Path) -> SkillSdkResult<File> {
    checked_open(path, LeafLinks::Reject).map_err(|failure| failure.sdk(path))
}

pub(crate) fn json<T: DeserializeOwned>(path: &Path) -> SkillSdkResult<T> {
    Ok(serde_json::from_reader(BufReader::new(open(path)?))?)
}

pub(crate) fn read(path: &Path) -> SkillSdkResult<Vec<u8>> {
    let mut bytes = Vec::new();
    open(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn read_optional(path: &Path) -> SkillSdkResult<Option<Vec<u8>>> {
    let mut file = match checked_open(path, LeafLinks::Reject) {
        Ok(file) => file,
        Err(OpenFailure::Io(error)) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(failure) => return Err(failure.sdk(path)),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

pub(crate) fn manifest_text(path: &Path, links: LeafLinks) -> SkillSdkResult<String> {
    let result = checked_open(path, links).and_then(|mut file| {
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        Ok(text)
    });
    result.map_err(|failure| match (links, failure) {
        (_, OpenFailure::Io(error)) => SkillSdkError::new(
            "manifest_read_failed",
            format!("path={} error={error}", path.display()),
        )
        .phase("manifest"),
        (LeafLinks::Follow, failure) => {
            SkillSdkError::new("manifest_read_failed", failure.sdk(path).to_string())
                .phase("manifest")
        }
        (_, failure) => failure.sdk(path).phase("manifest"),
    })
}

#[cfg(test)]
#[path = "metadata_file_tests.rs"]
mod tests;

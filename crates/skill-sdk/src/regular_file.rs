//! Regular descriptors with caller-selected leaf-link policy, not ancestor confinement.
use std::fs::{self, File, Metadata, OpenOptions};
use std::io;
use std::path::Path;

#[derive(Clone, Copy)]
pub(crate) enum LeafLinks {
    Reject,
    Follow,
}

#[derive(Debug)]
pub(crate) enum OpenFailure {
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

pub(crate) fn open_descriptor(path: &Path, links: LeafLinks) -> Result<File, OpenFailure> {
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
        // Open the reparse point itself, never its target for strict reads.
        // https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-createfilew
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        if matches!(links, LeafLinks::Reject) {
            options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        }
        // Explicit anonymous SQOS does not delegate caller credentials.
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

pub(crate) fn checked_open(path: &Path, links: LeafLinks) -> Result<File, OpenFailure> {
    let metadata = match links {
        LeafLinks::Reject => fs::symlink_metadata(path)?,
        LeafLinks::Follow => fs::metadata(path)?,
    };
    if !regular(&metadata, links) {
        return Err(OpenFailure::InvalidTarget);
    }
    // Type-check the actual descriptor again; Unix flags prevent FIFO/link races.
    open_descriptor(path, links)
}

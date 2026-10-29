use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

pub(super) fn publish(workspace_root: &Path, path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "evidence_parent_missing"))?;
    crate::secure_workspace_fs::ensure_private_workspace_directory(workspace_root, parent)?;
    match verify(workspace_root, path, bytes) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    crate::secure_workspace_fs::atomic_write_workspace_file(workspace_root, path, bytes)?;
    verify(workspace_root, path, bytes)
}

fn verify(workspace_root: &Path, path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = crate::secure_workspace_fs::open_workspace_file(workspace_root, path)?;
    let metadata = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: geteuid takes no arguments and does not access caller memory.
        let owner = unsafe { libc::geteuid() };
        if metadata.uid() != owner
            || metadata.nlink() != 1
            || metadata.permissions().mode() & 0o7777 != 0o600
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "evidence_file_not_private",
            ));
        }
    }
    if metadata.len() != bytes.len() as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "evidence_digest_mismatch",
        ));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut limited = (&mut file).take(bytes.len() as u64 + 1);
    loop {
        let count = limited.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if digest.finalize() != Sha256::digest(bytes) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "evidence_digest_mismatch",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "evidence_storage_tests.rs"]
mod tests;

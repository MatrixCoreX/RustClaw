//! Complete streamed artifact hashes, with no new byte cap or read deadline.
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::regular_file::{checked_open, LeafLinks, OpenFailure};
use crate::{SkillSdkError, SkillSdkResult};

fn failure(path: &Path, error: OpenFailure) -> SkillSdkError {
    match error {
        OpenFailure::Io(error) => SkillSdkError::new(
            "artifact_read_failed",
            format!("path={} error={error}", path.display()),
        ),
        OpenFailure::InvalidTarget => {
            SkillSdkError::new("artifact_read_target_invalid", path.display().to_string())
        }
        #[cfg(not(any(unix, windows)))]
        OpenFailure::UnsupportedPlatform => {
            SkillSdkError::new("artifact_read_platform_unsupported", std::env::consts::OS)
        }
    }
}

pub(crate) fn digest(path: &Path, links: LeafLinks) -> SkillSdkResult<String> {
    let mut file = checked_open(path, links).map_err(|error| failure(path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| failure(path, OpenFailure::Io(error)))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

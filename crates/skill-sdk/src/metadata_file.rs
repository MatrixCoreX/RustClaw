//! Regular-file reads for host-owned metadata, not arbitrary ancestor confinement.
#[cfg(test)]
use std::fs;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

use serde::de::DeserializeOwned;

#[cfg(test)]
use crate::regular_file::open_descriptor;
pub(crate) use crate::regular_file::LeafLinks;
use crate::regular_file::{checked_open, OpenFailure};
use crate::{SkillSdkError, SkillSdkResult};

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

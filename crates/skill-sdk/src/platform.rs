use serde::{Deserialize, Serialize};

use crate::{SkillSdkError, SkillSdkResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostPlatform {
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

impl HostPlatform {
    pub fn current() -> Self {
        Self {
            os: normalize_os(std::env::consts::OS)
                .unwrap_or(std::env::consts::OS)
                .to_string(),
            arch: normalize_arch(std::env::consts::ARCH)
                .unwrap_or(std::env::consts::ARCH)
                .to_string(),
            target: Some(env!("RUSTCLAW_BUILD_TARGET").to_string()),
        }
    }

    pub fn from_target(target: &str) -> SkillSdkResult<Self> {
        let raw = target.trim().to_ascii_lowercase();
        let os = if raw.contains("apple-darwin") {
            "macos"
        } else if raw.contains("linux") {
            "linux"
        } else if raw.contains("windows") {
            "windows"
        } else {
            return Err(SkillSdkError::new(
                "platform_target_unsupported",
                format!("target={target}"),
            ));
        };
        let arch = raw
            .split('-')
            .next()
            .and_then(normalize_arch)
            .ok_or_else(|| {
                SkillSdkError::new("platform_arch_unsupported", format!("target={target}"))
            })?;
        Ok(Self {
            os: os.to_string(),
            arch: arch.to_string(),
            target: Some(raw),
        })
    }

    pub fn matches(&self, supported_os: &[String], supported_arch: &[String]) -> bool {
        token_list_matches(supported_os, &self.os, normalize_os)
            && token_list_matches(supported_arch, &self.arch, normalize_arch)
    }
}

fn token_list_matches(
    values: &[String],
    current: &str,
    normalize: fn(&str) -> Option<&'static str>,
) -> bool {
    values.is_empty()
        || values.iter().any(|value| {
            let token = value.trim().to_ascii_lowercase();
            token == "any" || normalize(&token).is_some_and(|value| value == current)
        })
}

pub fn normalize_os(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "linux" | "gnu/linux" => Some("linux"),
        "macos" | "darwin" | "osx" => Some("macos"),
        "windows" | "win32" => Some("windows"),
        _ => None,
    }
}

pub fn normalize_arch(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" => Some("x86_64"),
        "aarch64" | "arm64" => Some("aarch64"),
        "arm" | "armv7" | "armv7l" => Some("armv7"),
        _ => None,
    }
}

#[cfg(test)]
#[path = "platform_tests.rs"]
mod tests;

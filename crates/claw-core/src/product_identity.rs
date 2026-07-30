//! Brand-neutral product identity and protocol naming.
//!
//! Product-facing code must read these values instead of embedding a product
//! name. Runtime and protocol code use only neutral canonical identifiers.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::{env, ffi::OsString};

pub const AUTH_KEY_HEADER: &str = "x-agent-key";
pub const CLIENT_ORIGIN_HEADER: &str = "x-agent-client";
pub const INTERNAL_SKILL_TOKEN_HEADER: &str = "x-agent-internal-skill-token";
pub const INTERNAL_LLM_TOKEN_HEADER: &str = "x-agent-internal-llm-token";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductIdentity {
    display_name: String,
    release_artifact_id: String,
    terminal_banner: String,
    release_repository: String,
    small_screen_splash_image: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ProductIdentityConfig {
    schema_version: u32,
    display_name: String,
    release_artifact_id: String,
    terminal_banner: String,
    release_repository: String,
    small_screen_splash_image: String,
}

impl ProductIdentity {
    fn load() -> Self {
        load_identity_config().unwrap_or_else(|error| panic!("{error}"))
    }

    fn from_config(config: &ProductIdentityConfig) -> Result<Self, String> {
        if config.schema_version != 1 {
            return Err(format!(
                "unsupported product identity schema version: {}",
                config.schema_version
            ));
        }
        let display_name = required_value("display_name", &config.display_name)?;
        let release_artifact_id =
            required_slug("release_artifact_id", &config.release_artifact_id)?;
        let terminal_banner = required_value("terminal_banner", &config.terminal_banner)?;
        let release_repository = required_repository(&config.release_repository)?;
        let small_screen_splash_image = required_filename(
            "small_screen_splash_image",
            &config.small_screen_splash_image,
        )?;
        Ok(Self {
            display_name,
            release_artifact_id,
            terminal_banner,
            release_repository,
            small_screen_splash_image,
        })
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn release_artifact_id(&self) -> &str {
        &self.release_artifact_id
    }

    pub fn terminal_banner(&self) -> &str {
        &self.terminal_banner
    }

    pub fn release_repository(&self) -> &str {
        &self.release_repository
    }

    pub fn small_screen_splash_image(&self) -> &str {
        &self.small_screen_splash_image
    }
}

pub fn product_identity() -> &'static ProductIdentity {
    static IDENTITY: OnceLock<ProductIdentity> = OnceLock::new();
    IDENTITY.get_or_init(ProductIdentity::load)
}

pub fn env_string(suffix: &str) -> Result<String, env::VarError> {
    env::var(format!("APP_{suffix}"))
}

pub fn env_os(suffix: &str) -> Option<OsString> {
    env::var_os(format!("APP_{suffix}"))
}

fn load_identity_config() -> Result<ProductIdentity, String> {
    let candidates = identity_config_candidates();
    for path in &candidates {
        if !path.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(path).map_err(|error| {
            format!(
                "failed to read product identity config {}: {error}",
                path.display()
            )
        })?;
        let config = toml::from_str::<ProductIdentityConfig>(&raw).map_err(|error| {
            format!(
                "invalid product identity config {}: {error}",
                path.display()
            )
        })?;
        return ProductIdentity::from_config(&config).map_err(|error| {
            format!(
                "invalid product identity config {}: {error}",
                path.display()
            )
        });
    }
    Err(format!(
        "product identity config not found; checked: {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn identity_config_candidates() -> Vec<PathBuf> {
    if let Some(path) = env::var_os("APP_PRODUCT_IDENTITY_CONFIG") {
        return vec![PathBuf::from(path)];
    }
    let mut candidates = Vec::new();
    if let Ok(current) = std::env::current_dir() {
        candidates.push(current.join("configs/product_identity.toml"));
    }
    if let Ok(executable) = std::env::current_exe() {
        let mut current = executable.parent().map(Path::to_path_buf);
        for _ in 0..8 {
            let Some(directory) = current else { break };
            candidates.push(directory.join("configs/product_identity.toml"));
            current = directory.parent().map(Path::to_path_buf);
        }
    }
    candidates.dedup();
    candidates
}

fn required_value(field: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    Ok(value.to_string())
}

fn required_slug(field: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if !valid_slug(value) {
        return Err(format!(
            "{field} must be 1-64 lowercase ASCII letters, digits, or hyphens and cannot start or end with a hyphen"
        ));
    }
    Ok(value.to_string())
}

fn required_repository(value: &str) -> Result<String, String> {
    let value = required_value("release_repository", value)?;
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    if owner.is_empty()
        || repository.is_empty()
        || parts.next().is_some()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err("release_repository must use owner/repository syntax".to_string());
    }
    Ok(value)
}

fn required_filename(field: &str, value: &str) -> Result<String, String> {
    let value = required_value(field, value)?;
    if value == "."
        || value == ".."
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
    {
        return Err(format!("{field} must be a safe file name"));
    }
    Ok(value)
}

fn valid_slug(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate.len() <= 64
        && candidate
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !candidate.starts_with('-')
        && !candidate.ends_with('-')
}

#[cfg(test)]
#[path = "product_identity_tests.rs"]
mod tests;

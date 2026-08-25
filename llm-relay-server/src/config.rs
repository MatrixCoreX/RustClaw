use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{anyhow, bail, Context};
use serde::Serialize;

use crate::quota::QuotaLimits;

#[derive(Clone, Debug)]
pub struct StoreConfig {
    pub database_path: PathBuf,
    pub key_pepper: String,
}

#[derive(Clone, Debug)]
pub struct RelayConfig {
    pub listen_addr: SocketAddr,
    pub store: StoreConfig,
    pub default_model: String,
    pub provider: ModelProvider,
    pub upstream_timeout: Duration,
    pub max_request_body_bytes: usize,
    pub max_messages: usize,
    pub max_tools: usize,
    pub max_inflight: usize,
    pub max_inflight_per_key: u32,
    pub limits: QuotaLimits,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelProvider {
    pub alias: String,
    pub base_url: String,
    #[serde(skip_serializing)]
    pub api_key: String,
    pub model: String,
    pub vendor: String,
}

impl StoreConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let database_path = env::var("RELAY_DATABASE_PATH")
            .unwrap_or_else(|_| "data/llm-relay/relay.db".to_owned())
            .into();
        let key_pepper = required_env("RELAY_KEY_PEPPER")?;
        if key_pepper.len() < 32 {
            bail!("RELAY_KEY_PEPPER must contain at least 32 bytes");
        }
        Ok(Self {
            database_path,
            key_pepper,
        })
    }
}

impl RelayConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let listen_addr: SocketAddr = env_or("RELAY_LISTEN_ADDR", "127.0.0.1:8796")
            .parse()
            .context("RELAY_LISTEN_ADDR must be a socket address, for example 127.0.0.1:8796")?;
        if !listen_addr.ip().is_loopback() && !env_bool("RELAY_ALLOW_PUBLIC_BIND", false)? {
            bail!("public relay binding requires RELAY_ALLOW_PUBLIC_BIND=true");
        }

        let alias = env_or("RELAY_PUBLIC_MODEL", "minimax");
        validate_alias(&alias)?;
        let base_url = env_or("RELAY_UPSTREAM_BASE_URL", "https://api.minimaxi.com/v1")
            .trim_end_matches('/')
            .to_owned();
        if !base_url.starts_with("https://") && !env_bool("RELAY_ALLOW_INSECURE_UPSTREAM", false)? {
            bail!("RELAY_UPSTREAM_BASE_URL must use https");
        }

        let provider = ModelProvider {
            alias: alias.clone(),
            base_url,
            api_key: required_env("RELAY_UPSTREAM_API_KEY")?,
            model: env_or("RELAY_UPSTREAM_MODEL", "MiniMax-M3"),
            vendor: env_or("RELAY_UPSTREAM_VENDOR", "minimax"),
        };
        let max_request_body_bytes = env_usize("RELAY_MAX_REQUEST_BODY_BYTES", 2 * 1024 * 1024)?;
        if !(1024..=16 * 1024 * 1024).contains(&max_request_body_bytes) {
            bail!("RELAY_MAX_REQUEST_BODY_BYTES must be between 1024 and 16777216");
        }
        let max_inflight = env_usize("RELAY_MAX_INFLIGHT", 16)?;
        let max_inflight_per_key = env_u32("RELAY_MAX_INFLIGHT_PER_KEY", 4)?;
        if max_inflight == 0 || max_inflight_per_key == 0 {
            bail!("relay inflight limits must be positive");
        }

        Ok(Self {
            listen_addr,
            store: StoreConfig::from_env()?,
            default_model: alias,
            provider,
            upstream_timeout: Duration::from_secs(env_u64("RELAY_UPSTREAM_TIMEOUT_SECONDS", 180)?),
            max_request_body_bytes,
            max_messages: env_usize("RELAY_MAX_MESSAGES", 256)?,
            max_tools: env_usize("RELAY_MAX_TOOLS", 128)?,
            max_inflight,
            max_inflight_per_key,
            limits: QuotaLimits::from_env()?,
        })
    }

    pub fn select_provider(&self, requested_model: Option<&str>) -> Option<&ModelProvider> {
        let requested = requested_model.unwrap_or("default");
        (requested == "default" || requested == self.provider.alias).then_some(&self.provider)
    }
}

impl ModelProvider {
    pub fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

fn validate_alias(alias: &str) -> anyhow::Result<()> {
    if alias.is_empty()
        || alias.len() > 64
        || !alias
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        bail!("RELAY_PUBLIC_MODEL contains invalid characters");
    }
    Ok(())
}

fn required_env(name: &str) -> anyhow::Result<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("{name} is required"))
}

fn env_or(name: &str, default_value: &str) -> String {
    env::var(name).unwrap_or_else(|_| default_value.to_owned())
}

pub fn env_u64(name: &str, default_value: u64) -> anyhow::Result<u64> {
    match env::var(name) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("{name} must be a non-negative integer")),
        Err(_) => Ok(default_value),
    }
}

pub fn env_u32(name: &str, default_value: u32) -> anyhow::Result<u32> {
    let value = env_u64(name, u64::from(default_value))?;
    u32::try_from(value).map_err(|_| anyhow!("{name} is too large"))
}

fn env_usize(name: &str, default_value: usize) -> anyhow::Result<usize> {
    let value = env_u64(name, default_value as u64)?;
    usize::try_from(value).map_err(|_| anyhow!("{name} is too large"))
}

fn env_bool(name: &str, default_value: bool) -> anyhow::Result<bool> {
    match env::var(name) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => bail!("{name} must be a boolean"),
        },
        Err(_) => Ok(default_value),
    }
}

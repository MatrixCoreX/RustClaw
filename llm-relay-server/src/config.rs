use std::{env, net::SocketAddr, time::Duration};

use anyhow::{anyhow, Context};
use serde::Serialize;

use crate::quota::QuotaLimits;

#[derive(Clone, Debug)]
pub struct RelayConfig {
    pub listen_addr: SocketAddr,
    pub api_keys: Vec<String>,
    pub default_model: String,
    pub providers: Vec<ModelProvider>,
    pub upstream_timeout: Duration,
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

impl RelayConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let listen_addr = env_or("RELAY_LISTEN_ADDR", "127.0.0.1:8788")
            .parse()
            .context("RELAY_LISTEN_ADDR must be a socket address, for example 127.0.0.1:8788")?;

        let api_keys = env_or("RELAY_API_KEYS", "dev-local-key")
            .split(',')
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(ToOwned::to_owned)
            .collect();

        let default_model = env_or("RELAY_PUBLIC_MODEL", "default");
        let upstream_timeout = Duration::from_secs(env_u64("RELAY_UPSTREAM_TIMEOUT_SECONDS", 60)?);
        let providers = load_providers(&default_model);

        Ok(Self {
            listen_addr,
            api_keys,
            default_model,
            providers,
            upstream_timeout,
            limits: QuotaLimits::from_env()?,
        })
    }

    pub fn select_provider(&self, requested_model: Option<&str>) -> Option<&ModelProvider> {
        let requested_model = requested_model.unwrap_or(&self.default_model);
        self.providers.iter().find(|provider| {
            provider.alias == requested_model
                || provider.model == requested_model
                || (requested_model == "default" && provider.alias == self.default_model)
        })
    }
}

impl Serialize for RelayConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct PublicConfig<'a> {
            listen_addr: String,
            api_keys_count: usize,
            default_model: &'a str,
            providers: &'a [ModelProvider],
            upstream_timeout_seconds: u64,
            limits: &'a QuotaLimits,
        }

        let public = PublicConfig {
            listen_addr: self.listen_addr.to_string(),
            api_keys_count: self.api_keys.len(),
            default_model: &self.default_model,
            providers: &self.providers,
            upstream_timeout_seconds: self.upstream_timeout.as_secs(),
            limits: &self.limits,
        };
        public.serialize(serializer)
    }
}

impl ModelProvider {
    pub fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

fn load_providers(default_model: &str) -> Vec<ModelProvider> {
    let mut providers = Vec::new();

    providers.push(ModelProvider {
        alias: default_model.to_owned(),
        base_url: env_or("RELAY_UPSTREAM_BASE_URL", "https://api.openai.com/v1")
            .trim_end_matches('/')
            .to_owned(),
        api_key: env::var("RELAY_UPSTREAM_API_KEY").unwrap_or_default(),
        model: env_or("RELAY_UPSTREAM_MODEL", "gpt-4o-mini"),
        vendor: env_or("RELAY_UPSTREAM_VENDOR", "openai"),
    });

    push_optional_provider(
        &mut providers,
        "minimax",
        "https://api.minimaxi.com/v1",
        "MiniMax-M3",
        "RELAY_MINIMAX",
    );
    push_optional_provider(
        &mut providers,
        "deepseek",
        "https://api.deepseek.com/v1",
        "deepseek-chat",
        "RELAY_DEEPSEEK",
    );
    push_optional_provider(
        &mut providers,
        "mimo",
        "https://token-plan-sgp.xiaomimimo.com/v1",
        "mimo-v2.5-pro",
        "RELAY_MIMO",
    );

    providers
}

fn push_optional_provider(
    providers: &mut Vec<ModelProvider>,
    alias: &str,
    default_base_url: &str,
    default_model: &str,
    env_prefix: &str,
) {
    let api_key_name = format!("{env_prefix}_API_KEY");
    let api_key = env::var(&api_key_name).unwrap_or_default();
    if api_key.is_empty() {
        return;
    }

    providers.push(ModelProvider {
        alias: env_or(&format!("{env_prefix}_ALIAS"), alias),
        base_url: env_or(&format!("{env_prefix}_BASE_URL"), default_base_url)
            .trim_end_matches('/')
            .to_owned(),
        api_key,
        model: env_or(&format!("{env_prefix}_MODEL"), default_model),
        vendor: alias.to_owned(),
    });
}
fn env_or(name: &str, default_value: &str) -> String {
    env::var(name).unwrap_or_else(|_| default_value.to_owned())
}

pub fn env_u64(name: &str, default_value: u64) -> anyhow::Result<u64> {
    match env::var(name) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("{name} must be a positive integer")),
        Err(_) => Ok(default_value),
    }
}

pub fn env_u32(name: &str, default_value: u32) -> anyhow::Result<u32> {
    let value = env_u64(name, u64::from(default_value))?;
    u32::try_from(value).map_err(|_| anyhow!("{name} is too large"))
}

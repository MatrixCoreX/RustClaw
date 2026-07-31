use super::*;
use std::net::SocketAddr;

pub(super) enum TelegramUpdateTransport {
    Polling,
    Webhook(TelegramWebhookRuntime),
}

pub(super) struct TelegramWebhookRuntime {
    pub(super) listen: SocketAddr,
    pub(super) public_url: reqwest::Url,
    pub(super) secret_token: String,
}

pub(super) fn resolve_telegram_update_transport<F>(
    update_mode: &str,
    webhook_listen: &str,
    webhook_public_url: &str,
    webhook_secret_env: &str,
    env_value: &F,
) -> anyhow::Result<TelegramUpdateTransport>
where
    F: Fn(&str) -> Option<String>,
{
    match update_mode.trim().to_ascii_lowercase().as_str() {
        "polling" => Ok(TelegramUpdateTransport::Polling),
        "webhook" => resolve_telegram_webhook_runtime(
            webhook_listen,
            webhook_public_url,
            webhook_secret_env,
            env_value,
        )
        .map(TelegramUpdateTransport::Webhook),
        _ => Err(anyhow!("telegram_update_mode_invalid")),
    }
}

fn resolve_telegram_webhook_runtime<F>(
    webhook_listen: &str,
    webhook_public_url: &str,
    webhook_secret_env: &str,
    env_value: &F,
) -> anyhow::Result<TelegramWebhookRuntime>
where
    F: Fn(&str) -> Option<String>,
{
    let listen = webhook_listen
        .trim()
        .parse::<SocketAddr>()
        .map_err(|_| anyhow!("telegram_webhook_listen_invalid"))?;
    if !listen.ip().is_loopback() {
        return Err(anyhow!("telegram_webhook_listen_must_be_loopback"));
    }

    let public_url = webhook_public_url
        .trim()
        .parse::<reqwest::Url>()
        .map_err(|_| anyhow!("telegram_webhook_public_url_invalid"))?;
    if public_url.scheme() != "https"
        || public_url.host_str().is_none()
        || !public_url.username().is_empty()
        || public_url.password().is_some()
        || public_url.fragment().is_some()
        || !matches!(public_url.port_or_known_default(), Some(443 | 8443))
    {
        return Err(anyhow!("telegram_webhook_public_url_invalid"));
    }

    let secret_env = webhook_secret_env.trim();
    if secret_env.is_empty()
        || secret_env.len() > 128
        || !secret_env.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_uppercase() || (index > 0 && (byte.is_ascii_digit() || byte == b'_'))
        })
    {
        return Err(anyhow!("telegram_webhook_secret_env_invalid"));
    }
    let secret_token = env_value(secret_env)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("telegram_webhook_secret_environment_missing"))?;
    if secret_token.len() > 256
        || !secret_token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(anyhow!("telegram_webhook_secret_invalid"));
    }

    Ok(TelegramWebhookRuntime {
        listen,
        public_url,
        secret_token,
    })
}

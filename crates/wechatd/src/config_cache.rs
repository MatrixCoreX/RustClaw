//! Per-user `getconfig` cache with TTL + backoff (aligned with OpenClaw weixin `WeixinConfigManager`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use reqwest::Client;
use tracing::debug;

use crate::config_section::WechatSection;
use crate::ilink;

const CONFIG_CACHE_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const CONFIG_CACHE_INITIAL_RETRY_MS: u64 = 2_000;
const CONFIG_CACHE_MAX_RETRY_MS: u64 = 60 * 60 * 1000;

#[derive(Clone, Default)]
pub struct CachedWeixinConfig {
    pub typing_ticket: String,
}

struct CacheEntry {
    config: CachedWeixinConfig,
    next_fetch_at: Instant,
    retry_delay_ms: u64,
}

fn stable_mix_user_id(user_id: &str) -> u64 {
    user_id
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64))
}

/// Deterministic jitter in `[0, CONFIG_CACHE_TTL_MS)` so refreshes spread out.
fn cache_ttl_jitter_ms(user_id: &str) -> u64 {
    stable_mix_user_id(user_id) % CONFIG_CACHE_TTL_MS
}

pub struct WeixinConfigManager {
    cache: HashMap<String, CacheEntry>,
}

impl WeixinConfigManager {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Resolve `typing_ticket` for `ilink_user_id`, using cache when valid.
    pub async fn typing_ticket_for_user(
        &mut self,
        client: &Client,
        section: &WechatSection,
        base_url: &str,
        token: &str,
        user_id: &str,
        context_token: Option<&str>,
    ) -> String {
        let now = Instant::now();
        let should_fetch = match self.cache.get(user_id) {
            None => true,
            Some(e) => now >= e.next_fetch_at,
        };

        if should_fetch {
            let mut fetch_ok = false;
            match ilink::get_config(client, section, base_url, token, user_id, context_token).await
            {
                Ok(Some(ticket)) => {
                    // ret == 0 from upstream; ticket may be empty (no typing support).
                    self.cache.insert(
                        user_id.to_string(),
                        CacheEntry {
                            config: CachedWeixinConfig {
                                typing_ticket: ticket,
                            },
                            next_fetch_at: now
                                + Duration::from_millis(cache_ttl_jitter_ms(user_id)),
                            retry_delay_ms: CONFIG_CACHE_INITIAL_RETRY_MS,
                        },
                    );
                    debug!(
                        target: "wechatd",
                        "wechatd: getconfig ok for user_id_len={}",
                        user_id.len()
                    );
                    fetch_ok = true;
                }
                Ok(None) => {
                    debug!(
                        target: "wechatd",
                        "wechatd: getconfig ret!=0 for user_id_len={}",
                        user_id.len()
                    );
                }
                Err(err) => {
                    debug!(
                        target: "wechatd",
                        "wechatd: getconfig failed (ignored): {}",
                        err
                    );
                }
            }

            if !fetch_ok {
                let entry = self.cache.get_mut(user_id);
                let now = Instant::now();
                match entry {
                    Some(e) => {
                        let next_delay =
                            (e.retry_delay_ms * 2).min(CONFIG_CACHE_MAX_RETRY_MS);
                        e.retry_delay_ms = next_delay;
                        e.next_fetch_at = now + Duration::from_millis(next_delay);
                    }
                    None => {
                        self.cache.insert(
                            user_id.to_string(),
                            CacheEntry {
                                config: CachedWeixinConfig::default(),
                                next_fetch_at: now
                                    + Duration::from_millis(CONFIG_CACHE_INITIAL_RETRY_MS),
                                retry_delay_ms: CONFIG_CACHE_INITIAL_RETRY_MS,
                            },
                        );
                    }
                }
            }
        }

        self.cache
            .get(user_id)
            .map(|e| e.config.typing_ticket.clone())
            .unwrap_or_default()
    }
}

impl Default for WeixinConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

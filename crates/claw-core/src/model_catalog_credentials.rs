//! Credential presence only; never retain or serialize credential values.
use std::collections::BTreeMap;
use std::path::Path;

use super::providers::TextProvider;
use super::{matches_hosted_relay, string_field};
use crate::config::LlmCredentialSource;
use crate::llm_provider_descriptors::credential_vendor_name;
use crate::secrets::EnvFileSecretsBroker;

pub(super) struct CredentialSnapshot {
    broker: EnvFileSecretsBroker,
    states: BTreeMap<(String, bool), String>,
}

impl CredentialSnapshot {
    pub(super) fn new(workspace: &Path) -> Self {
        Self {
            broker: EnvFileSecretsBroker::new(crate::git_remote_config::git_credential_store_path(
                workspace,
            )),
            states: BTreeMap::new(),
        }
    }

    pub(super) fn state(
        &mut self,
        table: &toml::map::Map<String, toml::Value>,
        provider: &str,
        model: &str,
        config: &toml::Value,
        supports_text: bool,
    ) -> String {
        let base = string_field(table, "base_url");
        if is_loopback(&base) {
            return "not_required_local".into();
        }
        if matches_hosted_relay(config, provider, model, &base) {
            return "device_enrollment".into();
        }
        // Independent media keys are valid. Ordinary text TOML keys are not.
        if !supports_text && !string_field(table, "api_key").is_empty() {
            return "configured_inline".into();
        }
        self.lookup_state(provider, true)
    }

    fn lookup_state(&mut self, vendor: &str, vendor_environment_fallback: bool) -> String {
        let key = (vendor.to_string(), vendor_environment_fallback);
        if let Some(state) = self.states.get(&key) {
            return state.clone();
        }
        let state = if vendor_environment_fallback {
            // Canonical presence is one request snapshot even when text and
            // media use different fallback contracts for this same namespace.
            let canonical = self.lookup_state(vendor, false);
            if canonical == "missing" {
                let value = crate::config::llm_vendor_api_key_from_env_with(vendor, &|name| {
                    std::env::var(name).ok()
                });
                if value.is_empty() {
                    "missing"
                } else {
                    "configured_env"
                }
                .to_string()
            } else {
                canonical
            }
        } else {
            match self
                .broker
                .lookup_with_source(&crate::secrets::text_secret_name_for_vendor(vendor))
            {
                Ok(Some((secret, _))) if !secret.is_empty() => "configured_env",
                Ok(_) => "missing",
                Err(_) => "unavailable",
            }
            .to_string()
        };
        self.states.insert(key, state.clone());
        state
    }

    pub(super) fn text_state(&mut self, provider: &TextProvider) -> String {
        let config = &provider.config;
        if config.params.device_key_enrollment {
            return "device_enrollment".into();
        }
        if is_loopback(&config.base_url) {
            return "not_required_local".into();
        }
        let inline = || {
            if config.api_key.trim().is_empty() {
                "missing"
            } else {
                "configured_inline"
            }
            .to_string()
        };
        if provider.generated {
            let Some(vendor) = credential_vendor_name(&config.name) else {
                return "unavailable".into();
            };
            return self.lookup_state(vendor, true);
        }
        if config.params.credential_source == LlmCredentialSource::Resolved {
            return inline();
        }
        let Some(vendor) = credential_vendor_name(&config.name) else {
            return inline();
        };
        match self.lookup_state(vendor, false).as_str() {
            "missing" => inline(),
            state => state.to_string(),
        }
    }
}

pub(super) fn is_loopback(base: &str) -> bool {
    let Ok(url) = url::Url::parse(base) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && match url.host() {
            Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
            Some(url::Host::Ipv4(host)) => host.is_loopback(),
            Some(url::Host::Ipv6(host)) => host.is_loopback(),
            None => false,
        }
}

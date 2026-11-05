//! Credential presence only; never retain or serialize credential values.
use std::collections::BTreeMap;
use std::path::Path;

use super::{matches_hosted_relay, string_field};
use crate::secrets::EnvFileSecretsBroker;

pub(super) struct CredentialSnapshot {
    broker: EnvFileSecretsBroker,
    states: BTreeMap<String, String>,
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
        if let Some(state) = self.states.get(provider) {
            return state.clone();
        }
        let state = match self.broker.lookup_text_vendor_with_source(provider) {
            Ok(Some(_)) => "configured_env",
            Ok(None) => "missing",
            Err(_) => "unavailable",
        }
        .to_string();
        self.states.insert(provider.into(), state.clone());
        state
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

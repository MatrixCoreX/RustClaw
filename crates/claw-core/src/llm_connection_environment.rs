//! Identity of scoped connection credentials exported by the host runner.
//! Protocol aliases describe one selected connection, not additional vendors.
use crate::config::llm_vendor_api_key_env_names;
use crate::llm_provider_descriptors::ProviderProtocol;

pub const SELECTED_VENDOR_ENV: &str = "APP_SELECTED_LLM_VENDOR";
pub const SELECTED_PROTOCOL_ENV: &str = "APP_SELECTED_LLM_PROVIDER_TYPE";
pub const SELECTED_RELAY_ENV: &str = "APP_SELECTED_LLM_HOSTED_RELAY";
pub const SELECTED_IDENTITY_ENV: [&str; 3] = [
    SELECTED_VENDOR_ENV,
    SELECTED_PROTOCOL_ENV,
    SELECTED_RELAY_ENV,
];

/// Receipt-fixed values must not impersonate the host's selected connection.
/// Identity still crosses only environment names declared by that receipt.
pub fn apply_host_identity_environment(
    command: &mut std::process::Command,
    declared: &[String],
    read: impl Fn(&str) -> Option<std::ffi::OsString>,
) {
    for name in SELECTED_IDENTITY_ENV {
        command.env_remove(name);
        if declared.iter().any(|declared| declared == name) {
            if let Some(value) = read(name) {
                command.env(name, value);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEnvironment {
    Standalone,
    Selected {
        vendor: String,
        protocol: ProviderProtocol,
        hosted_relay: bool,
    },
    Invalid,
}

impl ConnectionEnvironment {
    pub fn from_env_with(read: impl Fn(&str) -> Option<String>) -> Self {
        let value = |name| {
            read(name)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let vendor = value(SELECTED_VENDOR_ENV);
        let protocol = value(SELECTED_PROTOCOL_ENV);
        let relay = value(SELECTED_RELAY_ENV);
        if vendor.is_none() && protocol.is_none() && relay.is_none() {
            return Self::Standalone;
        }
        let (Some(vendor), Some(protocol)) = (vendor, protocol) else {
            return Self::Invalid;
        };
        let Some(protocol) = ProviderProtocol::from_runtime(&protocol) else {
            return Self::Invalid;
        };
        let hosted_relay = match relay.as_deref() {
            None | Some("false") => false,
            Some("true") if protocol == ProviderProtocol::OpenAiCompatible => true,
            _ => return Self::Invalid,
        };
        Self::Selected {
            vendor: vendor.to_ascii_lowercase(),
            protocol,
            hosted_relay,
        }
    }

    /// Alias ownership only. This does not grant receipt/policy/credential access.
    pub fn permits_vendor_credential(&self, vendor: &str) -> bool {
        match self {
            Self::Standalone => true,
            Self::Invalid => false,
            Self::Selected {
                vendor: selected,
                hosted_relay,
                ..
            } => {
                selected.eq_ignore_ascii_case(vendor)
                    || (*hosted_relay && vendor.eq_ignore_ascii_case("minimax"))
                    || llm_vendor_api_key_env_names(selected)
                        .iter()
                        .any(|name| llm_vendor_api_key_env_names(vendor).contains(name))
            }
        }
    }

    pub fn is_enrolled_relay(&self) -> bool {
        matches!(
            self,
            Self::Selected {
                hosted_relay: true,
                ..
            }
        )
    }
}

#[cfg(test)]
#[path = "llm_connection_environment_tests.rs"]
mod tests;

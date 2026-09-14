//! Versioned, operator-maintained provider error vocabulary. No task semantics.
use std::{collections::HashSet, path::Path, sync::OnceLock};

use serde::Deserialize;

use super::client::ProviderErrorKind;

const BUILTIN: &str = include_str!("../../../../configs/llm_provider_errors.toml");
static RULES: OnceLock<ErrorRules> = OnceLock::new();

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ErrorRules {
    pub schema_version: u32,
    pub revision: String,
    pub code_paths: Vec<String>,
    pub success_codes: Vec<String>,
    #[serde(default)]
    pub bindings: Vec<Binding>,
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub provider_name: String,
    pub profile: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Profile {
    pub id: String,
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub provider_names: Vec<String>,
    pub sources: Vec<String>,
    #[serde(default)]
    pub code_paths: Vec<String>,
    #[serde(default)]
    pub failure_code_paths: Vec<String>,
    #[serde(default)]
    pub success_codes: Option<Vec<String>>,
    #[serde(default)]
    pub message_code_paths: Vec<String>,
    #[serde(default)]
    pub message_code_http_statuses: Vec<u16>,
    #[serde(default)]
    pub http_status_paths: Vec<String>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rule {
    pub id: String,
    pub class: String,
    #[serde(default)]
    pub codes: Vec<String>,
    /// Optional HTTP guard; when codes are absent this is a status-only rule.
    #[serde(default)]
    pub http_statuses: Vec<u16>,
}

impl ErrorRules {
    pub(super) fn parse(raw: &str) -> Result<Self, String> {
        let rules: Self =
            toml::from_str(raw).map_err(|error| format!("provider_error_rules_parse:{error}"))?;
        if rules.schema_version != 1 || rules.revision.trim().is_empty() {
            return Err("provider_error_rules_version_invalid".into());
        }
        validate_paths(&rules.code_paths)?;
        if rules.code_paths.is_empty() || rules.success_codes.is_empty() {
            return Err("provider_error_rules_code_paths_empty".into());
        }
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        let mut hosts = HashSet::new();
        for profile in &rules.profiles {
            if !profile.message_code_paths.is_empty()
                && (profile.message_code_http_statuses.is_empty()
                    || profile
                        .message_code_http_statuses
                        .iter()
                        .any(|s| !(400..600).contains(s)))
            {
                return Err("provider_error_rules_message_code_scope_invalid".into());
            }
            if profile
                .success_codes
                .as_ref()
                .is_some_and(|codes| codes.is_empty())
            {
                return Err("provider_error_rules_success_codes_empty".into());
            }
            if !machine_token(&profile.id) || !ids.insert(profile.id.as_str()) {
                return Err("provider_error_rules_profile_invalid_or_duplicate".into());
            }
            if profile.sources.is_empty()
                || profile.sources.iter().any(|url| {
                    reqwest::Url::parse(url).map_or(true, |url| {
                        url.scheme() != "https" || url.host_str().is_none()
                    })
                })
            {
                return Err("provider_error_rules_source_invalid".into());
            }
            for name in &profile.provider_names {
                if name.trim().is_empty() || !names.insert(name) {
                    return Err("provider_error_rules_name_invalid_or_duplicate".into());
                }
            }
            for host in &profile.hosts {
                if host.is_empty()
                    || host.starts_with('.')
                    || host.ends_with('.')
                    || !host.bytes().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-'
                    })
                    || !hosts.insert(host)
                {
                    return Err("provider_error_rules_host_invalid_or_duplicate".into());
                }
            }
            for paths in [
                &profile.code_paths,
                &profile.failure_code_paths,
                &profile.message_code_paths,
                &profile.http_status_paths,
            ] {
                validate_paths(paths)?;
            }
            if profile.id == "default"
                && (!profile.message_code_paths.is_empty()
                    || !profile.failure_code_paths.is_empty()
                    || !profile.hosts.is_empty()
                    || !profile.provider_names.is_empty())
            {
                return Err("provider_error_rules_default_scope_invalid".into());
            }
            let mut rule_ids = HashSet::new();
            for rule in &profile.rules {
                if !machine_token(&rule.id)
                    || !rule_ids.insert(&rule.id)
                    || ProviderErrorKind::from_str(&rule.class).is_none()
                    || matches!(
                        rule.class.as_str(),
                        "local_non_retryable" | "transport_retryable"
                    )
                    || (rule.codes.is_empty() && rule.http_statuses.is_empty())
                    || rule.codes.iter().any(|code| code.trim().is_empty())
                    || rule
                        .http_statuses
                        .iter()
                        .any(|status| !(100..=599).contains(status))
                {
                    return Err("provider_error_rules_rule_invalid".into());
                }
            }
        }
        if !ids.contains("default") {
            return Err("provider_error_rules_default_missing".into());
        }
        let mut bindings = HashSet::new();
        for binding in &rules.bindings {
            if binding.provider_name.trim().is_empty()
                || !ids.contains(binding.profile.as_str())
                || !bindings.insert(&binding.provider_name)
            {
                return Err("provider_error_rules_binding_invalid".into());
            }
        }
        Ok(rules)
    }

    pub(super) fn profile(&self, name: &str, base_url: &str) -> &Profile {
        if let Some(binding) = self.bindings.iter().find(|b| b.provider_name == name) {
            return self.by_id(&binding.profile);
        }
        // A known endpoint beats a display/provider name. Most-specific host wins.
        let url = reqwest::Url::parse(base_url).ok();
        if let Some(host) = url.as_ref().and_then(reqwest::Url::host_str) {
            if let Some((profile, _)) = self
                .profiles
                .iter()
                .flat_map(|p| {
                    p.hosts
                        .iter()
                        .filter(move |suffix| {
                            host == *suffix
                                || host
                                    .strip_suffix(suffix.as_str())
                                    .is_some_and(|p| p.ends_with('.'))
                        })
                        .map(move |suffix| (p, suffix.len()))
                })
                .max_by_key(|(_, len)| *len)
            {
                return profile;
            }
        }
        self.profiles
            .iter()
            .find(|p| p.provider_names.iter().any(|n| n == name))
            .unwrap_or_else(|| self.by_id("default"))
    }

    pub(super) fn by_id(&self, id: &str) -> &Profile {
        self.profiles
            .iter()
            .find(|p| p.id == id)
            .expect("validated profile")
    }
}

fn machine_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
}

fn validate_paths(paths: &[String]) -> Result<(), String> {
    for path in paths {
        if !path.starts_with('/') || path == "/" || path.as_bytes().contains(&b'\0') {
            return Err("provider_error_rules_pointer_invalid".into());
        }
        let mut bytes = path.bytes();
        while let Some(byte) = bytes.next() {
            if byte == b'~' && !matches!(bytes.next(), Some(b'0' | b'1')) {
                return Err("provider_error_rules_pointer_invalid".into());
            }
        }
    }
    Ok(())
}

pub(crate) fn initialize(config_path: &str) -> anyhow::Result<()> {
    let explicit = claw_core::product_identity::env_string("LLM_PROVIDER_ERRORS_CONFIG").ok();
    let path = explicit
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(config_path)
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("llm_provider_errors.toml")
        });
    let rules = load(&path, explicit.is_some()).map_err(anyhow::Error::msg)?;
    tracing::info!(revision = %rules.revision, profiles = rules.profiles.len(), "provider_error_rules_loaded");
    RULES
        .set(rules)
        .map_err(|_| anyhow::anyhow!("provider_error_rules_already_initialized"))
}

pub(super) fn load(path: &Path, required: bool) -> Result<ErrorRules, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => BUILTIN.into(),
        Err(error) => return Err(format!("provider_error_rules_read:{error}")),
    };
    ErrorRules::parse(&raw)
}

pub(super) fn active() -> &'static ErrorRules {
    RULES.get_or_init(|| ErrorRules::parse(BUILTIN).expect("tested built-in provider error rules"))
}

#[cfg(test)]
#[path = "error_rules_tests.rs"]
mod tests;

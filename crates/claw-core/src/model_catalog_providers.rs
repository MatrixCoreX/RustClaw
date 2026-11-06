//! Request-owned text-provider projection of the same typed runtime inputs.
use super::{contains_model, string_list_field};
use crate::config::{LlmConfig, LlmProviderConfig};
use crate::llm_provider_descriptors::{
    configured_vendor_configs, credential_vendor_name, effective_provider_configs,
    matches_hosted_relay, matches_provider_override, process_overrides, ProviderProtocol,
};
use std::collections::BTreeMap;

type Table = toml::map::Map<String, toml::Value>;
pub(super) type Targets = BTreeMap<(String, String), Table>;

#[derive(Clone)]
pub(super) struct TextProvider {
    pub config: LlmProviderConfig,
    pub generated: bool,
    table: Table,
    aliases: Vec<String>,
}

impl TextProvider {
    pub(super) fn table(&self) -> Table {
        let mut table = self.table.clone();
        if self.generated {
            table.remove("api_key");
        }
        for (field, value) in [
            ("model", self.config.model.as_str()),
            ("base_url", self.config.base_url.as_str()),
            ("api_format", self.config.provider_type.as_str()),
        ] {
            table.insert(field.into(), toml::Value::String(value.into()));
        }
        table.insert(
            "models".into(),
            toml::Value::Array(
                self.aliases
                    .iter()
                    .cloned()
                    .map(toml::Value::String)
                    .collect(),
            ),
        );
        table.insert(
            "input_modalities".into(),
            toml::Value::Array(
                self.config
                    .input_modalities
                    .iter()
                    .cloned()
                    .map(toml::Value::String)
                    .collect(),
            ),
        );
        for (field, value) in [
            (
                "timeout_seconds",
                i64::try_from(self.config.timeout_seconds).ok(),
            ),
            (
                "context_window_tokens",
                self.config
                    .context_window_tokens
                    .and_then(|value| i64::try_from(value).ok()),
            ),
        ] {
            match value {
                Some(value) => {
                    table.insert(field.into(), toml::Value::Integer(value));
                }
                None => {
                    table.remove(field);
                }
            }
        }
        table
    }
}

pub(super) fn provider_key(config: &LlmProviderConfig) -> &str {
    if let Some(vendor) = credential_vendor_name(&config.name) {
        if matches_provider_override(&config.name, &config.provider_type, vendor) {
            return vendor;
        }
    }
    config.name.trim()
}

pub(super) struct ProviderSnapshot {
    llm: Option<LlmConfig>,
    sources: Vec<TextProvider>,
    active: Vec<LlmProviderConfig>,
    pub explicit: bool,
    selected: (String, String),
}

impl ProviderSnapshot {
    pub(super) fn new(config: &toml::Value) -> Self {
        let raw = config.get("llm").and_then(toml::Value::as_table);
        let llm: Option<LlmConfig> = config
            .get("llm")
            .cloned()
            .and_then(|value| value.try_into().ok());
        let explicit = raw
            .and_then(|table| table.get("providers"))
            .and_then(toml::Value::as_array)
            .is_some_and(|values| !values.is_empty());
        let (provider_override, model_override) = process_overrides();
        let active = llm
            .as_ref()
            .map(|llm| {
                effective_provider_configs(
                    llm,
                    provider_override.as_deref(),
                    model_override.as_deref(),
                )
            })
            .unwrap_or_default();
        let mut sources = Vec::new();
        if let Some(llm) = &llm {
            if explicit {
                let rows = raw
                    .and_then(|table| table.get("providers"))
                    .and_then(toml::Value::as_array);
                for (index, provider) in llm.providers.iter().enumerate() {
                    if ProviderProtocol::from_runtime(&provider.provider_type).is_none() {
                        continue;
                    }
                    let table = rows
                        .and_then(|rows| rows.get(index))
                        .and_then(toml::Value::as_table)
                        .cloned()
                        .unwrap_or_default();
                    sources.push(TextProvider {
                        config: provider.clone(),
                        generated: false,
                        table,
                        aliases: Vec::new(),
                    });
                }
            } else {
                for provider in configured_vendor_configs(llm) {
                    let table = raw
                        .and_then(|table| table.get(provider_key(&provider)))
                        .and_then(toml::Value::as_table)
                        .cloned()
                        .unwrap_or_default();
                    let aliases = string_list_field(&table, "models");
                    sources.push(TextProvider {
                        config: provider,
                        generated: true,
                        table,
                        aliases,
                    });
                }
            }
        }
        sources.sort_by_key(|provider| provider.config.priority);
        let requested = |name: &str| {
            raw.and_then(|table| table.get(name))
                .and_then(toml::Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let selected = active
            .first()
            .map(|provider| (provider_key(provider).into(), provider.model.clone()))
            .unwrap_or_else(|| {
                (
                    provider_override.unwrap_or_else(|| requested("selected_vendor")),
                    model_override.unwrap_or_else(|| requested("selected_model")),
                )
            });
        Self {
            llm,
            sources,
            active,
            explicit,
            selected,
        }
    }

    pub(super) fn selected(&self) -> &(String, String) {
        &self.selected
    }

    pub(super) fn for_model(&self, provider: &str, model: &str) -> Option<TextProvider> {
        let mut matching = self
            .sources
            .iter()
            .filter(|source| provider_key(&source.config) == provider);
        let first = matching.next()?;
        let matches_model = |source: &TextProvider| {
            source.config.model.trim() == model || contains_model(&source.aliases, model)
        };
        let declared = matches_model(first)
            || matching.any(matches_model)
            || self
                .active
                .iter()
                .any(|source| provider_key(source) == provider && source.model.trim() == model);
        if !declared {
            return None;
        }
        // An explicit name may denote a fallback group. Per-task selection
        // overrides that entire group, so its first actual transport owns metadata.
        let mut source: TextProvider = first.clone();
        source.config.model = model.into();
        source.config.params.device_key_enrollment = self
            .llm
            .as_ref()
            .is_some_and(|llm| matches_hosted_relay(llm, &source.config));
        Some(source)
    }

    pub(super) fn add_text_targets(&self, targets: &mut Targets) {
        for (provider, model) in self
            .sources
            .iter()
            .map(|source| (provider_key(&source.config), source.config.model.trim()))
            .chain(
                self.active
                    .iter()
                    .map(|source| (provider_key(source), source.model.trim())),
            )
        {
            if model.is_empty() {
                continue;
            }
            if let Some(source) = self.for_model(provider, model) {
                targets.insert((provider.into(), model.into()), source.table());
            }
        }
    }
}

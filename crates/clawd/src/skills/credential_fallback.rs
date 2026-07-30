use std::collections::BTreeSet;

use claw_core::secrets::{provision_secret_envs, ProvisionError, SecretValue, SecretsBroker};
use claw_core::skill_registry::Capability;

use crate::llm_gateway::SelectedLlmConnection;

pub(super) struct ProvisionedSkillSecrets {
    pub(super) envs: Vec<(String, SecretValue)>,
    pub(super) fallback_credentials: Vec<String>,
}

fn selected_llm_fallback_matches(secret_name: &str, selected_vendor: &str) -> bool {
    let vendor = selected_vendor.trim().to_ascii_lowercase();
    if vendor.is_empty() {
        return false;
    }
    let canonical = secret_name.trim().to_ascii_lowercase();
    canonical
        .strip_suffix(&format!("_{vendor}_api_key"))
        .is_some_and(|usage| !usage.is_empty())
}

pub(super) fn provision_skill_secret_envs(
    broker: &dyn SecretsBroker,
    capabilities: &[Capability],
    selected_llm: Option<&SelectedLlmConnection>,
) -> Result<ProvisionedSkillSecrets, ProvisionError> {
    match provision_secret_envs(broker, capabilities) {
        Ok(envs) => Ok(ProvisionedSkillSecrets {
            envs,
            fallback_credentials: Vec::new(),
        }),
        Err(ProvisionError::MissingSecrets { missing }) => {
            let allowed_fallbacks: BTreeSet<&str> = capabilities
                .iter()
                .filter_map(|capability| match capability {
                    Capability::LlmCredentialFallback(name) => Some(name.as_str()),
                    _ => None,
                })
                .collect();
            let selected_llm = selected_llm.filter(|connection| {
                !connection.api_key.trim().is_empty()
                    && capabilities
                        .iter()
                        .any(|capability| matches!(capability, Capability::Llm))
            });
            let fallback_credentials: BTreeSet<String> = selected_llm
                .map(|connection| {
                    missing
                        .iter()
                        .filter_map(move |name| {
                            (allowed_fallbacks.contains(name.as_str())
                                && selected_llm_fallback_matches(name, &connection.vendor))
                            .then(|| name.clone())
                        })
                        .collect()
                })
                .unwrap_or_default();
            if fallback_credentials.is_empty() {
                return Err(ProvisionError::MissingSecrets { missing });
            }

            let remaining_capabilities: Vec<Capability> = capabilities
                .iter()
                .filter(|capability| match capability {
                    Capability::Secrets(name) => !fallback_credentials.contains(name),
                    _ => true,
                })
                .cloned()
                .collect();
            let envs = provision_secret_envs(broker, &remaining_capabilities)?;
            Ok(ProvisionedSkillSecrets {
                envs,
                fallback_credentials: fallback_credentials.into_iter().collect(),
            })
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use claw_core::secrets::{SecretValue, SecretsError};

    use super::*;

    struct FixtureBroker(HashMap<String, String>);

    impl SecretsBroker for FixtureBroker {
        fn lookup(&self, name: &str) -> Result<Option<SecretValue>, SecretsError> {
            Ok(self.0.get(name).cloned().map(SecretValue::new))
        }
    }

    fn selected(vendor: &str, key: &str) -> SelectedLlmConnection {
        SelectedLlmConnection {
            vendor: vendor.to_string(),
            provider_type: "openai_compat".to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            model: "fixture-model".to_string(),
            api_key: key.to_string(),
        }
    }

    #[test]
    fn missing_matching_multimodal_secret_uses_selected_llm_credential() {
        let caps = vec![
            Capability::Llm,
            Capability::Secrets("image_generation_minimax_api_key".to_string()),
            Capability::LlmCredentialFallback("image_generation_minimax_api_key".to_string()),
        ];
        let provisioned = provision_skill_secret_envs(
            &FixtureBroker(HashMap::new()),
            &caps,
            Some(&selected("minimax", "main-key")),
        )
        .expect("selected LLM fallback");

        assert!(provisioned.envs.is_empty());
        assert_eq!(
            provisioned.fallback_credentials,
            vec!["image_generation_minimax_api_key"]
        );
    }

    #[test]
    fn dedicated_secret_stays_preferred_over_selected_llm_credential() {
        let caps = vec![
            Capability::Llm,
            Capability::Secrets("image_generation_minimax_api_key".to_string()),
            Capability::LlmCredentialFallback("image_generation_minimax_api_key".to_string()),
        ];
        let broker = FixtureBroker(HashMap::from([(
            "image_generation_minimax_api_key".to_string(),
            "dedicated-key".to_string(),
        )]));
        let provisioned =
            provision_skill_secret_envs(&broker, &caps, Some(&selected("minimax", "main-key")))
                .expect("dedicated credential");

        assert_eq!(provisioned.envs.len(), 1);
        assert!(provisioned.fallback_credentials.is_empty());
        assert_eq!(provisioned.envs[0].1.expose(), "dedicated-key");
    }

    #[test]
    fn fallback_rejects_vendor_mismatch_missing_key_and_non_multimodal_secret() {
        let broker = FixtureBroker(HashMap::new());
        for (secret, selected) in [
            (
                "image_generation_openai_api_key",
                Some(selected("minimax", "main-key")),
            ),
            (
                "image_generation_minimax_api_key",
                Some(selected("minimax", "")),
            ),
            ("database_password", Some(selected("minimax", "main-key"))),
        ] {
            let caps = vec![
                Capability::Llm,
                Capability::Secrets(secret.to_string()),
                Capability::LlmCredentialFallback(secret.to_string()),
            ];
            assert!(matches!(
                provision_skill_secret_envs(&broker, &caps, selected.as_ref()),
                Err(ProvisionError::MissingSecrets { .. })
            ));
        }
    }

    #[test]
    fn fallback_requires_explicit_llm_capability() {
        let caps = vec![
            Capability::Secrets("image_generation_minimax_api_key".to_string()),
            Capability::LlmCredentialFallback("image_generation_minimax_api_key".to_string()),
        ];
        assert!(matches!(
            provision_skill_secret_envs(
                &FixtureBroker(HashMap::new()),
                &caps,
                Some(&selected("minimax", "main-key")),
            ),
            Err(ProvisionError::MissingSecrets { .. })
        ));
    }

    #[test]
    fn fallback_requires_the_exact_declared_secret_name() {
        let caps = vec![
            Capability::Llm,
            Capability::Secrets("image_generation_minimax_api_key".to_string()),
            Capability::LlmCredentialFallback("image_edit_minimax_api_key".to_string()),
        ];
        assert!(matches!(
            provision_skill_secret_envs(
                &FixtureBroker(HashMap::new()),
                &caps,
                Some(&selected("minimax", "main-key")),
            ),
            Err(ProvisionError::MissingSecrets { .. })
        ));
    }
}

fn effective_model_provider_key(state: &AppState, vendor: &str) -> anyhow::Result<String> {
    effective_model_provider_credential(state, vendor).map(|(key, _)| key)
}

fn effective_model_provider_credential(
    state: &AppState,
    vendor: &str,
) -> anyhow::Result<(String, &'static str)> {
    let broker = claw_core::secrets::EnvFileSecretsBroker::new(
        claw_core::git_remote_config::git_credential_store_path(&state.skill_rt.workspace_root),
    );
    if let Some((secret, source)) = broker
        .lookup_text_vendor_with_source(vendor)
        .map_err(|_| model_settings_failure("credentials", "provider_keys_read_failed"))?
        .filter(|(secret, _)| !secret.is_empty())
    {
        return Ok((secret.expose().to_string(), source.machine_name()));
    }
    Ok((String::new(), "none"))
}

fn model_provider_display_mask(value: &str) -> bool {
    let chars = value.chars().collect::<Vec<_>>();
    ((4..=8).contains(&chars.len()) && chars.iter().all(|value| *value == '*'))
        || (chars.len() == 12 && chars[4..8].iter().all(|value| *value == '*'))
}

fn model_provider_uses_device_enrollment(config: &toml::Value, vendor: &str) -> bool {
    let llm = config.get("llm");
    let provider = llm.and_then(|value| value.get(vendor));
    let selected = llm
        .and_then(|value| value.get("selected_vendor"))
        .and_then(toml::Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(vendor));
    let model = selected
        .then(|| {
            llm.and_then(|value| value.get("selected_model"))
                .and_then(toml::Value::as_str)
        })
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            provider
                .and_then(|value| value.get("model"))
                .and_then(toml::Value::as_str)
        })
        .unwrap_or("");
    let base = provider
        .and_then(|value| value.get("base_url"))
        .and_then(toml::Value::as_str)
        .unwrap_or("");
    matches_hosted_relay_preset(config, vendor, model, base)
}

fn prepare_text_provider_keys(
    state: &AppState,
    file: &mut ModelSettingsFile,
    requested: &HashMap<String, String>,
) -> anyhow::Result<Vec<(String, String)>> {
    if requested.is_empty() {
        return Ok(Vec::new());
    }
    file.value
        .get("llm")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| anyhow::anyhow!("model_settings_llm_section_required"))?;
    let mut updates = Vec::new();
    for (vendor, raw) in requested {
        if vendor != &vendor.trim().to_ascii_lowercase()
            || claw_core::config::llm_vendor_api_key_env_names(vendor).is_empty()
        {
            return Err(model_settings_input_failure("provider_keys_vendor_invalid"));
        }
        let key = raw.trim();
        if key.is_empty() {
            continue;
        }
        if model_provider_uses_device_enrollment(&file.value, vendor) {
            if key == mask_secret("") {
                continue;
            }
            return Err(model_settings_input_failure(
                "hosted_relay_manual_key_not_allowed",
            ));
        }
        claw_core::secrets::model_environment::validate_update(vendor, key)
            .map_err(|_| model_settings_input_failure("provider_keys_value_invalid"))?;
        let effective = effective_model_provider_key(state, vendor)?;
        if key == effective || key == mask_secret(&effective) {
            continue;
        }
        if model_provider_display_mask(key) {
            return Err(model_settings_input_failure("provider_keys_mask_invalid"));
        }
        updates.push((vendor.clone(), key.to_string()));
    }
    let llm = file
        .value
        .get_mut("llm")
        .and_then(toml::Value::as_table_mut)
        .expect("llm table validated");
    for (vendor, _) in &updates {
        let provider = llm
            .entry(vendor.clone())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("model_settings_provider_not_table"))?;
        // Text-model credentials are deliberately ignored in TOML at runtime.
        // Publish only an empty reference here; the actual key goes to the broker.
        provider.insert("api_key".into(), toml::Value::String(String::new()));
        file.changed = true;
    }
    Ok(updates)
}

fn publish_model_provider_keys(
    state: &AppState,
    files: [ModelSettingsFile; 5],
    updates: &[(String, String)],
    persistence: Option<PathBuf>,
) -> anyhow::Result<()> {
    use claw_core::secrets::model_environment::{with_saved_many, PublicationError};
    let values = updates
        .iter()
        .map(|(vendor, key)| (vendor.as_str(), key.as_str()))
        .collect::<Vec<_>>();
    with_saved_many(&state.skill_rt.workspace_root, &values, || {
        save_model_settings_files_with_persistence(state, files, persistence)
    })
    .map_err(|error| match error {
        PublicationError::Operation(error) => error,
        PublicationError::Environment(_) => {
            model_settings_failure("credentials", "provider_keys_environment_write_failed")
        }
        PublicationError::Recovery(_) => {
            model_settings_failure("credentials", "provider_keys_environment_rollback_failed")
        }
    })
}

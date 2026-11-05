fn write_model_config(state: &AppState, req: &ModelConfigUpdateRequest) -> anyhow::Result<()> {
    let mut files = load_model_settings_files(state, req.llm.is_some())?;
    if let Some(llm) = req.llm.as_ref() {
        let config = &mut files[0];
        let table = config
            .value
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("model_config_root_not_table"))?;
        let llm_table = table
            .entry("llm".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        // Preserve the existing ability to repair a non-table llm section.
        if !llm_table.is_table() {
            *llm_table = toml::Value::Table(toml::map::Map::new());
        }
        let llm_table = llm_table.as_table_mut().expect("llm table initialized");
        llm_table.insert(
            "selected_vendor".to_string(),
            toml::Value::String(llm.vendor.clone()),
        );
        llm_table.insert(
            "selected_model".to_string(),
            toml::Value::String(llm.model.clone()),
        );
        config.changed = true;
    }
    for (index, section, item) in req.module_updates() {
        if let Some(item) = item {
            upsert_model_section(&mut files[index].value, section, item)?;
            files[index].changed = true;
        }
    }
    save_model_settings_files(state, files)
}

fn write_provider_keys(state: &AppState, req: &ProviderKeysResponse) -> anyhow::Result<()> {
    let mut files = load_model_settings_files(state, !req.llm.is_empty())?;
    let updates = prepare_text_provider_keys(state, &mut files[0], &req.llm)?;
    for (index, sections) in [
        (1, &req.image),
        (2, &req.audio),
        (3, &req.video),
        (4, &req.music),
    ] {
        if sections.is_empty() {
            continue;
        }
        let file = &mut files[index];
        let table = file
            .value
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("model_settings_root_not_table"))?;
        for (section, vendors) in sections {
            if vendors.is_empty() {
                continue;
            }
            let section = table
                .entry(section.clone())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let section = section
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("model_settings_section_not_table"))?;
            let providers = section
                .entry("providers".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let providers = providers
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("model_settings_providers_not_table"))?;
            for (vendor, key) in vendors {
                if key.is_empty() {
                    continue;
                }
                let provider = providers
                    .entry(vendor.clone())
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
                let provider = provider
                    .as_table_mut()
                    .ok_or_else(|| anyhow::anyhow!("model_settings_provider_not_table"))?;
                file.changed |= update_model_provider_key(provider, key);
            }
        }
    }
    publish_model_provider_keys(
        state,
        files,
        &updates,
        crate::configuration_files::persistence_path(&state.skill_rt.workspace_root),
    )
}

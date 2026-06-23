fn default_agent_id() -> String {
    "main".to_string()
}

fn default_telegram_access_mode() -> String {
    "public".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelConfigItem {
    vendor: String,
    model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_key_configured: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_key_masked: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    available_models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    risk_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dry_run_supported: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_provider: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_supported: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unsupported_reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct ModelConfigResponse {
    llm: ModelConfigItem,
    image_edit: ModelConfigItem,
    image_generation: ModelConfigItem,
    image_vision: ModelConfigItem,
    audio_transcribe: ModelConfigItem,
    audio_synthesize: ModelConfigItem,
    video_generation: ModelConfigItem,
    music_generation: ModelConfigItem,
    restart_required: bool,
}

#[derive(Debug, Deserialize)]
struct ModelConfigUpdateRequest {
    llm: Option<ModelConfigItem>,
    image_edit: Option<ModelConfigItem>,
    image_generation: Option<ModelConfigItem>,
    image_vision: Option<ModelConfigItem>,
    audio_transcribe: Option<ModelConfigItem>,
    audio_synthesize: Option<ModelConfigItem>,
    video_generation: Option<ModelConfigItem>,
    music_generation: Option<ModelConfigItem>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProviderKeysResponse {
    #[serde(default)]
    llm: HashMap<String, String>,
    #[serde(default)]
    image: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    audio: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    video: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    music: HashMap<String, HashMap<String, String>>,
}

fn default_model_item() -> ModelConfigItem {
    ModelConfigItem {
        vendor: String::new(),
        model: String::new(),
        base_url: None,
        api_key: None,
        api_key_configured: None,
        api_key_masked: None,
        capabilities: Vec::new(),
        available_models: Vec::new(),
        risk_level: None,
        dry_run_supported: None,
        external_provider: None,
        provider_supported: None,
        unsupported_reason: None,
    }
}

fn read_toml_value(path: &Path) -> toml::Value {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|_| String::new());
    toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()))
}

fn read_model_section(value: &toml::Value, section: &str) -> ModelConfigItem {
    let Some(table) = value.get(section).and_then(|t| t.as_table()) else {
        return model_item_with_capability_metadata(default_model_item(), section);
    };
    let vendor = table
        .get("default_vendor")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let model = table
        .get("default_model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let provider = table
        .get("providers")
        .and_then(|v| v.as_table())
        .and_then(|providers| providers.get(&vendor))
        .and_then(|v| v.as_table());
    let base_url = provider
        .and_then(|p| p.get("base_url"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let api_key = provider
        .and_then(|p| p.get("api_key"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let item = ModelConfigItem {
        vendor,
        model,
        base_url,
        api_key: None,
        api_key_configured: Some(api_key.is_some()),
        api_key_masked: api_key.map(mask_secret),
        capabilities: Vec::new(),
        available_models: read_section_model_cache(table),
        risk_level: None,
        dry_run_supported: None,
        external_provider: None,
        provider_supported: None,
        unsupported_reason: None,
    };
    model_item_with_capability_metadata(item, section)
}

fn read_section_model_cache(table: &toml::map::Map<String, toml::Value>) -> Vec<String> {
    let mut models = table
        .get("models")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    models
}

fn read_llm_model_cache(config: &toml::Value, vendor: &str) -> Vec<String> {
    let mut models = config
        .get("llm")
        .and_then(toml::Value::as_table)
        .and_then(|llm| llm.get(vendor.trim()))
        .and_then(toml::Value::as_table)
        .and_then(|vendor_table| vendor_table.get("models"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    models
}

fn model_item_with_capability_metadata(
    mut item: ModelConfigItem,
    section: &str,
) -> ModelConfigItem {
    let (capability, risk_level, dry_run_supported, external_provider) =
        match section.trim() {
            "llm" => ("text.chat", "medium", false, true),
            "image_edit" => ("image.edit", "high", true, true),
            "image_generation" => ("image.generate", "high", true, true),
            "image_vision" => ("image.understand", "medium", false, true),
            "audio_transcribe" => ("audio.transcribe", "medium", false, true),
            "audio_synthesize" => ("audio.synthesize", "high", true, true),
            "video_generation" => ("video.generate", "high", true, true),
            "music_generation" => ("music.generate", "high", true, true),
            _ => ("unknown", "unknown", false, false),
        };
    if capability != "unknown" {
        item.capabilities = vec![capability.to_string()];
    }
    item.risk_level = Some(risk_level.to_string());
    item.dry_run_supported = Some(dry_run_supported);
    item.external_provider = Some(external_provider);
    let (provider_supported, unsupported_reason) = provider_support_status(&item);
    item.provider_supported = provider_supported;
    item.unsupported_reason = unsupported_reason;
    item
}

fn provider_support_status(item: &ModelConfigItem) -> (Option<bool>, Option<String>) {
    if item.vendor.trim().is_empty() {
        return (Some(false), Some("provider_not_configured".to_string()));
    }
    if item.model.trim().is_empty() {
        return (Some(false), Some("model_not_configured".to_string()));
    }
    if !item.available_models.is_empty()
        && !item
            .available_models
            .iter()
            .any(|model| model.trim() == item.model.trim())
    {
        return (Some(false), Some("model_not_in_available_models".to_string()));
    }
    (Some(true), None)
}

fn upsert_model_section(
    value: &mut toml::Value,
    section: &str,
    item: &ModelConfigItem,
) -> anyhow::Result<()> {
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("model_config_root_not_table"))?;
    let section_value = root
        .entry(section.to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let section_table = section_value
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("model_config_section_not_table"))?;
    section_table.insert(
        "default_vendor".to_string(),
        toml::Value::String(item.vendor.clone()),
    );
    section_table.insert(
        "default_model".to_string(),
        toml::Value::String(item.model.clone()),
    );

    let vendor = item.vendor.trim();
    if vendor.is_empty() {
        return Ok(());
    }
    let should_update_provider = item
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some()
        || item
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_some()
        || !item.model.trim().is_empty();
    if !should_update_provider {
        return Ok(());
    }

    let providers_value = section_table
        .entry("providers".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let providers = providers_value
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("model_config_providers_not_table"))?;
    let provider_value = providers
        .entry(vendor.to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let provider = provider_value
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("model_config_provider_not_table"))?;
    if let Some(base_url) = item
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        provider.insert(
            "base_url".to_string(),
            toml::Value::String(base_url.to_string()),
        );
    }
    if !item.model.trim().is_empty() {
        provider.insert(
            "model".to_string(),
            toml::Value::String(item.model.trim().to_string()),
        );
    }
    if let Some(api_key) = item
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        provider.insert("api_key".to_string(), toml::Value::String(api_key.to_string()));
    }
    Ok(())
}

fn read_model_config(state: &AppState) -> anyhow::Result<ModelConfigResponse> {
    let root = &state.skill_rt.workspace_root;

    let config_path = root.join("configs/config.toml");
    let config_raw = std::fs::read_to_string(&config_path).unwrap_or_else(|_| String::new());
    let config: toml::Value =
        toml::from_str(&config_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let llm = config
        .get("llm")
        .and_then(|t| t.as_table())
        .map(|t| {
            let vendor = t
                .get("selected_vendor")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let model = t
                .get("selected_model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            model_item_with_capability_metadata(
                ModelConfigItem {
                    available_models: read_llm_model_cache(&config, &vendor),
                    vendor,
                    model,
                    base_url: None,
                    api_key: None,
                    api_key_configured: None,
                    api_key_masked: None,
                    capabilities: Vec::new(),
                    risk_level: None,
                    dry_run_supported: None,
                    external_provider: None,
                    provider_supported: None,
                    unsupported_reason: None,
                },
                "llm",
            )
        })
        .unwrap_or_else(|| model_item_with_capability_metadata(default_model_item(), "llm"));

    let image = read_toml_value(&root.join("configs/image.toml"));
    let image_edit = read_model_section(&image, "image_edit");
    let image_generation = read_model_section(&image, "image_generation");
    let image_vision = read_model_section(&image, "image_vision");

    let audio = read_toml_value(&root.join("configs/audio.toml"));
    let audio_transcribe = read_model_section(&audio, "audio_transcribe");
    let audio_synthesize = read_model_section(&audio, "audio_synthesize");

    let video = read_toml_value(&root.join("configs/video.toml"));
    let video_generation = read_model_section(&video, "video_generation");

    let music = read_toml_value(&root.join("configs/music.toml"));
    let music_generation = read_model_section(&music, "music_generation");

    Ok(ModelConfigResponse {
        llm,
        image_edit,
        image_generation,
        image_vision,
        audio_transcribe,
        audio_synthesize,
        video_generation,
        music_generation,
        restart_required: true,
    })
}

fn write_model_config(state: &AppState, req: &ModelConfigUpdateRequest) -> anyhow::Result<()> {
    let root = &state.skill_rt.workspace_root;

    if let Some(ref llm) = req.llm {
        let path = root.join("configs/config.toml");
        let raw = std::fs::read_to_string(&path)?;
        let mut value: toml::Value = toml::from_str(&raw)?;
        if let Some(t) = value.get_mut("llm").and_then(|v| v.as_table_mut()) {
            t.insert(
                "selected_vendor".to_string(),
                toml::Value::String(llm.vendor.clone()),
            );
            t.insert(
                "selected_model".to_string(),
                toml::Value::String(llm.model.clone()),
            );
        } else {
            let mut tbl = toml::map::Map::new();
            tbl.insert(
                "selected_vendor".to_string(),
                toml::Value::String(llm.vendor.clone()),
            );
            tbl.insert(
                "selected_model".to_string(),
                toml::Value::String(llm.model.clone()),
            );
            value
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("config.toml root is not a table"))?
                .insert("llm".to_string(), toml::Value::Table(tbl));
        }
        std::fs::write(&path, toml::to_string_pretty(&value)?)?;
    }

    let mut image_modified = false;
    let image_path = root.join("configs/image.toml");
    let image_raw = std::fs::read_to_string(&image_path).unwrap_or_else(|_| String::new());
    let mut image: toml::Value =
        toml::from_str(&image_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    for (section, item) in [
        ("image_edit", req.image_edit.as_ref()),
        ("image_generation", req.image_generation.as_ref()),
        ("image_vision", req.image_vision.as_ref()),
    ] {
        if let Some(it) = item {
            image_modified = true;
            upsert_model_section(&mut image, section, it)?;
        }
    }
    if image_modified {
        std::fs::write(&image_path, toml::to_string_pretty(&image)?)?;
    }

    let mut audio_modified = false;
    let audio_path = root.join("configs/audio.toml");
    let audio_raw = std::fs::read_to_string(&audio_path).unwrap_or_else(|_| String::new());
    let mut audio: toml::Value =
        toml::from_str(&audio_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    for (section, item) in [
        ("audio_transcribe", req.audio_transcribe.as_ref()),
        ("audio_synthesize", req.audio_synthesize.as_ref()),
    ] {
        if let Some(it) = item {
            audio_modified = true;
            upsert_model_section(&mut audio, section, it)?;
        }
    }
    if audio_modified {
        std::fs::write(&audio_path, toml::to_string_pretty(&audio)?)?;
    }

    if let Some(ref it) = req.video_generation {
        let video_path = root.join("configs/video.toml");
        let video_raw = std::fs::read_to_string(&video_path).unwrap_or_else(|_| String::new());
        let mut video: toml::Value = toml::from_str(&video_raw)
            .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
        upsert_model_section(&mut video, "video_generation", it)?;
        std::fs::write(&video_path, toml::to_string_pretty(&video)?)?;
    }

    if let Some(ref it) = req.music_generation {
        let music_path = root.join("configs/music.toml");
        let music_raw = std::fs::read_to_string(&music_path).unwrap_or_else(|_| String::new());
        let mut music: toml::Value = toml::from_str(&music_raw)
            .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
        upsert_model_section(&mut music, "music_generation", it)?;
        std::fs::write(&music_path, toml::to_string_pretty(&music)?)?;
    }

    Ok(())
}

fn read_llm_provider_keys(config: &toml::Value) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(llm) = config.get("llm").and_then(|v| v.as_table()) else {
        return out;
    };
    for (k, v) in llm {
        if let Some(tbl) = v.as_table() {
            if let Some(ak) = tbl.get("api_key").and_then(|a| a.as_str()) {
                out.insert(k.clone(), mask_secret(ak));
            }
        }
    }
    out
}

fn read_module_provider_keys(
    config: &toml::Value,
    sections: &[&str],
) -> HashMap<String, HashMap<String, String>> {
    let mut out = HashMap::new();
    for section in sections {
        let mut vendors = HashMap::new();
        if let Some(providers) = config
            .get(section)
            .and_then(|v| v.get("providers"))
            .and_then(|v| v.as_table())
        {
            for (vendor, tbl) in providers {
                if let Some(t) = tbl.as_table() {
                    if let Some(ak) = t.get("api_key").and_then(|a| a.as_str()) {
                        vendors.insert(vendor.clone(), mask_secret(ak));
                    }
                }
            }
        }
        out.insert((*section).to_string(), vendors);
    }
    out
}

fn read_image_provider_keys(image: &toml::Value) -> HashMap<String, HashMap<String, String>> {
    read_module_provider_keys(image, &["image_edit", "image_generation", "image_vision"])
}

fn read_audio_provider_keys(audio: &toml::Value) -> HashMap<String, HashMap<String, String>> {
    read_module_provider_keys(audio, &["audio_synthesize", "audio_transcribe"])
}

fn read_provider_keys(state: &AppState) -> anyhow::Result<ProviderKeysResponse> {
    let root = &state.skill_rt.workspace_root;

    let config_path = root.join("configs/config.toml");
    let config_raw = std::fs::read_to_string(&config_path).unwrap_or_else(|_| String::new());
    let config: toml::Value =
        toml::from_str(&config_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let llm = read_llm_provider_keys(&config);

    let image_path = root.join("configs/image.toml");
    let image_raw = std::fs::read_to_string(&image_path).unwrap_or_else(|_| String::new());
    let image: toml::Value =
        toml::from_str(&image_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let image_keys = read_image_provider_keys(&image);

    let audio_path = root.join("configs/audio.toml");
    let audio_raw = std::fs::read_to_string(&audio_path).unwrap_or_else(|_| String::new());
    let audio: toml::Value =
        toml::from_str(&audio_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let audio_keys = read_audio_provider_keys(&audio);

    let video_path = root.join("configs/video.toml");
    let video_raw = std::fs::read_to_string(&video_path).unwrap_or_else(|_| String::new());
    let video: toml::Value =
        toml::from_str(&video_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let video_keys = read_module_provider_keys(&video, &["video_generation"]);

    let music_path = root.join("configs/music.toml");
    let music_raw = std::fs::read_to_string(&music_path).unwrap_or_else(|_| String::new());
    let music: toml::Value =
        toml::from_str(&music_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let music_keys = read_module_provider_keys(&music, &["music_generation"]);

    Ok(ProviderKeysResponse {
        llm,
        image: image_keys,
        audio: audio_keys,
        video: video_keys,
        music: music_keys,
    })
}

fn write_provider_keys(state: &AppState, req: &ProviderKeysResponse) -> anyhow::Result<()> {
    let root = &state.skill_rt.workspace_root;

    if !req.llm.is_empty() {
        let path = root.join("configs/config.toml");
        let raw = std::fs::read_to_string(&path)?;
        let mut config: toml::Value = toml::from_str(&raw)?;
        let llm = config
            .get_mut("llm")
            .and_then(|v| v.as_table_mut())
            .ok_or_else(|| anyhow::anyhow!("config.toml has no [llm] table"))?;
        for (vendor, new_key) in &req.llm {
            if new_key.is_empty() {
                continue;
            }
            let entry = llm
                .entry(vendor.clone())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if let Some(t) = entry.as_table_mut() {
                t.insert("api_key".to_string(), toml::Value::String(new_key.clone()));
            }
        }
        std::fs::write(&path, toml::to_string_pretty(&config)?)?;
    }

    if !req.image.is_empty() {
        let path = root.join("configs/image.toml");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
        let mut image: toml::Value =
            toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
        let root_t = image
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("image.toml root not a table"))?;
        for (section, vendors) in &req.image {
            if vendors.is_empty() {
                continue;
            }
            let section_t = root_t
                .entry(section.clone())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let providers = section_t
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("image.toml [{}] not a table", section))?
                .entry("providers".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let prov_t = providers
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("providers not a table"))?;
            for (vendor, new_key) in vendors {
                if new_key.is_empty() {
                    continue;
                }
                let entry = prov_t
                    .entry(vendor.clone())
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
                if let Some(t) = entry.as_table_mut() {
                    t.insert("api_key".to_string(), toml::Value::String(new_key.clone()));
                }
            }
        }
        std::fs::write(&path, toml::to_string_pretty(&image)?)?;
    }

    if !req.audio.is_empty() {
        let path = root.join("configs/audio.toml");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
        let mut audio: toml::Value =
            toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
        let root_t = audio
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("audio.toml root not a table"))?;
        for (section, vendors) in &req.audio {
            if vendors.is_empty() {
                continue;
            }
            let section_t = root_t
                .entry(section.clone())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let providers = section_t
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("audio.toml [{}] not a table", section))?
                .entry("providers".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let prov_t = providers
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("providers not a table"))?;
            for (vendor, new_key) in vendors {
                if new_key.is_empty() {
                    continue;
                }
                let entry = prov_t
                    .entry(vendor.clone())
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
                if let Some(t) = entry.as_table_mut() {
                    t.insert("api_key".to_string(), toml::Value::String(new_key.clone()));
                }
            }
        }
        std::fs::write(&path, toml::to_string_pretty(&audio)?)?;
    }

    if !req.video.is_empty() {
        let path = root.join("configs/video.toml");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
        let mut video: toml::Value =
            toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
        let root_t = video
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("video_toml_root_not_table"))?;
        for (section, vendors) in &req.video {
            if vendors.is_empty() {
                continue;
            }
            let section_t = root_t
                .entry(section.clone())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let providers = section_t
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("video_toml_section_not_table"))?
                .entry("providers".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let prov_t = providers
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("providers not a table"))?;
            for (vendor, new_key) in vendors {
                if new_key.is_empty() {
                    continue;
                }
                let entry = prov_t
                    .entry(vendor.clone())
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
                if let Some(t) = entry.as_table_mut() {
                    t.insert("api_key".to_string(), toml::Value::String(new_key.clone()));
                }
            }
        }
        std::fs::write(&path, toml::to_string_pretty(&video)?)?;
    }

    if !req.music.is_empty() {
        let path = root.join("configs/music.toml");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
        let mut music: toml::Value =
            toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
        let root_t = music
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("music_toml_root_not_table"))?;
        for (section, vendors) in &req.music {
            if vendors.is_empty() {
                continue;
            }
            let section_t = root_t
                .entry(section.clone())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let providers = section_t
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("music_toml_section_not_table"))?
                .entry("providers".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let prov_t = providers
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("providers not a table"))?;
            for (vendor, new_key) in vendors {
                if new_key.is_empty() {
                    continue;
                }
                let entry = prov_t
                    .entry(vendor.clone())
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
                if let Some(t) = entry.as_table_mut() {
                    t.insert("api_key".to_string(), toml::Value::String(new_key.clone()));
                }
            }
        }
        std::fs::write(&path, toml::to_string_pretty(&music)?)?;
    }

    Ok(())
}

async fn get_model_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<ModelConfigResponse>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return (
            resp.0,
            Json(ApiResponse {
                ok: resp.1.ok,
                data: None,
                error: resp.1.error.clone(),
            }),
        );
    }
    match read_model_config(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read model config failed: {err}")),
            }),
        ),
    }
}

async fn update_model_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ModelConfigUpdateRequest>,
) -> (StatusCode, Json<ApiResponse<ModelConfigResponse>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return (
            resp.0,
            Json(ApiResponse {
                ok: resp.1.ok,
                data: None,
                error: resp.1.error.clone(),
            }),
        );
    }
    if let Err(err) = write_model_config(&state, &req) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write model config failed: {err}")),
            }),
        );
    }
    match read_model_config(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: None,
                error: Some(format!("saved but re-read failed: {err}")),
            }),
        ),
    }
}

async fn get_provider_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<ProviderKeysResponse>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return (
            resp.0,
            Json(ApiResponse {
                ok: resp.1.ok,
                data: None,
                error: resp.1.error.clone(),
            }),
        );
    }
    match read_provider_keys(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read provider keys failed: {err}")),
            }),
        ),
    }
}

async fn update_provider_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ProviderKeysResponse>,
) -> (StatusCode, Json<ApiResponse<ProviderKeysResponse>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return (
            resp.0,
            Json(ApiResponse {
                ok: resp.1.ok,
                data: None,
                error: resp.1.error.clone(),
            }),
        );
    }
    if let Err(err) = write_provider_keys(&state, &req) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write provider keys failed: {err}")),
            }),
        );
    }
    match read_provider_keys(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: None,
                error: Some(format!("saved but re-read failed: {err}")),
            }),
        ),
    }
}

async fn restart_clawd(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can restart RustClaw".to_string()),
            }),
        );
    }

    match schedule_binary_restart_with_start_all(&state) {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "message": "restart triggered; start-all-bin.sh will restart RustClaw in a few seconds",
                    "restart_triggered": true,
                    "script": "start-all-bin.sh",
                    "log": "logs/restart-system.log"
                })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(err),
            }),
        ),
    }
}

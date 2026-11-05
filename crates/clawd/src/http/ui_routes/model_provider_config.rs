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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capability_family: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    input_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    output_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    available_models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_window_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    async_job_supported: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shared_quota_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shared_quota_note_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model_list_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capability_source: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_enabled: Option<bool>,
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

impl ModelConfigUpdateRequest {
    fn module_updates(&self) -> [(usize, &'static str, Option<&ModelConfigItem>); 7] {
        [
            (1, "image_edit", self.image_edit.as_ref()),
            (1, "image_generation", self.image_generation.as_ref()),
            (1, "image_vision", self.image_vision.as_ref()),
            (2, "audio_transcribe", self.audio_transcribe.as_ref()),
            (2, "audio_synthesize", self.audio_synthesize.as_ref()),
            (3, "video_generation", self.video_generation.as_ref()),
            (4, "music_generation", self.music_generation.as_ref()),
        ]
    }
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
        capability_family: None,
        input_modalities: Vec::new(),
        output_modalities: Vec::new(),
        available_models: Vec::new(),
        context_window_tokens: None,
        async_job_supported: None,
        shared_quota_group: None,
        shared_quota_note_key: None,
        model_list_source: None,
        capability_source: None,
        risk_level: None,
        dry_run_supported: None,
        external_provider: None,
        provider_supported: None,
        unsupported_reason: None,
        runtime_enabled: None,
    }
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
    let available_models = read_section_model_cache(table, &vendor);

    let item = ModelConfigItem {
        vendor,
        model,
        base_url,
        api_key: None,
        api_key_configured: Some(api_key.is_some()),
        api_key_masked: api_key.map(mask_secret),
        capabilities: Vec::new(),
        capability_family: None,
        input_modalities: Vec::new(),
        output_modalities: Vec::new(),
        available_models,
        context_window_tokens: read_context_window_tokens(table),
        async_job_supported: None,
        shared_quota_group: None,
        shared_quota_note_key: None,
        model_list_source: None,
        capability_source: None,
        risk_level: None,
        dry_run_supported: None,
        external_provider: None,
        provider_supported: None,
        unsupported_reason: None,
        runtime_enabled: None,
    };
    let mut item = model_item_with_capability_metadata(item, section);
    if section == "audio_transcribe" {
        item.runtime_enabled = Some(audio_transcription_runtime_enabled(table, &item));
    }
    item
}

fn audio_transcription_runtime_enabled(
    table: &toml::map::Map<String, toml::Value>,
    item: &ModelConfigItem,
) -> bool {
    if item.vendor == "custom" && item.model == "local-whisper" {
        let configured = table
            .get("local_server_enabled")
            .and_then(toml::Value::as_bool)
            .unwrap_or(false);
        return local_whisper_effectively_enabled(
            configured,
            std::env::var("APP_LOCAL_WHISPER_ENABLED").ok().as_deref(),
        );
    }

    item.provider_supported == Some(true)
}

fn local_whisper_effectively_enabled(configured: bool, runtime_override: Option<&str>) -> bool {
    match runtime_override.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("auto") => configured,
        Some("1" | "true" | "yes") => true,
        Some("0" | "false" | "no") => false,
        Some(_) => false,
    }
}

fn read_section_model_cache(
    table: &toml::map::Map<String, toml::Value>,
    vendor: &str,
) -> Vec<String> {
    let generic_models = table
        .get("models")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let vendor_models = if vendor.trim().is_empty() {
        Vec::new()
    } else {
        table
            .get(&format!("{}_models", vendor.trim()))
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    };
    let mut models = if vendor_models.is_empty() {
        generic_models
    } else {
        vendor_models
    };
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

fn read_context_window_tokens(table: &toml::map::Map<String, toml::Value>) -> Option<usize> {
    table
        .get("context_window_tokens")
        .and_then(toml::Value::as_integer)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn read_llm_context_window_tokens(config: &toml::Value, vendor: &str) -> Option<usize> {
    config
        .get("llm")
        .and_then(toml::Value::as_table)
        .and_then(|llm| llm.get(vendor.trim()))
        .and_then(toml::Value::as_table)
        .and_then(read_context_window_tokens)
}

fn model_item_with_capability_metadata(
    mut item: ModelConfigItem,
    section: &str,
) -> ModelConfigItem {
    let metadata = model_capability_metadata(section);
    if let Some(capability) = metadata.capability {
        item.capabilities = vec![capability.to_string()];
    }
    item.capability_family = Some(metadata.family.to_string());
    item.input_modalities = metadata
        .input_modalities
        .iter()
        .map(|value| value.to_string())
        .collect();
    item.output_modalities = metadata
        .output_modalities
        .iter()
        .map(|value| value.to_string())
        .collect();
    item.async_job_supported = Some(metadata.async_job_supported);
    if item.shared_quota_group.is_none() && metadata.shared_quota_by_provider {
        let vendor = item.vendor.trim();
        if !vendor.is_empty() {
            item.shared_quota_group = Some(format!("provider_account:{vendor}"));
            item.shared_quota_note_key = Some("provider_account_shared_quota".to_string());
        }
    }
    item.model_list_source = Some(if item.available_models.is_empty() {
        "not_configured".to_string()
    } else {
        "static_config".to_string()
    });
    item.capability_source = Some("static_metadata".to_string());
    item.risk_level = Some(metadata.risk_level.to_string());
    item.dry_run_supported = Some(metadata.dry_run_supported);
    item.external_provider = Some(metadata.external_provider);
    let (provider_supported, unsupported_reason) = provider_support_status(&item);
    item.provider_supported = provider_supported;
    item.unsupported_reason = unsupported_reason;
    item
}

#[derive(Debug, Clone, Copy)]
struct ModelCapabilityMetadata {
    capability: Option<&'static str>,
    family: &'static str,
    input_modalities: &'static [&'static str],
    output_modalities: &'static [&'static str],
    risk_level: &'static str,
    dry_run_supported: bool,
    external_provider: bool,
    async_job_supported: bool,
    shared_quota_by_provider: bool,
}

fn model_capability_metadata(section: &str) -> ModelCapabilityMetadata {
    match section.trim() {
        "llm" => ModelCapabilityMetadata {
            capability: Some("text.chat"),
            family: "text",
            input_modalities: &["text"],
            output_modalities: &["text"],
            risk_level: "medium",
            dry_run_supported: false,
            external_provider: true,
            async_job_supported: false,
            shared_quota_by_provider: true,
        },
        "image_edit" => ModelCapabilityMetadata {
            capability: Some("image.edit"),
            family: "image",
            input_modalities: &["text", "image"],
            output_modalities: &["image"],
            risk_level: "high",
            dry_run_supported: false,
            external_provider: true,
            async_job_supported: false,
            shared_quota_by_provider: true,
        },
        "image_generation" => ModelCapabilityMetadata {
            capability: Some("image.generate"),
            family: "image",
            input_modalities: &["text"],
            output_modalities: &["image"],
            risk_level: "high",
            dry_run_supported: true,
            external_provider: true,
            async_job_supported: true,
            shared_quota_by_provider: true,
        },
        "image_vision" => ModelCapabilityMetadata {
            capability: Some("image.understand"),
            family: "image",
            input_modalities: &["image", "text"],
            output_modalities: &["text"],
            risk_level: "medium",
            dry_run_supported: false,
            external_provider: true,
            async_job_supported: false,
            shared_quota_by_provider: true,
        },
        "audio_transcribe" => ModelCapabilityMetadata {
            capability: Some("audio.transcribe"),
            family: "audio",
            input_modalities: &["audio"],
            output_modalities: &["text"],
            risk_level: "medium",
            dry_run_supported: false,
            external_provider: true,
            async_job_supported: false,
            shared_quota_by_provider: true,
        },
        "audio_synthesize" => ModelCapabilityMetadata {
            capability: Some("audio.synthesize"),
            family: "audio",
            input_modalities: &["text"],
            output_modalities: &["audio"],
            risk_level: "high",
            dry_run_supported: true,
            external_provider: true,
            async_job_supported: true,
            shared_quota_by_provider: true,
        },
        "video_generation" => ModelCapabilityMetadata {
            capability: Some("video.generate"),
            family: "video",
            input_modalities: &["text", "image", "video"],
            output_modalities: &["video"],
            risk_level: "high",
            dry_run_supported: true,
            external_provider: true,
            async_job_supported: true,
            shared_quota_by_provider: true,
        },
        "music_generation" => ModelCapabilityMetadata {
            capability: Some("music.generate"),
            family: "music",
            input_modalities: &["text", "audio"],
            output_modalities: &["audio", "music"],
            risk_level: "high",
            dry_run_supported: true,
            external_provider: true,
            async_job_supported: true,
            shared_quota_by_provider: true,
        },
        _ => ModelCapabilityMetadata {
            capability: None,
            family: "unknown",
            input_modalities: &[],
            output_modalities: &[],
            risk_level: "unknown",
            dry_run_supported: false,
            external_provider: false,
            async_job_supported: false,
            shared_quota_by_provider: false,
        },
    }
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
        update_model_provider_key(provider, api_key);
    }
    Ok(())
}

fn read_model_config(state: &AppState) -> anyhow::Result<ModelConfigResponse> {
    let root = &state.skill_rt.workspace_root;

    let files = load_model_settings_files(root, false)?;
    let config = &files[0].value;
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
            let context_window_tokens = read_llm_context_window_tokens(config, &vendor);
            model_item_with_capability_metadata(
                ModelConfigItem {
                    available_models: read_llm_model_cache(config, &vendor),
                    vendor,
                    model,
                    base_url: None,
                    api_key: None,
                    api_key_configured: None,
                    api_key_masked: None,
                    capabilities: Vec::new(),
                    capability_family: None,
                    input_modalities: Vec::new(),
                    output_modalities: Vec::new(),
                    context_window_tokens,
                    async_job_supported: None,
                    shared_quota_group: None,
                    shared_quota_note_key: None,
                    model_list_source: None,
                    capability_source: None,
                    risk_level: None,
                    dry_run_supported: None,
                    external_provider: None,
                    provider_supported: None,
                    unsupported_reason: None,
                    runtime_enabled: None,
                },
                "llm",
            )
        })
        .unwrap_or_else(|| model_item_with_capability_metadata(default_model_item(), "llm"));

    let image = &files[1].value;
    let image_edit = read_model_section(image, "image_edit");
    let image_generation = read_model_section(image, "image_generation");
    let image_vision = read_model_section(image, "image_vision");

    let audio = &files[2].value;
    let audio_transcribe = read_model_section(audio, "audio_transcribe");
    let audio_synthesize = read_model_section(audio, "audio_synthesize");

    let video = &files[3].value;
    let video_generation = read_model_section(video, "video_generation");

    let music = &files[4].value;
    let music_generation = read_model_section(music, "music_generation");

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
    let files = load_model_settings_files(&state.skill_rt.workspace_root, false)?;
    Ok(ProviderKeysResponse {
        llm: read_llm_provider_keys(&files[0].value),
        image: read_image_provider_keys(&files[1].value),
        audio: read_audio_provider_keys(&files[2].value),
        video: read_module_provider_keys(&files[3].value, &["video_generation"]),
        music: read_module_provider_keys(&files[4].value, &["music_generation"]),
    })
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
                error: Some("only admin can restart the agent runtime".to_string()),
            }),
        );
    }

    match schedule_binary_restart_with_start_all(&state) {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "message": "restart triggered; start-all-bin.sh will restart the agent runtime in a few seconds",
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

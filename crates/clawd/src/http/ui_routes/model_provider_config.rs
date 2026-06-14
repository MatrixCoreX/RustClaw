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
}

#[derive(Debug, Serialize)]
struct ModelConfigResponse {
    llm: ModelConfigItem,
    image_edit: ModelConfigItem,
    image_generation: ModelConfigItem,
    image_vision: ModelConfigItem,
    audio_transcribe: ModelConfigItem,
    audio_synthesize: ModelConfigItem,
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
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProviderKeysResponse {
    #[serde(default)]
    llm: HashMap<String, String>,
    #[serde(default)]
    image: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    audio: HashMap<String, HashMap<String, String>>,
}

fn default_model_item() -> ModelConfigItem {
    ModelConfigItem {
        vendor: String::new(),
        model: String::new(),
    }
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
        .map(|t| ModelConfigItem {
            vendor: t
                .get("selected_vendor")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            model: t
                .get("selected_model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .unwrap_or_else(default_model_item);

    let image_path = root.join("configs/image.toml");
    let image_raw = std::fs::read_to_string(&image_path).unwrap_or_else(|_| String::new());
    let image: toml::Value =
        toml::from_str(&image_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    let read_image_section = |section: &str| -> ModelConfigItem {
        image
            .get(section)
            .and_then(|t| t.as_table())
            .map(|t| ModelConfigItem {
                vendor: t
                    .get("default_vendor")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                model: t
                    .get("default_model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
            .unwrap_or_else(default_model_item)
    };
    let image_edit = read_image_section("image_edit");
    let image_generation = read_image_section("image_generation");
    let image_vision = read_image_section("image_vision");

    let audio_path = root.join("configs/audio.toml");
    let audio_raw = std::fs::read_to_string(&audio_path).unwrap_or_else(|_| String::new());
    let audio: toml::Value =
        toml::from_str(&audio_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    let audio_transcribe = audio
        .get("audio_transcribe")
        .and_then(|t| t.as_table())
        .map(|t| ModelConfigItem {
            vendor: t
                .get("default_vendor")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            model: t
                .get("default_model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .unwrap_or_else(default_model_item);

    let audio_synthesize = audio
        .get("audio_synthesize")
        .and_then(|t| t.as_table())
        .map(|t| ModelConfigItem {
            vendor: t
                .get("default_vendor")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            model: t
                .get("default_model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .unwrap_or_else(default_model_item);

    Ok(ModelConfigResponse {
        llm,
        image_edit,
        image_generation,
        image_vision,
        audio_transcribe,
        audio_synthesize,
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
            let tbl = image
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("image.toml root is not a table"))?
                .entry(section.to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if let Some(t) = tbl.as_table_mut() {
                t.insert(
                    "default_vendor".to_string(),
                    toml::Value::String(it.vendor.clone()),
                );
                t.insert(
                    "default_model".to_string(),
                    toml::Value::String(it.model.clone()),
                );
            }
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

    if let Some(ref it) = req.audio_transcribe {
        audio_modified = true;
        let tbl = audio
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("audio.toml root is not a table"))?
            .entry("audio_transcribe".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if let Some(t) = tbl.as_table_mut() {
            t.insert(
                "default_vendor".to_string(),
                toml::Value::String(it.vendor.clone()),
            );
            t.insert(
                "default_model".to_string(),
                toml::Value::String(it.model.clone()),
            );
        }
    }
    if let Some(ref it) = req.audio_synthesize {
        audio_modified = true;
        let tbl = audio
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("audio.toml root is not a table"))?
            .entry("audio_synthesize".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if let Some(t) = tbl.as_table_mut() {
            t.insert(
                "default_vendor".to_string(),
                toml::Value::String(it.vendor.clone()),
            );
            t.insert(
                "default_model".to_string(),
                toml::Value::String(it.model.clone()),
            );
        }
    }
    if audio_modified {
        std::fs::write(&audio_path, toml::to_string_pretty(&audio)?)?;
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

fn read_image_provider_keys(image: &toml::Value) -> HashMap<String, HashMap<String, String>> {
    let mut out = HashMap::new();
    for section in ["image_edit", "image_generation", "image_vision"] {
        let mut vendors = HashMap::new();
        if let Some(providers) = image
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
        out.insert(section.to_string(), vendors);
    }
    out
}

fn read_audio_provider_keys(audio: &toml::Value) -> HashMap<String, HashMap<String, String>> {
    let mut out = HashMap::new();
    for section in ["audio_synthesize", "audio_transcribe"] {
        let mut vendors = HashMap::new();
        if let Some(providers) = audio
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
        out.insert(section.to_string(), vendors);
    }
    out
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

    Ok(ProviderKeysResponse {
        llm,
        image: image_keys,
        audio: audio_keys,
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

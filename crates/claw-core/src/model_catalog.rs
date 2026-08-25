use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelCatalogError {
    #[error("model_catalog_read_failed:{path}:{source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("model_catalog_parse_failed:{path}:{source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCatalog {
    pub schema_version: u32,
    pub selected_provider: String,
    pub selected_model: String,
    pub entries: Vec<ModelCatalogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCatalogEntry {
    pub schema_version: u32,
    pub provider: String,
    pub model: String,
    pub models: Vec<String>,
    pub api_style: String,
    pub base_url_kind: String,
    pub context_window_tokens: Option<usize>,
    pub timeout_seconds: Option<u64>,
    pub credential_state: String,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    pub supports_text: bool,
    pub supports_image_input: bool,
    pub supports_video_input: bool,
    pub supports_audio_input: bool,
    pub supports_image_understanding: bool,
    pub supports_audio_transcription: bool,
    pub supports_image_generation: bool,
    pub supports_image_edit: bool,
    pub supports_audio_generation: bool,
    pub supports_video_generation: bool,
    pub supports_music_generation: bool,
    pub async_required: bool,
    pub dry_run_supported: bool,
    pub active_text_provider: bool,
    pub config_source: Vec<String>,
    #[serde(default)]
    pub capability_source: Vec<String>,
}

#[derive(Debug, Clone)]
struct CatalogInputs {
    config: toml::Value,
    image: toml::Value,
    audio: toml::Value,
    video: toml::Value,
    music: toml::Value,
    env_values: BTreeMap<String, String>,
}

pub fn build_model_catalog_from_workspace(
    workspace_root: impl AsRef<Path>,
) -> Result<ModelCatalog, ModelCatalogError> {
    let root = workspace_root.as_ref();
    let inputs = CatalogInputs {
        config: read_required_toml(&root.join("configs/config.toml"))?,
        image: read_optional_toml(&root.join("configs/image.toml"))?,
        audio: read_optional_toml(&root.join("configs/audio.toml"))?,
        video: read_optional_toml(&root.join("configs/video.toml"))?,
        music: read_optional_toml(&root.join("configs/music.toml"))?,
        env_values: read_runtime_env_values(root),
    };
    Ok(build_model_catalog(&inputs))
}

fn build_model_catalog(inputs: &CatalogInputs) -> ModelCatalog {
    let llm = inputs.config.get("llm").and_then(toml::Value::as_table);
    let selected_provider = llm
        .and_then(|table| table.get("selected_vendor"))
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let selected_model = llm
        .and_then(|table| table.get("selected_model"))
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();

    let mut provider_tables = BTreeMap::new();
    if let Some(llm) = llm {
        for (provider, value) in llm {
            let Some(table) = value.as_table() else {
                continue;
            };
            if table.get("model").and_then(toml::Value::as_str).is_some() {
                let model = string_field(table, "model");
                if !model.is_empty() {
                    provider_tables.insert((provider.trim().to_string(), model), table.clone());
                }
            }
        }
    }
    collect_module_targets(inputs, &mut provider_tables);

    let entries = provider_tables
        .into_iter()
        .map(|((provider, _), table)| {
            catalog_entry(
                &provider,
                &table,
                inputs,
                &selected_provider,
                &selected_model,
            )
        })
        .collect();

    ModelCatalog {
        schema_version: 2,
        selected_provider,
        selected_model,
        entries,
    }
}

fn collect_module_targets(
    inputs: &CatalogInputs,
    targets: &mut BTreeMap<(String, String), toml::map::Map<String, toml::Value>>,
) {
    for (config, section_name) in [
        (&inputs.image, "image_edit"),
        (&inputs.image, "image_generation"),
        (&inputs.image, "image_vision"),
        (&inputs.audio, "audio_synthesize"),
        (&inputs.audio, "audio_transcribe"),
        (&inputs.video, "video_generation"),
        (&inputs.music, "music_generation"),
    ] {
        let Some(section) = section(config, section_name) else {
            continue;
        };
        let provider = string_field(section, "default_vendor");
        let model = string_field(section, "default_model");
        if provider.is_empty() || model.is_empty() {
            continue;
        }
        let mut target = inputs
            .config
            .get("llm")
            .and_then(toml::Value::as_table)
            .and_then(|llm| llm.get(&provider))
            .and_then(toml::Value::as_table)
            .cloned()
            .unwrap_or_default();
        if let Some(provider_table) = section
            .get("providers")
            .and_then(toml::Value::as_table)
            .and_then(|providers| providers.get(&provider))
            .and_then(toml::Value::as_table)
        {
            for (key, value) in provider_table {
                if !value.as_str().is_some_and(|value| value.trim().is_empty()) {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
        target.insert("model".to_string(), toml::Value::String(model.clone()));
        let models = provider_models(Some(section), &provider);
        if !models.is_empty() {
            target.insert(
                "models".to_string(),
                toml::Value::Array(models.into_iter().map(toml::Value::String).collect()),
            );
        }
        targets.insert((provider, model), target);
    }
}

fn catalog_entry(
    provider: &str,
    llm_table: &toml::map::Map<String, toml::Value>,
    inputs: &CatalogInputs,
    selected_provider: &str,
    selected_model: &str,
) -> ModelCatalogEntry {
    let model = string_field(llm_table, "model");
    let models = string_list_field(llm_table, "models");
    let image_vision_models = provider_models(section(&inputs.image, "image_vision"), provider);
    let image_generation_models =
        provider_models(section(&inputs.image, "image_generation"), provider);
    let image_edit_models = provider_models(section(&inputs.image, "image_edit"), provider);
    let audio_transcribe_models =
        provider_models(section(&inputs.audio, "audio_transcribe"), provider);
    let audio_synthesize_models =
        provider_models(section(&inputs.audio, "audio_synthesize"), provider);
    let video_generation_models =
        provider_models(section(&inputs.video, "video_generation"), provider);
    let music_generation_models =
        provider_models(section(&inputs.music, "music_generation"), provider);

    let text_table = inputs
        .config
        .get("llm")
        .and_then(toml::Value::as_table)
        .and_then(|llm| llm.get(provider))
        .and_then(toml::Value::as_table);
    let supports_text = text_table.is_some_and(|table| {
        string_field(table, "model") == model
            || contains_model(&string_list_field(table, "models"), &model)
    });
    let supports_image_understanding = contains_model(&image_vision_models, &model);
    let supports_audio_transcription = contains_model(&audio_transcribe_models, &model);
    let supports_image_generation = contains_model(&image_generation_models, &model);
    let supports_image_edit = contains_model(&image_edit_models, &model);
    let supports_audio_generation = contains_model(&audio_synthesize_models, &model);
    let supports_video_generation = contains_model(&video_generation_models, &model);
    let supports_music_generation = contains_model(&music_generation_models, &model);
    let (input_modalities, output_modalities) = model_modalities(
        provider,
        &model,
        text_table,
        supports_text,
        supports_image_understanding,
        supports_audio_transcription,
        supports_image_generation,
        supports_image_edit,
        supports_audio_generation,
        supports_video_generation,
        supports_music_generation,
    );
    let supports_image_input = has_modality(&input_modalities, "image");
    let supports_video_input = has_modality(&input_modalities, "video");
    let supports_audio_input = has_modality(&input_modalities, "audio");
    let async_required = supports_image_generation
        || supports_image_edit
        || supports_audio_generation
        || supports_video_generation
        || supports_music_generation;

    ModelCatalogEntry {
        schema_version: 2,
        provider: provider.to_string(),
        model: model.clone(),
        models,
        api_style: api_style_token(llm_table.get("api_format").and_then(toml::Value::as_str)),
        base_url_kind: base_url_kind(&string_field(llm_table, "base_url")),
        context_window_tokens: usize_field(llm_table, "context_window_tokens"),
        timeout_seconds: u64_field(llm_table, "timeout_seconds"),
        credential_state: credential_state(
            llm_table,
            provider,
            &model,
            &inputs.config,
            &inputs.env_values,
        ),
        input_modalities,
        output_modalities,
        supports_text,
        supports_image_input,
        supports_video_input,
        supports_audio_input,
        supports_image_understanding,
        supports_audio_transcription,
        supports_image_generation,
        supports_image_edit,
        supports_audio_generation,
        supports_video_generation,
        supports_music_generation,
        async_required,
        dry_run_supported: async_required,
        active_text_provider: provider == selected_provider && model == selected_model,
        config_source: vec![
            "configs/config.toml".to_string(),
            "configs/image.toml".to_string(),
            "configs/audio.toml".to_string(),
            "configs/video.toml".to_string(),
            "configs/music.toml".to_string(),
            format!("prompts/layers/vendor_patches/{provider}"),
        ],
        capability_source: capability_sources(provider, &model),
    }
}

#[allow(clippy::too_many_arguments)]
fn model_modalities(
    provider: &str,
    model: &str,
    text_table: Option<&toml::map::Map<String, toml::Value>>,
    supports_text: bool,
    supports_image_understanding: bool,
    supports_audio_transcription: bool,
    supports_image_generation: bool,
    supports_image_edit: bool,
    supports_audio_generation: bool,
    supports_video_generation: bool,
    supports_music_generation: bool,
) -> (Vec<String>, Vec<String>) {
    let mut input = Vec::new();
    let mut output = Vec::new();
    if supports_text {
        push_unique(&mut input, "text");
        push_unique(&mut output, "text");
        let is_active_table_model =
            text_table.is_some_and(|table| string_field(table, "model") == model);
        if is_active_table_model {
            for modality in text_table
                .and_then(|table| configured_modalities(table, "input_modalities"))
                .unwrap_or_default()
            {
                push_unique(&mut input, &modality);
            }
            for modality in text_table
                .and_then(|table| configured_modalities(table, "output_modalities"))
                .unwrap_or_default()
            {
                push_unique(&mut output, &modality);
            }
        }
    }
    if provider == "minimax" && model == "MiniMax-M3" {
        for modality in ["text", "image", "video"] {
            push_unique(&mut input, modality);
        }
        push_unique(&mut output, "text");
    }
    if supports_image_understanding {
        push_unique(&mut input, "image");
        push_unique(&mut input, "text");
        push_unique(&mut output, "text");
    }
    if supports_audio_transcription {
        push_unique(&mut input, "audio");
        push_unique(&mut output, "text");
    }
    if supports_image_generation {
        push_unique(&mut input, "text");
        push_unique(&mut output, "image");
    }
    if supports_image_edit {
        push_unique(&mut input, "text");
        push_unique(&mut input, "image");
        push_unique(&mut output, "image");
    }
    if supports_audio_generation {
        push_unique(&mut input, "text");
        push_unique(&mut output, "audio");
    }
    if supports_video_generation {
        push_unique(&mut input, "text");
        push_unique(&mut input, "image");
        push_unique(&mut output, "video");
    }
    if supports_music_generation {
        push_unique(&mut input, "text");
        if model.contains("cover") {
            push_unique(&mut input, "audio");
        }
        push_unique(&mut output, "audio");
        push_unique(&mut output, "music");
    }
    (input, output)
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|current| current == value) {
        values.push(value.to_string());
    }
}

fn capability_sources(provider: &str, model: &str) -> Vec<String> {
    let source = match (provider, model) {
        ("minimax", "MiniMax-M3") => {
            Some("https://platform.minimaxi.com/docs/guides/text-generation")
        }
        ("minimax", model) if model.starts_with("image-") => {
            Some("https://platform.minimaxi.com/docs/guides/image-generation")
        }
        ("minimax", model) if model.starts_with("speech-") => {
            Some("https://platform.minimaxi.com/docs/api-reference/api-overview")
        }
        ("minimax", "MiniMax-H3") => {
            Some("https://platform.minimaxi.com/docs/guides/video-generation")
        }
        ("minimax", model) if model.starts_with("MiniMax-Hailuo-") => {
            Some("https://platform.minimaxi.com/docs/api-reference/api-overview")
        }
        ("minimax", model) if model.starts_with("music-") => {
            Some("https://platform.minimaxi.com/docs/guides/music-generation")
        }
        ("custom", "local-whisper") => Some("https://github.com/ggerganov/whisper.cpp"),
        _ => None,
    };
    source
        .map(|source| vec![source.to_string()])
        .unwrap_or_else(|| vec!["runtime_config".to_string()])
}

fn read_required_toml(path: &Path) -> Result<toml::Value, ModelCatalogError> {
    let raw = std::fs::read_to_string(path).map_err(|source| ModelCatalogError::Read {
        path: display_path(path),
        source,
    })?;
    toml::from_str(&raw).map_err(|source| ModelCatalogError::Parse {
        path: display_path(path),
        source,
    })
}

fn read_optional_toml(path: &Path) -> Result<toml::Value, ModelCatalogError> {
    match std::fs::read_to_string(path) {
        Ok(raw) => toml::from_str(&raw).map_err(|source| ModelCatalogError::Parse {
            path: display_path(path),
            source,
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(toml::Value::Table(toml::map::Map::new()))
        }
        Err(source) => Err(ModelCatalogError::Read {
            path: display_path(path),
            source,
        }),
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn section<'a>(
    value: &'a toml::Value,
    section_name: &str,
) -> Option<&'a toml::map::Map<String, toml::Value>> {
    value.get(section_name).and_then(toml::Value::as_table)
}

fn provider_models(
    section: Option<&toml::map::Map<String, toml::Value>>,
    provider: &str,
) -> Vec<String> {
    let Some(section) = section else {
        return Vec::new();
    };
    string_list_field(section, &format!("{provider}_models"))
}

fn contains_model(models: &[String], model: &str) -> bool {
    !model.trim().is_empty() && models.iter().any(|candidate| candidate == model)
}

fn configured_modalities(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
) -> Option<Vec<String>> {
    let values = string_list_field(table, key)
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .fold(Vec::new(), |mut acc, value| {
            if !acc.iter().any(|existing| existing == &value) {
                acc.push(value);
            }
            acc
        });
    (!values.is_empty()).then_some(values)
}

fn has_modality(modalities: &[String], target: &str) -> bool {
    modalities.iter().any(|value| value == target)
}

fn string_field(table: &toml::map::Map<String, toml::Value>, key: &str) -> String {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn string_list_field(table: &toml::map::Map<String, toml::Value>, key: &str) -> Vec<String> {
    table
        .get(key)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn usize_field(table: &toml::map::Map<String, toml::Value>, key: &str) -> Option<usize> {
    table
        .get(key)
        .and_then(toml::Value::as_integer)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn u64_field(table: &toml::map::Map<String, toml::Value>, key: &str) -> Option<u64> {
    table
        .get(key)
        .and_then(toml::Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn credential_state(
    table: &toml::map::Map<String, toml::Value>,
    provider: &str,
    model: &str,
    config: &toml::Value,
    env_values: &BTreeMap<String, String>,
) -> String {
    let base_url = string_field(table, "base_url");
    if base_url.starts_with("http://127.0.0.1")
        || base_url.starts_with("http://localhost")
        || base_url.starts_with("http://[::1]")
    {
        return "not_required_local".to_string();
    }
    if matches_hosted_relay(config, provider, model, &base_url) {
        return "device_enrollment".to_string();
    }
    if !string_field(table, "api_key").is_empty() {
        return "configured_inline".to_string();
    }
    if provider_credential_env_vars(provider).iter().any(|name| {
        std::env::var(name).is_ok_and(|value| !value.trim().is_empty())
            || env_values
                .get(*name)
                .is_some_and(|value| !value.trim().is_empty())
    }) {
        return "configured_env".to_string();
    }
    "missing".to_string()
}

fn matches_hosted_relay(config: &toml::Value, provider: &str, model: &str, base_url: &str) -> bool {
    let Some(relay) = config
        .get("llm")
        .and_then(toml::Value::as_table)
        .and_then(|llm| llm.get("hosted_relay"))
        .and_then(toml::Value::as_table)
    else {
        return false;
    };
    relay
        .get("enabled")
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
        && string_field(relay, "vendor") == provider
        && string_field(relay, "model") == model
        && string_field(relay, "base_url") == base_url
        && base_url.starts_with("https://")
}

fn read_runtime_env_values(workspace_root: &Path) -> BTreeMap<String, String> {
    runtime_env_file_candidates(workspace_root)
        .into_iter()
        .find_map(|path| {
            let raw = std::fs::read_to_string(path).ok()?;
            Some(parse_runtime_env_file(&raw))
        })
        .unwrap_or_default()
}

fn runtime_env_file_candidates(workspace_root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(path) = std::env::var("CHINESE_PROVIDER_ENV_FILE") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }
    candidates.push(workspace_root.join("../runtime_env_filled.sh"));
    candidates.push(PathBuf::from("/home/guagua/runtime_env_filled.sh"));
    candidates
}

fn parse_runtime_env_file(raw: &str) -> BTreeMap<String, String> {
    raw.lines()
        .filter_map(parse_runtime_env_line)
        .collect::<BTreeMap<_, _>>()
}

fn parse_runtime_env_line(raw: &str) -> Option<(String, String)> {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line).trim();
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }
    Some((key.to_string(), unquote_env_value(value.trim()).to_string()))
}

fn unquote_env_value(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let first = bytes[0];
        let last = bytes[value.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn provider_credential_env_vars(provider: &str) -> &'static [&'static str] {
    match provider {
        "anthropic" => &["ANTHROPIC_API_KEY"],
        "custom" => &["CUSTOM_API_KEY"],
        "deepseek" => &["DEEPSEEK_API_KEY"],
        "google" => &["GOOGLE_API_KEY"],
        "grok" => &["GROK_API_KEY"],
        "minimax" => &["MINIMAX_API_KEY"],
        "mimo" => &["MIMO_API_KEY", "XIAOMI_API_KEY"],
        "openai" => &["OPENAI_API_KEY"],
        "qwen" => &["QWEN_API_KEY", "DASHSCOPE_API_KEY"],
        _ => &[],
    }
}

fn api_style_token(raw: Option<&str>) -> String {
    match raw.unwrap_or_default().trim() {
        "" | "openai_compat" | "openai_compatible" => "openai_compatible",
        "anthropic_claude" | "anthropic_messages" => "anthropic_messages",
        "google_gemini" | "gemini" => "google_gemini",
        _ => "custom_or_unknown",
    }
    .to_string()
}

fn base_url_kind(base_url: &str) -> String {
    let token = if base_url.contains("api.minimaxi.com") {
        "minimax_official_openai_compat"
    } else if base_url.starts_with("http://127.0.0.1")
        || base_url.starts_with("http://localhost")
        || base_url.starts_with("http://[::1]")
    {
        "local_loopback"
    } else if base_url.contains("xiaomimimo.com") {
        "mimo_token_plan_openai_compat"
    } else if base_url.contains("dashscope.aliyuncs.com/compatible-mode") {
        "qwen_dashscope_openai_compat"
    } else if base_url.contains("api.deepseek.com") {
        "deepseek_official_openai_compat"
    } else if base_url.contains("api.openai.com") {
        "openai_official"
    } else if base_url.contains("generativelanguage.googleapis.com") {
        "google_gemini_official"
    } else if base_url.contains("api.anthropic.com") {
        "anthropic_official"
    } else if base_url.contains("api.x.ai") {
        "grok_official"
    } else if base_url.contains("dashscope.aliyuncs.com/api/v1") {
        "qwen_dashscope_native"
    } else {
        "custom_or_unknown"
    };
    token.to_string()
}

#[cfg(test)]
#[path = "model_catalog_tests.rs"]
mod tests;

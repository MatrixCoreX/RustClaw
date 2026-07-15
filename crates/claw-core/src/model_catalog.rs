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
    pub supports_text: bool,
    pub supports_image_input: bool,
    pub supports_video_input: bool,
    pub supports_audio_input: bool,
    pub supports_image_understanding: bool,
    pub supports_audio_transcription: bool,
    pub supports_image_generation: bool,
    pub supports_audio_generation: bool,
    pub supports_video_generation: bool,
    pub supports_music_generation: bool,
    pub async_required: bool,
    pub dry_run_supported: bool,
    pub active_text_provider: bool,
    pub config_source: Vec<String>,
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
                provider_tables.insert(provider.trim().to_string(), table);
            }
        }
    }

    let entries = provider_tables
        .into_iter()
        .map(|(provider, table)| {
            catalog_entry(
                &provider,
                table,
                inputs,
                &selected_provider,
                &selected_model,
            )
        })
        .collect();

    ModelCatalog {
        schema_version: 1,
        selected_provider,
        selected_model,
        entries,
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

    let supports_image_input = contains_model(&image_vision_models, &model);
    let supports_video_input = provider == "minimax" && model == "MiniMax-M3";
    let supports_audio_input = contains_model(&audio_transcribe_models, &model);
    let supports_image_understanding = !image_vision_models.is_empty();
    let supports_audio_transcription = !audio_transcribe_models.is_empty();
    let supports_image_generation =
        !image_generation_models.is_empty() || !image_edit_models.is_empty();
    let supports_audio_generation = !audio_synthesize_models.is_empty();
    let supports_video_generation = !video_generation_models.is_empty();
    let supports_music_generation = !music_generation_models.is_empty();
    let async_required = supports_image_generation
        || supports_audio_generation
        || supports_video_generation
        || supports_music_generation;

    ModelCatalogEntry {
        schema_version: 1,
        provider: provider.to_string(),
        model: model.clone(),
        models,
        api_style: api_style_token(llm_table.get("api_format").and_then(toml::Value::as_str)),
        base_url_kind: base_url_kind(&string_field(llm_table, "base_url")),
        context_window_tokens: usize_field(llm_table, "context_window_tokens"),
        timeout_seconds: u64_field(llm_table, "timeout_seconds"),
        credential_state: credential_state(llm_table, provider, &inputs.env_values),
        supports_text: true,
        supports_image_input,
        supports_video_input,
        supports_audio_input,
        supports_image_understanding,
        supports_audio_transcription,
        supports_image_generation,
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
    }
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
    env_values: &BTreeMap<String, String>,
) -> String {
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

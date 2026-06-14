use std::path::Path;

use serde::Deserialize;

use super::{
    clamp01, parse_provider, MapProvider, DEFAULT_KEYWORD, DEFAULT_PROVIDER, MAX_FETCH_CANDIDATES,
    MAX_RADIUS_METERS, MAX_TOP_K, MIN_RADIUS_METERS,
};

#[derive(Debug, Deserialize, Default)]
struct RootConfig {
    #[serde(default)]
    map_merchant: MapMerchantConfig,
}

#[derive(Debug, Clone, Deserialize)]
struct MapMerchantConfig {
    #[serde(default = "default_provider_string")]
    default_provider: String,
    #[serde(default = "default_radius_meters")]
    default_radius_meters: u32,
    #[serde(default = "default_top_k")]
    default_top_k: usize,
    #[serde(default = "default_fetch_candidates")]
    max_fetch_candidates: usize,
    #[serde(default = "default_sort_by")]
    default_sort_by: String,
    #[serde(default = "default_distance_weight")]
    distance_weight: f64,
    #[serde(default = "default_rating_weight")]
    rating_weight: f64,
    #[serde(default = "default_price_weight")]
    price_weight: f64,
    #[serde(default = "default_keyword_weight")]
    keyword_weight: f64,
    #[serde(default)]
    default_keyword: Option<String>,
    #[serde(default)]
    amap: ProviderConfig,
    #[serde(default)]
    google: ProviderConfig,
}

impl Default for MapMerchantConfig {
    fn default() -> Self {
        Self {
            default_provider: default_provider_string(),
            default_radius_meters: default_radius_meters(),
            default_top_k: default_top_k(),
            max_fetch_candidates: default_fetch_candidates(),
            default_sort_by: default_sort_by(),
            distance_weight: default_distance_weight(),
            rating_weight: default_rating_weight(),
            price_weight: default_price_weight(),
            keyword_weight: default_keyword_weight(),
            default_keyword: None,
            amap: ProviderConfig::default(),
            google: ProviderConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ProviderConfig {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct RuntimeConfig {
    pub(super) default_provider: MapProvider,
    pub(super) default_radius_meters: u32,
    pub(super) default_top_k: usize,
    pub(super) max_fetch_candidates: usize,
    pub(super) default_sort_by: String,
    pub(super) distance_weight: f64,
    pub(super) rating_weight: f64,
    pub(super) price_weight: f64,
    pub(super) keyword_weight: f64,
    pub(super) default_keyword: String,
    pub(super) amap: ProviderRuntime,
    pub(super) google: ProviderRuntime,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ProviderRuntime {
    pub(super) enabled: bool,
    pub(super) api_key: String,
}

fn load_config(workspace_root: &Path) -> RootConfig {
    let path = workspace_root.join("configs/map_merchant.toml");
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return RootConfig::default(),
    };
    toml::from_str::<RootConfig>(&raw).unwrap_or_default()
}

pub(super) fn resolve_runtime_config(workspace_root: &Path) -> RuntimeConfig {
    let cfg = load_config(workspace_root).map_merchant;
    RuntimeConfig {
        default_provider: parse_provider(Some(cfg.default_provider.as_str()))
            .unwrap_or(MapProvider::Amap),
        default_radius_meters: cfg
            .default_radius_meters
            .clamp(MIN_RADIUS_METERS, MAX_RADIUS_METERS),
        default_top_k: cfg.default_top_k.clamp(1, MAX_TOP_K),
        max_fetch_candidates: cfg.max_fetch_candidates.clamp(1, MAX_FETCH_CANDIDATES),
        default_sort_by: cfg.default_sort_by,
        distance_weight: clamp01(cfg.distance_weight),
        rating_weight: clamp01(cfg.rating_weight),
        price_weight: clamp01(cfg.price_weight),
        keyword_weight: clamp01(cfg.keyword_weight),
        default_keyword: cfg
            .default_keyword
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(DEFAULT_KEYWORD)
            .to_string(),
        amap: ProviderRuntime {
            enabled: cfg.amap.enabled,
            api_key: std::env::var("AMAP_API_KEY")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .or_else(|| {
                    cfg.amap
                        .api_key
                        .as_deref()
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or_default(),
        },
        google: ProviderRuntime {
            enabled: cfg.google.enabled,
            api_key: std::env::var("GOOGLE_MAPS_API_KEY")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .or_else(|| {
                    std::env::var("GOOGLE_PLACES_API_KEY")
                        .ok()
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                })
                .or_else(|| {
                    cfg.google
                        .api_key
                        .as_deref()
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or_default(),
        },
    }
}

fn default_true() -> bool {
    true
}

fn default_provider_string() -> String {
    DEFAULT_PROVIDER.to_string()
}

fn default_radius_meters() -> u32 {
    3000
}

fn default_top_k() -> usize {
    5
}

fn default_fetch_candidates() -> usize {
    12
}

fn default_sort_by() -> String {
    "balanced".to_string()
}

fn default_distance_weight() -> f64 {
    0.40
}

fn default_rating_weight() -> f64 {
    0.30
}

fn default_price_weight() -> f64 {
    0.15
}

fn default_keyword_weight() -> f64 {
    0.15
}

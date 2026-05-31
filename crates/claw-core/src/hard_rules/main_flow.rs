use serde::Deserialize;
use tracing::warn;

use crate::hard_rules::loader::read_toml_text;
use crate::hard_rules::types::MainFlowRules;

#[derive(Debug, Clone, Default, Deserialize)]
struct MainFlowRulesToml {
    #[serde(default)]
    whatsapp: WhatsappSection,
    #[serde(default)]
    trade_preview: TradePreviewSection,
    #[serde(default)]
    duplicate_affirmation: DuplicateAffirmationSection,
    #[serde(default)]
    runtime_channel: RuntimeChannelSection,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct WhatsappSection {
    #[serde(default)]
    web_adapters: Vec<String>,
    #[serde(default)]
    cloud_adapters: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TradePreviewSection {
    line_prefix: Option<String>,
    default_order_type: Option<String>,
    recent_window_secs: Option<i64>,
    recent_scan_limit: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DuplicateAffirmationSection {
    window_secs: Option<i64>,
    scan_limit: Option<i64>,
    #[serde(default)]
    statuses: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeChannelSection {
    #[serde(default)]
    whatsapp_aliases: Vec<String>,
}

fn normalize_list(items: Vec<String>) -> Vec<String> {
    items
        .into_iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn load_main_flow_rules(path: &str) -> MainFlowRules {
    let mut merged = MainFlowRules::defaults();
    let raw = match read_toml_text(path) {
        Ok(raw) => raw,
        Err(err) => {
            warn!(
                "hard_rules.main_flow read failed path={} error={}",
                path, err
            );
            return merged;
        }
    };
    let parsed = match toml::from_str::<MainFlowRulesToml>(&raw) {
        Ok(parsed) => parsed,
        Err(err) => {
            warn!(
                "hard_rules.main_flow parse failed path={} error={}",
                path, err
            );
            return merged;
        }
    };

    let web_adapters = normalize_list(parsed.whatsapp.web_adapters);
    if !web_adapters.is_empty() {
        merged.whatsapp_web_adapters = web_adapters;
    }
    let cloud_adapters = normalize_list(parsed.whatsapp.cloud_adapters);
    if !cloud_adapters.is_empty() {
        merged.whatsapp_cloud_adapters = cloud_adapters;
    }

    if let Some(prefix) = parsed.trade_preview.line_prefix {
        let v = prefix.trim().to_string();
        if !v.is_empty() {
            merged.trade_preview_line_prefix = v;
        }
    }
    if let Some(order_type) = parsed.trade_preview.default_order_type {
        let v = order_type.trim().to_ascii_lowercase();
        if !v.is_empty() {
            merged.trade_preview_default_order_type = v;
        }
    }
    if let Some(v) = parsed.trade_preview.recent_window_secs {
        if v >= 1 {
            merged.recent_trade_preview_window_secs = v;
        }
    }
    if let Some(v) = parsed.trade_preview.recent_scan_limit {
        if let Ok(parsed_limit) = usize::try_from(v) {
            if parsed_limit >= 1 {
                merged.recent_trade_preview_scan_limit = parsed_limit;
            }
        }
    }

    if let Some(v) = parsed.duplicate_affirmation.window_secs {
        if v >= 1 {
            merged.duplicate_affirmation_window_secs = v;
        }
    }
    if let Some(v) = parsed.duplicate_affirmation.scan_limit {
        if let Ok(parsed_limit) = usize::try_from(v) {
            if parsed_limit >= 1 {
                merged.duplicate_affirmation_scan_limit = parsed_limit;
            }
        }
    }
    let statuses = normalize_list(parsed.duplicate_affirmation.statuses);
    if !statuses.is_empty() {
        merged.duplicate_affirmation_statuses = statuses;
    }

    let whatsapp_aliases = normalize_list(parsed.runtime_channel.whatsapp_aliases);
    if !whatsapp_aliases.is_empty() {
        merged.runtime_whatsapp_channel_aliases = whatsapp_aliases;
    }

    merged
}

#[cfg(test)]
#[path = "main_flow_tests.rs"]
mod tests;

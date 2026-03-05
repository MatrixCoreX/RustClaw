use serde::Deserialize;

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
    crypto_price_alert: CryptoPriceAlertSection,
    #[serde(default)]
    runtime_channel: RuntimeChannelSection,
    #[serde(default)]
    classifier: ClassifierSection,
    #[serde(default)]
    resume: ResumeSection,
    #[serde(default)]
    task_status: TaskStatusSection,
    #[serde(default)]
    context: ContextSection,
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
struct CryptoPriceAlertSection {
    primary_action: Option<String>,
    #[serde(default)]
    actions: Vec<String>,
    #[serde(default)]
    fallback_actions: Vec<String>,
    #[serde(default)]
    unsupported_error_keywords: Vec<String>,
    triggered_tag: Option<String>,
    not_triggered_tag: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RuntimeChannelSection {
    #[serde(default)]
    whatsapp_aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ClassifierSection {
    #[serde(default)]
    direct_sources: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ResumeSection {
    #[serde(default)]
    continue_sources: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TaskStatusSection {
    queued: Option<String>,
    running: Option<String>,
    succeeded: Option<String>,
    failed: Option<String>,
    canceled: Option<String>,
    timeout: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ContextSection {
    low_confidence_threshold: Option<f64>,
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
    let Some(raw) = read_toml_text(path) else {
        return merged;
    };
    let Ok(parsed) = toml::from_str::<MainFlowRulesToml>(&raw) else {
        return merged;
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

    if let Some(primary) = parsed.crypto_price_alert.primary_action {
        let v = primary.trim().to_ascii_lowercase();
        if !v.is_empty() {
            merged.crypto_price_alert_primary_action = v;
        }
    }
    let actions = normalize_list(parsed.crypto_price_alert.actions);
    if !actions.is_empty() {
        merged.crypto_price_alert_actions = actions;
    }
    let fallback_actions = normalize_list(parsed.crypto_price_alert.fallback_actions);
    if !fallback_actions.is_empty() {
        merged.crypto_price_alert_fallback_actions = fallback_actions;
    }
    let unsupported_keywords = normalize_list(parsed.crypto_price_alert.unsupported_error_keywords);
    if !unsupported_keywords.is_empty() {
        merged.crypto_unsupported_error_keywords = unsupported_keywords;
    }
    if let Some(tag) = parsed.crypto_price_alert.triggered_tag {
        let v = tag.trim().to_string();
        if !v.is_empty() {
            merged.crypto_price_alert_triggered_tag = v;
        }
    }
    if let Some(tag) = parsed.crypto_price_alert.not_triggered_tag {
        let v = tag.trim().to_string();
        if !v.is_empty() {
            merged.crypto_price_alert_not_triggered_tag = v;
        }
    }
    let whatsapp_aliases = normalize_list(parsed.runtime_channel.whatsapp_aliases);
    if !whatsapp_aliases.is_empty() {
        merged.runtime_whatsapp_channel_aliases = whatsapp_aliases;
    }
    let classifier_sources = normalize_list(parsed.classifier.direct_sources);
    if !classifier_sources.is_empty() {
        merged.classifier_direct_sources = classifier_sources;
    }
    let resume_sources = normalize_list(parsed.resume.continue_sources);
    if !resume_sources.is_empty() {
        merged.resume_continue_sources = resume_sources;
    }
    if let Some(v) = parsed.task_status.queued {
        let s = v.trim().to_ascii_lowercase();
        if !s.is_empty() {
            merged.task_status_queued = s;
        }
    }
    if let Some(v) = parsed.task_status.running {
        let s = v.trim().to_ascii_lowercase();
        if !s.is_empty() {
            merged.task_status_running = s;
        }
    }
    if let Some(v) = parsed.task_status.succeeded {
        let s = v.trim().to_ascii_lowercase();
        if !s.is_empty() {
            merged.task_status_succeeded = s;
        }
    }
    if let Some(v) = parsed.task_status.failed {
        let s = v.trim().to_ascii_lowercase();
        if !s.is_empty() {
            merged.task_status_failed = s;
        }
    }
    if let Some(v) = parsed.task_status.canceled {
        let s = v.trim().to_ascii_lowercase();
        if !s.is_empty() {
            merged.task_status_canceled = s;
        }
    }
    if let Some(v) = parsed.task_status.timeout {
        let s = v.trim().to_ascii_lowercase();
        if !s.is_empty() {
            merged.task_status_timeout = s;
        }
    }
    if let Some(v) = parsed.context.low_confidence_threshold {
        if v.is_finite() {
            merged.context_low_confidence_threshold = v.clamp(0.0, 1.0);
        }
    }

    merged
}

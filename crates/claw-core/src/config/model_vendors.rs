pub fn llm_vendor_api_key_env_names(vendor: &str) -> &'static [&'static str] {
    match vendor.trim().to_ascii_lowercase().as_str() {
        "openai" => &["OPENAI_API_KEY"],
        "google" => &["GOOGLE_API_KEY"],
        "anthropic" => &["ANTHROPIC_API_KEY"],
        "grok" => &["GROK_API_KEY"],
        "deepseek" => &["DEEPSEEK_API_KEY"],
        "qwen" => &["QWEN_API_KEY"],
        "minimax" => &["MINIMAX_API_KEY"],
        "mimo" => &["XIAOMI_API_KEY", "MIMO_API_KEY"],
        "custom" => &["CUSTOM_API_KEY"],
        _ => &[],
    }
}

/// Ordinary text-provider deployment fallback. Keep alias precedence and
/// placeholder rejection identical for loading, settings and metadata reads.
pub fn llm_vendor_api_key_from_env_with<F>(vendor: &str, env_value: &F) -> String
where
    F: Fn(&str) -> Option<String>,
{
    llm_vendor_api_key_env_names(vendor)
        .iter()
        .filter_map(|name| env_value(name))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && !value.starts_with("REPLACE_ME"))
        .last()
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct MainFlowRules {
    pub whatsapp_web_adapters: Vec<String>,
    pub whatsapp_cloud_adapters: Vec<String>,
    pub trade_preview_line_prefix: String,
    pub trade_preview_default_order_type: String,
    pub recent_trade_preview_window_secs: i64,
    pub recent_trade_preview_scan_limit: usize,
    pub duplicate_affirmation_window_secs: i64,
    pub duplicate_affirmation_scan_limit: usize,
    pub duplicate_affirmation_statuses: Vec<String>,
    pub runtime_whatsapp_channel_aliases: Vec<String>,
}

impl MainFlowRules {
    pub fn defaults() -> Self {
        Self {
            whatsapp_web_adapters: vec!["whatsapp_web".to_string(), "wa_web".to_string()],
            whatsapp_cloud_adapters: vec!["whatsapp_cloud".to_string(), "wa_cloud".to_string()],
            trade_preview_line_prefix: "trade_preview ".to_string(),
            trade_preview_default_order_type: "market".to_string(),
            recent_trade_preview_window_secs: 600,
            recent_trade_preview_scan_limit: 24,
            duplicate_affirmation_window_secs: 120,
            duplicate_affirmation_scan_limit: 30,
            duplicate_affirmation_statuses: vec![
                "queued".to_string(),
                "running".to_string(),
                "succeeded".to_string(),
            ],
            runtime_whatsapp_channel_aliases: vec!["whatsapp".to_string()],
        }
    }
}

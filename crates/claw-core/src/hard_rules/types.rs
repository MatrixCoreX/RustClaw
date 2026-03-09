use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct VoiceModeIntentAliases {
    pub voice: Vec<String>,
    pub text: Vec<String>,
    pub both: Vec<String>,
    pub reset: Vec<String>,
    pub show: Vec<String>,
    pub none: Vec<String>,
}

impl VoiceModeIntentAliases {
    pub fn defaults() -> Self {
        Self {
            voice: vec![
                "voice-only".to_string(),
                "voice only".to_string(),
                "only voice".to_string(),
                "切到语音".to_string(),
                "语音回复".to_string(),
                "只用语音".to_string(),
                "仅语音".to_string(),
            ],
            text: vec![
                "text-only".to_string(),
                "text only".to_string(),
                "only text".to_string(),
                "切回文字".to_string(),
                "文字回复".to_string(),
                "只要文字".to_string(),
                "仅文字".to_string(),
                "只用文字".to_string(),
                "只打字".to_string(),
            ],
            both: vec![
                "both".to_string(),
                "voice and text".to_string(),
                "text and voice".to_string(),
                "语音和文字都要".to_string(),
                "语音和文本都发".to_string(),
                "两种都回复".to_string(),
            ],
            reset: vec![
                "reset".to_string(),
                "default mode".to_string(),
                "恢复默认".to_string(),
                "重置".to_string(),
            ],
            show: vec![
                "show".to_string(),
                "status".to_string(),
                "current mode".to_string(),
                "查看语音模式".to_string(),
                "当前是语音还是文字".to_string(),
            ],
            none: vec![
                "none".to_string(),
                "not a mode".to_string(),
                "no mode switch".to_string(),
                "不是模式切换".to_string(),
                "非模式切换".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub struct TradeRules {
    pub intent_keywords: Vec<String>,
    pub buy_keywords: Vec<String>,
    pub sell_keywords: Vec<String>,
    pub limit_keywords: Vec<String>,
    pub exchange_aliases: HashMap<String, Vec<String>>,
    pub default_exchange: String,
    pub qty_patterns: Vec<String>,
    pub price_patterns: Vec<String>,
    pub confirm_yes: Vec<String>,
    pub confirm_no: Vec<String>,
}

impl TradeRules {
    pub fn defaults() -> Self {
        let mut exchange_aliases = HashMap::new();
        exchange_aliases.insert("okx".to_string(), vec!["okx".to_string(), "欧易".to_string()]);
        exchange_aliases.insert(
            "binance".to_string(),
            vec!["binance".to_string(), "币安".to_string()],
        );

        Self {
            intent_keywords: vec![
                "下单".to_string(),
                "买入".to_string(),
                "卖出".to_string(),
                "开仓".to_string(),
                "平仓".to_string(),
                "交易".to_string(),
                "buy".to_string(),
                "sell".to_string(),
                "order".to_string(),
                "submit".to_string(),
            ],
            buy_keywords: vec![
                "buy".to_string(),
                "买入".to_string(),
                "买".to_string(),
                "开仓".to_string(),
            ],
            sell_keywords: vec![
                "sell".to_string(),
                "卖出".to_string(),
                "卖".to_string(),
                "平仓".to_string(),
            ],
            limit_keywords: vec!["limit".to_string(), "限价".to_string()],
            exchange_aliases,
            default_exchange: "binance".to_string(),
            qty_patterns: vec![
                r"(?i)(?:qty|数量|买入|买|卖出|卖)\s*[:=]?\s*([0-9]+(?:\.[0-9]+)?)".to_string(),
            ],
            price_patterns: vec![
                r"(?i)(?:price|px|价格|限价)\s*[:=]?\s*([0-9]+(?:\.[0-9]+)?)".to_string(),
            ],
            confirm_yes: vec![
                "yes".to_string(),
                "y".to_string(),
                "ok".to_string(),
                "确认".to_string(),
                "是".to_string(),
                "好的".to_string(),
                "同意".to_string(),
            ],
            confirm_no: vec![
                "no".to_string(),
                "n".to_string(),
                "取消".to_string(),
                "否".to_string(),
                "不用了".to_string(),
                "不".to_string(),
            ],
        }
    }
}

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
    pub crypto_price_alert_primary_action: String,
    pub crypto_price_alert_actions: Vec<String>,
    pub crypto_price_alert_fallback_actions: Vec<String>,
    pub crypto_unsupported_error_keywords: Vec<String>,
    pub crypto_price_alert_triggered_tag: String,
    pub crypto_price_alert_not_triggered_tag: String,
    pub runtime_whatsapp_channel_aliases: Vec<String>,
    pub classifier_direct_sources: Vec<String>,
    pub resume_continue_sources: Vec<String>,
    pub task_status_queued: String,
    pub task_status_running: String,
    pub task_status_succeeded: String,
    pub task_status_failed: String,
    pub task_status_canceled: String,
    pub task_status_timeout: String,
    pub context_low_confidence_threshold: f64,
    pub assistant_name_extract_markers: Vec<String>,
    pub assistant_name_invalid_values: Vec<String>,
    pub explicit_summary_markers: Vec<String>,
    pub summary_like_response_markers: Vec<String>,
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
            crypto_price_alert_primary_action: "price_alert_check".to_string(),
            crypto_price_alert_actions: vec![
                "price_alert_check".to_string(),
                "price_monitor".to_string(),
                "monitor_price".to_string(),
                "price_alert".to_string(),
                "volatility_alert".to_string(),
            ],
            crypto_price_alert_fallback_actions: vec![
                "price_monitor".to_string(),
                "monitor_price".to_string(),
                "price_alert".to_string(),
                "volatility_alert".to_string(),
            ],
            crypto_unsupported_error_keywords: vec![
                "unsupported action".to_string(),
                "不支持".to_string(),
            ],
            crypto_price_alert_triggered_tag: "[PRICE_ALERT_TRIGGERED]".to_string(),
            crypto_price_alert_not_triggered_tag: "[PRICE_ALERT_NOT_TRIGGERED]".to_string(),
            runtime_whatsapp_channel_aliases: vec!["whatsapp".to_string()],
            classifier_direct_sources: vec![
                "voice_mode_intent_detect".to_string(),
                "voice_mode_intent_detect_regression".to_string(),
            ],
            resume_continue_sources: vec!["resume_continue_execute".to_string()],
            task_status_queued: "queued".to_string(),
            task_status_running: "running".to_string(),
            task_status_succeeded: "succeeded".to_string(),
            task_status_failed: "failed".to_string(),
            task_status_canceled: "canceled".to_string(),
            task_status_timeout: "timeout".to_string(),
            context_low_confidence_threshold: 0.6,
            assistant_name_extract_markers: vec![
                "记住你的名字叫".to_string(),
                "记住你名字叫".to_string(),
                "记住你叫".to_string(),
                "以后叫你".to_string(),
                "以后我叫你".to_string(),
                "我给你取名叫".to_string(),
                "我给你起名叫".to_string(),
                "你的名字叫".to_string(),
                "你叫".to_string(),
                "call you ".to_string(),
                "your name is ".to_string(),
            ],
            assistant_name_invalid_values: vec![
                "executor".to_string(),
                "assistant".to_string(),
                "agent".to_string(),
                "系统".to_string(),
                "身份".to_string(),
                "formal identity".to_string(),
            ],
            explicit_summary_markers: vec![
                "总结".to_string(),
                "总结一下".to_string(),
                "汇总".to_string(),
                "概括".to_string(),
                "概述".to_string(),
                "小结".to_string(),
                "summary".to_string(),
                "summarize".to_string(),
                "recap".to_string(),
                "wrap up".to_string(),
            ],
            summary_like_response_markers: vec![
                "总结如下".to_string(),
                "汇总如下".to_string(),
                "概括如下".to_string(),
                "概述如下".to_string(),
                "处理结果".to_string(),
                "执行结果".to_string(),
                "完成情况".to_string(),
                "已完成".to_string(),
                "结果如下".to_string(),
                "summary:".to_string(),
                "summary".to_string(),
                "recap".to_string(),
            ],
        }
    }
}

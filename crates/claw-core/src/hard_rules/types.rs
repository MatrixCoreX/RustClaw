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

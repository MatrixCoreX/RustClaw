use std::collections::HashMap;

use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use crate::hard_rules::loader::read_toml_text;
use crate::hard_rules::types::TradeRules;

#[derive(Debug, Clone)]
pub struct CompiledTradeRules {
    pub rules: TradeRules,
    qty_patterns: Vec<Regex>,
    price_patterns: Vec<Regex>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TradeRulesToml {
    #[serde(default)]
    intent: IntentSection,
    #[serde(default)]
    side: SideSection,
    #[serde(default)]
    order_type: OrderTypeSection,
    #[serde(default)]
    exchange: ExchangeSection,
    #[serde(default)]
    confirm: ConfirmSection,
    #[serde(default)]
    regex: RegexSection,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct IntentSection {
    #[serde(default)]
    keywords: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct SideSection {
    #[serde(default)]
    buy_keywords: Vec<String>,
    #[serde(default)]
    sell_keywords: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OrderTypeSection {
    #[serde(default)]
    limit_keywords: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ExchangeSection {
    default: Option<String>,
    #[serde(default)]
    aliases: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ConfirmSection {
    #[serde(default)]
    yes: Vec<String>,
    #[serde(default)]
    no: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RegexSection {
    #[serde(default)]
    qty_patterns: Vec<String>,
    #[serde(default)]
    price_patterns: Vec<String>,
}

impl CompiledTradeRules {
    pub fn from_rules(rules: TradeRules) -> Self {
        let mut qty_patterns = rules
            .qty_patterns
            .iter()
            .filter_map(|p| Regex::new(p).ok())
            .collect::<Vec<_>>();
        let mut price_patterns = rules
            .price_patterns
            .iter()
            .filter_map(|p| Regex::new(p).ok())
            .collect::<Vec<_>>();

        if qty_patterns.is_empty() {
            qty_patterns = TradeRules::defaults()
                .qty_patterns
                .iter()
                .filter_map(|p| Regex::new(p).ok())
                .collect();
        }
        if price_patterns.is_empty() {
            price_patterns = TradeRules::defaults()
                .price_patterns
                .iter()
                .filter_map(|p| Regex::new(p).ok())
                .collect();
        }

        Self {
            rules,
            qty_patterns,
            price_patterns,
        }
    }
}

pub fn load_compiled_trade_rules(path: &str) -> CompiledTradeRules {
    let defaults = TradeRules::defaults();
    let Some(raw) = read_toml_text(path) else {
        return CompiledTradeRules::from_rules(defaults);
    };
    let Ok(parsed) = toml::from_str::<TradeRulesToml>(&raw) else {
        return CompiledTradeRules::from_rules(defaults);
    };

    let mut merged = defaults.clone();
    if !parsed.intent.keywords.is_empty() {
        merged.intent_keywords = normalize_list(parsed.intent.keywords);
    }
    if !parsed.side.buy_keywords.is_empty() {
        merged.buy_keywords = normalize_list(parsed.side.buy_keywords);
    }
    if !parsed.side.sell_keywords.is_empty() {
        merged.sell_keywords = normalize_list(parsed.side.sell_keywords);
    }
    if !parsed.order_type.limit_keywords.is_empty() {
        merged.limit_keywords = normalize_list(parsed.order_type.limit_keywords);
    }
    if let Some(default_exchange) = parsed.exchange.default {
        let v = default_exchange.trim().to_ascii_lowercase();
        if !v.is_empty() {
            merged.default_exchange = v;
        }
    }
    if !parsed.exchange.aliases.is_empty() {
        let mut aliases = HashMap::new();
        for (exchange, words) in parsed.exchange.aliases {
            let key = exchange.trim().to_ascii_lowercase();
            if key.is_empty() {
                continue;
            }
            let normalized = normalize_list(words);
            if !normalized.is_empty() {
                aliases.insert(key, normalized);
            }
        }
        if !aliases.is_empty() {
            merged.exchange_aliases = aliases;
        }
    }
    if !parsed.confirm.yes.is_empty() {
        merged.confirm_yes = normalize_list(parsed.confirm.yes);
    }
    if !parsed.confirm.no.is_empty() {
        merged.confirm_no = normalize_list(parsed.confirm.no);
    }
    if !parsed.regex.qty_patterns.is_empty() {
        merged.qty_patterns = parsed
            .regex
            .qty_patterns
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if !parsed.regex.price_patterns.is_empty() {
        merged.price_patterns = parsed
            .regex
            .price_patterns
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    CompiledTradeRules::from_rules(merged)
}

fn normalize_list(items: Vec<String>) -> Vec<String> {
    items
        .into_iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn contains_trade_intent(text: &str, rules: &CompiledTradeRules) -> bool {
    let t = text.to_ascii_lowercase();
    rules.rules.intent_keywords.iter().any(|k| t.contains(k))
}

pub fn detect_trade_side(text: &str, rules: &CompiledTradeRules) -> Option<&'static str> {
    let t = text.to_ascii_lowercase();
    if rules.rules.sell_keywords.iter().any(|k| t.contains(k)) {
        return Some("sell");
    }
    if rules.rules.buy_keywords.iter().any(|k| t.contains(k)) {
        return Some("buy");
    }
    None
}

pub fn detect_order_type(text: &str, rules: &CompiledTradeRules) -> &'static str {
    let t = text.to_ascii_lowercase();
    if rules.rules.limit_keywords.iter().any(|k| t.contains(k)) {
        "limit"
    } else {
        "market"
    }
}

pub fn detect_trade_exchange(text: &str, rules: &CompiledTradeRules) -> String {
    let t = text.to_ascii_lowercase();
    for (exchange, aliases) in &rules.rules.exchange_aliases {
        if aliases.iter().any(|k| t.contains(k)) {
            return exchange.clone();
        }
    }
    rules.rules.default_exchange.clone()
}

pub fn extract_trade_symbol(text: &str) -> Option<String> {
    let upper = text.to_ascii_uppercase().replace('-', "").replace('/', "");
    let re = Regex::new(r"([A-Z]{2,10}(USDT|USD))").ok()?;
    re.captures(&upper)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

pub fn extract_trade_qty(
    text: &str,
    symbol: &str,
    order_type: &str,
    rules: &CompiledTradeRules,
) -> Option<f64> {
    for re in &rules.qty_patterns {
        if let Some(c) = re.captures(text) {
            if let Some(m) = c.get(1) {
                if let Ok(v) = m.as_str().parse::<f64>() {
                    return Some(v);
                }
            }
        }
    }
    let by_symbol = Regex::new(&format!(
        r"(?i){}\s*[:=]?\s*([0-9]+(?:\.[0-9]+)?)",
        regex::escape(symbol)
    ))
    .ok()?;
    if let Some(c) = by_symbol.captures(text) {
        if let Some(m) = c.get(1) {
            return m.as_str().parse::<f64>().ok();
        }
    }
    if order_type == "market" {
        let generic = Regex::new(r"([0-9]+(?:\.[0-9]+)?)").ok()?;
        let mut last_num: Option<f64> = None;
        for cap in generic.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                if let Ok(v) = m.as_str().parse::<f64>() {
                    last_num = Some(v);
                }
            }
        }
        return last_num;
    }
    None
}

pub fn extract_trade_price(text: &str, rules: &CompiledTradeRules) -> Option<f64> {
    for re in &rules.price_patterns {
        if let Some(c) = re.captures(text) {
            if let Some(m) = c.get(1) {
                if let Ok(v) = m.as_str().parse::<f64>() {
                    return Some(v);
                }
            }
        }
    }
    None
}

pub fn is_yes_confirmation(text: &str, _rules: &CompiledTradeRules) -> bool {
    let t = text.trim().to_ascii_lowercase();
    // Hardcoded confirmation policy: only Y / YES is accepted.
    matches!(t.as_str(), "y" | "yes")
}

pub fn is_no_confirmation(text: &str, rules: &CompiledTradeRules) -> bool {
    let t = text.trim().to_ascii_lowercase();
    rules.rules.confirm_no.iter().any(|w| w == &t)
}

pub fn parse_trade_preview_submit_args(
    text: &str,
    rules: &CompiledTradeRules,
) -> Option<(Value, Value)> {
    if !contains_trade_intent(text, rules) {
        return None;
    }
    let side = detect_trade_side(text, rules)?;
    let symbol = extract_trade_symbol(text)?;
    let order_type = detect_order_type(text, rules);
    let qty = extract_trade_qty(text, &symbol, order_type, rules)?;
    if qty <= 0.0 {
        return None;
    }
    let price = if order_type == "limit" {
        extract_trade_price(text, rules)?
    } else {
        0.0
    };
    let exchange = detect_trade_exchange(text, rules);

    let mut base = serde_json::Map::new();
    base.insert("exchange".to_string(), Value::String(exchange));
    base.insert("symbol".to_string(), Value::String(symbol));
    base.insert("side".to_string(), Value::String(side.to_string()));
    base.insert(
        "order_type".to_string(),
        Value::String(order_type.to_string()),
    );
    base.insert("qty".to_string(), Value::from(qty));
    if order_type == "limit" {
        base.insert("price".to_string(), Value::from(price));
    }

    let mut preview = base.clone();
    preview.insert(
        "action".to_string(),
        Value::String("trade_preview".to_string()),
    );
    let mut submit = base;
    submit.insert(
        "action".to_string(),
        Value::String("trade_submit".to_string()),
    );
    submit.insert("confirm".to_string(), Value::Bool(true));
    Some((Value::Object(preview), Value::Object(submit)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_market_buy_request() {
        let rules = CompiledTradeRules::from_rules(TradeRules::defaults());
        let parsed = parse_trade_preview_submit_args("帮我买入 BTCUSDT 10", &rules).unwrap();
        let preview = parsed.0.as_object().unwrap();
        assert_eq!(
            preview.get("action").and_then(|v| v.as_str()),
            Some("trade_preview")
        );
        assert_eq!(preview.get("side").and_then(|v| v.as_str()), Some("buy"));
        assert_eq!(preview.get("qty").and_then(|v| v.as_f64()), Some(10.0));
    }

    #[test]
    fn parse_limit_requires_price() {
        let rules = CompiledTradeRules::from_rules(TradeRules::defaults());
        assert!(parse_trade_preview_submit_args("买入 BTCUSDT 数量 1 限价", &rules).is_none());
        assert!(
            parse_trade_preview_submit_args("买入 BTCUSDT 数量 1 限价 价格 1000", &rules).is_some()
        );
    }
}

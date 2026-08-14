//! 股票技能：查询 A 股和美股实时行情（单行 JSON stdin -> 单行 JSON stdout）

#![recursion_limit = "256"]

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use encoding_rs::GBK;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const STOCK_ALIAS_CHOICE_SCHEMA_RAW: &str =
    include_str!("../../../prompts/schemas/stock_alias_choice.schema.json");

static STOCK_ALIAS_CHOICE_SCHEMA: OnceLock<Value> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct CoreConfig {
    #[serde(default)]
    llm: LlmConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LlmConfig {
    #[serde(default)]
    selected_vendor: Option<String>,
    #[serde(default)]
    openai: Option<VendorConfig>,
    #[serde(default)]
    qwen: Option<VendorConfig>,
    #[serde(default)]
    deepseek: Option<VendorConfig>,
    #[serde(default)]
    grok: Option<VendorConfig>,
    #[serde(default)]
    minimax: Option<VendorConfig>,
    #[serde(default)]
    custom: Option<VendorConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct VendorConfig {
    base_url: String,
    api_key: String,
    model: String,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct StockConfigFile {
    #[serde(default)]
    stock: StockSkillConfig,
}

#[derive(Debug, Clone, Deserialize)]
struct StockSkillConfig {
    #[serde(default = "default_true")]
    enable_name_lookup: bool,
    #[serde(default = "default_true")]
    enable_llm_name_correction: bool,
    #[serde(default)]
    llm_vendor: Option<String>,
    #[serde(default)]
    llm_model: Option<String>,
    #[serde(default)]
    llm_timeout_seconds: Option<u64>,
    #[serde(default = "default_max_llm_candidates")]
    max_llm_candidates: usize,
    #[serde(default)]
    aliases: HashMap<String, String>,
    #[serde(default)]
    cleanup_tokens: Vec<String>,
}

impl Default for StockSkillConfig {
    fn default() -> Self {
        Self {
            enable_name_lookup: true,
            enable_llm_name_correction: true,
            llm_vendor: None,
            llm_model: None,
            llm_timeout_seconds: None,
            max_llm_candidates: default_max_llm_candidates(),
            aliases: HashMap::new(),
            cleanup_tokens: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct RuntimeConfig {
    llm: LlmConfig,
    stock: StockSkillConfig,
}

#[derive(Debug, Deserialize)]
struct InternalLlmApiResponse {
    ok: bool,
    data: Option<InternalLlmTextData>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InternalLlmTextData {
    text: String,
}

#[derive(Debug, Clone)]
struct ResolvedSymbol {
    code: String,
    market: StockMarket,
    correction: Option<SymbolCorrection>,
}

#[derive(Debug, Clone)]
struct SymbolCorrection {
    input: String,
    matched_name: String,
    used_llm: bool,
    reason_code: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StockMarket {
    China,
    UnitedStates,
}

impl StockMarket {
    fn token(self) -> &'static str {
        match self {
            Self::China => "cn",
            Self::UnitedStates => "us",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarketHint {
    Auto,
    China,
    UnitedStates,
}

impl MarketHint {
    fn accepts(self, market: StockMarket) -> bool {
        matches!(self, Self::Auto)
            || matches!((self, market), (Self::China, StockMarket::China))
            || matches!(
                (self, market),
                (Self::UnitedStates, StockMarket::UnitedStates)
            )
    }
}

#[derive(Debug, Clone)]
struct SymbolSearchCandidate {
    code: String,
    market: StockMarket,
    name: String,
    normalized_name: String,
    query_alias: String,
}

#[derive(Debug, Clone)]
struct AliasCandidate {
    alias: String,
    code: String,
    normalized: String,
    score: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum VendorKind {
    OpenAI,
    Qwen,
    DeepSeek,
    Grok,
    MiniMax,
    Custom,
}

/// Provider endpoints are protocol configuration, not natural-language routing rules.
const SINA_HQ_URL: &str = "https://hq.sinajs.cn/list=";
const SINA_SUGGEST_URL: &str = "https://suggest3.sinajs.cn/suggest/";
const TENCENT_HQ_URL: &str = "https://qt.gtimg.cn/q=";
const SINA_REFERER: &str = "https://finance.sina.com.cn";

fn main() -> anyhow::Result<()> {
    let runtime = load_runtime_config();
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => {
                let requested_symbol = requested_symbol_from_args(&req.args).map(str::to_string);
                match execute(req.args, &runtime) {
                    Ok((text, extra)) => Resp {
                        request_id: req.request_id,
                        status: "ok".to_string(),
                        text,
                        extra: Some(extra),
                        error_text: None,
                    },
                    Err(err) => {
                        let error_code = machine_error_code(&err);
                        let mut extra = stock_error_extra(error_code);
                        if let (Some(extra), Some(requested_symbol)) =
                            (extra.as_object_mut(), requested_symbol)
                        {
                            extra.insert(
                                "requested_symbol".to_string(),
                                Value::String(requested_symbol),
                            );
                        }
                        Resp {
                            request_id: req.request_id,
                            status: "error".to_string(),
                            text: String::new(),
                            extra: Some(extra),
                            error_text: Some(err),
                        }
                    }
                }
            }
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(stock_error_extra("invalid_input")),
                error_text: Some(format!("code=invalid_input detail={err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn requested_symbol_from_args(args: &Value) -> Option<&str> {
    let obj = args.as_object()?;
    obj.get("symbol")
        .or_else(|| obj.get("code"))
        .or_else(|| obj.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn execute(args: Value, runtime: &RuntimeConfig) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("quote")
        .trim()
        .to_ascii_lowercase();
    let market_hint = market_hint_from_args(obj)?;

    match action.as_str() {
        "preview_quote" => preview_quote_request(obj),
        "quote" | "query" => {
            let symbol = obj
                .get("symbol")
                .or_else(|| obj.get("code"))
                .or_else(|| obj.get("name"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    "code=missing_symbol required_any=symbol|code|name example=600519".to_string()
                })?;
            let resolved = resolve_symbol(symbol, market_hint, runtime)?;
            quote_stock(&resolved)
        }
        _ => Err(format!(
            "code=unsupported_action action={} allowed=preview_quote|quote|query",
            action
        )),
    }
}

fn preview_quote_request(obj: &serde_json::Map<String, Value>) -> Result<(String, Value), String> {
    let requested_symbol = obj
        .get("symbol")
        .or_else(|| obj.get("code"))
        .or_else(|| obj.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "code=missing_symbol required_any=symbol|code|name example=600519".to_string()
        })?;
    let market_hint = market_hint_from_args(obj)?;
    let direct = normalize_direct_symbol(requested_symbol, market_hint);
    let normalized_code = direct
        .as_ref()
        .map(|(market, code)| display_code(*market, code));
    let market = direct.as_ref().map(|(market, _)| market.token());
    let resolution_mode = if direct.is_some() {
        "direct_code"
    } else {
        "provider_search_or_configured_alias"
    };
    let text =
        format!("message_key=skill.stock.quote_preview_ready resolution_mode={resolution_mode}");
    Ok((
        text,
        json!({
            "schema_version": 1,
            "source_skill": "stock",
            "status": "ok",
            "message_key": "skill.stock.quote_preview_ready",
            "action": "preview_quote",
            "requested_symbol": requested_symbol,
            "normalized_code": normalized_code,
            "market": market,
            "resolution_mode": resolution_mode,
            "provider_candidates": ["sina_finance", "tencent_finance"],
            "would_execute": false,
            "external_call_count": 0,
        }),
    ))
}

fn stock_error_extra(error_kind: &str) -> Value {
    json!({
        "schema_version": 1,
        "status": "error",
        "error_code": error_kind,
        "message_key": format!("skill.stock.{error_kind}"),
        "source_skill": "stock",
        "retryable": error_is_retryable(error_kind),
    })
}

fn machine_error_code(error: &str) -> &str {
    error
        .split_ascii_whitespace()
        .find_map(|part| part.strip_prefix("code="))
        .filter(|code| {
            !code.is_empty()
                && code
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        })
        .unwrap_or("stock_execution_failed")
}

fn error_is_retryable(error_code: &str) -> bool {
    matches!(
        error_code,
        "http_client_build_failed"
            | "quote_request_failed"
            | "quote_response_read_failed"
            | "quote_http_status"
            | "quote_provider_chain_failed"
            | "symbol_search_request_failed"
            | "symbol_search_http_status"
            | "symbol_search_response_read_failed"
            | "symbol_search_unavailable"
    )
}

fn default_true() -> bool {
    true
}

fn default_max_llm_candidates() -> usize {
    8
}

fn load_runtime_config() -> RuntimeConfig {
    let root = workspace_root();
    let llm = std::fs::read_to_string(root.join("configs/config.toml"))
        .ok()
        .and_then(|s| toml::from_str::<CoreConfig>(&s).ok())
        .map(|cfg| cfg.llm)
        .unwrap_or_default();
    let stock = std::fs::read_to_string(stock_config_path(&root))
        .ok()
        .and_then(|s| toml::from_str::<StockConfigFile>(&s).ok())
        .map(|cfg| cfg.stock)
        .unwrap_or_default();
    RuntimeConfig { llm, stock }
}

fn stock_config_path(root: &Path) -> PathBuf {
    std::env::var("STOCK_CONFIG_PATH")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() || p.exists())
        .unwrap_or_else(|| root.join("configs/stock.toml"))
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn market_hint_from_args(obj: &serde_json::Map<String, Value>) -> Result<MarketHint, String> {
    match obj
        .get("market")
        .and_then(Value::as_str)
        .unwrap_or("auto")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "auto" => Ok(MarketHint::Auto),
        "cn" => Ok(MarketHint::China),
        "us" => Ok(MarketHint::UnitedStates),
        value => Err(format!(
            "code=invalid_market value={value} allowed=auto|cn|us"
        )),
    }
}

/// 将 A 股代码规范为新浪格式：上海 sh + 代码，深圳 sz + 代码。
fn normalize_code(input: &str) -> String {
    let s = input.trim();
    if s.to_ascii_lowercase().starts_with("sh") || s.to_ascii_lowercase().starts_with("sz") {
        return s.to_ascii_lowercase();
    }
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return s.to_string();
    }
    if digits.starts_with('6') {
        format!("sh{digits}")
    } else {
        format!("sz{digits}")
    }
}

fn display_code(market: StockMarket, provider_code: &str) -> String {
    match market {
        StockMarket::China => provider_code.to_ascii_uppercase(),
        StockMarket::UnitedStates => format!("US:{}", provider_code.to_ascii_uppercase()),
    }
}

fn normalize_direct_symbol(input: &str, market_hint: MarketHint) -> Option<(StockMarket, String)> {
    let raw = input.trim();
    let lower = raw.to_ascii_lowercase();
    if looks_like_stock_code(raw) && market_hint.accepts(StockMarket::China) {
        return Some((StockMarket::China, normalize_code(raw)));
    }

    let explicit_us = ["us:", "nasdaq:", "nyse:"]
        .iter()
        .find_map(|prefix| lower.strip_prefix(prefix))
        .or_else(|| lower.strip_suffix(".us"));
    let ticker = explicit_us.unwrap_or(raw).trim();
    let ticker_valid = !ticker.is_empty()
        && ticker.len() <= 12
        && ticker.chars().any(|ch| ch.is_ascii_alphabetic())
        && ticker
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-');
    let auto_ticker = raw == raw.to_ascii_uppercase();
    if ticker_valid
        && market_hint.accepts(StockMarket::UnitedStates)
        && (explicit_us.is_some() || matches!(market_hint, MarketHint::UnitedStates) || auto_ticker)
    {
        return Some((StockMarket::UnitedStates, ticker.to_ascii_uppercase()));
    }
    None
}

fn resolve_symbol(
    input: &str,
    market_hint: MarketHint,
    runtime: &RuntimeConfig,
) -> Result<ResolvedSymbol, String> {
    if let Some((market, code)) = normalize_direct_symbol(input, market_hint) {
        return Ok(ResolvedSymbol {
            code,
            market,
            correction: None,
        });
    }

    if !runtime.stock.enable_name_lookup {
        return Err("code=name_lookup_disabled config=configs/stock.toml".to_string());
    }

    let alias_map = build_alias_map(&runtime.stock.aliases, &runtime.stock.cleanup_tokens);
    let normalized_input = normalize_stock_name(input, &runtime.stock.cleanup_tokens);
    if normalized_input.is_empty() {
        return Err("code=symbol_unrecognized reason=empty_normalized_name".to_string());
    }

    if market_hint.accepts(StockMarket::China) {
        if let Some((alias, code)) = alias_map.get(&normalized_input) {
            return Ok(ResolvedSymbol {
                code: normalize_code(code),
                market: StockMarket::China,
                correction: symbol_correction(input, alias, false, "configured_alias"),
            });
        }
    }

    let search_result = search_symbol_via_sina(input, &normalized_input, market_hint, runtime);
    if let Ok(Some(candidate)) = &search_result {
        return Ok(ResolvedSymbol {
            code: candidate.code.clone(),
            market: candidate.market,
            correction: symbol_correction(input, &candidate.name, false, "provider_symbol_search"),
        });
    }

    let candidates = if market_hint.accepts(StockMarket::China) {
        best_alias_candidates(
            &normalized_input,
            &alias_map,
            runtime.stock.max_llm_candidates,
        )
    } else {
        Vec::new()
    };
    if let Some(best) = choose_direct_candidate(
        input,
        &normalized_input,
        &candidates,
        &runtime.stock.cleanup_tokens,
    ) {
        return Ok(ResolvedSymbol {
            code: normalize_code(&best.code),
            market: StockMarket::China,
            correction: symbol_correction(input, &best.alias, false, "configured_alias_fuzzy"),
        });
    }

    if runtime.stock.enable_llm_name_correction {
        if let Ok(Some(best)) = choose_candidate_via_llm(input, &candidates, runtime) {
            return Ok(ResolvedSymbol {
                code: normalize_code(&best.code),
                market: StockMarket::China,
                correction: symbol_correction(input, &best.alias, true, "llm_alias_correction"),
            });
        }
    }

    if let Err(error) = search_result {
        return Err(format!(
            "code=symbol_search_unavailable cause={}",
            machine_error_code(&error)
        ));
    }

    let suggestions = candidates
        .iter()
        .take(3)
        .map(|c| c.alias.as_str())
        .collect::<Vec<_>>();
    if suggestions.is_empty() {
        Err(format!(
            "code=symbol_not_found input={} markets=cn|us",
            input.trim()
        ))
    } else {
        Err(format!(
            "code=symbol_not_found input={} suggestions={} markets=cn|us",
            input.trim(),
            suggestions.join("|")
        ))
    }
}

fn search_symbol_via_sina(
    input: &str,
    normalized_input: &str,
    market_hint: MarketHint,
    runtime: &RuntimeConfig,
) -> Result<Option<SymbolSearchCandidate>, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("code=http_client_build_failed detail={e}"))?;
    let resp = client
        .get(SINA_SUGGEST_URL)
        .header("Referer", SINA_REFERER)
        .header("User-Agent", "agent-stock-skill/1.0")
        .query(&[("type", ""), ("key", input.trim()), ("name", "suggestdata")])
        .send()
        .map_err(|e| format!("code=symbol_search_request_failed detail={e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "code=symbol_search_http_status status={}",
            resp.status()
        ));
    }
    let body = decode_sina_body(
        &resp
            .bytes()
            .map_err(|e| format!("code=symbol_search_response_read_failed detail={e}"))?,
    );
    let candidates = parse_sina_suggestions(&body, &runtime.stock.cleanup_tokens)
        .into_iter()
        .filter(|candidate| market_hint.accepts(candidate.market))
        .collect::<Vec<_>>();
    Ok(choose_search_candidate(normalized_input, input, &candidates).cloned())
}

fn parse_sina_suggestions(body: &str, cleanup_tokens: &[String]) -> Vec<SymbolSearchCandidate> {
    let Some(content_start) = body.find('"').map(|index| index + 1) else {
        return Vec::new();
    };
    let Some(relative_end) = body[content_start..].rfind('"') else {
        return Vec::new();
    };
    body[content_start..content_start + relative_end]
        .split(';')
        .filter_map(|row| {
            let fields = row.split(',').map(str::trim).collect::<Vec<_>>();
            let market = match fields.get(1).copied() {
                Some("11") => StockMarket::China,
                Some("41") => StockMarket::UnitedStates,
                _ => return None,
            };
            let raw_code = fields.get(3).or_else(|| fields.get(2))?.trim();
            let code = match market {
                StockMarket::China => normalize_code(raw_code),
                StockMarket::UnitedStates => fields.get(2)?.trim().to_ascii_uppercase(),
            };
            let name = fields.get(4).or_else(|| fields.first())?.trim().to_string();
            let query_alias = fields.first()?.trim().to_string();
            if code.is_empty() || name.is_empty() {
                return None;
            }
            Some(SymbolSearchCandidate {
                code,
                market,
                normalized_name: normalize_stock_name(&name, cleanup_tokens),
                name,
                query_alias,
            })
        })
        .collect()
}

fn choose_search_candidate<'a>(
    normalized_input: &str,
    raw_input: &str,
    candidates: &'a [SymbolSearchCandidate],
) -> Option<&'a SymbolSearchCandidate> {
    let normalized_ascii = raw_input
        .trim()
        .trim_start_matches("US:")
        .trim_start_matches("us:")
        .to_ascii_uppercase();
    candidates
        .iter()
        .find(|candidate| {
            candidate.normalized_name == normalized_input
                || normalize_stock_name(&candidate.query_alias, &[]) == normalized_input
                || (candidate.market == StockMarket::UnitedStates
                    && candidate.code == normalized_ascii)
        })
        .or_else(|| candidates.first())
}

fn quote_stock(resolved: &ResolvedSymbol) -> Result<(String, Value), String> {
    match quote_via_sina(resolved) {
        Ok(result) => Ok(result),
        Err(primary) => match quote_via_tencent(resolved) {
            Ok(result) => Ok(result),
            Err(fallback) => Err(format!(
                "code=quote_provider_chain_failed primary={} fallback={}",
                machine_error_code(&primary),
                machine_error_code(&fallback)
            )),
        },
    }
}

fn quote_via_sina(resolved: &ResolvedSymbol) -> Result<(String, Value), String> {
    let provider_code = match resolved.market {
        StockMarket::China => normalize_code(&resolved.code),
        StockMarket::UnitedStates => format!("gb_{}", resolved.code.to_ascii_lowercase()),
    };
    let body = fetch_quote_body(&format!("{SINA_HQ_URL}{provider_code}"), "sina_finance")?;
    match resolved.market {
        StockMarket::China => parse_sina_hq(&body, &provider_code, resolved.correction.as_ref())
            .map(|result| add_market_metadata(result, resolved.market)),
        StockMarket::UnitedStates => {
            parse_sina_us_hq(&body, &resolved.code, resolved.correction.as_ref())
        }
    }
}

fn quote_via_tencent(resolved: &ResolvedSymbol) -> Result<(String, Value), String> {
    let provider_code = match resolved.market {
        StockMarket::China => normalize_code(&resolved.code),
        StockMarket::UnitedStates => format!("us{}", resolved.code.to_ascii_uppercase()),
    };
    let body = fetch_quote_body(
        &format!("{TENCENT_HQ_URL}{provider_code}"),
        "tencent_finance",
    )?;
    parse_tencent_hq(
        &body,
        resolved.market,
        &resolved.code,
        resolved.correction.as_ref(),
    )
}

fn fetch_quote_body(url: &str, provider: &str) -> Result<String, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("code=http_client_build_failed provider={provider} detail={e}"))?;
    let resp = client
        .get(url)
        .header("Referer", SINA_REFERER)
        .header("User-Agent", "agent-stock-skill/1.0")
        .send()
        .map_err(|e| format!("code=quote_request_failed provider={provider} detail={e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "code=quote_http_status provider={provider} status={}",
            resp.status()
        ));
    }
    Ok(decode_quote_body(&resp.bytes().map_err(|e| {
        format!("code=quote_response_read_failed provider={provider} detail={e}")
    })?))
}

fn add_market_metadata(mut result: (String, Value), market: StockMarket) -> (String, Value) {
    if let Some(extra) = result.1.as_object_mut() {
        extra.insert(
            "market".to_string(),
            Value::String(market.token().to_string()),
        );
        if let Some(quote) = extra.get_mut("quote").and_then(Value::as_object_mut) {
            quote.insert(
                "market".to_string(),
                Value::String(market.token().to_string()),
            );
        }
    }
    result.0.push_str(&format!("\nmarket={}", market.token()));
    result
}

fn decode_quote_body(bytes: &[u8]) -> String {
    let utf8 = String::from_utf8_lossy(bytes);
    if !utf8.contains('\u{fffd}') {
        return utf8.into_owned();
    }
    let (decoded, _, _) = GBK.decode(bytes);
    decoded.into_owned()
}

fn decode_sina_body(bytes: &[u8]) -> String {
    let utf8 = String::from_utf8_lossy(bytes);
    if utf8.contains("var hq_str_") && !utf8.contains('\u{fffd}') {
        return utf8.into_owned();
    }
    let (decoded, _, _) = GBK.decode(bytes);
    decoded.into_owned()
}

/// 解析新浪 var hq_str_sh600519="name,open,prev,current,...";
fn parse_sina_hq(
    body: &str,
    code: &str,
    correction: Option<&SymbolCorrection>,
) -> Result<(String, Value), String> {
    let prefix = "var hq_str_";
    let start = body
        .find(prefix)
        .ok_or_else(|| "code=sina_hq_missing".to_string())?;
    let rest = &body[start + prefix.len()..];
    rest.find('=')
        .ok_or_else(|| "code=sina_hq_format_invalid missing=equals".to_string())?;
    let content_start = rest
        .find('"')
        .ok_or_else(|| "code=sina_hq_format_invalid missing=opening_quote".to_string())?
        + 1;
    let content_end = content_start
        + rest[content_start..]
            .find('"')
            .ok_or_else(|| "code=sina_hq_format_invalid missing=closing_quote".to_string())?;
    let content = rest[content_start..content_end].trim();
    if content.is_empty() {
        return Err(format!("code=quote_empty symbol={code}", code = code));
    }

    let parts: Vec<&str> = content.split(',').map(str::trim).collect();
    if parts.len() < 4 {
        return Err(format!(
            "code=quote_fields_insufficient count={}",
            parts.len()
        ));
    }
    let name = parts[0];
    let open = parts.get(1).unwrap_or(&"");
    let prev_close = parts.get(2).unwrap_or(&"");
    let current = parts.get(3).unwrap_or(&"");
    let high = parts.get(4).unwrap_or(&"");
    let low = parts.get(5).unwrap_or(&"");
    let volume = parts.get(8).unwrap_or(&"");
    let date = parts.get(30).unwrap_or(&"");
    let time = parts.get(31).unwrap_or(&"");
    let normalized_code = code.to_uppercase();
    let observed_at =
        (!date.is_empty() && !time.is_empty()).then(|| format!("{date}T{time}+08:00"));

    let mut lines = vec![
        "message_key=stock.msg.quote".to_string(),
        "reason_code=stock_quote_observed".to_string(),
        format!("code={normalized_code}"),
        format!("normalized_code={normalized_code}"),
        format!("symbol={code}"),
        format!("name={name}"),
        format!("current={current}"),
        format!("price={current}"),
        "provider=sina_finance".to_string(),
        format!("open={open}"),
        format!("prev_close={prev_close}"),
        format!("high={high}"),
        format!("low={low}"),
        format!("volume={volume}"),
        format!("date={date}"),
        format!("time={time}"),
    ];
    if let Some(observed_at) = observed_at.as_deref() {
        lines.push(format!("observed_at={observed_at}"));
    }
    if let Some(correction) = correction {
        lines.push(format!("correction.input={}", correction.input));
        lines.push(format!(
            "correction.matched_name={}",
            correction.matched_name
        ));
        lines.push(format!("correction.used_llm={}", correction.used_llm));
    }
    let mut change_pct_value = None;
    if let (Ok(c), Ok(p)) = (current.parse::<f64>(), prev_close.parse::<f64>()) {
        if p > 0.0 {
            let pct = (c - p) / p * 100.0;
            change_pct_value = Some(pct);
            lines.push(format!("change_pct={pct:.4}"));
        }
    }
    let correction_value = correction.map(|correction| {
        json!({
            "input": correction.input.clone(),
            "matched_name": correction.matched_name.clone(),
            "used_llm": correction.used_llm,
            "reason_code": correction.reason_code,
        })
    });
    let extra = json!({
        "schema_version": 1,
        "message_key": "stock.msg.quote",
        "reason_code": "stock_quote_observed",
        "action": "quote",
        "source_skill": "stock",
        "status": "ok",
        "code": normalized_code,
        "normalized_code": normalized_code,
        "symbol": code,
        "name": name,
        "price": current,
        "provider": "sina_finance",
        "provider_id": "sina_finance",
        "currency": "CNY",
        "observed_at": observed_at,
        "open": open,
        "prev_close": prev_close,
        "current": current,
        "high": high,
        "low": low,
        "volume": volume,
        "date": date,
        "time": time,
        "change_pct": change_pct_value,
        "correction": correction_value,
        "quote": {
            "code": normalized_code,
            "normalized_code": normalized_code,
            "symbol": code,
            "name": name,
            "price": current,
            "provider": "sina_finance",
            "currency": "CNY",
            "observed_at": observed_at,
            "open": open,
            "prev_close": prev_close,
            "current": current,
            "high": high,
            "low": low,
            "volume": volume,
            "date": date,
            "time": time,
            "change_pct": change_pct_value
        }
    });
    Ok((lines.join("\n"), extra))
}

fn parse_sina_us_hq(
    body: &str,
    ticker: &str,
    correction: Option<&SymbolCorrection>,
) -> Result<(String, Value), String> {
    let content = extract_quoted_payload(body, "var hq_str_gb_")?;
    let parts = content.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() < 27 {
        return Err(format!(
            "code=quote_fields_insufficient provider=sina_finance market=us count={}",
            parts.len()
        ));
    }
    let observed = parts.get(3).copied().unwrap_or_default();
    let (date, time) = observed.split_once(' ').unwrap_or((observed, ""));
    let observed_at =
        (!date.is_empty() && !time.is_empty()).then(|| format!("{date}T{time}+08:00"));
    let change_pct = parts.get(2).and_then(|value| value.parse::<f64>().ok());
    Ok(build_quote_result(
        StockMarket::UnitedStates,
        "sina_finance",
        ticker,
        parts.first().copied().unwrap_or_default(),
        parts.get(1).copied().unwrap_or_default(),
        parts.get(5).copied().unwrap_or_default(),
        parts.get(26).copied().unwrap_or_default(),
        parts.get(6).copied().unwrap_or_default(),
        parts.get(7).copied().unwrap_or_default(),
        parts.get(10).copied().unwrap_or_default(),
        date,
        time,
        observed_at,
        change_pct,
        "USD",
        correction,
    ))
}

fn parse_tencent_hq(
    body: &str,
    market: StockMarket,
    canonical_code: &str,
    correction: Option<&SymbolCorrection>,
) -> Result<(String, Value), String> {
    let content = extract_quoted_payload(body, "v_")?;
    let parts = content.split('~').map(str::trim).collect::<Vec<_>>();
    if parts.len() < 35 {
        return Err(format!(
            "code=quote_fields_insufficient provider=tencent_finance market={} count={}",
            market.token(),
            parts.len()
        ));
    }
    let timestamp = parts.get(30).copied().unwrap_or_default();
    let (date, time) = parse_compact_timestamp(timestamp);
    let observed_at =
        (!date.is_empty() && !time.is_empty()).then(|| format!("{date}T{time}+08:00"));
    let change_pct = parts.get(32).and_then(|value| value.parse::<f64>().ok());
    let currency = parts
        .get(35)
        .copied()
        .filter(|value| !value.is_empty())
        .unwrap_or(match market {
            StockMarket::China => "CNY",
            StockMarket::UnitedStates => "USD",
        });
    Ok(build_quote_result(
        market,
        "tencent_finance",
        canonical_code,
        parts.get(1).copied().unwrap_or_default(),
        parts.get(3).copied().unwrap_or_default(),
        parts.get(5).copied().unwrap_or_default(),
        parts.get(4).copied().unwrap_or_default(),
        parts.get(33).copied().unwrap_or_default(),
        parts.get(34).copied().unwrap_or_default(),
        parts.get(6).copied().unwrap_or_default(),
        &date,
        &time,
        observed_at,
        change_pct,
        currency,
        correction,
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_quote_result(
    market: StockMarket,
    provider: &str,
    canonical_code: &str,
    name: &str,
    current: &str,
    open: &str,
    prev_close: &str,
    high: &str,
    low: &str,
    volume: &str,
    date: &str,
    time: &str,
    observed_at: Option<String>,
    change_pct: Option<f64>,
    currency: &str,
    correction: Option<&SymbolCorrection>,
) -> (String, Value) {
    let normalized_code = display_code(market, canonical_code);
    let correction_value = correction.map(|correction| {
        json!({
            "input": correction.input,
            "matched_name": correction.matched_name,
            "used_llm": correction.used_llm,
            "reason_code": correction.reason_code,
        })
    });
    let mut lines = vec![
        "message_key=stock.msg.quote".to_string(),
        "reason_code=stock_quote_observed".to_string(),
        format!("normalized_code={normalized_code}"),
        format!("market={}", market.token()),
        format!("name={name}"),
        format!("price={current}"),
        format!("provider={provider}"),
        format!("currency={currency}"),
        format!("open={open}"),
        format!("prev_close={prev_close}"),
        format!("high={high}"),
        format!("low={low}"),
        format!("volume={volume}"),
    ];
    if let Some(observed_at) = observed_at.as_deref() {
        lines.push(format!("observed_at={observed_at}"));
    }
    if let Some(change_pct) = change_pct {
        lines.push(format!("change_pct={change_pct:.4}"));
    }
    let quote = json!({
        "code": normalized_code,
        "normalized_code": normalized_code,
        "symbol": canonical_code,
        "market": market.token(),
        "name": name,
        "price": current,
        "current": current,
        "open": open,
        "prev_close": prev_close,
        "high": high,
        "low": low,
        "volume": volume,
        "currency": currency,
        "provider": provider,
        "observed_at": observed_at,
        "date": date,
        "time": time,
        "change_pct": change_pct,
    });
    let extra = json!({
        "schema_version": 1,
        "source_skill": "stock",
        "status": "ok",
        "message_key": "stock.msg.quote",
        "reason_code": "stock_quote_observed",
        "action": "quote",
        "code": normalized_code,
        "normalized_code": normalized_code,
        "symbol": canonical_code,
        "market": market.token(),
        "name": name,
        "price": current,
        "current": current,
        "open": open,
        "prev_close": prev_close,
        "high": high,
        "low": low,
        "volume": volume,
        "currency": currency,
        "provider": provider,
        "provider_id": provider,
        "observed_at": observed_at,
        "date": date,
        "time": time,
        "change_pct": change_pct,
        "correction": correction_value,
        "quote": quote,
    });
    (lines.join("\n"), extra)
}

fn extract_quoted_payload<'a>(body: &'a str, expected_prefix: &str) -> Result<&'a str, String> {
    if !body.contains(expected_prefix) {
        return Err(format!(
            "code=quote_response_contract_invalid expected_prefix={expected_prefix}"
        ));
    }
    let start = body
        .find('"')
        .map(|index| index + 1)
        .ok_or_else(|| "code=quote_response_contract_invalid missing=opening_quote".to_string())?;
    let end = start
        + body[start..].rfind('"').ok_or_else(|| {
            "code=quote_response_contract_invalid missing=closing_quote".to_string()
        })?;
    let content = body[start..end].trim();
    if content.is_empty() {
        return Err("code=quote_empty".to_string());
    }
    Ok(content)
}

fn parse_compact_timestamp(raw: &str) -> (String, String) {
    let digits = raw.chars().filter(char::is_ascii_digit).collect::<String>();
    if digits.len() < 14 {
        return (String::new(), String::new());
    }
    (
        format!("{}-{}-{}", &digits[0..4], &digits[4..6], &digits[6..8]),
        format!("{}:{}:{}", &digits[8..10], &digits[10..12], &digits[12..14]),
    )
}

fn looks_like_stock_code(input: &str) -> bool {
    let s = input.trim().to_ascii_lowercase();
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 6 {
        return true;
    }
    (s.starts_with("sh") || s.starts_with("sz")) && digits.len() == 6
}

fn build_alias_map(
    aliases: &HashMap<String, String>,
    cleanup_tokens: &[String],
) -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    for (alias, code) in aliases {
        let normalized = normalize_stock_name(alias, cleanup_tokens);
        if normalized.is_empty() {
            continue;
        }
        out.entry(normalized)
            .or_insert_with(|| (alias.trim().to_string(), code.trim().to_string()));
    }
    out
}

fn normalize_stock_name(input: &str, cleanup_tokens: &[String]) -> String {
    let mut s = input.trim().to_string();
    for token in cleanup_tokens
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        s = s.replace(token, "");
    }
    s.chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(ch))
        .flat_map(|ch| ch.to_lowercase())
        .collect::<String>()
}

fn best_alias_candidates(
    normalized_input: &str,
    alias_map: &HashMap<String, (String, String)>,
    limit: usize,
) -> Vec<AliasCandidate> {
    let mut out = alias_map
        .iter()
        .map(|(normalized, (alias, code))| AliasCandidate {
            alias: alias.clone(),
            code: code.clone(),
            normalized: normalized.clone(),
            score: score_alias_candidate(normalized_input, normalized),
        })
        .filter(|c| c.score > 0)
        .collect::<Vec<_>>();
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.alias.len().cmp(&b.alias.len()))
    });
    out.truncate(limit.max(1));
    out
}

fn score_alias_candidate(input: &str, alias: &str) -> i64 {
    if input == alias {
        return 10_000;
    }
    if alias.contains(input) || input.contains(alias) {
        return 7_000 - (alias.len() as i64 - input.len() as i64).abs() * 10;
    }
    let dist = levenshtein(input, alias) as i64;
    let len_gap = (alias.len() as i64 - input.len() as i64).abs();
    let shared = shared_chars(input, alias) as i64;
    5_000 - dist * 700 - len_gap * 40 + shared * 50
}

fn choose_direct_candidate<'a>(
    raw_input: &str,
    normalized_input: &str,
    candidates: &'a [AliasCandidate],
    cleanup_tokens: &[String],
) -> Option<&'a AliasCandidate> {
    let best = candidates.first()?;
    if best.normalized == normalized_input {
        return Some(best);
    }
    if best.normalized.contains(normalized_input) || normalized_input.contains(&best.normalized) {
        return Some(best);
    }
    let second_score = candidates.get(1).map(|c| c.score).unwrap_or(i64::MIN);
    if best.score >= 4_200 && best.score - second_score >= 900 {
        return Some(best);
    }
    if levenshtein(
        &normalize_stock_name(raw_input, cleanup_tokens),
        &best.normalized,
    ) <= 1
        && best.score >= 3_800
    {
        return Some(best);
    }
    None
}

fn choose_candidate_via_llm<'a>(
    raw_input: &str,
    candidates: &'a [AliasCandidate],
    runtime: &RuntimeConfig,
) -> Result<Option<&'a AliasCandidate>, String> {
    if candidates.is_empty() {
        return Ok(None);
    }
    let candidate_names = candidates
        .iter()
        .map(|c| format!("{} -> {}", c.alias, c.code))
        .collect::<Vec<_>>();
    let system = "You normalize A-share stock-name typos. Select exactly one alias from the provided candidates, or NONE when uncertain. Output one-line JSON only: {\"alias\":\"<candidate alias>\"} or {\"alias\":\"NONE\"}.";
    let user = format!(
        "raw_input: {}\ncandidates:\n{}\nReturn exactly one candidate alias from the list, or NONE.",
        raw_input.trim(),
        candidate_names.join("\n")
    );
    let internal_timeout_secs = runtime.stock.llm_timeout_seconds.unwrap_or(15).max(1);
    let content = match call_internal_llm_text(
        "skills/stock/name_correction",
        system,
        &user,
        runtime.stock.llm_vendor.as_deref(),
        runtime.stock.llm_model.as_deref(),
        0.0,
        64,
        internal_timeout_secs,
    ) {
        Some(result) => result?,
        None => {
            let Some((vendor_cfg, model, timeout_secs)) = resolve_llm_vendor(runtime) else {
                return Ok(None);
            };
            call_openai_compatible_chat(vendor_cfg, &model, timeout_secs, system, &user)?
        }
    };
    let alias = parse_llm_alias_response(&content)?;
    if alias.eq_ignore_ascii_case("NONE") {
        return Ok(None);
    }
    Ok(candidates.iter().find(|c| c.alias == alias))
}

fn resolve_llm_vendor(runtime: &RuntimeConfig) -> Option<(&VendorConfig, String, u64)> {
    let requested = runtime
        .stock
        .llm_vendor
        .as_deref()
        .and_then(parse_vendor_kind)
        .or_else(|| {
            runtime
                .llm
                .selected_vendor
                .as_deref()
                .and_then(parse_vendor_kind)
        });
    let mut order = Vec::new();
    if let Some(v) = requested {
        order.push(v);
    }
    for v in [
        VendorKind::Qwen,
        VendorKind::OpenAI,
        VendorKind::DeepSeek,
        VendorKind::Grok,
        VendorKind::MiniMax,
        VendorKind::Custom,
    ] {
        if !order.contains(&v) {
            order.push(v);
        }
    }

    for vendor in order {
        let cfg = match vendor {
            VendorKind::OpenAI => runtime.llm.openai.as_ref(),
            VendorKind::Qwen => runtime.llm.qwen.as_ref(),
            VendorKind::DeepSeek => runtime.llm.deepseek.as_ref(),
            VendorKind::Grok => runtime.llm.grok.as_ref(),
            VendorKind::MiniMax => runtime.llm.minimax.as_ref(),
            VendorKind::Custom => runtime.llm.custom.as_ref(),
        };
        let Some(cfg) = cfg else {
            continue;
        };
        if cfg.api_key.trim().is_empty()
            || cfg.base_url.trim().is_empty()
            || cfg.model.trim().is_empty()
        {
            continue;
        }
        let model = runtime
            .stock
            .llm_model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(cfg.model.trim())
            .to_string();
        let timeout_secs = runtime
            .stock
            .llm_timeout_seconds
            .or(cfg.timeout_seconds)
            .unwrap_or(15)
            .max(1);
        return Some((cfg, model, timeout_secs));
    }
    None
}

fn call_internal_llm_text(
    prompt_source: &str,
    system_prompt: &str,
    user_prompt: &str,
    vendor: Option<&str>,
    model: Option<&str>,
    temperature: f64,
    max_tokens: u64,
    timeout_secs: u64,
) -> Option<Result<String, String>> {
    let url = std::env::var("AGENT_INTERNAL_LLM_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let token = std::env::var("AGENT_INTERNAL_LLM_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let body = json!({
        "skill_name": "stock",
        "prompt_source": prompt_source,
        "system": system_prompt,
        "user": user_prompt,
        "vendor": vendor.map(str::trim).filter(|value| !value.is_empty()),
        "model": model.map(str::trim).filter(|value| !value.is_empty()),
        "temperature": temperature,
        "max_tokens": max_tokens
    });
    let result = (|| {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs.max(5)))
            .build()
            .map_err(|e| format!("code=internal_llm_client_build_failed detail={e}"))?;
        let resp = client
            .post(url)
            .header("x-agent-internal-llm-token", token)
            .json(&body)
            .send()
            .map_err(|e| format!("code=internal_llm_request_failed detail={e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(format!(
                "code=internal_llm_http_status status={} body={}",
                status, body
            ));
        }
        let parsed: InternalLlmApiResponse = resp
            .json()
            .map_err(|e| format!("code=internal_llm_json_failed detail={e}"))?;
        if !parsed.ok {
            return Err(parsed
                .error
                .unwrap_or_else(|| "code=internal_llm_failed".to_string()));
        }
        parsed
            .data
            .map(|data| data.text)
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| "code=internal_llm_empty_content".to_string())
    })();
    Some(result)
}

fn parse_vendor_kind(raw: &str) -> Option<VendorKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "openai" => Some(VendorKind::OpenAI),
        "qwen" => Some(VendorKind::Qwen),
        "deepseek" => Some(VendorKind::DeepSeek),
        "grok" => Some(VendorKind::Grok),
        "minimax" => Some(VendorKind::MiniMax),
        "custom" => Some(VendorKind::Custom),
        _ => None,
    }
}

fn call_openai_compatible_chat(
    vendor_cfg: &VendorConfig,
    model: &str,
    timeout_secs: u64,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, String> {
    let url = format!(
        "{}/chat/completions",
        vendor_cfg.base_url.trim_end_matches('/')
    );
    let body = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt}
        ],
        "temperature": 0.0,
        "max_tokens": 64
    });
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("code=llm_client_build_failed detail={e}"))?;
    let resp = client
        .post(url)
        .bearer_auth(vendor_cfg.api_key.trim())
        .json(&body)
        .send()
        .map_err(|e| format!("code=llm_request_failed detail={e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!(
            "code=llm_http_status status={} body={}",
            status, body
        ));
    }
    let v: Value = resp
        .json()
        .map_err(|e| format!("code=llm_json_failed detail={e}"))?;
    let content = v
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("message"))
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "code=llm_empty_content".to_string())?;
    Ok(content.to_string())
}

fn parse_llm_alias_response(content: &str) -> Result<String, String> {
    let trimmed = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        if let Some(alias) = parse_alias_from_json_value(&v) {
            return Ok(alias);
        }
    }
    let line = trimmed.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return Err("code=llm_empty_alias".to_string());
    }
    Ok(line.trim_matches('"').to_string())
}

fn stock_alias_choice_schema() -> &'static Value {
    STOCK_ALIAS_CHOICE_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(STOCK_ALIAS_CHOICE_SCHEMA_RAW)
            .expect("stock_alias_choice schema must be valid JSON")
    })
}

fn schema_requires_field(schema: &Value, name: &str) -> bool {
    schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|fields| fields.iter().any(|field| field.as_str() == Some(name)))
        .unwrap_or(false)
}

fn schema_declared_fields(schema: &Value) -> Option<&serde_json::Map<String, Value>> {
    schema.get("properties")?.as_object()
}

fn schema_allows_additional_properties(schema: &Value) -> bool {
    schema
        .get("additionalProperties")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn schema_string_is_valid(schema: &Value, name: &str, value: &str) -> bool {
    let property = match schema.get("properties").and_then(|v| v.get(name)) {
        Some(property) => property,
        None => return false,
    };
    if property.get("type").and_then(|v| v.as_str()) != Some("string") {
        return false;
    }
    let min_length = property
        .get("minLength")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    value.chars().count() >= min_length
}

fn parse_alias_from_json_value(value: &Value) -> Option<String> {
    let schema = stock_alias_choice_schema();
    let object = value.as_object()?;
    if !schema_allows_additional_properties(schema) {
        let declared_fields = schema_declared_fields(schema)?;
        if object.keys().any(|key| !declared_fields.contains_key(key)) {
            return None;
        }
    }
    if schema_requires_field(schema, "alias") && !object.contains_key("alias") {
        return None;
    }
    let alias = object.get("alias")?.as_str()?.trim();
    if !schema_string_is_valid(schema, "alias", alias) {
        return None;
    }
    Some(alias.to_string())
}

fn symbol_correction(
    input: &str,
    matched_name: &str,
    used_llm: bool,
    reason_code: &'static str,
) -> Option<SymbolCorrection> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    Some(SymbolCorrection {
        input: raw.to_string(),
        matched_name: matched_name.trim().to_string(),
        used_llm,
        reason_code,
    })
}

fn shared_chars(a: &str, b: &str) -> usize {
    let mut count = 0usize;
    for ch in a.chars() {
        if b.contains(ch) {
            count += 1;
        }
    }
    count
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars = a.chars().collect::<Vec<_>>();
    let b_chars = b.chars().collect::<Vec<_>>();
    if a_chars.is_empty() {
        return b_chars.len();
    }
    if b_chars.is_empty() {
        return a_chars.len();
    }
    let mut prev = (0..=b_chars.len()).collect::<Vec<_>>();
    let mut curr = vec![0usize; b_chars.len() + 1];
    for (i, a_ch) in a_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, b_ch) in b_chars.iter().enumerate() {
            let cost = if a_ch == b_ch { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        prev.clone_from(&curr);
    }
    prev[b_chars.len()]
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

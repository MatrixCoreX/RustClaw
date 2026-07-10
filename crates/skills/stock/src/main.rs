//! 股票技能：查询 A 股实时行情（单行 JSON stdin -> 单行 JSON stdout）
//! 数据来源：新浪财经 hq.sinajs.cn，需 Referer

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
    include_str!("../../../../prompts/schemas/stock_alias_choice.schema.json");

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
        }
    }
}

#[derive(Debug, Clone)]
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
    correction: Option<SymbolCorrection>,
}

#[derive(Debug, Clone)]
struct SymbolCorrection {
    input: String,
    matched_name: String,
    used_llm: bool,
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

/// 新浪 A 股行情：上海 sh + 代码，深圳 sz + 代码
const SINA_HQ_URL: &str = "http://hq.sinajs.cn/list=";
const SINA_REFERER: &str = "https://finance.sina.com.cn";

fn main() -> anyhow::Result<()> {
    let runtime = load_runtime_config();
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args, &runtime) {
                Ok((text, extra)) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    extra: Some(extra),
                    error_text: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    extra: Some(stock_error_extra("stock_quote_failed")),
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(stock_error_extra("invalid_input")),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
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

    match action.as_str() {
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
            let resolved = resolve_symbol(symbol, runtime)?;
            quote_a_share(&resolved)
        }
        _ => Err(format!(
            "code=unsupported_action action={} allowed=quote|query",
            action
        )),
    }
}

fn stock_error_extra(error_kind: &str) -> Value {
    json!({
        "status": "error",
        "error_kind": error_kind,
        "source_skill": "stock",
    })
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

/// 将用户输入的代码规范为新浪格式：shXXXXXX 或 szXXXXXX
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

fn resolve_symbol(input: &str, runtime: &RuntimeConfig) -> Result<ResolvedSymbol, String> {
    if looks_like_stock_code(input) {
        return Ok(ResolvedSymbol {
            code: normalize_code(input),
            correction: None,
        });
    }

    if !runtime.stock.enable_name_lookup {
        return Err("code=name_lookup_disabled config=configs/stock.toml".to_string());
    }

    let alias_map = build_alias_map(&runtime.stock.aliases);
    let normalized_input = normalize_stock_name(input);
    if normalized_input.is_empty() {
        return Err("code=symbol_unrecognized reason=empty_normalized_name".to_string());
    }

    if let Some((alias, code)) = alias_map.get(&normalized_input) {
        return Ok(ResolvedSymbol {
            code: code.clone(),
            correction: symbol_correction(input, alias, false),
        });
    }

    let candidates = best_alias_candidates(
        &normalized_input,
        &alias_map,
        runtime.stock.max_llm_candidates,
    );
    if let Some(best) = choose_direct_candidate(input, &normalized_input, &candidates) {
        return Ok(ResolvedSymbol {
            code: best.code.clone(),
            correction: symbol_correction(input, &best.alias, false),
        });
    }

    if runtime.stock.enable_llm_name_correction {
        if let Some(best) = choose_candidate_via_llm(input, &candidates, runtime)? {
            return Ok(ResolvedSymbol {
                code: best.code.clone(),
                correction: symbol_correction(input, &best.alias, true),
            });
        }
    }

    let suggestions = candidates
        .iter()
        .take(3)
        .map(|c| c.alias.as_str())
        .collect::<Vec<_>>();
    if suggestions.is_empty() {
        Err(format!(
            "code=symbol_alias_not_found input={} config=configs/stock.toml",
            input.trim()
        ))
    } else {
        Err(format!(
            "code=symbol_alias_not_found input={} suggestions={} config=configs/stock.toml",
            input.trim(),
            suggestions.join("|")
        ))
    }
}

fn quote_a_share(resolved: &ResolvedSymbol) -> Result<(String, Value), String> {
    let code = normalize_code(&resolved.code);
    let url = format!("{SINA_HQ_URL}{code}");
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("code=http_client_build_failed detail={e}"))?;

    let resp = client
        .get(&url)
        .header("Referer", SINA_REFERER)
        .header("User-Agent", "RustClaw-Stock-Skill/1.0")
        .send()
        .map_err(|e| format!("code=quote_request_failed detail={e}"))?;

    if !resp.status().is_success() {
        return Err(format!("code=quote_http_status status={}", resp.status()));
    }

    let body = decode_sina_body(
        &resp
            .bytes()
            .map_err(|e| format!("code=quote_response_read_failed detail={e}"))?,
    );

    parse_sina_hq(&body, &code, resolved.correction.as_ref())
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

    let mut lines = vec![
        "message_key=stock.msg.quote".to_string(),
        "reason_code=stock_quote_observed".to_string(),
        format!("code={}", code.to_uppercase()),
        format!("symbol={code}"),
        format!("name={name}"),
        format!("current={current}"),
        format!("open={open}"),
        format!("prev_close={prev_close}"),
        format!("high={high}"),
        format!("low={low}"),
        format!("volume={volume}"),
        format!("date={date}"),
        format!("time={time}"),
    ];
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
            "reason_code": if correction.used_llm {
                "llm_alias_correction"
            } else {
                "alias_match"
            }
        })
    });
    let extra = json!({
        "message_key": "stock.msg.quote",
        "reason_code": "stock_quote_observed",
        "action": "quote",
        "source_skill": "stock",
        "status": "ok",
        "code": code.to_uppercase(),
        "symbol": code,
        "name": name,
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
            "code": code.to_uppercase(),
            "symbol": code,
            "name": name,
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

fn looks_like_stock_code(input: &str) -> bool {
    let s = input.trim().to_ascii_lowercase();
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 6 {
        return true;
    }
    (s.starts_with("sh") || s.starts_with("sz")) && digits.len() == 6
}

fn build_alias_map(aliases: &HashMap<String, String>) -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    for (alias, code) in aliases {
        let normalized = normalize_stock_name(alias);
        if normalized.is_empty() {
            continue;
        }
        out.entry(normalized)
            .or_insert_with(|| (alias.trim().to_string(), code.trim().to_string()));
    }
    out
}

fn normalize_stock_name(input: &str) -> String {
    let mut s = input.trim().to_string();
    for token in [
        "股票代码",
        "股票代号",
        "股票名称",
        "股票",
        "股价",
        "行情",
        "A股",
        "a股",
        "股份有限公司",
        "股份",
        "有限公司",
        "集团",
        "控股",
        "公司",
    ] {
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
    if levenshtein(&normalize_stock_name(raw_input), &best.normalized) <= 1 && best.score >= 3_800 {
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
    let system = "你是 A 股股票名称纠错器。只能从候选列表中选择一个最可能匹配的名称；如果没有把握就返回 NONE。只输出一行 JSON，如 {\"alias\":\"中国移动\"} 或 {\"alias\":\"NONE\"}。";
    let user = format!(
        "用户输入：{}\n候选列表：\n{}\n请只从候选里选一个最可能的标准名称；没有把握就返回 NONE。",
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
    let url = std::env::var("RUSTCLAW_INTERNAL_LLM_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let token = std::env::var("RUSTCLAW_INTERNAL_LLM_TOKEN")
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
            .map_err(|e| format!("创建内部 LLM 客户端失败: {e}"))?;
        let resp = client
            .post(url)
            .header("x-rustclaw-internal-llm-token", token)
            .json(&body)
            .send()
            .map_err(|e| format!("内部 LLM 名称纠错请求失败: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(format!("内部 LLM 名称纠错失败 HTTP {}: {}", status, body));
        }
        let parsed: InternalLlmApiResponse = resp
            .json()
            .map_err(|e| format!("解析内部 LLM 名称纠错响应失败: {e}"))?;
        if !parsed.ok {
            return Err(parsed
                .error
                .unwrap_or_else(|| "内部 LLM 名称纠错失败".to_string()));
        }
        parsed
            .data
            .map(|data| data.text)
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| "内部 LLM 名称纠错返回空内容".to_string())
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
        .map_err(|e| format!("创建 LLM 客户端失败: {e}"))?;
    let resp = client
        .post(url)
        .bearer_auth(vendor_cfg.api_key.trim())
        .json(&body)
        .send()
        .map_err(|e| format!("LLM 名称纠错请求失败: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("LLM 名称纠错失败 HTTP {}: {}", status, body));
    }
    let v: Value = resp
        .json()
        .map_err(|e| format!("解析 LLM 名称纠错响应失败: {e}"))?;
    let content = v
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("message"))
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "LLM 名称纠错返回空内容".to_string())?;
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
        return Err("LLM 名称纠错返回空别名".to_string());
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

fn symbol_correction(input: &str, matched_name: &str, used_llm: bool) -> Option<SymbolCorrection> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    Some(SymbolCorrection {
        input: raw.to_string(),
        matched_name: matched_name.trim().to_string(),
        used_llm,
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

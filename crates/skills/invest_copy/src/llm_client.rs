//! 使用与 clawd 主程序一致的默认 OpenAI 兼容端点：`OPENAI_*` 环境变量（由 skill-runner 从当前
//! `openai_compat` provider 注入）或回退读取 `WORKSPACE_ROOT/configs/config.toml` 中的 `[llm]`。

use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

const CONFIG_REL: &str = "configs/config.toml";

#[derive(Debug, Clone)]
pub struct LlmCreds {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout_secs: u64,
    /// `env_openai` | `config_toml`
    pub source: &'static str,
}

#[derive(Debug, Deserialize)]
struct ConfigToml {
    llm: LlmTables,
}

#[derive(Debug, Deserialize)]
struct LlmTables {
    #[serde(default)]
    selected_vendor: Option<String>,
    #[serde(default)]
    selected_model: Option<String>,
    #[serde(default)]
    openai: Option<VendorRow>,
    #[serde(default)]
    minimax: Option<VendorRow>,
    #[serde(default)]
    mimo: Option<VendorRow>,
    #[serde(default)]
    deepseek: Option<VendorRow>,
    #[serde(default)]
    qwen: Option<VendorRow>,
    #[serde(default)]
    custom: Option<VendorRow>,
    #[serde(default)]
    grok: Option<VendorRow>,
}

#[derive(Debug, Deserialize)]
struct VendorRow {
    base_url: String,
    #[serde(default)]
    api_key: String,
    model: String,
    #[serde(default = "default_vendor_timeout")]
    timeout_seconds: u64,
}

fn default_vendor_timeout() -> u64 {
    60
}

fn find_workspace_root() -> Option<PathBuf> {
    if let Ok(s) = std::env::var("WORKSPACE_ROOT") {
        let p = PathBuf::from(s.trim());
        if p.join(CONFIG_REL).is_file() {
            return Some(p);
        }
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join(CONFIG_REL).is_file() {
            return Some(dir);
        }
        dir = dir.parent()?.to_path_buf();
        if dir.as_os_str().is_empty() {
            break;
        }
    }
    None
}

/// 优先：非空 `OPENAI_API_KEY` + `OPENAI_BASE_URL`（缺省 `https://api.openai.com/v1`）+ `OPENAI_MODEL`。
/// 否则：解析 `configs/config.toml` 中 `[llm.selected_vendor]` 对应子表（需非空 api_key）。
pub fn resolve_llm_credentials() -> Result<LlmCreds, String> {
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    let key = key.trim();
    if !key.is_empty() {
        let base = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
        let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
        let timeout_secs = std::env::var("SKILL_TIMEOUT_SECONDS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(60)
            .min(120);
        return Ok(LlmCreds {
            base_url: base.trim_end_matches('/').to_string(),
            api_key: key.to_string(),
            model: model.trim().to_string(),
            timeout_secs,
            source: "env_openai",
        });
    }

    let root = find_workspace_root().ok_or_else(|| {
        "无法解析 LLM 凭据：OPENAI_API_KEY 为空，且未找到含 configs/config.toml 的工作区（请设置 WORKSPACE_ROOT 或在仓库根运行）".to_string()
    })?;
    let raw = std::fs::read_to_string(root.join(CONFIG_REL))
        .map_err(|e| format!("读取 configs/config.toml 失败: {e}"))?;
    let cfg: ConfigToml =
        toml::from_str(&raw).map_err(|e| format!("解析 configs/config.toml: {e}"))?;
    let vendor = cfg
        .llm
        .selected_vendor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "[llm].selected_vendor 未设置，且 OPENAI_API_KEY 为空".to_string())?;
    let vnorm = match vendor.to_ascii_lowercase().as_str() {
        "xiaomi" => "mimo".to_string(),
        other => other.to_string(),
    };
    let row = match vnorm.as_str() {
        "openai" => cfg.llm.openai,
        "minimax" => cfg.llm.minimax,
        "mimo" | "xiaomi" => cfg.llm.mimo,
        "deepseek" => cfg.llm.deepseek,
        "qwen" => cfg.llm.qwen,
        "custom" => cfg.llm.custom,
        "grok" => cfg.llm.grok,
        _ => {
            return Err(format!(
                "invest_copy 当前仅从 config 支持 openai_compat 类厂商（openai/minimax/mimo/deepseek/qwen/custom/grok），当前为 `{vendor}`；可改用环境变量 OPENAI_* 或 args.use_heuristic=true"
            ));
        }
    }
    .ok_or_else(|| format!("configs/config.toml 缺少 [llm.{vnorm}] 段"))?;

    let api_key = row.api_key.trim();
    if api_key.is_empty() {
        return Err(format!(
            "[llm.{vnorm}].api_key 为空；请填写密钥或使用 args.use_heuristic=true 走离线规则摘要"
        ));
    }

    let model = cfg
        .llm
        .selected_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(row.model.as_str())
        .to_string();

    Ok(LlmCreds {
        base_url: row.base_url.trim_end_matches('/').to_string(),
        api_key: api_key.to_string(),
        model,
        timeout_secs: row.timeout_seconds.max(1).min(120),
        source: "config_toml",
    })
}

/// OpenAI 兼容 `POST /chat/completions`
pub fn chat_completion(creds: &LlmCreds, system: &str, user: &str) -> Result<String, String> {
    let url = format!("{}/chat/completions", creds.base_url);
    let body = json!({
        "model": creds.model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ],
        "temperature": 0.35,
        "max_tokens": 4096
    });
    let client = Client::builder()
        .timeout(Duration::from_secs(creds.timeout_secs.max(5)))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let resp = client
        .post(&url)
        .bearer_auth(&creds.api_key)
        .json(&body)
        .send()
        .map_err(|e| format!("LLM 请求失败: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let t = resp.text().unwrap_or_default();
        return Err(format!("LLM 返回 {}: {}", status, truncate(&t, 800)));
    }
    let v: Value = resp.json().map_err(|e| format!("解析 LLM JSON: {e}"))?;
    extract_assistant_text(&v).ok_or_else(|| {
        format!(
            "LLM 响应缺少正文: {}",
            truncate(&serde_json::to_string(&v).unwrap_or_default(), 400)
        )
    })
}

fn extract_assistant_text(v: &Value) -> Option<String> {
    let t = v.pointer("/choices/0/message/content")?.as_str()?.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "…"
}

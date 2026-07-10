//! Prefer the internal clawd text LLM gateway when invoked by clawd.
//! Standalone execution falls back to `OPENAI_*` or `WORKSPACE_ROOT/configs/config.toml`.

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

#[derive(Debug, Clone)]
pub struct LlmTextOutput {
    pub text: String,
    pub source: &'static str,
    pub model: String,
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
    model: String,
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

/// Prefer non-empty `OPENAI_API_KEY` + `OPENAI_BASE_URL` + `OPENAI_MODEL`.
/// Otherwise read the selected vendor table from `configs/config.toml`.
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
        format!(
            "code=llm_credentials_missing reason=workspace_config_not_found config_rel={CONFIG_REL} env=WORKSPACE_ROOT"
        )
    })?;
    let raw = std::fs::read_to_string(root.join(CONFIG_REL))
        .map_err(|e| format!("code=config_read_failed path={CONFIG_REL} error={e}"))?;
    let cfg: ConfigToml = toml::from_str(&raw)
        .map_err(|e| format!("code=config_parse_failed path={CONFIG_REL} error={e}"))?;
    let vendor = cfg
        .llm
        .selected_vendor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "code=selected_vendor_missing field=llm.selected_vendor env_openai_api_key=empty"
                .to_string()
        })?;
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
                "code=unsupported_llm_vendor vendor={vendor} supported=openai,minimax,mimo,deepseek,qwen,custom,grok fallback_env=OPENAI_* offline_arg=use_heuristic"
            ));
        }
    }
    .ok_or_else(|| format!("code=vendor_config_missing section=llm.{vnorm} path={CONFIG_REL}"))?;

    let api_key = row.api_key.trim();
    if api_key.is_empty() {
        return Err(format!(
            "code=vendor_api_key_empty section=llm.{vnorm} field=api_key offline_arg=use_heuristic"
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

pub fn chat_completion_default(system: &str, user: &str) -> Result<LlmTextOutput, String> {
    if let Some(result) = internal_chat_completion(system, user) {
        return result;
    }
    let creds = resolve_llm_credentials()?;
    let text = chat_completion(&creds, system, user)?;
    Ok(LlmTextOutput {
        text,
        source: creds.source,
        model: creds.model,
    })
}

fn internal_chat_completion(system: &str, user: &str) -> Option<Result<LlmTextOutput, String>> {
    let url = std::env::var("RUSTCLAW_INTERNAL_LLM_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let token = std::env::var("RUSTCLAW_INTERNAL_LLM_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let timeout_secs = std::env::var("SKILL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(60)
        .min(120);
    let body = json!({
        "skill_name": "invest_copy",
        "prompt_source": "skills/invest_copy/draft",
        "system": system,
        "user": user,
        "temperature": 0.35,
        "max_tokens": 4096
    });
    let result = (|| {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs.max(5)))
            .build()
            .map_err(|e| format!("code=internal_llm_client_build_failed error={e}"))?;
        let resp = client
            .post(url)
            .header("x-rustclaw-internal-llm-token", token)
            .json(&body)
            .send()
            .map_err(|e| format!("code=internal_llm_request_failed error={e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let t = resp.text().unwrap_or_default();
            return Err(format!(
                "code=internal_llm_http_error status={} body={}",
                status,
                truncate(&t, 800)
            ));
        }
        let parsed: InternalLlmApiResponse = resp
            .json()
            .map_err(|e| format!("code=internal_llm_json_parse_failed error={e}"))?;
        if !parsed.ok {
            return Err(parsed
                .error
                .unwrap_or_else(|| "code=internal_llm_call_failed".to_string()));
        }
        let data = parsed
            .data
            .ok_or_else(|| "code=internal_llm_missing_data".to_string())?;
        if data.text.trim().is_empty() {
            return Err("code=internal_llm_empty_text".to_string());
        }
        Ok(LlmTextOutput {
            text: data.text,
            source: "clawd_internal",
            model: data.model,
        })
    })();
    Some(result)
}

/// OpenAI-compatible `POST /chat/completions`.
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
        .map_err(|e| format!("code=llm_client_build_failed error={e}"))?;
    let resp = client
        .post(&url)
        .bearer_auth(&creds.api_key)
        .json(&body)
        .send()
        .map_err(|e| format!("code=llm_request_failed error={e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let t = resp.text().unwrap_or_default();
        return Err(format!(
            "code=llm_http_error status={} body={}",
            status,
            truncate(&t, 800)
        ));
    }
    let v: Value = resp
        .json()
        .map_err(|e| format!("code=llm_json_parse_failed error={e}"))?;
    extract_assistant_text(&v).ok_or_else(|| {
        format!(
            "code=llm_empty_assistant_text body={}",
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

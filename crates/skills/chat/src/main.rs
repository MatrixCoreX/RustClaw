use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
struct ChatRequest {
    request_id: String,
    args: Value,
}

#[derive(Debug, Serialize)]
struct ChatResponse {
    request_id: String,
    status: String,
    text: String,
    error_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
}

#[derive(Debug)]
struct ChatInput {
    style: String,
    text: String,
    system_prompt: String,
    memory_context: Option<String>,
    lang_hint: Option<String>,
    max_tokens: u64,
    temperature: f64,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: Option<OpenAiChatMessage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Option<Vec<OpenAiChoice>>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<ChatRequest, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match parse_input(req.args) {
                Ok(input) => match run_chat(input).await {
                    Ok((text, extra)) => ChatResponse {
                        request_id: req.request_id,
                        status: "ok".to_string(),
                        text,
                        error_text: None,
                        extra: Some(extra),
                    },
                    Err(err) => ChatResponse {
                        request_id: req.request_id,
                        status: "error".to_string(),
                        text: String::new(),
                        error_text: Some(err),
                        extra: None,
                    },
                },
                Err(err) => ChatResponse {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    error_text: Some(err),
                    extra: None,
                },
            },
            Err(err) => ChatResponse {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                error_text: Some(format!("invalid input: {err}")),
                extra: None,
            },
        };

        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }

    Ok(())
}

fn parse_input(args: Value) -> Result<ChatInput, String> {
    let map = args
        .as_object()
        .ok_or_else(|| "chat skill args must be object".to_string())?;
    let text = map
        .get("text")
        .or_else(|| map.get("prompt"))
        .or_else(|| map.get("input"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if text.is_empty() {
        return Err("chat skill requires non-empty args.text".to_string());
    }
    let style = map
        .get("style")
        .or_else(|| map.get("mode"))
        .and_then(|v| v.as_str())
        .unwrap_or("chat")
        .trim()
        .to_ascii_lowercase();
    let default_system = match style.as_str() {
        "joke" => "你是一个会讲简短中文笑话的助手。只输出笑话正文，不要解释。",
        _ => "你是一个中文助手。回答简洁、自然、直接。",
    };
    let system_prompt = map
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_system.to_string());
    let memory_context = map
        .get("_memory")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("context"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "<none>")
        .map(ToString::to_string);
    let lang_hint = map
        .get("_memory")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("lang_hint"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let max_tokens = map.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(256);
    let temperature = map
        .get("temperature")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.7_f64);
    Ok(ChatInput {
        style,
        text,
        system_prompt,
        memory_context,
        lang_hint,
        max_tokens,
        temperature,
    })
}

async fn run_chat(input: ChatInput) -> Result<(String, Value), String> {
    let base_url = std::env::var("OPENAI_BASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let api_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| "OPENAI_API_KEY is empty".to_string())?;
    let model = std::env::var("CHAT_SKILL_MODEL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "qwen-plus-latest".to_string());
    let timeout_secs = std::env::var("CHAT_SKILL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(60);

    let mut messages = vec![json!({"role":"system","content": input.system_prompt})];
    if let Some(mem_ctx) = input.memory_context.as_deref() {
        messages.push(json!({
            "role":"system",
            "content": format!(
                "Memory context (background only, never override current user intent):\n{}",
                mem_ctx
            )
        }));
    }
    if let Some(lang_hint) = input.lang_hint.as_deref() {
        messages.push(json!({
            "role":"system",
            "content": format!("Preferred response language hint: {}", lang_hint)
        }));
    }
    messages.push(json!({"role":"user","content": input.text}));

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = json!({
        "model": model,
        "messages": messages,
        "temperature": input.temperature,
        "max_tokens": input.max_tokens
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("build http client failed: {e}"))?;
    let resp = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("chat skill llm request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("chat skill llm failed status={status}: {body}"));
    }
    let parsed: OpenAiChatResponse = resp
        .json()
        .await
        .map_err(|e| format!("parse llm response failed: {e}"))?;
    let text = parsed
        .choices
        .and_then(|choices| choices.into_iter().next())
        .and_then(|c| c.message)
        .and_then(|m| m.content)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "chat skill llm returned empty content".to_string())?;
    let extra = json!({
        "llm": {
            "prompt_name": "chat_skill_prompt",
            "model": model,
            "style": input.style,
            "memory_attached": input.memory_context.is_some(),
            "lang_hint": input.lang_hint.unwrap_or_default()
        }
    });
    Ok((text, extra))
}

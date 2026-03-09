use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_CHAT_SYSTEM_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/chat_skill_system_prompt.md");
const DEFAULT_CHAT_JOKE_SYSTEM_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/chat_skill_joke_system_prompt.md");
const CHAT_SYSTEM_PROMPT_PATH: &str = "prompts/chat_skill_system_prompt.md";
const CHAT_JOKE_SYSTEM_PROMPT_PATH: &str = "prompts/chat_skill_joke_system_prompt.md";

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
    prompt_file: String,
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
    let workspace_root = workspace_root();
    let (default_system, prompt_file) = match style.as_str() {
        "joke" => (
            load_prompt_template(
            &workspace_root,
            CHAT_JOKE_SYSTEM_PROMPT_PATH,
            DEFAULT_CHAT_JOKE_SYSTEM_PROMPT_TEMPLATE,
            ),
            CHAT_JOKE_SYSTEM_PROMPT_PATH.to_string(),
        ),
        _ => (
            load_prompt_template(
            &workspace_root,
            CHAT_SYSTEM_PROMPT_PATH,
            DEFAULT_CHAT_SYSTEM_PROMPT_TEMPLATE,
            ),
            CHAT_SYSTEM_PROMPT_PATH.to_string(),
        ),
    };
    let explicit_system_prompt = map
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let prompt_file = if explicit_system_prompt.is_some() {
        "inline_system_prompt".to_string()
    } else {
        prompt_file
    };
    let system_prompt = explicit_system_prompt.unwrap_or(default_system);
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
        prompt_file,
        memory_context,
        lang_hint,
        max_tokens,
        temperature,
    })
}

async fn run_chat(input: ChatInput) -> Result<(String, Value), String> {
    eprintln!(
        "skill_prompt_use skill=chat style={} prompt_file={}",
        input.style, input.prompt_file
    );
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
            "prompt_file": input.prompt_file,
            "model": model,
            "style": input.style,
            "memory_attached": input.memory_context.is_some(),
            "lang_hint": input.lang_hint.unwrap_or_default()
        }
    });
    Ok((text, extra))
}

fn load_prompt_template(workspace_root: &Path, rel_path: &str, default_template: &str) -> String {
    let path = workspace_root.join(rel_path);
    match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => default_template.to_string(),
    }
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

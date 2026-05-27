use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    #[allow(dead_code)]
    #[serde(default)]
    context: Option<Value>,
    #[allow(dead_code)]
    #[serde(default)]
    user_id: i64,
    #[allow(dead_code)]
    #[serde(default)]
    chat_id: i64,
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

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args) {
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
                    extra: Some(json!({"error_kind": err.kind})),
                    error_text: Some(err.message),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(json!({"error_kind":"invalid_input"})),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }

    Ok(())
}

struct SkillErr {
    kind: &'static str,
    message: String,
}

impl SkillErr {
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

fn execute(args: Value) -> Result<(String, Value), SkillErr> {
    let obj = args
        .as_object()
        .ok_or_else(|| SkillErr::new("invalid_args", "args must be object"))?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SkillErr::new("missing_action", "action is required"))?;

    match action {
        "ping" => Ok((
            "pong".to_string(),
            json!({"action":"ping","ok":true,"message":"pong"}),
        )),
        _ => Err(SkillErr::new(
            "unsupported_action",
            format!("unsupported action: {action}; use \"ping\""),
        )),
    }
}

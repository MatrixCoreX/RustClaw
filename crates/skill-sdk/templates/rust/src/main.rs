use std::io::{self, BufRead};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize)]
struct Request {
    request_id: String,
    #[allow(dead_code)]
    args: Value,
}

#[derive(Serialize)]
struct Response {
    request_id: String,
    status: &'static str,
    text: String,
    error_text: Option<String>,
    extra: Value,
}

fn respond(request: Request) -> Response {
    Response {
        request_id: request.request_id,
        status: "ok",
        text: String::new(),
        error_text: None,
        extra: json!({"result": {"handled": true}}),
    }
}

fn main() {
    let mut lines = io::stdin().lock().lines();
    let response = match lines.next() {
        Some(Ok(line)) => serde_json::from_str::<Request>(&line)
            .map(respond)
            .unwrap_or_else(|error| Response {
                request_id: "invalid".to_string(),
                status: "error",
                text: String::new(),
                error_text: Some(error.to_string()),
                extra: json!({
                    "error_code": "request_invalid",
                    "message_key": "skill.request_invalid"
                }),
            }),
        _ => Response {
            request_id: "missing".to_string(),
            status: "error",
            text: String::new(),
            error_text: Some("request record is required".to_string()),
            extra: json!({
                "error_code": "request_missing",
                "message_key": "skill.request_missing"
            }),
        },
    };
    println!("{}", serde_json::to_string(&response).expect("response JSON"));
}

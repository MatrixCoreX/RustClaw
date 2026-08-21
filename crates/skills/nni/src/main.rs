use std::io::{self, BufRead, Write};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const SKILL_NAME: &str = "nni";

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Action {
    Status,
    DeviceStatus,
    HeartbeatStatus,
    HeartbeatEnable,
    HeartbeatDisable,
    HeartbeatNow,
    NetworkStats,
    MyRewards,
    RewardApr,
    BancorMarket,
    BancorAccount,
    BancorMarketTrades,
    BancorCandles,
    BancorQuote,
}

impl Action {
    fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::DeviceStatus => "device_status",
            Self::HeartbeatStatus => "heartbeat_status",
            Self::HeartbeatEnable => "heartbeat_enable",
            Self::HeartbeatDisable => "heartbeat_disable",
            Self::HeartbeatNow => "heartbeat_now",
            Self::NetworkStats => "network_stats",
            Self::MyRewards => "my_rewards",
            Self::RewardApr => "reward_apr",
            Self::BancorMarket => "bancor_market",
            Self::BancorAccount => "bancor_account",
            Self::BancorMarketTrades => "bancor_market_trades",
            Self::BancorCandles => "bancor_candles",
            Self::BancorQuote => "bancor_quote",
        }
    }

    fn mutates(self) -> bool {
        matches!(
            self,
            Self::HeartbeatEnable | Self::HeartbeatDisable | Self::HeartbeatNow
        )
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Args {
    action: Action,
    #[serde(default, rename = "_memory", skip_serializing)]
    _memory: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_price_usd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    interval: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end_time_ts: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    side: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pay_asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pay_amount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slippage_bps: Option<u16>,
}

impl Args {
    fn validate(&self) -> Result<(), SkillError> {
        let mut supplied = Vec::new();
        if self.limit.is_some() {
            supplied.push("limit");
        }
        if self.device_price_usd.is_some() {
            supplied.push("device_price_usd");
        }
        if self.interval.is_some() {
            supplied.push("interval");
        }
        if self.end_time_ts.is_some() {
            supplied.push("end_time_ts");
        }
        if self.side.is_some() {
            supplied.push("side");
        }
        if self.pay_asset.is_some() {
            supplied.push("pay_asset");
        }
        if self.pay_amount.is_some() {
            supplied.push("pay_amount");
        }
        if self.slippage_bps.is_some() {
            supplied.push("slippage_bps");
        }
        let allowed: &[&str] = match self.action {
            Action::MyRewards | Action::BancorAccount | Action::BancorMarketTrades => &["limit"],
            Action::RewardApr => &["device_price_usd"],
            Action::BancorCandles => &["limit", "interval", "end_time_ts"],
            Action::BancorQuote => &["side", "pay_asset", "pay_amount", "slippage_bps"],
            _ => &[],
        };
        let invalid: Vec<&str> = supplied
            .into_iter()
            .filter(|field| !allowed.contains(field))
            .collect();
        if !invalid.is_empty() {
            return Err(SkillError::new(
                "nni_argument_invalid",
                false,
                json!({"action": self.action.as_str(), "invalid_fields": invalid}),
            ));
        }
        if self.action == Action::RewardApr && self.device_price_usd.is_none() {
            return Err(SkillError::new(
                "nni_argument_invalid",
                false,
                json!({
                    "action": self.action.as_str(),
                    "missing_fields": ["device_price_usd"],
                }),
            ));
        }
        if self.action == Action::BancorQuote {
            let missing: Vec<&str> = [
                ("side", self.side.is_none()),
                ("pay_amount", self.pay_amount.is_none()),
            ]
            .into_iter()
            .filter_map(|(field, absent)| absent.then_some(field))
            .collect();
            if !missing.is_empty() {
                return Err(SkillError::new(
                    "nni_argument_invalid",
                    false,
                    json!({"action": self.action.as_str(), "missing_fields": missing}),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct Request {
    request_id: String,
    args: Args,
    #[allow(dead_code)]
    user_id: i64,
    #[allow(dead_code)]
    chat_id: i64,
    #[serde(default)]
    #[allow(dead_code)]
    context: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RequestEnvelope {
    request_id: String,
    args: Value,
    user_id: i64,
    chat_id: i64,
    #[serde(default)]
    context: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    request_id: String,
    status: String,
    text: String,
    error_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    ok: bool,
    data: Option<Value>,
    error: Option<String>,
}

fn gateway_error_from_response(
    status: reqwest::StatusCode,
    payload: ApiResponse,
    action: Action,
) -> SkillError {
    let gateway_error = payload.error;
    let envelope = payload.data.unwrap_or(Value::Null);
    let code = envelope
        .get("error_code")
        .and_then(Value::as_str)
        .unwrap_or("nni_internal_request_failed")
        .to_string();
    let retryable = envelope
        .get("retryable")
        .and_then(Value::as_bool)
        .unwrap_or(status.is_server_error());
    let details = envelope.get("details").cloned().unwrap_or_else(|| {
        json!({
            "http_status": status.as_u16(),
            "gateway_envelope": envelope.clone(),
            "gateway_error": gateway_error,
        })
    });
    let has_effect_evidence = envelope.get("failure_phase").is_some()
        || envelope.get("side_effect_applied").is_some()
        || envelope.get("recovery_action").is_some();
    if has_effect_evidence {
        SkillError::from_gateway(code, retryable, details, &envelope)
    } else {
        response_contract_error(action, code, retryable, details)
    }
}

#[derive(Debug)]
struct SkillError {
    code: String,
    retryable: bool,
    details: Value,
    failure_phase: Option<String>,
    side_effect_applied: Option<bool>,
    recovery_action: Option<String>,
}

impl SkillError {
    fn new(code: impl Into<String>, retryable: bool, details: Value) -> Self {
        Self {
            code: code.into(),
            retryable,
            details,
            failure_phase: Some("pre_dispatch".to_string()),
            side_effect_applied: Some(false),
            recovery_action: Some("replan_arguments".to_string()),
        }
    }

    fn uncertain(code: impl Into<String>, retryable: bool, details: Value) -> Self {
        Self {
            code: code.into(),
            retryable,
            details,
            failure_phase: None,
            side_effect_applied: None,
            recovery_action: Some("reconcile_before_retry".to_string()),
        }
    }

    fn from_gateway(
        code: impl Into<String>,
        retryable: bool,
        details: Value,
        envelope: &Value,
    ) -> Self {
        Self {
            code: code.into(),
            retryable,
            details,
            failure_phase: envelope
                .get("failure_phase")
                .and_then(Value::as_str)
                .map(str::to_string),
            side_effect_applied: envelope.get("side_effect_applied").and_then(Value::as_bool),
            recovery_action: envelope
                .get("recovery_action")
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    }

    fn into_response(self, request_id: String, action: Option<Action>) -> Response {
        let message_key = format!("skill.{SKILL_NAME}.{}", self.code);
        Response {
            request_id,
            status: "error".to_string(),
            text: String::new(),
            error_text: Some(self.code.clone()),
            extra: Some(json!({
                "schema_version": 1,
                "source_skill": SKILL_NAME,
                "status": "error",
                "action": action.map(Action::as_str),
                "error_code": self.code,
                "message_key": message_key,
                "retryable": self.retryable,
                "failure_phase": self.failure_phase,
                "side_effect_applied": self.side_effect_applied,
                "recovery_action": self.recovery_action,
                "details": self.details,
            })),
        }
    }
}

fn response_contract_error(
    action: Action,
    code: impl Into<String>,
    retryable: bool,
    details: Value,
) -> SkillError {
    if action.mutates() {
        SkillError::uncertain(code, retryable, details)
    } else {
        SkillError::new(code, retryable, details)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let response = handle_line(&line?).await;
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    Ok(())
}

async fn handle_line(line: &str) -> Response {
    let envelope = match serde_json::from_str::<RequestEnvelope>(line) {
        Ok(envelope) => envelope,
        Err(error) => {
            return SkillError::new(
                "nni_invalid_input",
                false,
                json!({"decode_error": error.to_string()}),
            )
            .into_response("unknown".to_string(), None)
        }
    };
    let request_id = envelope.request_id.clone();
    let args = match serde_json::from_value::<Args>(envelope.args) {
        Ok(args) => args,
        Err(error) => {
            return SkillError::new(
                "nni_invalid_input",
                false,
                json!({"decode_error": error.to_string()}),
            )
            .into_response(request_id, None)
        }
    };
    let request = Request {
        request_id: envelope.request_id,
        args,
        user_id: envelope.user_id,
        chat_id: envelope.chat_id,
        context: envelope.context,
    };
    let request_id = request.request_id.clone();
    let action = request.args.action;
    if let Err(error) = request.args.validate() {
        return error.into_response(request_id, Some(action));
    }
    match call_internal_gateway(&request.args).await {
        Ok(extra) => Response {
            request_id,
            status: "ok".to_string(),
            text: extra.to_string(),
            error_text: None,
            extra: Some(extra),
        },
        Err(error) => error.into_response(request_id, Some(action)),
    }
}

async fn call_internal_gateway(args: &Args) -> Result<Value, SkillError> {
    let url = required_env_value(
        "AGENT_INTERNAL_NNI_URL",
        std::env::var("AGENT_INTERNAL_NNI_URL").ok(),
    )?;
    let token = required_env_value(
        "AGENT_INTERNAL_NNI_TOKEN",
        std::env::var("AGENT_INTERNAL_NNI_TOKEN").ok(),
    )?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(skill_timeout_seconds()))
        .build()
        .map_err(|error| {
            SkillError::new(
                "nni_internal_client_failed",
                false,
                json!({"detail": error.to_string()}),
            )
        })?;
    let response = client
        .post(url)
        .header(
            claw_core::product_identity::INTERNAL_SKILL_TOKEN_HEADER,
            token,
        )
        .json(args)
        .send()
        .await
        .map_err(|error| {
            response_contract_error(
                args.action,
                "nni_internal_gateway_unavailable",
                true,
                json!({"detail": error.to_string()}),
            )
        })?;
    let status = response.status();
    let payload = response.json::<ApiResponse>().await.map_err(|error| {
        response_contract_error(
            args.action,
            "nni_internal_response_invalid",
            false,
            json!({"http_status": status.as_u16(), "detail": error.to_string()}),
        )
    })?;
    if payload.ok && status.is_success() {
        let envelope = payload.data.ok_or_else(|| {
            response_contract_error(
                args.action,
                "nni_internal_response_invalid",
                false,
                json!({"http_status": status.as_u16(), "missing_fields": ["data"]}),
            )
        })?;
        validate_success_envelope(&envelope, args.action)?;
        return Ok(envelope);
    }
    Err(gateway_error_from_response(status, payload, args.action))
}

fn validate_success_envelope(envelope: &Value, expected_action: Action) -> Result<(), SkillError> {
    let valid = envelope.get("schema_version").and_then(Value::as_u64) == Some(1)
        && envelope.get("source_skill").and_then(Value::as_str) == Some(SKILL_NAME)
        && envelope.get("status").and_then(Value::as_str) == Some("ok")
        && envelope.get("action").and_then(Value::as_str) == Some(expected_action.as_str())
        && envelope.get("data").is_some();
    if valid {
        Ok(())
    } else {
        Err(response_contract_error(
            expected_action,
            "nni_internal_response_invalid",
            false,
            json!({"expected_action": expected_action.as_str()}),
        ))
    }
}

fn required_env_value(name: &str, value: Option<String>) -> Result<String, SkillError> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SkillError::new(
                "nni_internal_gateway_unavailable",
                false,
                json!({"missing_environment": name}),
            )
        })
}

fn skill_timeout_seconds() -> u64 {
    std::env::var("SKILL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(30)
}

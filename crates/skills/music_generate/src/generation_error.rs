use serde_json::{json, Value};

#[derive(Debug)]
pub(super) struct GenerationError {
    pub message: String,
    http_status: Option<u16>,
    provider_status_code: Option<i64>,
    provider: Option<String>,
    model: Option<String>,
    rejected_without_generation: bool,
}

impl From<String> for GenerationError {
    fn from(message: String) -> Self {
        Self {
            message,
            http_status: None,
            provider_status_code: None,
            provider: None,
            model: None,
            rejected_without_generation: false,
        }
    }
}

impl GenerationError {
    pub fn provider_response(status: u16, value: &Value) -> Self {
        let code = value
            .pointer("/base_resp/status_code")
            .and_then(Value::as_i64);
        let has_result = ["/data/audio", "/data/task_id", "/task_id", "/job_id"]
            .iter()
            .any(|path| value.pointer(path).is_some_and(|v| !v.is_null()));
        Self {
            message: format!(
                "minimax music failed status={status}: {}",
                super::truncate(&value.to_string(), 400)
            ),
            http_status: Some(status),
            provider_status_code: code,
            provider: None,
            model: None,
            // This explicit API-unavailable response is not a transport failure.
            rejected_without_generation: status == 410 && code == Some(2153) && !has_result,
        }
    }

    pub fn with_provider(mut self, provider: &str, model: &str) -> Self {
        self.provider = Some(provider.to_string());
        self.model = Some(model.to_string());
        self
    }

    pub fn extra(&self) -> Value {
        let mut extra = super::error_extra(if self.rejected_without_generation {
            "provider_capability_unavailable"
        } else {
            "execution_failed"
        });
        if let Some(provider) = &self.provider {
            extra["provider"] = json!(provider);
        }
        if let Some(model) = &self.model {
            extra["model"] = json!(model);
        }
        if let Some(status) = self.http_status {
            extra["status_code"] = json!(status);
        }
        if let Some(code) = self.provider_status_code {
            extra["provider_status_code"] = json!(code);
        }
        if self.rejected_without_generation {
            extra["failure_phase"] = json!("provider_rejected");
            extra["side_effect_applied"] = json!(false);
        }
        extra
    }
}

#[cfg(test)]
#[path = "generation_error_tests.rs"]
mod tests;

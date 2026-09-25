#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PolicyBlockError {
    pub(crate) decision: String,
    pub(crate) reason_code: String,
    pub(crate) observed_facts: Vec<String>,
    pub(crate) policy_boundary: Vec<String>,
}

fn structured_extra_value<'a>(
    structured: &'a StructuredSkillError,
    key: &str,
) -> Option<&'a Value> {
    structured.extra.as_ref()?.get(key)
}

fn structured_extra_string(structured: &StructuredSkillError, key: &str) -> Option<String> {
    structured_extra_value(structured, key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn structured_error_allows_bounded_replan(structured: &StructuredSkillError) -> bool {
    let Some(extra) = structured.extra.as_ref().and_then(Value::as_object) else {
        return false;
    };
    extra.get("retryable").and_then(Value::as_bool) == Some(true)
        && structured_error_proves_not_applied(structured)
        && extra.get("recovery_action").and_then(Value::as_str) == Some("replan_arguments")
}

fn structured_error_proves_not_applied(structured: &StructuredSkillError) -> bool {
    let Some(extra) = structured.extra.as_ref().and_then(Value::as_object) else {
        return false;
    };
    extra.get("side_effect_applied").and_then(Value::as_bool) == Some(false)
        && matches!(
            extra.get("failure_phase").and_then(Value::as_str),
            Some("pre_dispatch" | "provider_rejected" | "execution_no_effect")
        )
}

pub(crate) fn structured_skill_error_proves_not_applied(err: &str) -> bool {
    parse_structured_skill_error(err)
        .as_ref()
        .is_some_and(structured_error_proves_not_applied)
}

pub(crate) fn structured_skill_error_requests_replan(err: &str) -> bool {
    parse_structured_skill_error(err)
        .as_ref()
        .is_some_and(structured_error_allows_bounded_replan)
}

pub(crate) fn policy_block_error(
    reason_code: &str,
    observed_facts: Vec<String>,
    policy_boundary: Vec<String>,
) -> String {
    let decision = crate::policy_decision::PolicyDecision::Deny.as_token();
    let payload = json!({
        "decision": decision,
        "reason_code": reason_code.trim(),
        "permission_decision": {
            "decision": decision,
            "denied_by_policy": true,
            "needs_confirmation": false,
            "background_wait": false,
        },
        "observed_facts": observed_facts,
        "policy_boundary": policy_boundary,
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
    format!("{POLICY_BLOCK_ERROR_PREFIX}{encoded}")
}

pub(crate) fn parse_policy_block_error(err: &str) -> Option<PolicyBlockError> {
    let payload = err.trim().strip_prefix(POLICY_BLOCK_ERROR_PREFIX)?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    let reason_code = value
        .get("reason_code")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_string();
    let decision = value
        .get("decision")
        .and_then(|v| v.as_str())
        .or_else(|| {
            value
                .get("permission_decision")
                .and_then(|v| v.get("decision"))
                .and_then(|v| v.as_str())
        })
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(crate::policy_decision::PolicyDecision::Deny.as_token())
        .to_string();
    let strings_from_array = |key: &str| -> Vec<String> {
        value
            .get(key)
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::trim))
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(PolicyBlockError {
        decision,
        reason_code,
        observed_facts: strings_from_array("observed_facts"),
        policy_boundary: strings_from_array("policy_boundary"),
    })
}

fn policy_block_message_key(reason_code: &str) -> String {
    let normalized = reason_code
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "clawd.msg.policy.unknown".to_string()
    } else {
        format!("clawd.msg.policy.{normalized}")
    }
}

fn policy_observed_facts_value(facts: &[String]) -> Value {
    let mut object = Map::new();
    let mut unparsed = Vec::new();
    for fact in facts {
        let fact = fact.trim();
        if fact.is_empty() {
            continue;
        }
        if let Some((key, value)) = fact.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            if !key.is_empty() && !value.is_empty() {
                object.insert(key.to_string(), json!(value));
                continue;
            }
        }
        unparsed.push(fact.to_string());
    }
    if !unparsed.is_empty() {
        object.insert("unparsed".to_string(), json!(unparsed));
    }
    Value::Object(object)
}

fn policy_block_machine_payload(block: &PolicyBlockError) -> String {
    json!({
        "message_key": policy_block_message_key(&block.reason_code),
        "decision": &block.decision,
        "reason_code": block.reason_code,
        "permission_decision": {
            "decision": &block.decision,
            "denied_by_policy": true,
            "needs_confirmation": false,
            "background_wait": false,
        },
        "observed_facts": policy_observed_facts_value(&block.observed_facts),
        "policy_boundary_count": block.policy_boundary.len(),
    })
    .to_string()
}

fn parse_crypto_account_access_error(err: &str) -> Option<(String, String)> {
    let payload = err
        .trim()
        .strip_prefix(CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX)?;
    if let Ok(value) = serde_json::from_str::<Value>(payload) {
        let exchange = value
            .get("exchange")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let detail = value
            .get("detail")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("private exchange account access failed")
            .to_string();
        return Some((exchange, detail));
    }
    let detail = payload.trim();
    Some((
        String::new(),
        if detail.is_empty() {
            "private exchange account access failed".to_string()
        } else {
            detail.to_string()
        },
    ))
}

fn crypto_account_access_error_from_structured_extra(
    structured: &StructuredSkillError,
) -> Option<(String, String)> {
    let message_key = structured_extra_string(structured, "message_key");
    let structured_kind = structured.error_code.trim();
    let is_account_access = structured_kind == "account_access_failed"
        || structured_kind == "crypto_account_access_failed"
        || message_key.as_deref() == Some("crypto.err.account_access_failed");
    if !is_account_access {
        return None;
    }

    let legacy = parse_crypto_account_access_error(&structured.error_text);
    let exchange = structured_extra_string(structured, "exchange")
        .or_else(|| legacy.as_ref().map(|(exchange, _)| exchange.clone()))
        .unwrap_or_default();
    let detail = structured_extra_string(structured, "detail")
        .or_else(|| legacy.as_ref().map(|(_, detail)| detail.clone()))
        .unwrap_or_else(|| structured.error_text.trim().to_string())
        .trim()
        .to_string();
    Some((exchange, detail))
}

fn structured_crypto_account_access_error(
    skill_name: &str,
    structured: &StructuredSkillError,
) -> Option<(String, String)> {
    let effective_skill = if structured.skill.trim().is_empty() {
        skill_name
    } else {
        structured.skill.as_str()
    };
    if !effective_skill.eq_ignore_ascii_case("crypto") {
        return None;
    }
    if let Some(error) = crypto_account_access_error_from_structured_extra(structured) {
        return Some(error);
    }
    parse_crypto_account_access_error(&structured.error_text)
}

fn is_crypto_recoverable_i18n_message_key(message_key: &str) -> bool {
    matches!(
        message_key.trim(),
        "crypto.err.binance_not_bound"
            | "crypto.err.binance_credentials_incomplete"
            | "crypto.err.okx_not_bound"
            | "crypto.err.okx_credentials_incomplete"
    )
}

fn crypto_recoverable_i18n_error_from_structured(
    skill_name: &str,
    structured: &StructuredSkillError,
) -> Option<(String, String, String, String)> {
    let effective_skill = if structured.skill.trim().is_empty() {
        skill_name
    } else {
        structured.skill.as_str()
    };
    if !effective_skill.eq_ignore_ascii_case("crypto") {
        return None;
    }
    let message_key = structured_extra_string(structured, "message_key")?;
    if !is_crypto_recoverable_i18n_message_key(&message_key) {
        return None;
    }
    let error_code = structured.error_code.trim().to_string();
    let exchange = structured_extra_string(structured, "exchange").unwrap_or_default();
    let action = structured_extra_string(structured, "action").unwrap_or_default();
    Some((message_key, error_code, exchange, action))
}

pub(crate) fn policy_block_default_text(
    _state: &AppState,
    _task: &ClaimedTask,
    _user_text: &str,
    block: &PolicyBlockError,
) -> String {
    policy_block_machine_payload(block)
}

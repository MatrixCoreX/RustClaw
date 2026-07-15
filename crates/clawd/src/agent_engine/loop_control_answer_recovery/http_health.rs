use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpHealthFinding {
    status_code: Option<u64>,
    ok: Option<bool>,
    version: Option<String>,
    uptime_seconds: Option<u64>,
    running_length: Option<u64>,
    channel_gateway_healthy: Option<bool>,
    telegram_bot_healthy: Option<bool>,
    gateway_instances: Vec<HealthGatewayInstance>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HealthGatewayInstance {
    kind: String,
    name: String,
    status: String,
    healthy: Option<bool>,
}

pub(in crate::agent_engine::loop_control) fn try_recover_http_health_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !(route_requests_http_health_recovery(route))
        || route.output_contract.locator_kind != crate::OutputLocatorKind::Url
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap()
        || !verifier.missing_evidence_fields.iter().any(|field| {
            matches!(
                field.as_str(),
                "content_excerpt"
                    | "command_output"
                    | "output_format"
                    | "unsupported_claims"
                    | "any_of(command_output|content_excerpt|field_value)"
                    | "any_of(command_output|content_excerpt|count|field_value)"
            )
        })
    {
        return false;
    }
    let Some(finding) = observed_http_health_finding(reply) else {
        return false;
    };
    let (answer, reason) = latest_publishable_http_health_synthesis(reply)
        .map(|answer| (answer, "http_health_synthesis"))
        .unwrap_or_else(|| {
            (
                deterministic_http_health_status_line(&finding),
                "http_health_structured_status",
            )
        });
    if let Some(journal) = reply.task_journal.as_mut() {
        mark_answer_verifier_recovery_success(journal, &answer);
    }
    reply.text = answer.clone();
    reply.messages = vec![answer];
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!("answer_verifier_retry_exhausted_recovered_with_{reason}");
    true
}

fn route_requests_http_health_recovery(route: &crate::RouteResult) -> bool {
    route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::CommandOutputSummary,
        crate::OutputSemanticKind::ServiceStatus,
    ]) || (crate::machine_capability_ref::route_has_capability_action(
        route,
        &["browser", "http"],
        &["open", "get", "read", "extract"],
    ) && !crate::machine_capability_ref::route_has_capability_action(
        route,
        &["browser", "web"],
        &["search"],
    ))
}

fn observed_http_health_finding(reply: &AskReply) -> Option<HttpHealthFinding> {
    reply
        .task_journal
        .as_ref()?
        .step_results
        .iter()
        .rev()
        .filter(|step| {
            step.skill == "http_basic" && step.status == crate::executor::StepExecutionStatus::Ok
        })
        .filter_map(|step| step.output_excerpt.as_deref())
        .find_map(parse_http_health_finding)
}

fn latest_publishable_http_health_synthesis(reply: &AskReply) -> Option<String> {
    let answer = reply
        .task_journal
        .as_ref()?
        .step_results
        .iter()
        .rev()
        .find(|step| {
            step.skill == "synthesize_answer"
                && step.status == crate::executor::StepExecutionStatus::Ok
                && step
                    .output_excerpt
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty())
        })?
        .output_excerpt
        .as_deref()?
        .trim();
    if answer.is_empty()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || crate::finalize::is_execution_summary_message(answer)
        || looks_like_machine_key_value_status_line(answer)
    {
        return None;
    }
    Some(answer.to_string())
}

fn looks_like_machine_key_value_status_line(answer: &str) -> bool {
    let mut token_count = 0usize;
    let mut kv_count = 0usize;
    for token in answer.split_whitespace() {
        token_count += 1;
        if token.contains('=') {
            kv_count += 1;
        }
    }
    token_count > 0 && kv_count >= 3 && kv_count.saturating_mul(2) >= token_count
}

fn parse_http_health_finding(output: &str) -> Option<HttpHealthFinding> {
    let value = serde_json::from_str::<Value>(output).ok()?;
    let extra = value.get("extra").and_then(Value::as_object)?;
    let url = extra.get("url").and_then(Value::as_str)?;
    if !url.contains("/v1/health") {
        return None;
    }
    let status_code = extra.get("status_code").and_then(Value::as_u64);
    let body = extra.get("body_json").cloned().or_else(|| {
        extra
            .get("body_preview")
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
    })?;
    let data = body.get("data")?;
    Some(HttpHealthFinding {
        status_code,
        ok: body.get("ok").and_then(Value::as_bool),
        version: data
            .get("version")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        uptime_seconds: data.get("uptime_seconds").and_then(Value::as_u64),
        running_length: data.get("running_length").and_then(Value::as_u64),
        channel_gateway_healthy: data.get("channel_gateway_healthy").and_then(Value::as_bool),
        telegram_bot_healthy: data.get("telegram_bot_healthy").and_then(Value::as_bool),
        gateway_instances: health_gateway_instances(data),
    })
}

fn health_gateway_instances(data: &Value) -> Vec<HealthGatewayInstance> {
    data.get("gateway_instance_statuses")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(HealthGatewayInstance {
                        kind: item
                            .get("kind")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())?
                            .to_string(),
                        name: item
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())?
                            .to_string(),
                        status: item
                            .get("status")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())?
                            .to_string(),
                        healthy: item.get("healthy").and_then(Value::as_bool),
                    })
                })
                .take(4)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn deterministic_http_health_status_line(finding: &HttpHealthFinding) -> String {
    let reachable = finding
        .status_code
        .is_some_and(|code| (200..400).contains(&code))
        && finding.ok.unwrap_or(true);
    let mut parts = vec![format!(
        "http_reachability={}",
        if reachable {
            "reachable"
        } else {
            "unreachable"
        }
    )];
    if let Some(status_code) = finding.status_code {
        parts.push(format!("status_code={status_code}"));
    }
    if let Some(ok) = finding.ok {
        parts.push(format!("ok={ok}"));
    }
    if let Some(version) = finding.version.as_deref() {
        parts.push(format!("version={version}"));
    }
    if let Some(uptime_seconds) = finding.uptime_seconds {
        parts.push(format!("uptime_seconds={uptime_seconds}"));
    }
    if let Some(running_length) = finding.running_length {
        parts.push(format!("running_length={running_length}"));
    }
    if let Some(channel_gateway_healthy) = finding.channel_gateway_healthy {
        parts.push(format!("channel_gateway_healthy={channel_gateway_healthy}"));
    }
    if let Some(telegram_bot_healthy) = finding.telegram_bot_healthy {
        parts.push(format!("telegram_bot_healthy={telegram_bot_healthy}"));
    }
    if !finding.gateway_instances.is_empty() {
        let statuses = finding
            .gateway_instances
            .iter()
            .map(|item| {
                format!(
                    "{}:{}:{}:{}",
                    item.kind,
                    item.name,
                    item.status,
                    item.healthy
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        parts.push(format!("gateway_instances={statuses}"));
    }
    parts.join(" ")
}

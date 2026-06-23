async fn health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<HealthResponse>>) {
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
        return (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        );
    }
    let queue_length = task_count_by_status(&state, "queued").unwrap_or_default();
    let running_length = task_count_by_status(&state, "running").unwrap_or_default();
    let running_oldest_age_seconds = oldest_running_task_age_seconds(&state).unwrap_or(0);
    let legacy_telegramd_stats = telegramd_process_stats();
    let channel_gateway_stats = channel_gateway_process_stats();
    let whatsappd_stats = whatsappd_process_stats();
    let wa_webd_stats = wa_webd_process_stats();
    let webd_stats = webd_process_stats();
    let wechatd_stats = wechatd_process_stats();
    let channel_gateway_process_count = channel_gateway_stats.map(|(count, _)| count);
    let channel_gateway_memory_rss_bytes = channel_gateway_stats.map(|(_, rss_bytes)| rss_bytes);
    let channel_gateway_healthy = channel_gateway_process_count.map(|count| count > 0);
    // Telegram 健康状态优先看 channel-gateway（新架构），
    // 但仅在其进程数 > 0 时才覆盖 legacy telegramd；否则回退到 legacy 进程统计。
    let telegram_uses_gateway_stats =
        matches!(channel_gateway_stats, Some((count, _)) if count > 0);
    let telegramd_stats = match (channel_gateway_stats, legacy_telegramd_stats) {
        (Some((count, rss_bytes)), _) if count > 0 => Some((count, rss_bytes)),
        (_, legacy) => legacy,
    };
    let telegramd_process_count = telegramd_stats.map(|(count, _)| count);
    let telegramd_memory_rss_bytes = if telegram_uses_gateway_stats {
        // channel-gateway RSS is shared by multiple adapters and cannot be attributed safely here.
        None
    } else {
        telegramd_stats.map(|(_, rss_bytes)| rss_bytes)
    };
    let telegramd_healthy = telegramd_process_count.map(|count| count > 0);
    let whatsappd_process_count_raw = whatsappd_stats.map(|(count, _)| count);
    let whatsappd_memory_rss_bytes_raw = whatsappd_stats.map(|(_, rss_bytes)| rss_bytes);
    let wa_webd_process_count_raw = wa_webd_stats.map(|(count, _)| count);
    let wa_webd_memory_rss_bytes_raw = wa_webd_stats.map(|(_, rss_bytes)| rss_bytes);
    let webd_process_count = webd_stats.map(|(count, _)| count);
    let webd_memory_rss_bytes = webd_stats.map(|(_, rss_bytes)| rss_bytes);
    let webd_healthy = webd_process_count.map(|count| count > 0);
    let wechatd_process_count = wechatd_stats.map(|(count, _)| count);
    let wechatd_memory_rss_bytes = wechatd_stats.map(|(_, rss_bytes)| rss_bytes);
    let wechatd_healthy = wechatd_process_count.map(|count| count > 0);
    let feishud_stats = feishud_process_stats();
    let feishud_process_count_raw = feishud_stats.map(|(count, _)| count);
    let feishud_memory_rss_bytes_raw = feishud_stats.map(|(_, rss_bytes)| rss_bytes);
    let larkd_stats = larkd_process_stats();
    let larkd_process_count_raw = larkd_stats.map(|(count, _)| count);
    let larkd_memory_rss_bytes_raw = larkd_stats.map(|(_, rss_bytes)| rss_bytes);
    let (user_count, bound_channel_count, bound_channels) =
        auth_user_summary_counts(&state).unwrap_or_default();
    let telegram_configured_bot_names = state
        .channels
        .telegram_configured_bot_names
        .as_ref()
        .clone();
    let telegram_bot_statuses = read_telegram_bot_statuses(
        &state.skill_rt.workspace_root,
        &telegram_configured_bot_names,
    );
    let mut gateway_instance_statuses_by_scope =
        read_gateway_instance_statuses(&state.skill_rt.workspace_root);
    let whatsapp_cloud_gateway_healthy = gateway_instance_statuses_by_scope
        .get("whatsapp_cloud:primary")
        .map(|s| s.healthy);
    let whatsapp_web_gateway_healthy = gateway_instance_statuses_by_scope
        .get("whatsapp_web:primary")
        .map(|s| s.healthy);
    let feishu_gateway_healthy = gateway_instance_statuses_by_scope
        .get("feishu:primary")
        .map(|s| s.healthy);
    let lark_gateway_healthy = gateway_instance_statuses_by_scope
        .get("lark:primary")
        .map(|s| s.healthy);

    // 其他通信端也增加“网关状态回退”，防止独立进程未启用时 UI 误判未启动。
    let whatsappd_process_count = match whatsappd_process_count_raw {
        Some(count) if count > 0 => Some(count),
        _ if whatsapp_cloud_gateway_healthy == Some(true) => channel_gateway_process_count,
        _ => whatsappd_process_count_raw,
    };
    let whatsappd_memory_rss_bytes = match whatsappd_process_count_raw {
        Some(count) if count > 0 => whatsappd_memory_rss_bytes_raw,
        // channel-gateway RSS is shared by multiple adapters and cannot be attributed safely here.
        _ if whatsapp_cloud_gateway_healthy == Some(true) => None,
        _ => whatsappd_memory_rss_bytes_raw,
    };
    let whatsappd_healthy = match whatsappd_process_count_raw {
        Some(count) if count > 0 => Some(true),
        _ => whatsapp_cloud_gateway_healthy
            .or_else(|| whatsappd_process_count_raw.map(|count| count > 0)),
    };

    let wa_webd_process_count = match wa_webd_process_count_raw {
        Some(count) if count > 0 => Some(count),
        _ if whatsapp_web_gateway_healthy == Some(true) => channel_gateway_process_count,
        _ => wa_webd_process_count_raw,
    };
    let wa_webd_memory_rss_bytes = match wa_webd_process_count_raw {
        Some(count) if count > 0 => wa_webd_memory_rss_bytes_raw,
        // channel-gateway RSS is shared by multiple adapters and cannot be attributed safely here.
        _ if whatsapp_web_gateway_healthy == Some(true) => None,
        _ => wa_webd_memory_rss_bytes_raw,
    };
    let wa_webd_healthy = match wa_webd_process_count_raw {
        Some(count) if count > 0 => Some(true),
        _ => whatsapp_web_gateway_healthy
            .or_else(|| wa_webd_process_count_raw.map(|count| count > 0)),
    };

    let feishud_process_count = match feishud_process_count_raw {
        Some(count) if count > 0 => Some(count),
        _ if feishu_gateway_healthy == Some(true) => channel_gateway_process_count,
        _ => feishud_process_count_raw,
    };
    let feishud_memory_rss_bytes = match feishud_process_count_raw {
        Some(count) if count > 0 => feishud_memory_rss_bytes_raw,
        // channel-gateway RSS is shared by multiple adapters and cannot be attributed safely here.
        _ if feishu_gateway_healthy == Some(true) => None,
        _ => feishud_memory_rss_bytes_raw,
    };
    let feishud_healthy = match feishud_process_count_raw {
        Some(count) if count > 0 => Some(true),
        _ => feishu_gateway_healthy.or_else(|| feishud_process_count_raw.map(|count| count > 0)),
    };

    let larkd_process_count = match larkd_process_count_raw {
        Some(count) if count > 0 => Some(count),
        _ if lark_gateway_healthy == Some(true) => channel_gateway_process_count,
        _ => larkd_process_count_raw,
    };
    let larkd_memory_rss_bytes = match larkd_process_count_raw {
        Some(count) if count > 0 => larkd_memory_rss_bytes_raw,
        // channel-gateway RSS is shared by multiple adapters and cannot be attributed safely here.
        _ if lark_gateway_healthy == Some(true) => None,
        _ => larkd_memory_rss_bytes_raw,
    };
    let larkd_healthy = match larkd_process_count_raw {
        Some(count) if count > 0 => Some(true),
        _ => lark_gateway_healthy.or_else(|| larkd_process_count_raw.map(|count| count > 0)),
    };
    let mut gateway_instance_statuses = Vec::new();
    for bot_status in &telegram_bot_statuses {
        let scope = format!("telegram:{}", bot_status.name);
        gateway_instance_statuses.push(
            gateway_instance_statuses_by_scope
                .remove(&scope)
                .unwrap_or_else(|| GatewayInstanceRuntimeStatus {
                    kind: "telegram".to_string(),
                    name: bot_status.name.clone(),
                    scope,
                    healthy: bot_status.healthy,
                    status: bot_status.status.clone(),
                    last_heartbeat_ts: bot_status.last_heartbeat_ts,
                    last_error: bot_status.last_error.clone(),
                }),
        );
    }
    if state.channels.whatsapp_cloud_enabled {
        let scope = "whatsapp_cloud:primary".to_string();
        gateway_instance_statuses.push(
            gateway_instance_statuses_by_scope
                .remove(&scope)
                .unwrap_or_else(|| GatewayInstanceRuntimeStatus {
                    kind: "whatsapp_cloud".to_string(),
                    name: "primary".to_string(),
                    scope,
                    healthy: whatsappd_healthy.unwrap_or(false),
                    status: if whatsappd_healthy.unwrap_or(false) {
                        "running".to_string()
                    } else {
                        "stopped".to_string()
                    },
                    last_heartbeat_ts: None,
                    last_error: None,
                }),
        );
    }
    if state.channels.whatsapp_web_enabled {
        let scope = "whatsapp_web:primary".to_string();
        gateway_instance_statuses.push(
            gateway_instance_statuses_by_scope
                .remove(&scope)
                .unwrap_or_else(|| GatewayInstanceRuntimeStatus {
                    kind: "whatsapp_web".to_string(),
                    name: "primary".to_string(),
                    scope,
                    healthy: wa_webd_healthy.unwrap_or(false),
                    status: if wa_webd_healthy.unwrap_or(false) {
                        "running".to_string()
                    } else {
                        "stopped".to_string()
                    },
                    last_heartbeat_ts: None,
                    last_error: None,
                }),
        );
    }
    if state.channels.wechat_send_config.is_some() {
        let scope = "wechat:primary".to_string();
        gateway_instance_statuses.push(
            gateway_instance_statuses_by_scope
                .remove(&scope)
                .unwrap_or_else(|| GatewayInstanceRuntimeStatus {
                    kind: "wechat".to_string(),
                    name: "primary".to_string(),
                    scope,
                    healthy: wechatd_healthy.unwrap_or(false),
                    status: if wechatd_healthy.unwrap_or(false) {
                        "running".to_string()
                    } else {
                        "stopped".to_string()
                    },
                    last_heartbeat_ts: None,
                    last_error: None,
                }),
        );
    }
    if state.channels.feishu_send_config.is_some() {
        let scope = "feishu:primary".to_string();
        gateway_instance_statuses.push(
            gateway_instance_statuses_by_scope
                .remove(&scope)
                .unwrap_or_else(|| GatewayInstanceRuntimeStatus {
                    kind: "feishu".to_string(),
                    name: "primary".to_string(),
                    scope,
                    healthy: feishud_healthy.unwrap_or(false),
                    status: if feishud_healthy.unwrap_or(false) {
                        "running".to_string()
                    } else {
                        "stopped".to_string()
                    },
                    last_heartbeat_ts: None,
                    last_error: None,
                }),
        );
    }
    if state.channels.lark_send_config.is_some() {
        let scope = "lark:primary".to_string();
        gateway_instance_statuses.push(
            gateway_instance_statuses_by_scope
                .remove(&scope)
                .unwrap_or_else(|| GatewayInstanceRuntimeStatus {
                    kind: "lark".to_string(),
                    name: "primary".to_string(),
                    scope,
                    healthy: larkd_healthy.unwrap_or(false),
                    status: if larkd_healthy.unwrap_or(false) {
                        "running".to_string()
                    } else {
                        "stopped".to_string()
                    },
                    last_heartbeat_ts: None,
                    last_error: None,
                }),
        );
    }
    gateway_instance_statuses.extend(gateway_instance_statuses_by_scope.into_values());
    let data = HealthResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        queue_length,
        worker_state: "running".to_string(),
        uptime_seconds: state.worker.started_at.elapsed().as_secs(),
        memory_rss_bytes: current_rss_bytes(),
        running_length,
        task_timeout_seconds: state.worker.worker_task_timeout_seconds,
        running_oldest_age_seconds,
        telegramd_healthy,
        telegramd_process_count,
        telegramd_memory_rss_bytes,
        channel_gateway_healthy,
        channel_gateway_process_count,
        channel_gateway_memory_rss_bytes,
        whatsappd_healthy,
        whatsappd_process_count,
        whatsappd_memory_rss_bytes,
        telegram_bot_healthy: telegramd_healthy,
        telegram_bot_process_count: telegramd_process_count,
        telegram_bot_memory_rss_bytes: telegramd_memory_rss_bytes,
        telegram_configured_bot_count: telegram_configured_bot_names.len(),
        telegram_configured_bot_names,
        telegram_bot_statuses,
        gateway_instance_statuses,
        whatsapp_cloud_healthy: whatsappd_healthy,
        whatsapp_cloud_process_count: whatsappd_process_count,
        whatsapp_cloud_memory_rss_bytes: whatsappd_memory_rss_bytes,
        whatsapp_web_healthy: wa_webd_healthy,
        whatsapp_web_process_count: wa_webd_process_count,
        whatsapp_web_memory_rss_bytes: wa_webd_memory_rss_bytes,
        webd_healthy,
        webd_process_count,
        webd_memory_rss_bytes,
        wechatd_healthy,
        wechatd_process_count,
        wechatd_memory_rss_bytes,
        feishud_healthy,
        feishud_process_count,
        feishud_memory_rss_bytes,
        larkd_healthy,
        larkd_process_count,
        larkd_memory_rss_bytes,
        user_count,
        bound_channel_count,
        bound_channels,
        future_adapters_enabled: state.channels.future_adapters_enabled.as_ref().clone(),
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

async fn list_skills(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let mut skills: Vec<String> = state.get_skills_list().iter().cloned().collect();
    skills.retain(|s| !hide_skill_in_ui(&state, s));
    skills.sort_unstable();
    let skill_items = skills
        .iter()
        .map(|name| build_skill_list_item(&state, name))
        .collect::<Vec<_>>();
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "skills": skills,
                "skill_items": skill_items,
                "skill_runner_path": state.skill_rt.skill_runner_path.display().to_string(),
            })),
            error: None,
        }),
    )
}

async fn list_capabilities(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let mut names = BTreeSet::new();
    if let Some(registry) = state.get_skills_registry().as_ref() {
        for name in registry.all_names() {
            if !hide_skill_in_ui(&state, &name) {
                names.insert(name);
            }
        }
    }
    for name in state.get_skills_list().iter() {
        if !hide_skill_in_ui(&state, name) {
            names.insert(name.clone());
        }
    }
    let skill_items = names
        .iter()
        .map(|name| build_skill_list_item(&state, name))
        .collect::<Vec<_>>();
    let capability_items = capability_items_from_skill_items(&skill_items);
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "skill_items": skill_items,
                "capability_items": capability_items,
            })),
            error: None,
        }),
    )
}

fn build_skill_list_item(state: &AppState, skill_name: &str) -> SkillListItem {
    let registry_entry = state
        .get_skills_registry()
        .and_then(|registry| registry.get(skill_name).cloned());
    let planner_kind = state
        .get_skills_registry()
        .and_then(|registry| registry.planner_kind(skill_name));
    let availability = registry_entry
        .as_ref()
        .map(crate::skill_availability::evaluate_entry_availability);
    let enabled = skill_enabled_for_state(state, skill_name);
    let platform_available = availability
        .as_ref()
        .map(crate::skill_availability::SkillRuntimeAvailability::is_available);
    let runtime_available = platform_available.map(|available| available && enabled);
    let unavailable_reason = skill_unavailable_reason(enabled, availability.as_ref());
    let description = registry_entry
        .as_ref()
        .and_then(|entry| entry.description.as_ref())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| ui_skill_description(state, skill_name));
    SkillListItem {
        name: skill_name.to_string(),
        description,
        kind: registry_entry
            .as_ref()
            .map(|entry| skill_kind_token(entry.kind).to_string()),
        planner_kind: planner_kind.map(|kind| kind.as_token().to_string()),
        group: registry_entry.as_ref().and_then(|entry| {
            entry
                .group
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        }),
        risk_level: registry_entry.as_ref().and_then(|entry| {
            entry
                .risk_level
                .map(skill_risk_token)
                .map(ToString::to_string)
        }),
        auto_invocable: registry_entry
            .as_ref()
            .and_then(|entry| entry.auto_invocable),
        requires_confirmation: registry_entry
            .as_ref()
            .and_then(|entry| entry.requires_confirmation),
        side_effect: registry_entry.as_ref().and_then(|entry| entry.side_effect),
        retryable: registry_entry.as_ref().and_then(|entry| entry.retryable),
        output_kind: registry_entry
            .as_ref()
            .map(|entry| output_kind_token(entry.output_kind).to_string()),
        enabled: Some(enabled),
        runtime_available,
        unavailable_reason,
        current_os: availability.as_ref().map(|item| item.current_os.clone()),
        unsupported_os: availability
            .as_ref()
            .and_then(|item| item.unsupported_os.clone()),
        missing_required_bins: availability
            .as_ref()
            .map(|item| item.missing_required_bins.clone())
            .filter(|items| !items.is_empty()),
        missing_optional_bins: availability
            .as_ref()
            .map(|item| item.missing_optional_bins.clone())
            .filter(|items| !items.is_empty()),
        supported_os: registry_entry
            .as_ref()
            .map(|entry| entry.supported_os.clone())
            .filter(|items| !items.is_empty()),
        required_bins: registry_entry
            .as_ref()
            .map(|entry| entry.required_bins.clone())
            .filter(|items| !items.is_empty()),
        optional_bins: registry_entry
            .as_ref()
            .map(|entry| entry.optional_bins.clone())
            .filter(|items| !items.is_empty()),
        platform_notes: registry_entry
            .as_ref()
            .map(|entry| entry.platform_notes.clone())
            .filter(|items| !items.is_empty()),
        planner_capabilities: registry_entry
            .as_ref()
            .map(|entry| {
                entry
                    .planner_capabilities
                    .iter()
                    .map(|capability| capability.name.clone())
                    .collect::<Vec<_>>()
            })
            .filter(|items| !items.is_empty()),
        capabilities: registry_entry
            .as_ref()
            .map(|entry| {
                entry
                    .resolved_capabilities
                    .iter()
                    .map(|cap| cap.as_token())
                    .collect::<Vec<_>>()
            })
            .filter(|items| !items.is_empty()),
    }
}

fn skill_enabled_for_state(state: &AppState, skill_name: &str) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains(skill_name)
}

fn skill_unavailable_reason(
    enabled: bool,
    availability: Option<&crate::skill_availability::SkillRuntimeAvailability>,
) -> Option<String> {
    if !enabled {
        return Some("skill_disabled".to_string());
    }
    let availability = availability?;
    if availability.unsupported_os.is_some() {
        return Some("unsupported_os".to_string());
    }
    if !availability.missing_required_bins.is_empty() {
        return Some("missing_required_bins".to_string());
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CapabilityListItem {
    skill_name: String,
    capability: String,
    capability_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    planner_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    risk_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_available: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unavailable_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_kind: Option<String>,
}

fn capability_items_from_skill_items(skill_items: &[SkillListItem]) -> Vec<CapabilityListItem> {
    let mut items = Vec::new();
    for skill in skill_items {
        if let Some(capabilities) = skill.planner_capabilities.as_ref() {
            for capability in capabilities {
                items.push(capability_list_item(skill, capability, "planner_capability"));
            }
        }
        if let Some(capabilities) = skill.capabilities.as_ref() {
            for capability in capabilities {
                items.push(capability_list_item(skill, capability, "runtime_capability"));
            }
        }
    }
    items.sort_by(|a, b| {
        a.skill_name
            .cmp(&b.skill_name)
            .then_with(|| a.capability_kind.cmp(&b.capability_kind))
            .then_with(|| a.capability.cmp(&b.capability))
    });
    items
}

fn capability_list_item(
    skill: &SkillListItem,
    capability: &str,
    capability_kind: &str,
) -> CapabilityListItem {
    CapabilityListItem {
        skill_name: skill.name.clone(),
        capability: capability.to_string(),
        capability_kind: capability_kind.to_string(),
        planner_kind: skill.planner_kind.clone(),
        risk_level: skill.risk_level.clone(),
        enabled: skill.enabled,
        runtime_available: skill.runtime_available,
        unavailable_reason: skill.unavailable_reason.clone(),
        output_kind: skill.output_kind.clone(),
    }
}

fn skill_kind_token(kind: SkillKind) -> &'static str {
    match kind {
        SkillKind::Builtin => "builtin",
        SkillKind::Runner => "runner",
        SkillKind::External => "external",
    }
}

fn output_kind_token(kind: claw_core::skill_registry::OutputKind) -> &'static str {
    match kind {
        claw_core::skill_registry::OutputKind::Text => "text",
        claw_core::skill_registry::OutputKind::File => "file",
        claw_core::skill_registry::OutputKind::Image => "image",
        claw_core::skill_registry::OutputKind::Mixed => "mixed",
    }
}

fn skill_risk_token(kind: claw_core::skill_registry::SkillRiskLevel) -> &'static str {
    match kind {
        claw_core::skill_registry::SkillRiskLevel::Unknown => "unknown",
        claw_core::skill_registry::SkillRiskLevel::Low => "low",
        claw_core::skill_registry::SkillRiskLevel::Medium => "medium",
        claw_core::skill_registry::SkillRiskLevel::High => "high",
    }
}

fn ui_skill_description(state: &AppState, skill_name: &str) -> Option<String> {
    let registry_prompt_rel_path = state.skill_registry_prompt_rel_path(skill_name)?;
    let vendor = crate::bootstrap::prompts::active_prompt_vendor_name(state);
    let (raw, _) = prompt_layers::load_prompt_template_for_vendor(
        &state.skill_rt.workspace_root,
        &vendor,
        &registry_prompt_rel_path,
        "",
    );
    extract_skill_description_from_prompt(&raw)
}

fn extract_skill_description_from_prompt(raw: &str) -> Option<String> {
    let frontmatter = parse_skill_frontmatter(raw);
    if !frontmatter.description.trim().is_empty() {
        return Some(frontmatter.description.trim().to_string());
    }

    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- Description:") {
            let value = rest.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        if let Some(rest) = trimmed.strip_prefix("description:") {
            let value = rest.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    None
}

async fn import_external_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ImportSkillRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let source = req.source.trim();
    if source.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("source is required".to_string()),
            }),
        );
    }
    let enabled = req.enabled.unwrap_or(true);

    let raw_name = guess_bundle_name_from_path_or_source(source, "external-skill");
    let canonical_name = slugify_skill_name(&raw_name);
    let bundle_rel_dir = format!("third_party/clawhub/{canonical_name}");
    let bundle_dir = state.skill_rt.workspace_root.join(&bundle_rel_dir);
    if bundle_dir.exists() {
        if let Err(err) = std::fs::remove_dir_all(&bundle_dir) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("remove old imported bundle failed: {err}")),
                }),
            );
        }
    }

    let skill_md = match materialize_import_source(&state, source, &bundle_dir).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(err),
                }),
            );
        }
    };
    finalize_imported_bundle(
        &state,
        &bundle_dir,
        &bundle_rel_dir,
        source,
        enabled,
        &skill_md,
    )
}

async fn import_external_skill_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }

    let mut bundle_name = String::new();
    let mut enabled = true;
    let mut uploaded_files: Vec<(PathBuf, Vec<u8>)> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "bundle_name" => {
                if let Ok(text) = field.text().await {
                    bundle_name = text.trim().to_string();
                }
            }
            "enabled" => {
                if let Ok(text) = field.text().await {
                    enabled = text.trim() != "false";
                }
            }
            "files" => {
                let raw_path = field
                    .file_name()
                    .map(str::to_string)
                    .unwrap_or_else(|| "SKILL.md".to_string());
                let Some(rel_path) = sanitize_upload_relative_path(&raw_path) else {
                    continue;
                };
                let Ok(bytes) = field.bytes().await else {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some("read uploaded file failed".to_string()),
                        }),
                    );
                };
                uploaded_files.push((rel_path, bytes.to_vec()));
            }
            _ => {}
        }
    }

    if uploaded_files.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("no uploaded files found".to_string()),
            }),
        );
    }

    let guessed_name = if !bundle_name.trim().is_empty() {
        bundle_name.trim().to_string()
    } else {
        uploaded_files
            .first()
            .and_then(|(path, _)| path.components().next())
            .and_then(|part| match part {
                std::path::Component::Normal(v) => v.to_str(),
                _ => None,
            })
            .unwrap_or("uploaded-skill")
            .to_string()
    };
    let canonical_name = slugify_skill_name(&guessed_name);
    let bundle_rel_dir = format!("third_party/clawhub/{canonical_name}");
    let bundle_dir = state.skill_rt.workspace_root.join(&bundle_rel_dir);
    if bundle_dir.exists() {
        if let Err(err) = std::fs::remove_dir_all(&bundle_dir) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("remove old uploaded bundle failed: {err}")),
                }),
            );
        }
    }
    if let Err(err) = std::fs::create_dir_all(&bundle_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("create upload bundle dir failed: {err}")),
            }),
        );
    }

    let mut skill_md_path = None;
    for (rel_path, bytes) in uploaded_files {
        let normalized = rel_path
            .strip_prefix(&guessed_name)
            .ok()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or(rel_path);
        let target_path = bundle_dir.join(&normalized);
        if let Some(parent) = target_path.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("create uploaded subdirectory failed: {err}")),
                    }),
                );
            }
        }
        if let Err(err) = std::fs::write(&target_path, bytes) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("write uploaded file failed: {err}")),
                }),
            );
        }
        if normalized
            .file_name()
            .and_then(|v| v.to_str())
            .map(|name| name.eq_ignore_ascii_case("SKILL.md"))
            .unwrap_or(false)
        {
            skill_md_path = Some(target_path);
        }
    }

    let skill_md_path = skill_md_path.unwrap_or_else(|| bundle_dir.join("SKILL.md"));
    let skill_md = match std::fs::read_to_string(&skill_md_path) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!(
                        "uploaded bundle is missing readable SKILL.md: {err}"
                    )),
                }),
            );
        }
    };

    finalize_imported_bundle(
        &state,
        &bundle_dir,
        &bundle_rel_dir,
        &format!("upload:{guessed_name}"),
        enabled,
        &skill_md,
    )
}

#[derive(Debug, Deserialize)]
struct UpdateSkillsConfigRequest {
    #[serde(default)]
    skill_switches: HashMap<String, bool>,
}

#[derive(Debug, Deserialize)]
struct ImportSkillRequest {
    source: String,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UpdateLlmConfigRequest {
    selected_vendor: String,
    selected_model: String,
    #[serde(default)]
    vendor_base_url: Option<String>,
    #[serde(default)]
    vendor_api_key: Option<String>,
    #[serde(default)]
    vendor_api_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelegramBotConfigItem {
    name: String,
    #[serde(default)]
    bot_token: String,
    #[serde(default)]
    bot_token_configured: bool,
    #[serde(default)]
    bot_token_masked: Option<String>,
    #[serde(default = "default_agent_id")]
    agent_id: String,
    #[serde(default)]
    allowlist: Vec<i64>,
    #[serde(default = "default_telegram_access_mode")]
    access_mode: String,
    #[serde(default)]
    allowed_telegram_usernames: Vec<String>,
    #[serde(default)]
    is_primary: bool,
}

#[derive(Debug, Serialize)]
struct TelegramConfigResponse {
    config_path: String,
    bots: Vec<TelegramBotConfigItem>,
    agents: Vec<AgentConfigItem>,
    restart_required: bool,
}

#[derive(Debug, Deserialize)]
struct UpdateTelegramConfigRequest {
    #[serde(default)]
    bots: Vec<TelegramBotConfigItem>,
    #[serde(default)]
    agents: Vec<AgentConfigItem>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WechatConfigResponse {
    config_path: String,
    enabled: bool,
    listen: String,
    clawd_base_url: String,
    api_base_url: String,
    wechat_uin_base64: String,
    request_timeout_seconds: u64,
    longpoll_timeout_ms: u64,
    text_chunk_chars: usize,
    bot_token_configured: bool,
    saved_session_present: bool,
    restart_required: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct FeishuConfigResponse {
    config_path: String,
    enabled: bool,
    mode: String,
    listen: String,
    clawd_base_url: String,
    api_base_url: String,
    app_id: String,
    app_secret: String,
    verification_token_configured: bool,
    encrypt_key_configured: bool,
    bind_ready: bool,
    current_key_bound: bool,
    restart_required: bool,
}

#[derive(Debug, Deserialize)]
struct UpdateWechatConfigRequest {
    enabled: bool,
    listen: String,
    clawd_base_url: String,
    api_base_url: String,
    #[serde(default)]
    wechat_uin_base64: String,
    request_timeout_seconds: u64,
    longpoll_timeout_ms: u64,
    text_chunk_chars: usize,
}

#[derive(Debug, Deserialize)]
struct UpdateFeishuConfigRequest {
    #[serde(default)]
    app_id: String,
    #[serde(default)]
    app_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentConfigItem {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    persona_prompt: String,
    #[serde(default)]
    preferred_vendor: Option<String>,
    #[serde(default)]
    preferred_model: Option<String>,
    #[serde(default)]
    allowed_skills: Vec<String>,
}

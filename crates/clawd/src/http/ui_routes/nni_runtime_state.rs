fn read_nni_config(state: &AppState) -> anyhow::Result<NniConfigResponse> {
    let path = nni_runtime_config_path(state);
    let config = read_nni_runtime_config(state)?;
    let heartbeat_state = read_nni_heartbeat_runtime_state(state)?;
    let heartbeat_state_token = if !config.joined {
        "disabled"
    } else if heartbeat_state.network_authorization == "rejected" {
        "rejected"
    } else if heartbeat_state.last_heartbeat_error.is_some() {
        if heartbeat_state.last_heartbeat_network_failures > 0 {
            "waiting_network"
        } else {
            "degraded"
        }
    } else if heartbeat_state.last_heartbeat_at_ts.is_some() {
        "active"
    } else {
        "enabling"
    };
    let now = u64::try_from(current_unix_ts()).unwrap_or_default();
    let next_heartbeat_due_at_ts = nni_next_heartbeat_due_at_ts(
        config.joined,
        heartbeat_state_token,
        &heartbeat_state,
        now,
    );
    Ok(NniConfigResponse {
        selected_node_url: config.selected_node_url,
        bancor_service_node_url: config.bancor_service_node_url,
        asset_service_node_url: config.asset_service_node_url,
        remote_nodes: config.remote_nodes,
        joined: config.joined,
        asset_owner_pubkey: config.asset_owner_pubkey,
        heartbeat_interval_seconds: NNI_HEARTBEAT_INTERVAL_SECONDS,
        heartbeat_network_retry_limit: NNI_HEARTBEAT_NETWORK_RETRY_LIMIT,
        heartbeat_request_count: heartbeat_state.heartbeat_request_count,
        last_heartbeat_at_ts: heartbeat_state.last_heartbeat_at_ts,
        last_heartbeat_error: heartbeat_state.last_heartbeat_error,
        last_heartbeat_error_code: heartbeat_state.last_heartbeat_error_code,
        last_heartbeat_error_at_ts: heartbeat_state.last_heartbeat_error_at_ts,
        last_heartbeat_network_failures: heartbeat_state.last_heartbeat_network_failures,
        last_heartbeat_attempt_at_ts: heartbeat_state.last_heartbeat_attempt_at_ts,
        consecutive_heartbeat_failures: heartbeat_state.consecutive_heartbeat_failures,
        last_success_node_host: heartbeat_state.last_success_node_host,
        network_authorization: heartbeat_state.network_authorization,
        heartbeat_state: heartbeat_state_token.to_string(),
        next_heartbeat_due_at_ts,
        worker_running: true,
        config_path: path.display().to_string(),
    })
}

fn nni_next_heartbeat_due_at_ts(
    joined: bool,
    heartbeat_state: &str,
    runtime_state: &NniHeartbeatRuntimeState,
    now: u64,
) -> Option<u64> {
    if !joined {
        return None;
    }
    let due = match heartbeat_state {
        "active" => runtime_state
            .last_heartbeat_at_ts
            .map(|base| base.saturating_add(NNI_HEARTBEAT_INTERVAL_SECONDS)),
        "enabling" | "waiting_network" | "degraded" => runtime_state
            .last_heartbeat_attempt_at_ts
            .map(|base| base.saturating_add(NNI_HEARTBEAT_POLL_SECONDS)),
        _ => return None,
    };
    Some(due.unwrap_or(now))
}

fn nni_runtime_config_path(state: &AppState) -> PathBuf {
    state
        .skill_rt
        .workspace_root
        .join("data")
        .join("nni")
        .join("runtime-config.json")
}

fn read_nni_runtime_config(state: &AppState) -> anyhow::Result<NniRuntimeConfig> {
    let path = nni_runtime_config_path(state);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(legacy) = read_legacy_nni_config(state)? {
                write_nni_runtime_config(state, &legacy)?;
                return Ok(legacy);
            }
            return Ok(NniRuntimeConfig::default());
        }
        Err(error) => return Err(error.into()),
    };
    if raw.trim().is_empty() {
        return Ok(NniRuntimeConfig::default());
    }
    let mut config: NniRuntimeConfig = serde_json::from_str(&raw)?;
    let migrated = config.schema_version == 1;
    if migrated {
        config.schema_version = NNI_RUNTIME_CONFIG_SCHEMA_VERSION;
    } else if config.schema_version != NNI_RUNTIME_CONFIG_SCHEMA_VERSION {
        anyhow::bail!(
            "nni_runtime_config_schema_unsupported:{}",
            config.schema_version
        );
    }
    config.remote_nodes = normalize_nni_node_urls(&config.remote_nodes)
        .map_err(|error| anyhow::anyhow!(error))?;
    config.selected_node_url = normalize_selected_nni_node(
        config.selected_node_url.as_deref(),
        &config.remote_nodes,
    )?;
    config.asset_service_node_url = normalize_selected_nni_node(
        config
            .asset_service_node_url
            .as_deref()
            .or(config.selected_node_url.as_deref()),
        &config.remote_nodes,
    )?;
    config.bancor_service_node_url = normalize_selected_nni_node(
        config
            .bancor_service_node_url
            .as_deref()
            .or(config.selected_node_url.as_deref()),
        &config.remote_nodes,
    )?;
    config.asset_owner_pubkey = config
        .asset_owner_pubkey
        .as_deref()
        .map(normalize_nni_owner_public_key)
        .transpose()
        .map_err(anyhow::Error::msg)?;
    if migrated {
        write_nni_runtime_config(state, &config)?;
    }
    Ok(config)
}

fn read_legacy_nni_config(state: &AppState) -> anyhow::Result<Option<NniRuntimeConfig>> {
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let parsed: toml::Value = toml::from_str(&raw)?;
    let Some(nni) = parsed.get("nni") else {
        return Ok(None);
    };
    let remote_nodes = nni
        .get("remote_nodes")
        .and_then(toml_value_string_list)
        .map(|values| normalize_nni_node_urls(&values))
        .transpose()
        .map_err(|error| anyhow::anyhow!(error))?
        .unwrap_or_default();
    let joined = nni
        .get("joined")
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    Ok(Some(NniRuntimeConfig {
        schema_version: NNI_RUNTIME_CONFIG_SCHEMA_VERSION,
        selected_node_url: remote_nodes.first().cloned(),
        bancor_service_node_url: remote_nodes.first().cloned(),
        asset_service_node_url: remote_nodes.first().cloned(),
        remote_nodes,
        joined,
        asset_owner_pubkey: None,
    }))
}

fn persist_nni_asset_owner_pubkey(
    state: &AppState,
    owner_pubkey: &str,
    replace_existing: bool,
) -> anyhow::Result<()> {
    let normalized = normalize_nni_owner_public_key(owner_pubkey).map_err(anyhow::Error::msg)?;
    let mut config = read_nni_runtime_config(state)?;
    if !replace_existing
        && config
        .asset_owner_pubkey
        .as_ref()
        .is_some_and(|current| current != &normalized)
    {
        anyhow::bail!("nni_asset_owner_conflict");
    }
    config.asset_owner_pubkey = Some(normalized);
    // Asset authorization and heartbeat participation are separate user actions.
    config.joined = false;
    write_nni_runtime_config(state, &config)
}

fn clear_nni_asset_owner_binding(state: &AppState) -> anyhow::Result<()> {
    let mut config = read_nni_runtime_config(state)?;
    config.asset_owner_pubkey = None;
    config.joined = false;
    write_nni_runtime_config(state, &config)
}

fn write_nni_runtime_config(
    state: &AppState,
    config: &NniRuntimeConfig,
) -> anyhow::Result<()> {
    let path = nni_runtime_config_path(state);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("nni_runtime_config_parent_missing"))?;
    fs::create_dir_all(parent)?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".runtime-config.{}.{}.tmp",
        std::process::id(),
        suffix
    ));
    let result = (|| -> anyhow::Result<()> {
        let mut bytes = serde_json::to_vec_pretty(config)?;
        bytes.push(b'\n');
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn nni_heartbeat_runtime_state_path(state: &AppState) -> PathBuf {
    state
        .skill_rt
        .workspace_root
        .join("data")
        .join("nni")
        .join("heartbeat-state.json")
}

fn read_nni_heartbeat_runtime_state(state: &AppState) -> anyhow::Result<NniHeartbeatRuntimeState> {
    let path = nni_heartbeat_runtime_state_path(state);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(NniHeartbeatRuntimeState::default());
        }
        Err(error) => return Err(error.into()),
    };
    if raw.trim().is_empty() {
        return Ok(NniHeartbeatRuntimeState::default());
    }
    let mut runtime_state: NniHeartbeatRuntimeState = serde_json::from_str(&raw)?;
    if !matches!(runtime_state.schema_version, 1 | NNI_HEARTBEAT_RUNTIME_STATE_SCHEMA_VERSION) {
        anyhow::bail!(
            "nni_heartbeat_runtime_state_schema_unsupported:{}",
            runtime_state.schema_version
        );
    }
    runtime_state.schema_version = NNI_HEARTBEAT_RUNTIME_STATE_SCHEMA_VERSION;
    if runtime_state.network_authorization.trim().is_empty() {
        runtime_state.network_authorization = nni_unknown_network_authorization();
    }
    Ok(runtime_state)
}

fn write_nni_heartbeat_runtime_state(
    state: &AppState,
    runtime_state: &NniHeartbeatRuntimeState,
) -> anyhow::Result<()> {
    let path = nni_heartbeat_runtime_state_path(state);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("nni_heartbeat_runtime_state_parent_missing"))?;
    fs::create_dir_all(parent)?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".heartbeat-state.{}.{}.tmp",
        std::process::id(),
        suffix
    ));
    let result = (|| -> anyhow::Result<()> {
        let mut bytes = serde_json::to_vec_pretty(runtime_state)?;
        bytes.push(b'\n');
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_nni_heartbeat_error_records(
    state: &AppState,
) -> anyhow::Result<Vec<NniHeartbeatErrorRecord>> {
    let mut records = read_nni_heartbeat_error_records_from_log(state)?;
    records.sort_by(|left, right| {
        let ts_order = right
            .created_at_ts
            .unwrap_or_default()
            .cmp(&left.created_at_ts.unwrap_or_default());
        ts_order.then_with(|| right.id.cmp(&left.id))
    });
    records.truncate(NNI_HEARTBEAT_ERROR_HISTORY_LIMIT);
    Ok(records)
}

fn clear_nni_heartbeat_error_records(state: &AppState) -> anyhow::Result<Value> {
    let existing_count = read_nni_heartbeat_error_records(state)?.len();
    let mut runtime_state = read_nni_heartbeat_runtime_state(state)?;
    runtime_state.last_heartbeat_error = None;
    runtime_state.last_heartbeat_error_code = None;
    runtime_state.last_heartbeat_error_at_ts = None;
    runtime_state.last_heartbeat_network_failures = 0;
    runtime_state.consecutive_heartbeat_failures = 0;
    write_nni_heartbeat_runtime_state(state, &runtime_state)?;
    rewrite_nni_log_without_event_kinds(
        state,
        &[
            "heartbeat_error_record",
            "heartbeat_failed",
            "heartbeat_tick_error",
            "heartbeat_network_retry",
        ],
    )?;
    Ok(json!({
        "status": "nni_heartbeat_errors_cleared",
        "deleted_records": existing_count,
        "runtime_state_path": nni_heartbeat_runtime_state_path(state).display().to_string(),
        "log_path": nni_log_path(state).display().to_string(),
    }))
}

fn read_nni_heartbeat_error_records_from_log(
    state: &AppState,
) -> anyhow::Result<Vec<NniHeartbeatErrorRecord>> {
    Ok(read_nni_log_payloads(state, "heartbeat_error_record")?
        .into_iter()
        .filter_map(|payload| serde_json::from_value::<NniHeartbeatErrorRecord>(payload).ok())
        .collect())
}

fn record_nni_heartbeat_error_event(
    state: &AppState,
    error: &str,
    created_at_ts: Option<u64>,
    network: bool,
) {
    let next_id = read_nni_heartbeat_error_records(state)
        .unwrap_or_default()
        .iter()
        .map(|record| record.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let record = NniHeartbeatErrorRecord {
        id: next_id,
        created_at_ts,
        error: error.to_string(),
        network,
    };
    append_nni_log_event_best_effort(
        state,
        "heartbeat_error_record",
        serde_json::to_value(record).unwrap_or_else(|_| json!({})),
    );
}

fn write_nni_config(
    state: &AppState,
    remote_nodes: Option<&[String]>,
    joined: Option<bool>,
) -> anyhow::Result<NniConfigResponse> {
    write_nni_config_with_selected_node(state, remote_nodes, None, None, None, joined)
}

fn write_nni_config_with_selected_node(
    state: &AppState,
    remote_nodes: Option<&[String]>,
    selected_node_url: Option<&str>,
    bancor_service_node_url: Option<&str>,
    asset_service_node_url: Option<&str>,
    joined: Option<bool>,
) -> anyhow::Result<NniConfigResponse> {
    let mut config = read_nni_runtime_config(state)?;
    let previous_selected_node_url = config.selected_node_url.clone();
    if let Some(remote_nodes) = remote_nodes {
        config.remote_nodes = normalize_nni_node_urls(remote_nodes)
            .map_err(|error| anyhow::anyhow!(error))?;
    }
    let next_selected_node_url = normalize_selected_nni_node(
        selected_node_url.or(config.selected_node_url.as_deref()),
        &config.remote_nodes,
    )?;
    if config.joined
        && joined != Some(false)
        && previous_selected_node_url != next_selected_node_url
    {
        anyhow::bail!("nni_selected_node_change_requires_stop");
    }
    config.selected_node_url = next_selected_node_url;
    let current_bancor_node = config
        .bancor_service_node_url
        .as_deref()
        .filter(|candidate| config.remote_nodes.iter().any(|node| node == *candidate));
    config.bancor_service_node_url = normalize_selected_nni_node(
        bancor_service_node_url
            .or(current_bancor_node)
            .or(config.selected_node_url.as_deref()),
        &config.remote_nodes,
    )?;
    let current_asset_node = config
        .asset_service_node_url
        .as_deref()
        .filter(|candidate| config.remote_nodes.iter().any(|node| node == *candidate));
    config.asset_service_node_url = normalize_selected_nni_node(
        asset_service_node_url
            .or(current_asset_node)
            .or(config.selected_node_url.as_deref()),
        &config.remote_nodes,
    )?;
    if let Some(joined) = joined {
        config.joined = joined;
    }
    write_nni_runtime_config(state, &config)?;
    read_nni_config(state)
}

fn normalize_selected_nni_node(
    selected_node_url: Option<&str>,
    remote_nodes: &[String],
) -> anyhow::Result<Option<String>> {
    if remote_nodes.is_empty() {
        return Ok(None);
    }
    let selected = selected_node_url
        .map(normalize_nni_node_url)
        .transpose()
        .map_err(|error| anyhow::anyhow!(error))?;
    match selected {
        Some(selected) if remote_nodes.contains(&selected) => Ok(Some(selected)),
        Some(_) => anyhow::bail!("nni_selected_node_not_bound"),
        None => Ok(remote_nodes.first().cloned()),
    }
}

struct NniHeartbeatStatusUpdate<'a> {
    heartbeat_at_ts: Option<u64>,
    attempt_at_ts: Option<u64>,
    error: Option<&'a str>,
    error_code: Option<&'a str>,
    error_at_ts: Option<u64>,
    error_network: bool,
    request_count: Option<u64>,
    network_failures: Option<u64>,
    success_node_url: Option<&'a str>,
    network_authorization: Option<&'a str>,
}

fn nni_node_host(node_url: &str) -> Option<String> {
    reqwest::Url::parse(node_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
}

fn write_nni_heartbeat_status(
    state: &AppState,
    update: NniHeartbeatStatusUpdate<'_>,
) -> anyhow::Result<NniConfigResponse> {
    let mut runtime_state = read_nni_heartbeat_runtime_state(state)?;
    if let Some(ts) = update.heartbeat_at_ts {
        runtime_state.last_heartbeat_at_ts = Some(ts);
        runtime_state.consecutive_heartbeat_failures = 0;
    }
    if let Some(ts) = update.attempt_at_ts {
        runtime_state.last_heartbeat_attempt_at_ts = Some(ts);
    }
    if let Some(count) = update.request_count {
        runtime_state.heartbeat_request_count = count;
    }
    if let Some(count) = update.network_failures {
        runtime_state.last_heartbeat_network_failures = count;
    }
    runtime_state.last_heartbeat_error = update
        .error
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    runtime_state.last_heartbeat_error_code = update
        .error_code
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    runtime_state.last_heartbeat_error_at_ts = update.error_at_ts.filter(|value| *value > 0);
    if runtime_state.last_heartbeat_error.is_some() {
        runtime_state.consecutive_heartbeat_failures =
            runtime_state.consecutive_heartbeat_failures.saturating_add(1);
    }
    if let Some(node_url) = update.success_node_url {
        runtime_state.last_success_node_host = nni_node_host(node_url);
    }
    if let Some(authorization) = update.network_authorization {
        runtime_state.network_authorization = authorization.to_string();
    }
    write_nni_heartbeat_runtime_state(state, &runtime_state)?;
    if let Some(error) = update
        .error
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        record_nni_heartbeat_error_event(
            state,
            error,
            update.error_at_ts,
            update.error_network,
        );
    }
    read_nni_config(state)
}

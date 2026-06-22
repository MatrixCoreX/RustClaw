const NNI_REQUEST_RECORD_HISTORY_LIMIT: usize = 500;
const NNI_LOG_FILE_NAME: &str = "nni.log";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NniRequestRecord {
    id: u64,
    request_kind: String,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    user_key: Option<String>,
    #[serde(default)]
    device_pubkey: Option<String>,
    #[serde(default)]
    node_url: Option<String>,
    #[serde(default)]
    compliant: Option<bool>,
    status: String,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    created_at_ts: Option<u64>,
    #[serde(default)]
    signature_present: bool,
    #[serde(default)]
    challenge_present: bool,
}

fn nni_request_record(request_kind: &str, status: &str) -> NniRequestRecord {
    NniRequestRecord {
        id: 0,
        request_kind: request_kind.to_string(),
        task_id: None,
        user_key: None,
        device_pubkey: None,
        node_url: None,
        compliant: None,
        status: status.to_string(),
        error_code: None,
        created_at_ts: None,
        signature_present: false,
        challenge_present: false,
    }
}

fn record_nni_request_event(state: &AppState, record: NniRequestRecord) {
    let _ = write_nni_request_record(state, record);
}

fn write_nni_request_record(state: &AppState, mut record: NniRequestRecord) -> anyhow::Result<()> {
    let records = read_nni_request_records(state)?;
    record.id = records
        .iter()
        .map(|record| record.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    if record.created_at_ts.is_none() {
        record.created_at_ts = Some(u64::try_from(current_unix_ts()).unwrap_or_default());
    }
    append_nni_log_event(state, "request_record", serde_json::to_value(record)?)
}

fn read_nni_request_records(state: &AppState) -> anyhow::Result<Vec<NniRequestRecord>> {
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
    let parsed: toml::Value =
        toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let mut records = parse_nni_request_records(&parsed);
    records.extend(read_nni_request_records_from_log(state)?);
    records.sort_by(|left, right| {
        let ts_order = right
            .created_at_ts
            .unwrap_or_default()
            .cmp(&left.created_at_ts.unwrap_or_default());
        ts_order.then_with(|| right.id.cmp(&left.id))
    });
    records.truncate(NNI_REQUEST_RECORD_HISTORY_LIMIT);
    Ok(records)
}

fn clear_nni_request_records(state: &AppState) -> anyhow::Result<Value> {
    let existing_count = read_nni_request_records(state)?.len();
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
    let output = upsert_section_key_line(&raw, "nni", "request_records", "[]");
    write_runtime_config_file(state, &output)?;
    rewrite_nni_log_without_event_kinds(state, &["request_record"])?;
    Ok(json!({
        "status": "nni_request_records_cleared",
        "deleted_records": existing_count,
        "config_path": path.display().to_string(),
        "log_path": nni_log_path(state).display().to_string(),
    }))
}

fn parse_nni_request_records(parsed: &toml::Value) -> Vec<NniRequestRecord> {
    parsed
        .get("nni")
        .and_then(|value| value.get("request_records"))
        .and_then(toml::Value::as_array)
        .map(|records| records.iter().filter_map(parse_nni_request_record).collect())
        .unwrap_or_default()
}

fn parse_nni_request_record(value: &toml::Value) -> Option<NniRequestRecord> {
    let table = value.as_table()?;
    let request_kind = table
        .get("request_kind")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("nni_join")
        .to_string();
    let status = table
        .get("status")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string();
    let id = table
        .get("id")
        .and_then(toml::Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(0);
    let optional_string = |key: &str| {
        table
            .get(key)
            .and_then(toml::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let created_at_ts = table
        .get("created_at_ts")
        .and_then(toml::Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0);
    Some(NniRequestRecord {
        id,
        request_kind,
        task_id: optional_string("task_id"),
        user_key: optional_string("user_key"),
        device_pubkey: optional_string("device_pubkey"),
        node_url: optional_string("node_url"),
        compliant: table.get("compliant").and_then(toml::Value::as_bool),
        status,
        error_code: optional_string("error_code"),
        created_at_ts,
        signature_present: table
            .get("signature_present")
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
        challenge_present: table
            .get("challenge_present")
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
    })
}

fn nni_log_path(state: &AppState) -> PathBuf {
    state.skill_rt.workspace_root.join("logs").join(NNI_LOG_FILE_NAME)
}

fn append_nni_log_event(state: &AppState, event_kind: &str, payload: Value) -> anyhow::Result<()> {
    let log_path = nni_log_path(state);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let line = json!({
        "ts": current_unix_ts(),
        "event_kind": event_kind,
        "payload": payload,
    });
    serde_json::to_writer(&mut file, &line)?;
    writeln!(file)?;
    Ok(())
}

fn append_nni_log_event_best_effort(state: &AppState, event_kind: &str, payload: Value) {
    let _ = append_nni_log_event(state, event_kind, payload);
}

fn read_nni_log_payloads(state: &AppState, event_kind: &str) -> anyhow::Result<Vec<Value>> {
    let path = nni_log_path(state);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let mut payloads = Vec::new();
    for line in raw.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value
            .get("event_kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind == event_kind)
        {
            if let Some(payload) = value.get("payload") {
                payloads.push(payload.clone());
            }
        }
    }
    Ok(payloads)
}

fn read_nni_request_records_from_log(state: &AppState) -> anyhow::Result<Vec<NniRequestRecord>> {
    Ok(read_nni_log_payloads(state, "request_record")?
        .into_iter()
        .filter_map(|payload| serde_json::from_value::<NniRequestRecord>(payload).ok())
        .collect())
}

fn rewrite_nni_log_without_event_kinds(
    state: &AppState,
    event_kinds: &[&str],
) -> anyhow::Result<()> {
    let path = nni_log_path(state);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let mut kept = Vec::new();
    for line in raw.lines() {
        let remove = serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|value| {
                value
                    .get("event_kind")
                    .and_then(Value::as_str)
                    .map(|kind| event_kinds.iter().any(|candidate| candidate == &kind))
            })
            .unwrap_or(false);
        if !remove {
            kept.push(line);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = kept.join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    std::fs::write(path, output)?;
    Ok(())
}

const NNI_REQUEST_RECORD_HISTORY_LIMIT: usize = 500;

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
    if let Err(err) = write_nni_request_record(state, record) {
        tracing::warn!("nni request record write failed: {err}");
    }
}

fn write_nni_request_record(state: &AppState, mut record: NniRequestRecord) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(state.skill_rt.workspace_root.join("configs/config.toml"))
        .unwrap_or_else(|_| String::new());
    let parsed: toml::Value =
        toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let mut records = parse_nni_request_records(&parsed);
    record.id = records
        .iter()
        .map(|record| record.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    if record.created_at_ts.is_none() {
        record.created_at_ts = Some(u64::try_from(current_unix_ts()).unwrap_or_default());
    }
    records.insert(0, record);
    records.truncate(NNI_REQUEST_RECORD_HISTORY_LIMIT);
    let rendered_records = render_nni_request_records(&records);
    let output = upsert_section_key_line(&raw, "nni", "request_records", &rendered_records);
    write_runtime_config_file(state, &output)?;
    Ok(())
}

fn read_nni_request_records(state: &AppState) -> anyhow::Result<Vec<NniRequestRecord>> {
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
    let parsed: toml::Value =
        toml::from_str(&raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let mut records = parse_nni_request_records(&parsed);
    records.sort_by(|left, right| {
        let ts_order = right
            .created_at_ts
            .unwrap_or_default()
            .cmp(&left.created_at_ts.unwrap_or_default());
        ts_order.then_with(|| right.id.cmp(&left.id))
    });
    Ok(records)
}

fn clear_nni_request_records(state: &AppState) -> anyhow::Result<Value> {
    let existing_count = read_nni_request_records(state)?.len();
    let path = state.skill_rt.workspace_root.join("configs/config.toml");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
    let output = upsert_section_key_line(&raw, "nni", "request_records", "[]");
    write_runtime_config_file(state, &output)?;
    Ok(json!({
        "status": "nni_request_records_cleared",
        "deleted_records": existing_count,
        "config_path": path.display().to_string(),
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

fn render_nni_request_records(records: &[NniRequestRecord]) -> String {
    let values = records
        .iter()
        .map(|record| {
            let mut table = toml::map::Map::new();
            table.insert(
                "id".to_string(),
                toml::Value::Integer(i64::try_from(record.id).unwrap_or(i64::MAX)),
            );
            table.insert(
                "request_kind".to_string(),
                toml::Value::String(record.request_kind.clone()),
            );
            if let Some(task_id) = &record.task_id {
                table.insert("task_id".to_string(), toml::Value::String(task_id.clone()));
            }
            if let Some(user_key) = &record.user_key {
                table.insert("user_key".to_string(), toml::Value::String(user_key.clone()));
            }
            if let Some(device_pubkey) = &record.device_pubkey {
                table.insert(
                    "device_pubkey".to_string(),
                    toml::Value::String(device_pubkey.clone()),
                );
            }
            if let Some(node_url) = &record.node_url {
                table.insert("node_url".to_string(), toml::Value::String(node_url.clone()));
            }
            if let Some(compliant) = record.compliant {
                table.insert("compliant".to_string(), toml::Value::Boolean(compliant));
            }
            table.insert(
                "status".to_string(),
                toml::Value::String(record.status.clone()),
            );
            if let Some(error_code) = &record.error_code {
                table.insert(
                    "error_code".to_string(),
                    toml::Value::String(error_code.clone()),
                );
            }
            if let Some(created_at_ts) = record.created_at_ts {
                table.insert(
                    "created_at_ts".to_string(),
                    toml::Value::Integer(i64::try_from(created_at_ts).unwrap_or(i64::MAX)),
                );
            }
            table.insert(
                "signature_present".to_string(),
                toml::Value::Boolean(record.signature_present),
            );
            table.insert(
                "challenge_present".to_string(),
                toml::Value::Boolean(record.challenge_present),
            );
            toml::Value::Table(table)
        })
        .collect::<Vec<_>>();
    toml::Value::Array(values).to_string()
}

const FACTORY_RESET_WEBD_USERNAME: &str = "rustclaw";
const FACTORY_RESET_WEBD_PASSWORD: &str = "123456";

#[derive(Debug, Default, Serialize)]
struct FactoryResetConfigScrubResult {
    files_scanned: usize,
    files_updated: usize,
    fields_cleared: usize,
    errors: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
struct FactoryResetLogsResult {
    files_deleted: usize,
    directories_deleted: usize,
    bytes_deleted: u64,
    errors: Vec<String>,
}

async fn factory_reset_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can run factory reset".to_string()),
            }),
        );
    }

    let config = scrub_factory_reset_config_files(&state.skill_rt.workspace_root);
    let logs = clear_factory_reset_logs_dir(&state.skill_rt.workspace_root);
    let db = match factory_reset_auth_state(&state) {
        Ok(result) => result,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("factory reset database failed: {err}")),
                }),
            );
        }
    };

    let mut warnings = Vec::new();
    warnings.extend(config.errors.iter().cloned());
    warnings.extend(logs.errors.iter().cloned());

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "status": if warnings.is_empty() {
                    "factory_reset_completed"
                } else {
                    "factory_reset_completed_with_warnings"
                },
                "admin_user_key": db.admin_user_key,
                "webd_username": FACTORY_RESET_WEBD_USERNAME,
                "webd_password": FACTORY_RESET_WEBD_PASSWORD,
                "database": factory_reset_db_json(&db),
                "config": config,
                "logs": logs,
                "warnings": warnings,
            })),
            error: None,
        }),
    )
}

fn factory_reset_db_json(db: &FactoryResetDbResult) -> Value {
    json!({
        "auth_keys_deleted": db.auth_keys_deleted,
        "webd_accounts_deleted": db.webd_accounts_deleted,
        "channel_bindings_deleted": db.channel_bindings_deleted,
        "exchange_credentials_deleted": db.exchange_credentials_deleted,
        "pending_bind_sessions_deleted": db.pending_bind_sessions_deleted,
        "recent_memories_deleted": db.recent_memories_deleted,
        "preferences_deleted": db.preferences_deleted,
        "long_term_memories_deleted": db.long_term_memories_deleted,
        "memory_facts_deleted": db.memory_facts_deleted,
        "memory_retrieval_rows_deleted": db.memory_retrieval_rows_deleted,
        "followup_frames_deleted": db.followup_frames_deleted,
        "clarify_states_deleted": db.clarify_states_deleted,
        "observed_facts_states_deleted": db.observed_facts_states_deleted,
        "conversation_states_deleted": db.conversation_states_deleted,
        "audit_logs_deleted": db.audit_logs_deleted,
    })
}

fn scrub_factory_reset_config_files(workspace_root: &Path) -> FactoryResetConfigScrubResult {
    let mut result = FactoryResetConfigScrubResult::default();
    let mut seen = BTreeSet::new();
    for relative in ["configs", "docker/config"] {
        scan_factory_reset_config_dir(&workspace_root.join(relative), &mut seen, &mut result);
    }
    result
}

fn scan_factory_reset_config_dir(
    dir: &Path,
    seen: &mut BTreeSet<PathBuf>,
    result: &mut FactoryResetConfigScrubResult,
) {
    let Ok(meta) = fs::symlink_metadata(dir) else {
        return;
    };
    if meta.file_type().is_symlink() {
        result
            .errors
            .push(format!("skip symlink config directory: {}", dir.display()));
        return;
    }
    if !meta.is_dir() {
        return;
    }
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if !seen.insert(canonical) {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            result.errors.push(format!(
                "read config directory failed: {}: {err}",
                dir.display()
            ));
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            result
                .errors
                .push(format!("read config entry type failed: {}", path.display()));
            continue;
        };
        if file_type.is_symlink() {
            result
                .errors
                .push(format!("skip symlink config path: {}", path.display()));
            continue;
        }
        if file_type.is_dir() {
            scan_factory_reset_config_dir(&path, seen, result);
            continue;
        }
        if !file_type.is_file() || path.extension().and_then(|v| v.to_str()) != Some("toml") {
            continue;
        }
        result.files_scanned += 1;
        match scrub_factory_reset_config_file(&path) {
            Ok(fields_cleared) => {
                if fields_cleared > 0 {
                    result.files_updated += 1;
                    result.fields_cleared += fields_cleared;
                }
            }
            Err(err) => {
                result.errors.push(format!(
                    "scrub config file failed: {}: {err}",
                    path.display()
                ));
            }
        }
    }
}

fn scrub_factory_reset_config_file(path: &Path) -> anyhow::Result<usize> {
    let raw = fs::read_to_string(path)?;
    let mut output = String::with_capacity(raw.len());
    let mut fields_cleared = 0;
    for segment in raw.split_inclusive('\n') {
        let (line, changed) = scrub_factory_reset_config_line(segment);
        if changed {
            fields_cleared += 1;
        }
        output.push_str(&line);
    }
    if fields_cleared > 0 && output != raw {
        fs::write(path, output)?;
    }
    Ok(fields_cleared)
}

fn scrub_factory_reset_config_line(segment: &str) -> (String, bool) {
    let (body, ending) = if let Some(stripped) = segment.strip_suffix("\r\n") {
        (stripped, "\r\n")
    } else if let Some(stripped) = segment.strip_suffix('\n') {
        (stripped, "\n")
    } else {
        (segment, "")
    };
    let trimmed = body.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('[') {
        return (segment.to_string(), false);
    }
    let Some((left, right)) = body.split_once('=') else {
        return (segment.to_string(), false);
    };
    let key_name = left.trim();
    if !is_factory_reset_sensitive_config_key(key_name) {
        return (segment.to_string(), false);
    }
    let comment = right
        .find('#')
        .map(|idx| format!(" {}", right[idx..].trim_start()))
        .unwrap_or_default();
    let replacement = if right.trim_start().starts_with('[') {
        "[]"
    } else {
        "\"\""
    };
    (
        format!("{} = {}{}{}", left.trim_end(), replacement, comment, ending),
        true,
    )
}

fn is_factory_reset_sensitive_config_key(raw: &str) -> bool {
    let lower = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches('`')
        .to_ascii_lowercase();
    let tokens: Vec<&str> = lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.iter().any(|token| {
        matches!(
            *token,
            "key" | "token" | "secret" | "password" | "passphrase" | "credential" | "credentials"
        )
    }) {
        return true;
    }
    let compact: String = lower
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    matches!(
        compact.as_str(),
        "apikey"
            | "apisecret"
            | "appsecret"
            | "appid"
            | "clientid"
            | "clientsecret"
            | "botsecret"
            | "bottoken"
            | "accesstoken"
            | "refreshtoken"
            | "privatekey"
            | "adminkey"
            | "userkey"
    )
}

fn clear_factory_reset_logs_dir(workspace_root: &Path) -> FactoryResetLogsResult {
    let mut result = FactoryResetLogsResult::default();
    let logs_dir = workspace_root.join("logs");
    let Ok(meta) = fs::symlink_metadata(&logs_dir) else {
        return result;
    };
    if meta.file_type().is_symlink() {
        result
            .errors
            .push(format!("skip symlink logs directory: {}", logs_dir.display()));
        return result;
    }
    if !meta.is_dir() {
        return result;
    }
    let entries = match fs::read_dir(&logs_dir) {
        Ok(entries) => entries,
        Err(err) => {
            result
                .errors
                .push(format!("read logs directory failed: {err}"));
            return result;
        }
    };
    for entry in entries.flatten() {
        clear_factory_reset_log_path(&entry.path(), &mut result);
    }
    result
}

fn clear_factory_reset_log_path(path: &Path, result: &mut FactoryResetLogsResult) {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) => {
            result
                .errors
                .push(format!("read log path failed: {}: {err}", path.display()));
            return;
        }
    };
    if meta.is_dir() && !meta.file_type().is_symlink() {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(err) => {
                result
                    .errors
                    .push(format!("read log directory failed: {}: {err}", path.display()));
                return;
            }
        };
        for entry in entries.flatten() {
            clear_factory_reset_log_path(&entry.path(), result);
        }
        match fs::remove_dir(path) {
            Ok(()) => result.directories_deleted += 1,
            Err(err) => result
                .errors
                .push(format!("delete log directory failed: {}: {err}", path.display())),
        }
        return;
    }
    let bytes = meta.len();
    match fs::remove_file(path) {
        Ok(()) => {
            result.files_deleted += 1;
            result.bytes_deleted = result.bytes_deleted.saturating_add(bytes);
        }
        Err(err) => result
            .errors
            .push(format!("delete log file failed: {}: {err}", path.display())),
    }
}

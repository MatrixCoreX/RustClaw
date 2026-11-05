const AIPP_MEDIA_RECORD_LIMIT: usize = 50;
const AIPP_MEDIA_RECORD_SCAN_LIMIT: usize = 100_000;
const AIPP_MEDIA_RECORD_MAX_BYTES: u64 = 1024 * 1024;
const AIPP_MEDIA_STATE_MAX_BYTES: u64 = 1024 * 1024;
const AIPP_TASK_ACTIVITY_LIMIT: usize = 50;
const AIPP_TASK_ACTIVITY_INPUT_MAX_CHARS: usize = 20_000;
const AIPP_TASK_ACTIVITY_RESULT_MAX_CHARS: usize = 65_536;
const AIPP_TASK_ACTIVITY_ERROR_MAX_CHARS: usize = 4_096;
const AIPP_TASK_ACTIVITY_ARTIFACT_LIMIT: usize = 64;
const AIPP_TASK_ACTIVITY_URL_LIMIT: usize = 16;
const AIPP_TASK_ACTIVITY_REMOVE_LIMIT: usize = 50;
const AIPP_MEDIA_REMOVE_LIMIT: usize = 500;
const AIPP_PAGE_SIZE: usize = 30;
const AIPP_TASK_ACTIVITY_TASK_ID_MAX: usize = 128;
const AIPP_TASK_ACTIVITY_MATCH_SQL: &str = r#"
             WHERE (:channel IS NULL OR tasks.channel = :channel)
               AND COALESCE(tasks.idempotency_key, '') NOT LIKE 'nni-capability:%'
               AND COALESCE(CASE WHEN json_valid(tasks.payload_json)
                   THEN json_extract(tasks.payload_json, '$._task_source') END, '') != 'distributed_work'
               AND COALESCE(CASE WHEN json_valid(tasks.payload_json)
                   THEN json_extract(tasks.payload_json, '$._task_source') END, '') != 'aipp_account_control'
               AND (
                    :communication_only = 0
                    OR tasks.channel IN ('telegram', 'whatsapp', 'feishu', 'lark', 'wechat')
               )
               AND (:status IS NULL OR tasks.status = :status)
               AND (
                    :needle IS NULL
                    OR INSTR(
                        LOWER(CASE WHEN json_valid(tasks.payload_json)
                            THEN COALESCE(
                                json_extract(tasks.payload_json, '$.text'),
                                json_extract(tasks.payload_json, '$.args.share'),
                                ''
                            )
                            ELSE '' END),
                        LOWER(:needle)
                    ) > 0
                    OR INSTR(
                        LOWER(CASE WHEN json_valid(tasks.result_json)
                            THEN COALESCE(json_extract(tasks.result_json, '$.text'), '')
                            ELSE '' END),
                        LOWER(:needle)
                    ) > 0
               )
               AND (
                    tasks.task_id IN (
                        SELECT task_event_stream.task_id
                          FROM task_event_stream
                         WHERE json_extract(task_event_stream.event_json, '$.event_kind') = 'tool_finished'
                           AND json_extract(task_event_stream.event_json, '$.payload.skill') = :skill_name
                           AND COALESCE(
                               json_extract(task_event_stream.event_json, '$.payload.requested_action_ref'),
                               ''
                           ) != (json_extract(task_event_stream.event_json, '$.payload.skill') || '.capabilities')
                           AND task_event_stream.created_at_ms > :cleared_through_event_ms
                        UNION
                        SELECT task_event_archive.task_id
                          FROM task_event_archive
                         WHERE json_extract(task_event_archive.event_json, '$.event_kind') = 'tool_finished'
                           AND json_extract(task_event_archive.event_json, '$.payload.skill') = :skill_name
                           AND COALESCE(
                               json_extract(task_event_archive.event_json, '$.payload.requested_action_ref'),
                               ''
                           ) != (json_extract(task_event_archive.event_json, '$.payload.skill') || '.capabilities')
                           AND task_event_archive.created_at_ms > :cleared_through_event_ms
                    )
               )
               AND tasks.task_id NOT IN (SELECT value FROM json_each(:hidden_json))
"#;

#[derive(Debug, Deserialize, Default)]
struct AippItemsRemoveRequest {
    #[serde(default)]
    task_ids: Vec<String>,
    #[serde(default)]
    global_sequences: Vec<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct AippMediaQuery {
    limit: Option<usize>,
    before_sequence: Option<u64>,
    cursor_sequence: Option<u64>,
    sort_order: Option<String>,
    kind: Option<String>,
    platform: Option<String>,
    channel: Option<String>,
    status: Option<String>,
    query: Option<String>,
}

#[derive(Debug, Serialize)]
struct AippCatalogItem {
    skill_name: String,
    package_version: String,
    renderer: String,
    data_contract: String,
    icon: String,
    default_locale: String,
    titles: BTreeMap<String, String>,
    descriptions: BTreeMap<String, String>,
    installed: bool,
    entrypoint: Option<String>,
    bridge_capabilities: Vec<String>,
    task_channel_scope: Option<String>,
    account_controls: Option<skill_sdk::AippAccountControls>,
}

struct ActiveAippPackage {
    manifest: skill_sdk::PackageManifest,
    aipp: skill_sdk::AippSpec,
    package_root: PathBuf,
    // Request-owned verification evidence, never an execution grant or shared cache.
    admission_binding: Option<crate::skill_admission::AdmissionExecutionBinding>,
}

fn active_aipp_package(
    state: &AppState,
    skill_name: &str,
) -> Result<Option<ActiveAippPackage>, String> {
    let view = state.get_skill_views_snapshot();
    active_aipp_package_in_view(state, skill_name, &view)
}

fn active_aipp_package_in_view(
    state: &AppState,
    skill_name: &str,
    view: &crate::SkillViewsSnapshot,
) -> Result<Option<ActiveAippPackage>, String> {
    let registry = view
        .registry
        .as_ref()
        .ok_or_else(|| "aipp_registry_unavailable".to_string())?;
    if !view.skills_list.contains(skill_name) || registry.get(skill_name).is_none() {
        return Ok(None);
    }

    let admission_binding = view.binding.admission_bindings.get(skill_name).cloned();
    let (manifest, package_root) = if let Some(binding) = admission_binding.as_ref() {
        let store = skill_sdk::InstallReceiptStore::new(skill_package_root(state));
        let pointer = store
            .current_pointer(skill_name)
            .map_err(|_| "aipp_install_pointer_invalid".to_string())?;
        if pointer.version != binding.version
            || pointer.receipt_digest != binding.install_receipt_digest
        {
            return Err("aipp_generation_binding_mismatch".to_string());
        }
        let verified = store
            .verified_current_metadata(skill_name)
            .map_err(|_| "aipp_install_receipt_invalid".to_string())?;
        if verified.manifest.aipp.is_none() {
            return Ok(None);
        }
        let manifest_digest = verified
            .manifest
            .digest()
            .map_err(|_| "aipp_manifest_digest_failed".to_string())?;
        let receipt_digest = verified
            .receipt
            .digest()
            .map_err(|_| "aipp_receipt_digest_failed".to_string())?;
        if manifest_digest != binding.manifest_digest
            || receipt_digest != binding.install_receipt_digest
            || verified.manifest.package.version != binding.version
        {
            return Err("aipp_generation_binding_mismatch".to_string());
        }
        let package_root = verified.install_dir;
        (verified.manifest, package_root)
    } else {
        let Some(manifest_path) = registry
            .package_manifest_path(skill_name)
            .map(|relative| state.skill_rt.workspace_root.join(relative))
        else {
            return Ok(None);
        };
        let manifest = skill_sdk::PackageManifest::load(&manifest_path)
            .map_err(|_| "aipp_manifest_invalid".to_string())?;
        let package_root = manifest_path
            .parent()
            .ok_or_else(|| "aipp_install_path_invalid".to_string())?
            .to_path_buf();
        (manifest, package_root)
    };
    let Some(aipp) = manifest.aipp.clone() else {
        return Ok(None);
    };
    Ok(Some(ActiveAippPackage {
        manifest,
        aipp,
        package_root,
        admission_binding,
    }))
}

fn aipp_admission_service(
    state: &AppState,
) -> Result<crate::skill_admission::SkillAdmissionService, String> {
    let config = claw_core::config::AppConfig::load(&state.reload_ctx.config_path_for_reload)
        .map_err(|_| "aipp_runtime_config_invalid".to_string())?;
    crate::skill_admission::SkillAdmissionService::from_config(
        &state.skill_rt.workspace_root,
        &config,
    )
    .map_err(|_| "aipp_install_state_unavailable".to_string())
}

fn aipp_is_installed(state: &AppState, skill_name: &str) -> bool {
    aipp_admission_service(state)
        .and_then(|service| {
            service
                .aipp_is_installed(skill_name)
                .map_err(|_| "aipp_install_state_unavailable".to_string())
        })
        .unwrap_or(false)
}

async fn get_aipp_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    tokio::task::spawn_blocking(move || read_aipp_catalog(&state, &headers))
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "aipp_catalog_read_join_failed");
            aipp_api_error(StatusCode::INTERNAL_SERVER_ERROR, "aipp_read_task_failed")
        })
}

fn read_aipp_catalog(
    state: &AppState,
    headers: &HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(state, headers) {
        return response;
    }
    let view = state.get_skill_views_snapshot();
    crate::api_ok(json!({ "schema_version": 1, "apps": project_aipp_catalog(state, &view) }))
}

fn project_aipp_catalog(
    state: &AppState,
    view: &crate::SkillViewsSnapshot,
) -> Vec<AippCatalogItem> {
    let mut apps = Vec::new();
    let admission = aipp_admission_service(state).ok();
    let mut names = view.skills_list.iter().cloned().collect::<Vec<_>>();
    names.sort_unstable();
    for name in names {
        match active_aipp_package_in_view(state, &name, view) {
            Ok(Some(active)) => {
                let installed = admission
                    .as_ref()
                    .and_then(|service| service.aipp_is_installed(&name).ok())
                    .unwrap_or(false);
                let manifest = active.manifest;
                let aipp = active.aipp;
                apps.push(AippCatalogItem {
                    skill_name: name,
                    package_version: manifest.package.version,
                    renderer: aipp.renderer,
                    data_contract: aipp.data_contract,
                    icon: aipp.icon,
                    default_locale: aipp.default_locale,
                    titles: aipp.titles,
                    descriptions: aipp.descriptions,
                    installed,
                    entrypoint: aipp.entrypoint,
                    bridge_capabilities: aipp.bridge_capabilities,
                    account_controls: aipp.account_controls,
                    task_channel_scope: aipp.task_channel_scope,
                })
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(skill = %name, error = %error, "AiPP catalog entry rejected");
            }
        }
    }
    apps
}

async fn set_aipp_install_state(
    state: &AppState,
    skill_name: &str,
    installed: bool,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if !matches!(active_aipp_package(state, skill_name), Ok(Some(_))) {
        return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available");
    }
    let service = match aipp_admission_service(state) {
        Ok(service) => service,
        Err(error) => return aipp_api_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
    };
    match service.set_aipp_installed(skill_name, installed) {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "schema_version": 1,
                    "skill_name": skill_name,
                    "installed": installed,
                })),
                error: None,
            }),
        ),
        Err(_) => aipp_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "aipp_install_state_update_failed",
        ),
    }
}

async fn install_aipp(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill_name): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    set_aipp_install_state(&state, &skill_name, true).await
}

async fn remove_aipp(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill_name): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    set_aipp_install_state(&state, &skill_name, false).await
}

fn aipp_media_item(record: &Value) -> Option<Value> {
    let sequence = record.get("global_sequence")?.as_u64()?;
    let kind = record.get("kind")?.as_str()?;
    if !matches!(kind, "video" | "image") {
        return None;
    }
    let https_url = |key: &str| {
        record.get(key).and_then(Value::as_str).filter(|value| {
            value.len() <= 4096
                && value.starts_with("https://")
                && !value.chars().any(char::is_control)
        })
    };
    let source_url = if kind == "video" {
        https_url("video_page_url")
    } else {
        https_url("source_page_url")
    };
    let image_url = if kind == "image" {
        https_url("image_url")
    } else {
        None
    };
    let preview_available = ["cover_screenshot_path", "image_screenshot_path"]
        .iter()
        .any(|key| {
            record
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        });
    Some(json!({
        "schema_version": 1,
        "global_sequence": sequence,
        "sequence": record.get("sequence").and_then(Value::as_u64),
        "post_sequence": record.get("post_sequence").and_then(Value::as_u64),
        "image_sequence": record.get("image_sequence").and_then(Value::as_u64),
        "kind": kind,
        "platform": bounded_aipp_text(record.get("platform"), 64),
        "source_mode": bounded_aipp_text(record.get("source_mode"), 64),
        "search_keyword": bounded_aipp_text(record.get("search_keyword"), 512),
        "title": bounded_aipp_text(record.get("title"), 512),
        "platform_text": bounded_aipp_text(record.get("platform_text"), 32_768),
        "source_url": source_url,
        "image_url": image_url,
        "preview_available": preview_available,
        "discovered_at": bounded_aipp_optional_text(record.get("discovered_at"), 64),
        "published_at": bounded_aipp_optional_text(record.get("published_at"), 64),
        "publication_text": bounded_aipp_optional_text(record.get("publication_text"), 128),
        "engagement": aipp_engagement(record),
        "account_use": public_account_use(record.get("account_use")),
    }))
}

fn bounded_aipp_text(value: Option<&Value>, max_chars: usize) -> String {
    value
        .and_then(Value::as_str)
        .map(|value| value.chars().take(max_chars).collect())
        .unwrap_or_default()
}

fn bounded_aipp_optional_text(value: Option<&Value>, max_chars: usize) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(|value| value.chars().take(max_chars).collect())
}

fn valid_task_activity_filter(value: Option<&str>, allowed: &[&str]) -> Result<(), String> {
    if value.is_none_or(|value| allowed.contains(&value)) {
        Ok(())
    } else {
        Err("aipp_task_activity_filter_invalid".to_string())
    }
}

fn task_activity_urls(input: &str) -> Vec<String> {
    static URL_CANDIDATE: OnceLock<regex::Regex> = OnceLock::new();
    let pattern = URL_CANDIDATE
        .get_or_init(|| regex::Regex::new(r#"https?://[^\s<>\"']+"#).expect("static URL pattern"));
    let mut urls = BTreeSet::new();
    for candidate in pattern.find_iter(input).map(|value| {
        value.as_str().trim_end_matches([
            '.', ',', ';', ':', '!', '?', ')', ']', '}', '。', '，', '；', '：', '！', '？', '）',
            '】', '》',
        ])
    }) {
        if candidate.len() > 4_096 || candidate.chars().any(char::is_control) {
            continue;
        }
        let Ok(parsed) = reqwest::Url::parse(candidate) else {
            continue;
        };
        if matches!(parsed.scheme(), "http" | "https")
            && parsed.host_str().is_some()
            && parsed.username().is_empty()
            && parsed.password().is_none()
        {
            urls.insert(candidate.to_string());
        }
        if urls.len() >= AIPP_TASK_ACTIVITY_URL_LIMIT {
            break;
        }
    }
    urls.into_iter().collect()
}

fn task_activity_artifacts(task_id: &str, result: &Value) -> Vec<Value> {
    let route_prefix = format!("/v1/tasks/{task_id}/artifacts/");
    result
        .get("artifacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(AIPP_TASK_ACTIVITY_ARTIFACT_LIMIT)
        .filter_map(|artifact| {
            let id = bounded_aipp_optional_text(artifact.get("id"), 256)?;
            let filename = bounded_aipp_optional_text(artifact.get("filename"), 512)?;
            let bounded_route = |key: &str| {
                artifact.get(key).and_then(Value::as_str).filter(|value| {
                    value.len() <= 4_096
                        && value.starts_with(&route_prefix)
                        && !value.chars().any(char::is_control)
                })
            };
            let download_url = bounded_route("download_url")?;
            Some(json!({
                "schema_version": 1,
                "id": id,
                "filename": filename,
                "kind": bounded_aipp_text(artifact.get("kind"), 32),
                "mime_type": bounded_aipp_text(artifact.get("mime_type"), 128),
                "size_bytes": artifact.get("size_bytes").and_then(Value::as_u64),
                "download_url": download_url,
                "preview_url": bounded_route("preview_url"),
            }))
        })
        .collect()
}

fn task_activity_input_text(payload: &Value) -> String {
    let value = payload.get("text").filter(|value| {
        value
            .as_str()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
    });
    let value = value.or_else(|| payload.pointer("/args/share"));
    bounded_aipp_visible_text(value, AIPP_TASK_ACTIVITY_INPUT_MAX_CHARS)
}

fn bounded_aipp_visible_text(value: Option<&Value>, max_chars: usize) -> String {
    let Some(raw) = value.and_then(Value::as_str) else {
        return String::new();
    };
    crate::visible_text::sanitize_user_visible_text(raw)
        .chars()
        .take(max_chars)
        .collect()
}

fn task_activity_item(
    sequence: u64,
    task_id: &str,
    channel: &str,
    status: &str,
    payload_json: &str,
    result_json: Option<&str>,
    error_text: Option<&str>,
    created_at: &str,
    updated_at: &str,
    event_at_ms: u64,
    action_refs: Option<&str>,
) -> Value {
    let payload = serde_json::from_str::<Value>(payload_json).unwrap_or_else(|_| json!({}));
    let result = result_json
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| json!({}));
    let input_text = task_activity_input_text(&payload);
    let mut result_text =
        bounded_aipp_visible_text(result.get("text"), AIPP_TASK_ACTIVITY_RESULT_MAX_CHARS);
    let projected_error = error_text.and_then(|value| {
        let sanitized = crate::visible_text::sanitize_user_visible_text(value);
        let sanitized = sanitized.trim();
        (!sanitized.is_empty()).then(|| {
            sanitized
                .chars()
                .take(AIPP_TASK_ACTIVITY_ERROR_MAX_CHARS)
                .collect::<String>()
        })
    });
    if projected_error.as_deref() == Some(result_text.as_str()) {
        result_text.clear();
    }
    let mut actions = action_refs
        .unwrap_or_default()
        .split(',')
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .map(str::to_string)
        .collect::<Vec<_>>();
    actions.sort_unstable();
    actions.dedup();
    json!({
        "schema_version": 1,
        "sequence": sequence,
        "task_id": task_id,
        "channel": channel,
        "status": status,
        "actions": actions,
        "input_text": input_text,
        "result_text": result_text,
        "error_text": projected_error,
        "source_urls": task_activity_urls(&input_text),
        "artifacts": task_activity_artifacts(task_id, &result),
        "account_use": task_account_use(&result),
        "created_at": created_at,
        "updated_at": updated_at,
        "event_at_ms": event_at_ms,
    })
}

fn valid_aipp_task_id(value: &str) -> bool {
    let len = value.len();
    (8..=AIPP_TASK_ACTIVITY_TASK_ID_MAX).contains(&len)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn normalize_aipp_task_ids(values: &[String]) -> Result<Vec<String>, String> {
    if values.is_empty() {
        return Err("aipp_task_activity_remove_empty".to_string());
    }
    if values.len() > AIPP_TASK_ACTIVITY_REMOVE_LIMIT {
        return Err("aipp_task_activity_remove_limit".to_string());
    }
    let mut task_ids = Vec::with_capacity(values.len());
    for value in values {
        let task_id = value.trim();
        if !valid_aipp_task_id(task_id) {
            return Err("aipp_task_activity_remove_invalid".to_string());
        }
        if !task_ids.iter().any(|existing| existing == task_id) {
            task_ids.push(task_id.to_string());
        }
    }
    Ok(task_ids)
}

fn normalize_aipp_record_sequences(values: &[u64]) -> Result<Vec<u64>, String> {
    if values.is_empty() {
        return Err("aipp_media_remove_empty".to_string());
    }
    if values.len() > AIPP_MEDIA_REMOVE_LIMIT {
        return Err("aipp_media_remove_limit".to_string());
    }
    let mut sequences = Vec::with_capacity(values.len());
    for sequence in values {
        if *sequence == 0 {
            return Err("aipp_media_remove_invalid".to_string());
        }
        if !sequences.contains(sequence) {
            sequences.push(*sequence);
        }
    }
    Ok(sequences)
}

fn aipp_media_record_exists(root: &Path, sequence: u64) -> bool {
    sequence > 0
        && root
            .join("records")
            .join(format!("{sequence:012}.json"))
            .is_file()
}

fn owned_aipp_task_activity_ids(
    db: &rusqlite::Connection,
    skill_name: &str,
    task_ids: &[String],
) -> Result<BTreeSet<String>, String> {
    let ids_json = serde_json::to_string(task_ids)
        .map_err(|_| "aipp_task_activity_query_failed".to_string())?;
    let mut statement = db
        .prepare(
            r#"
            SELECT DISTINCT task_id
              FROM (
                    SELECT task_id
                      FROM task_event_stream
                     WHERE json_extract(event_json, '$.event_kind') = 'tool_finished'
                       AND json_extract(event_json, '$.payload.skill') = :skill_name
                       AND COALESCE(
                           json_extract(event_json, '$.payload.requested_action_ref'),
                           ''
                       ) != (json_extract(event_json, '$.payload.skill') || '.capabilities')
                       AND task_id IN (SELECT value FROM json_each(:ids_json))
                    UNION
                    SELECT task_id
                      FROM task_event_archive
                     WHERE json_extract(event_json, '$.event_kind') = 'tool_finished'
                       AND json_extract(event_json, '$.payload.skill') = :skill_name
                       AND COALESCE(
                           json_extract(event_json, '$.payload.requested_action_ref'),
                           ''
                       ) != (json_extract(event_json, '$.payload.skill') || '.capabilities')
                       AND task_id IN (SELECT value FROM json_each(:ids_json))
              )
            "#,
        )
        .map_err(|_| "aipp_task_activity_query_failed".to_string())?;
    let mut rows = statement
        .query(rusqlite::named_params! {
            ":skill_name": skill_name,
            ":ids_json": ids_json,
        })
        .map_err(|_| "aipp_task_activity_query_failed".to_string())?;
    let mut owned = BTreeSet::new();
    while let Some(row) = rows
        .next()
        .map_err(|_| "aipp_task_activity_query_failed".to_string())?
    {
        if let Ok(task_id) = row.get::<_, String>(0) {
            owned.insert(task_id);
        }
    }
    Ok(owned)
}

fn read_aipp_task_activity_page(
    db: &rusqlite::Connection,
    skill_name: &str,
    task_channel_scope: &str,
    cleared_through_event_ms: u64,
    hidden_task_ids: &[String],
    query: &AippMediaQuery,
) -> Result<Value, String> {
    let communication_only = match task_channel_scope {
        "all" => false,
        "communication" => true,
        _ => return Err("aipp_task_activity_scope_invalid".to_string()),
    };
    let allowed_channels: &[&str] = if communication_only {
        &["telegram", "whatsapp", "feishu", "lark", "wechat"]
    } else {
        &["telegram", "whatsapp", "ui", "feishu", "lark", "wechat"]
    };
    valid_task_activity_filter(query.channel.as_deref(), allowed_channels)?;
    valid_task_activity_filter(
        query.status.as_deref(),
        &[
            "queued",
            "running",
            "succeeded",
            "failed",
            "canceled",
            "timeout",
        ],
    )?;
    let sort_order = aipp_media_sort_order(query)?;
    let limit = query
        .limit
        .unwrap_or(AIPP_PAGE_SIZE)
        .clamp(1, AIPP_TASK_ACTIVITY_LIMIT);
    let needle = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if needle.is_some_and(|value| value.chars().count() > 200) {
        return Err("aipp_task_activity_query_invalid".to_string());
    }
    let hidden_json = serde_json::to_string(hidden_task_ids)
        .map_err(|_| "aipp_task_activity_query_failed".to_string())?;
    // Share narrow identities between the total and page in one database snapshot.
    let sql = aipp_task_activity_page_sql(sort_order == "oldest");
    let mut statement = db
        .prepare(&sql)
        .map_err(|_| "aipp_task_activity_query_failed".to_string())?;
    let requested = (limit + 1) as i64;
    let mut rows = statement
        .query(rusqlite::named_params! {
            ":skill_name": skill_name,
            ":cleared_through_event_ms": cleared_through_event_ms as i64,
            ":communication_only": i64::from(communication_only),
            ":channel": query.channel.as_deref(),
            ":status": query.status.as_deref(),
            ":needle": needle,
            ":hidden_json": hidden_json.as_str(),
            ":cursor": query.cursor_sequence.or(query.before_sequence).map(|value| value as i64),
            ":limit": requested,
        })
        .map_err(|_| "aipp_task_activity_query_failed".to_string())?;
    let mut total_item_count = 0_u64;
    let mut items = Vec::with_capacity(limit + 1);
    while let Some(row) = rows
        .next()
        .map_err(|_| "aipp_task_activity_query_failed".to_string())?
    {
        total_item_count = row
            .get("total_item_count")
            .map_err(|_| "aipp_task_activity_query_failed".to_string())?;
        // A left-joined metadata row preserves the total even for an empty page.
        if matches!(row.get_ref(0), Ok(rusqlite::types::ValueRef::Null)) {
            continue;
        }
        items.push(task_activity_item(
            row.get::<_, u64>(0).unwrap_or_default(),
            &row.get::<_, String>(1).unwrap_or_default(),
            &row.get::<_, String>(2).unwrap_or_default(),
            &row.get::<_, String>(3).unwrap_or_default(),
            &row.get::<_, String>(4).unwrap_or_default(),
            row.get::<_, Option<String>>(5).ok().flatten().as_deref(),
            row.get::<_, Option<String>>(6).ok().flatten().as_deref(),
            &row.get::<_, String>(7).unwrap_or_default(),
            &row.get::<_, String>(8).unwrap_or_default(),
            row.get::<_, u64>(9).unwrap_or_default(),
            row.get::<_, Option<String>>(10).ok().flatten().as_deref(),
        ));
    }
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor_sequence = has_more
        .then(|| items.last()?.get("sequence")?.as_u64())
        .flatten();
    let updated_at_ms = items
        .iter()
        .filter_map(|item| item.get("event_at_ms").and_then(Value::as_u64))
        .max();
    Ok(json!({
        "schema_version": 1,
        "page_item_count": items.len(),
        "total_item_count": total_item_count,
        "items": items,
        "sort_order": sort_order,
        "next_cursor_sequence": next_cursor_sequence,
        "updated_at_ms": updated_at_ms,
    }))
}

fn record_matches_aipp_query(record: &Value, query: &AippMediaQuery) -> bool {
    if query.kind.as_deref().is_some_and(|kind| {
        !matches!(kind, "video" | "image")
            || record.get("kind").and_then(Value::as_str) != Some(kind)
    }) {
        return false;
    }
    if query.platform.as_deref().is_some_and(|platform| {
        platform.len() > 64 || record.get("platform").and_then(Value::as_str) != Some(platform)
    }) {
        return false;
    }
    let Some(needle) = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    if needle.chars().count() > 200 {
        return false;
    }
    let needle = needle.to_lowercase();
    ["title", "platform_text", "search_keyword"]
        .iter()
        .filter_map(|key| record.get(key).and_then(Value::as_str))
        .any(|value| value.to_lowercase().contains(&needle))
}

fn aipp_media_sort_order(query: &AippMediaQuery) -> Result<&'static str, String> {
    match query.sort_order.as_deref().unwrap_or("newest") {
        "newest" => Ok("newest"),
        "oldest" => Ok("oldest"),
        _ => Err("aipp_media_sort_order_invalid".to_string()),
    }
}

#[cfg(test)]
fn read_aipp_media_page(root: &Path, query: &AippMediaQuery) -> Result<Value, String> {
    read_aipp_media_page_with_hidden(root, query, &BTreeSet::new())
}

#[cfg(test)]
fn read_aipp_media_page_with_hidden(
    root: &Path,
    query: &AippMediaQuery,
    hidden_sequences: &BTreeSet<u64>,
) -> Result<Value, String> {
    read_aipp_media_page_with_platforms(root, query, hidden_sequences, &BTreeSet::new())
}

fn declared_media_platforms(input_schema: &Value) -> BTreeSet<String> {
    input_schema
        .pointer("/properties/platform/enum")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|value| {
            !value.is_empty() && value.len() <= 64 && !value.chars().any(char::is_control)
        })
        .take(32)
        .map(str::to_string)
        .collect()
}

fn read_aipp_media_page_with_platforms(
    root: &Path,
    query: &AippMediaQuery,
    hidden_sequences: &BTreeSet<u64>,
    declared_platforms: &BTreeSet<String>,
) -> Result<Value, String> {
    let records_root = root.join("records");
    let entries = match fs::read_dir(&records_root) {
        Ok(entries) => Some(entries),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("aipp_media_storage_read_failed".to_string()),
    };
    let sort_order = aipp_media_sort_order(query)?;
    let limit = query
        .limit
        .unwrap_or(AIPP_PAGE_SIZE)
        .clamp(1, AIPP_MEDIA_RECORD_LIMIT);
    let has_filter = query.kind.is_some()
        || query.platform.is_some()
        || query
            .query
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    let mut candidates = Vec::with_capacity(limit + 1);
    let mut matching_total = 0_usize;
    let mut visible_total = 0_usize;
    let mut scanned_records = 0_usize;
    let mut observed_platforms = declared_platforms.clone();
    let cursor_sequence = query.cursor_sequence.or_else(|| {
        (sort_order == "newest")
            .then_some(query.before_sequence)
            .flatten()
    });
    for entry in entries.into_iter().flatten().filter_map(Result::ok) {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.len() != 17 || !name.ends_with(".json") {
            continue;
        }
        scanned_records = scanned_records.saturating_add(1);
        if scanned_records > AIPP_MEDIA_RECORD_SCAN_LIMIT {
            return Err("aipp_media_record_limit_exceeded".to_string());
        }
        let file_sequence = name
            .strip_suffix(".json")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or_default();
        if file_sequence != 0 && hidden_sequences.contains(&file_sequence) {
            continue;
        }
        visible_total = visible_total.saturating_add(1);
        if !has_filter
            && cursor_sequence.is_some_and(|cursor| {
                if sort_order == "oldest" {
                    file_sequence <= cursor
                } else {
                    file_sequence >= cursor
                }
            })
        {
            continue;
        }
        if entry
            .metadata()
            .map(|metadata| metadata.len() > AIPP_MEDIA_RECORD_MAX_BYTES)
            .unwrap_or(true)
        {
            continue;
        }
        let raw = match fs::read(entry.path()) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        if raw.len() as u64 > AIPP_MEDIA_RECORD_MAX_BYTES {
            continue;
        }
        let record: Value = match serde_json::from_slice(&raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let sequence = record
            .get("global_sequence")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        if sequence == 0 || hidden_sequences.contains(&sequence) {
            continue;
        }
        if let Some(platform) = record
            .get("platform")
            .and_then(Value::as_str)
            .filter(|value| {
                !value.is_empty() && value.len() <= 64 && !value.chars().any(char::is_control)
            })
        {
            observed_platforms.insert(platform.to_string());
        }
        if has_filter && !record_matches_aipp_query(&record, query) {
            continue;
        }
        if has_filter {
            matching_total = matching_total.saturating_add(1);
        }
        if cursor_sequence.is_some_and(|cursor| {
            if sort_order == "oldest" {
                sequence <= cursor
            } else {
                sequence >= cursor
            }
        }) {
            continue;
        }
        if let Some(item) = aipp_media_item(&record) {
            candidates.push((file_sequence, item));
            candidates.sort_unstable_by(|left, right| {
                if sort_order == "oldest" {
                    left.0.cmp(&right.0)
                } else {
                    right.0.cmp(&left.0)
                }
            });
            if candidates.len() > limit + 1 {
                candidates.truncate(limit + 1);
            }
        }
    }
    if !has_filter {
        matching_total = visible_total;
    }
    let mut items = candidates
        .into_iter()
        .map(|(_, item)| item)
        .collect::<Vec<_>>();
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor_sequence = has_more
        .then(|| items.last()?.get("global_sequence")?.as_u64())
        .flatten();
    let next_before_sequence = (sort_order == "newest")
        .then_some(next_cursor_sequence)
        .flatten();
    let state_path = root.join("state.json");
    let state = fs::metadata(&state_path)
        .ok()
        .filter(|metadata| metadata.len() <= AIPP_MEDIA_STATE_MAX_BYTES)
        .and_then(|_| fs::read(state_path).ok())
        .filter(|raw| raw.len() as u64 <= AIPP_MEDIA_STATE_MAX_BYTES)
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .unwrap_or_else(|| json!({}));
    let mut platform_states = state
        .get("platforms")
        .and_then(Value::as_object)
        .map(|platforms| {
            platforms
                .iter()
                .filter(|(name, _)| name.len() <= 64)
                .take(32)
                .map(|(name, value)| {
                    let state = bounded_aipp_text(value.get("state"), 64);
                    (
                        name.clone(),
                        json!({
                            "state": if state.is_empty() { "unknown" } else { &state },
                            "enabled": value.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                            "paused": value.get("paused").and_then(Value::as_bool).unwrap_or(false),
                        }),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    for platform in observed_platforms {
        platform_states.entry(platform).or_insert_with(|| {
            json!({
                "state": "unknown",
                "enabled": false,
                "paused": false,
            })
        });
    }
    let active_run = state
        .get("active_run")
        .filter(|value| !value.is_null())
        .map(|run| {
            let platforms = run
                .get("platforms")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|value| value.len() <= 64)
                .take(16)
                .collect::<Vec<_>>();
            let count = |key: &str| {
                run.pointer(&format!("/counts/{key}"))
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
            };
            json!({
                "run_id": bounded_aipp_optional_text(run.get("run_id"), 128),
                "platforms": platforms,
                "lifecycle_state": bounded_aipp_optional_text(run.get("lifecycle_state"), 64),
                "started_at": bounded_aipp_optional_text(run.get("started_at"), 64),
                "heartbeat_at": bounded_aipp_optional_text(run.get("heartbeat_at"), 64),
                "counts": {
                    "items": count("items"),
                    "videos": count("videos"),
                    "images": count("images"),
                    "duplicates": count("duplicates"),
                    "failures": count("failures"),
                },
            })
        });
    Ok(json!({
        "schema_version": 1,
        "items": items,
        "matching_total": matching_total,
        "sort_order": sort_order,
        "next_cursor_sequence": next_cursor_sequence,
        "next_before_sequence": next_before_sequence,
        "platform_states": platform_states,
        "active_run": active_run,
        "updated_at": bounded_aipp_optional_text(state.get("updated_at"), 64),
    }))
}

async fn get_aipp_items(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill_name): AxumPath<String>,
    Query(query): Query<AippMediaQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    tokio::task::spawn_blocking(move || read_aipp_items(&state, &headers, &skill_name, &query))
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "aipp_items_read_join_failed");
            aipp_api_error(StatusCode::INTERNAL_SERVER_ERROR, "aipp_read_task_failed")
        })
}

fn read_aipp_items(
    state: &AppState,
    headers: &HeaderMap,
    skill_name: &str,
    query: &AippMediaQuery,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(state, headers) {
        return response;
    }
    let active = match active_aipp_package(state, skill_name) {
        Ok(Some(active)) => active,
        _ => return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available"),
    };
    let view = match aipp_admission_service(state).and_then(|service| {
        service
            .aipp_view_state(skill_name)
            .map_err(|_| "aipp_view_state_unavailable".to_string())
    }) {
        Ok(view) if view.installed => view,
        _ => return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available"),
    };
    if active.aipp.data_contract == "skill_task_activity_v1" {
        let task_channel_scope = active
            .aipp
            .task_channel_scope
            .clone()
            .unwrap_or_else(|| "all".to_string());
        let cleared_through_event_ms = view.cleared_through_event_ms;
        let hidden_task_ids = view.hidden_task_ids.into_iter().collect::<Vec<_>>();
        let page = state
            .core
            .db
            .get()
            .map_err(|_| "aipp_task_activity_database_unavailable".to_string())
            .and_then(|db| {
                read_aipp_task_activity_page(
                    &db,
                    skill_name,
                    &task_channel_scope,
                    cleared_through_event_ms,
                    &hidden_task_ids,
                    query,
                )
            });
        return match page {
            Ok(data) => crate::api_ok(data),
            Err(error) => {
                let status = match error.as_str() {
                    "aipp_task_activity_filter_invalid"
                    | "aipp_task_activity_query_invalid"
                    | "aipp_media_sort_order_invalid" => StatusCode::BAD_REQUEST,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                };
                aipp_api_error(status, &error)
            }
        };
    }
    if active.aipp.data_contract != "media_collection_v1" {
        return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available");
    }
    let root = match state.core.skill_storage.resolved_directory_path(skill_name) {
        Ok(path) => path,
        Err(_) => return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_storage_invalid"),
    };
    let hidden_sequences = view.hidden_record_sequences;
    let declared_platforms = active
        .manifest
        .capability_request
        .as_ref()
        .map(|request| declared_media_platforms(&request.input_schema))
        .unwrap_or_default();
    let page =
        read_aipp_media_page_with_platforms(&root, query, &hidden_sequences, &declared_platforms);
    match page {
        Ok(data) => crate::api_ok(data),
        Err(error) => aipp_api_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
    }
}

async fn clear_aipp_items(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill_name): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    let active = match active_aipp_package(&state, &skill_name) {
        Ok(Some(active)) if aipp_is_installed(&state, &skill_name) => active,
        _ => return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available"),
    };
    if active.aipp.data_contract != "skill_task_activity_v1" {
        return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_items_clear_unsupported");
    }
    let cleared_through_event_ms = {
        let db = match state.core.db.get() {
            Ok(db) => db,
            Err(_) => {
                return aipp_api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "aipp_items_clear_database_unavailable",
                );
            }
        };
        match db.query_row(
            "SELECT COALESCE(MAX(created_at_ms), 0)
               FROM (
                    SELECT created_at_ms
                      FROM task_event_stream
                     WHERE json_extract(event_json, '$.event_kind') = 'tool_finished'
                       AND json_extract(event_json, '$.payload.skill') = ?1
                    UNION ALL
                    SELECT created_at_ms
                      FROM task_event_archive
                     WHERE json_extract(event_json, '$.event_kind') = 'tool_finished'
                       AND json_extract(event_json, '$.payload.skill') = ?1
               )",
            rusqlite::params![skill_name],
            |row| row.get::<_, u64>(0),
        ) {
            Ok(sequence) => sequence,
            Err(_) => {
                return aipp_api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "aipp_items_clear_database_unavailable",
                );
            }
        }
    };
    let service = match aipp_admission_service(&state) {
        Ok(service) => service,
        Err(_) => {
            return aipp_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "aipp_view_state_unavailable",
            );
        }
    };
    if service
        .set_aipp_cleared_through_event_ms(&skill_name, cleared_through_event_ms)
        .is_err()
    {
        return aipp_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "aipp_items_clear_state_update_failed",
        );
    }
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "schema_version": 1,
                "skill_name": skill_name,
                "cleared_through_event_ms": cleared_through_event_ms,
                "source_records_preserved": true,
            })),
            error: None,
        }),
    )
}

async fn remove_aipp_items(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill_name): AxumPath<String>,
    Json(payload): Json<AippItemsRemoveRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    let active = match active_aipp_package(&state, &skill_name) {
        Ok(Some(active)) if aipp_is_installed(&state, &skill_name) => active,
        _ => return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available"),
    };
    let has_task_ids = !payload.task_ids.is_empty();
    let has_sequences = !payload.global_sequences.is_empty();
    if has_task_ids == has_sequences {
        return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_items_remove_invalid");
    }
    if active.aipp.data_contract == "media_collection_v1" {
        if has_task_ids {
            return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_items_remove_unsupported");
        }
        return remove_aipp_media_items(&state, &skill_name, &payload.global_sequences);
    }
    if active.aipp.data_contract != "skill_task_activity_v1" {
        return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_items_remove_unsupported");
    }
    if has_sequences {
        return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_items_remove_unsupported");
    }
    let task_ids = match normalize_aipp_task_ids(&payload.task_ids) {
        Ok(task_ids) => task_ids,
        Err(error) => {
            let status = if error == "aipp_task_activity_remove_limit" {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::BAD_REQUEST
            };
            return aipp_api_error(status, &error);
        }
    };
    let owned = {
        let db = match state.core.db.get() {
            Ok(db) => db,
            Err(_) => {
                return aipp_api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "aipp_task_activity_database_unavailable",
                );
            }
        };
        match owned_aipp_task_activity_ids(&db, &skill_name, &task_ids) {
            Ok(owned) => owned,
            Err(_) => {
                return aipp_api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "aipp_task_activity_query_failed",
                );
            }
        }
    };
    let service = match aipp_admission_service(&state) {
        Ok(service) => service,
        Err(_) => {
            return aipp_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "aipp_view_state_unavailable",
            );
        }
    };
    let already_hidden = match service.aipp_hidden_task_ids(&skill_name) {
        Ok(task_ids) => task_ids,
        Err(_) => {
            return aipp_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "aipp_view_state_unavailable",
            );
        }
    };
    if task_ids
        .iter()
        .any(|task_id| !owned.contains(task_id) && !already_hidden.contains(task_id))
    {
        return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_task_activity_remove_unknown");
    }
    let hidden = match service.hide_aipp_task_ids(&skill_name, &task_ids) {
        Ok(hidden) => hidden,
        Err(error) if error.code == "aipp_task_activity_hidden_limit" => {
            return aipp_api_error(StatusCode::PAYLOAD_TOO_LARGE, error.code);
        }
        Err(_) => {
            return aipp_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "aipp_items_remove_state_update_failed",
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "schema_version": 1,
                "removed_count": task_ids.len(),
                "newly_hidden_count": hidden,
                "source_records_preserved": true,
            })),
            error: None,
        }),
    )
}

fn remove_aipp_media_items(
    state: &AppState,
    skill_name: &str,
    requested: &[u64],
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let sequences = match normalize_aipp_record_sequences(requested) {
        Ok(sequences) => sequences,
        Err(error) => {
            let status = if error == "aipp_media_remove_limit" {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::BAD_REQUEST
            };
            return aipp_api_error(status, &error);
        }
    };
    let root = match state.core.skill_storage.resolved_directory_path(skill_name) {
        Ok(path) => path,
        Err(_) => return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_storage_invalid"),
    };
    let service = match aipp_admission_service(state) {
        Ok(service) => service,
        Err(_) => {
            return aipp_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "aipp_view_state_unavailable",
            );
        }
    };
    let already_hidden = match service.aipp_hidden_record_sequences(skill_name) {
        Ok(sequences) => sequences,
        Err(_) => {
            return aipp_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "aipp_view_state_unavailable",
            );
        }
    };
    if sequences.iter().any(|sequence| {
        !aipp_media_record_exists(&root, *sequence) && !already_hidden.contains(sequence)
    }) {
        return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_media_remove_unknown");
    }
    let hidden = match service.hide_aipp_record_sequences(skill_name, &sequences) {
        Ok(hidden) => hidden,
        Err(error) if error.code == "aipp_media_hidden_limit" => {
            return aipp_api_error(StatusCode::PAYLOAD_TOO_LARGE, error.code);
        }
        Err(_) => {
            return aipp_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "aipp_items_remove_state_update_failed",
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "schema_version": 1,
                "removed_count": sequences.len(),
                "newly_hidden_count": hidden,
                "source_records_preserved": true,
            })),
            error: None,
        }),
    )
}

fn aipp_api_error(status: StatusCode, error: &str) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: Some(json!({
                "error_code": error,
                "message_key": format!("clawd.ui.aipp.{error}"),
            })),
            error: Some(error.to_string()),
        }),
    )
}
